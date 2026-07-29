use super::*;
use crate::learner::{
    ActionConditionedGraphLearner, ExactGraphTableLearner, GraphActionInput, GraphLearnerContract,
    GraphLearningBatch, GraphNodeInput, GraphReplayPlan,
};
use crate::scheduler::{
    LearnedExpansionPriority, SearchRegime, rank_schedulable_expansions, rank_schedulable_nodes,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TacticExpansionLease {
    pub expansion_sha256: Digest,
    pub lease_sha256: Digest,
    pub descriptor: OptionActionDescriptor,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LeasedTacticQProposalBatch {
    pub batch: TacticQProposalBatch,
    pub leases: Vec<TacticExpansionLease>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TacticRestorationContract {
    pub plan: crate::state_graph::GraphRestorationPlan,
    pub receipt: crate::state_graph::RestoredStateReceipt,
}

impl TacticQCampaign {
    pub fn current_restoration_contract(
        &self,
    ) -> Result<TacticRestorationContract, TacticQCampaignError> {
        let graph = self
            .state_graph
            .as_ref()
            .ok_or(TacticQCampaignError::InvalidState(
                "current restoration requires a bound state graph",
            ))?;
        let route_checkpoint_sha256 =
            route_checkpoint(self.root_checkpoint_sha256, &self.route_tape)?;
        let node = crate::state_graph::ExactStateId {
            route_checkpoint_sha256,
            state_sha256: self.current.snapshot_sha256,
        };
        let plan = graph.restoration_plan(node)?;
        if graph.restoration_route(&plan)? != &self.route_tape {
            return Err(TacticQCampaignError::InvalidState(
                "current restoration route is detached from the graph",
            ));
        }
        let receipt = graph.validate_restored_state(&plan, &self.current.snapshot)?;
        Ok(TacticRestorationContract { plan, receipt })
    }

    pub fn graph_scheduled_root_and_frontier(
        &self,
        seed: u64,
        generation: u64,
        maximum_route_frames: usize,
    ) -> Result<[TacticCampaignBranch; 2], TacticQCampaignError> {
        let graph = self
            .state_graph
            .as_ref()
            .ok_or(TacticQCampaignError::InvalidState(
                "node scheduling requires a bound state graph",
            ))?;
        let root = graph_root_branch(graph)?;
        let regime = if graph.best_terminal_path().is_some() {
            SearchRegime::Optimization
        } else {
            SearchRegime::Discovery
        };
        let Some(selected) =
            rank_schedulable_nodes(graph, regime, maximum_route_frames as u64, seed, generation)?
                .into_iter()
                .next()
        else {
            return Ok([root.clone(), root]);
        };
        let node = graph
            .node(selected.node)
            .ok_or(TacticQCampaignError::InvalidState(
                "scheduled graph node disappeared",
            ))?;
        let route = graph.route(selected.node.route_checkpoint_sha256).ok_or(
            TacticQCampaignError::InvalidState("scheduled graph node route disappeared"),
        )?;
        let acquisition = TacticFrontierAcquisition {
            expansion_count: selected.completed_expansions,
            terminal: false,
            terminal_value_supported: selected.exact_terminal_ticks_to_go.is_some(),
            achieved_goal_value_supported: false,
            goal_reachability_supported: false,
            reward: 0.0,
            best_mean_q: None,
            best_goal_progress_per_tick: None,
            predicted_terminal_ticks_to_go: None,
            predicted_total_terminal_ticks: None,
            exact_terminal_ticks_to_go: selected.exact_terminal_ticks_to_go,
            exact_total_terminal_ticks: selected
                .exact_terminal_ticks_to_go
                .map(|ticks| selected.root_ticks.saturating_add(ticks)),
            maximum_ensemble_variance: None,
            generalized_nearest_distance: None,
            novelty_rank: 0,
            replayed_prefix_ticks: selected.root_ticks,
        };
        let frontier = TacticCampaignBranch {
            kind: TacticBranchKind::RetainedFrontier,
            logical_frontier: LogicalTacticFrontierRecord {
                identity_sha256: selected.node.route_checkpoint_sha256,
                state_sha256: selected.node.state_sha256,
                route_frames: route.frames.len() as u64,
                replayed_prefix_ticks: selected.root_ticks,
            },
            restorable_native_checkpoint: None,
            acquisition: Some(acquisition),
            state: node.state.clone(),
            route_tape: route.clone(),
            descriptor: None,
        };
        Ok([root, frontier])
    }

    /// Register every currently eligible action in the authoritative graph,
    /// rank the resulting untried expansions, and lease the exact batch that
    /// may be sent to native workers.
    pub fn lease_current_parameterized_batch(
        &mut self,
        batch: TacticQProposalBatch,
        eligible: &[OptionActionDescriptor],
        maximum_proposals: usize,
    ) -> Result<LeasedTacticQProposalBatch, TacticQCampaignError> {
        if maximum_proposals == 0
            || eligible.is_empty()
            || batch.ranking.learner_snapshot_sha256 != self.current.snapshot_sha256
        {
            return Err(TacticQCampaignError::InvalidState(
                "graph scheduling requires an eligible current-state ranking",
            ));
        }
        let mut eligible_identities = BTreeSet::new();
        for descriptor in eligible {
            descriptor.validate()?;
            if !batch.ranking.choices.iter().any(|choice| {
                choice.applicable
                    && choice.choice_id == descriptor.option_id
                    && choice.descriptor == *descriptor
            }) || !eligible_identities.insert(descriptor.content_sha256()?)
            {
                return Err(TacticQCampaignError::InvalidState(
                    "graph scheduling eligibility is duplicated or detached",
                ));
            }
        }

        let source_route_checkpoint =
            route_checkpoint(self.root_checkpoint_sha256, &self.route_tape)?;
        let source = crate::state_graph::ExactStateId {
            route_checkpoint_sha256: source_route_checkpoint,
            state_sha256: self.current.snapshot_sha256,
        };
        let mut graph = self
            .state_graph
            .clone()
            .ok_or(TacticQCampaignError::InvalidState(
                "graph scheduling requires a bound state graph",
            ))?;
        let source_node = graph
            .node(source)
            .ok_or(TacticQCampaignError::InvalidState(
                "current campaign boundary is absent from the state graph",
            ))?;
        if source_node.state != self.current.snapshot
            || graph.route(source_route_checkpoint) != Some(&self.route_tape)
        {
            return Err(TacticQCampaignError::InvalidState(
                "current campaign boundary is detached from the state graph",
            ));
        }

        let mut descriptors = BTreeMap::new();
        for descriptor in eligible {
            let expansion_sha256 = graph.register_action_expansion(source, descriptor.clone())?;
            descriptors.insert(expansion_sha256, descriptor.clone());
        }
        let exact_learner = ExactGraphTableLearner;
        let learner_contract = GraphLearnerContract::default();
        let learning_batch = GraphLearningBatch::from_graph(&graph)?;
        let exact_snapshot = if learning_batch.rows.is_empty() {
            exact_learner.fit(&learner_contract, &learning_batch)?
        } else {
            let policy_relevant_actions = descriptors
                .values()
                .map(|descriptor| {
                    descriptor
                        .content_sha256()
                        .map_err(TacticQCampaignError::Values)
                })
                .collect::<Result<BTreeSet<_>, _>>()?;
            let replay = GraphReplayPlan::build(
                &learner_contract,
                &learning_batch,
                &policy_relevant_actions,
                self.decision_index,
            )?;
            exact_learner.fit_prioritized(&learner_contract, &learning_batch, &replay)?
        };
        let graph_visits = graph
            .node(source)
            .map(|node| {
                1_u64
                    .saturating_add(node.incoming_segments.len() as u64)
                    .saturating_add(node.outgoing_expansions.len() as u64)
            })
            .ok_or(TacticQCampaignError::InvalidState(
                "scheduled graph source disappeared",
            ))?;
        let action_inputs = descriptors
            .iter()
            .map(|(expansion_sha256, descriptor)| GraphActionInput {
                expansion_sha256: *expansion_sha256,
                action: descriptor.clone(),
                graph_visits: 0,
            })
            .collect::<Vec<_>>();
        let exact_estimates = exact_learner.rank(
            &exact_snapshot,
            &GraphNodeInput {
                id: source,
                state: self.current.snapshot.clone(),
                graph_visits,
            },
            &action_inputs,
        )?;
        let exact_estimates = action_inputs
            .iter()
            .zip(exact_estimates)
            .map(|(action, estimate)| (action.expansion_sha256, estimate))
            .collect::<BTreeMap<_, _>>();
        let mut priorities = BTreeMap::new();
        for (expansion_sha256, descriptor) in &descriptors {
            let ranked = batch
                .ranking
                .values
                .ranked
                .iter()
                .find(|entry| entry.descriptor == *descriptor);
            let policy_rank = batch
                .proposals
                .iter()
                .position(|proposal| proposal.descriptor == *descriptor)
                .map(|rank| rank as u64);
            priorities.insert(
                *expansion_sha256,
                LearnedExpansionPriority {
                    policy_rank,
                    terminal_support_per_million: exact_estimates
                        .get(expansion_sha256)
                        .and_then(|estimate| estimate.terminal_support_per_million),
                    conditional_ticks_to_go: exact_estimates
                        .get(expansion_sha256)
                        .and_then(|estimate| estimate.conditional_ticks_to_terminal),
                    uncertainty_millionths: if exact_estimates
                        .get(expansion_sha256)
                        .is_some_and(|estimate| estimate.terminal_support_per_million.is_some())
                    {
                        exact_estimates[expansion_sha256].uncertainty_millionths
                    } else {
                        ranked
                            .map(|entry| variance_millionths(entry.ensemble_variance))
                            .transpose()?
                            .unwrap_or(u64::MAX)
                    },
                    prediction_error_millionths: exact_estimates
                        .get(expansion_sha256)
                        .map_or(0, |estimate| estimate.prediction_error_millionths),
                    ..Default::default()
                },
            );
        }

        let regime = if graph.best_terminal_path().is_some() {
            SearchRegime::Optimization
        } else {
            SearchRegime::Discovery
        };
        let ranked = rank_schedulable_expansions(&graph, regime, self.decision_index, &priorities)?;
        let selected = ranked
            .into_iter()
            .filter(|entry| {
                entry.source == source && descriptors.contains_key(&entry.expansion_sha256)
            })
            .take(maximum_proposals)
            .collect::<Vec<_>>();
        if selected.is_empty() {
            return Err(TacticQCampaignError::InvalidState(
                "current graph boundary has no schedulable expansion",
            ));
        }

        let mut proposals = Vec::with_capacity(selected.len());
        let mut leases = Vec::with_capacity(selected.len());
        for (slot, scheduled) in selected.into_iter().enumerate() {
            let descriptor = descriptors.remove(&scheduled.expansion_sha256).ok_or(
                TacticQCampaignError::InvalidState("scheduled expansion descriptor disappeared"),
            )?;
            let lease_sha256 =
                expansion_lease_sha256(self, scheduled.expansion_sha256, slot as u64);
            graph.lease_action_expansion(
                scheduled.expansion_sha256,
                lease_sha256,
                self.decision_index,
                self.decision_index
                    .checked_add(1)
                    .ok_or(TacticQCampaignError::InvalidState(
                        "graph lease generation overflowed",
                    ))?,
            )?;
            let proposal = batch
                .proposals
                .iter()
                .find(|proposal| proposal.descriptor == descriptor)
                .cloned()
                .unwrap_or_else(|| SelectedTactic {
                    schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
                    learner_snapshot_sha256: self.current.snapshot_sha256,
                    decision_index: self.decision_index,
                    descriptor: descriptor.clone(),
                    reason: TacticSelectionReason::GraphScheduler,
                    exploration_draw: 0,
                });
            proposals.push(proposal);
            leases.push(TacticExpansionLease {
                expansion_sha256: scheduled.expansion_sha256,
                lease_sha256,
                descriptor,
            });
        }
        graph.validate()?;
        self.state_graph = Some(graph);
        Ok(LeasedTacticQProposalBatch {
            batch: TacticQProposalBatch {
                ranking: batch.ranking,
                proposals,
            },
            leases,
        })
    }
}

fn variance_millionths(value: f64) -> Result<u64, TacticQCampaignError> {
    if !value.is_finite() || value < 0.0 {
        return Err(TacticQCampaignError::InvalidState(
            "graph scheduling uncertainty is invalid",
        ));
    }
    let scaled = value * 1_000_000.0;
    Ok(if scaled >= u64::MAX as f64 {
        u64::MAX
    } else {
        scaled.round() as u64
    })
}

fn expansion_lease_sha256(
    campaign: &TacticQCampaign,
    expansion_sha256: Digest,
    slot: u64,
) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight-tactic-expansion-lease/v1");
    hasher.update(campaign.execution_authority_sha256.0);
    hasher.update(campaign.root_checkpoint_sha256.0);
    hasher.update(campaign.decision_index.to_le_bytes());
    hasher.update(campaign.episode_group.to_le_bytes());
    hasher.update(expansion_sha256.0);
    hasher.update(slot.to_le_bytes());
    Digest(hasher.finalize().into())
}
