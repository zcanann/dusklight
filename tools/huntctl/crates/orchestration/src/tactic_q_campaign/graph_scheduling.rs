use super::*;
use crate::learner::{
    ActionConditionedGraphLearner, ExactGraphTableLearner, GraphActionInput, GraphLearnerContract,
    GraphLearningBatch, GraphNodeInput, GraphReplayPlan,
};
use crate::scheduler::{
    LearnedExpansionPriority, SearchRegime, rank_schedulable_expansions_validated,
    rank_schedulable_nodes_validated,
};
use std::time::Instant;

pub const TACTIC_SCHEDULER_DECISION_SCHEMA_V1: &str = "dusklight-tactic-scheduler-decision/v1";
pub const TACTIC_POLICY_EVALUATION_DECISION_SCHEMA_V1: &str =
    "dusklight-tactic-policy-evaluation-decision/v1";

/// Inclusive subphase timings for the graph-scheduling call. The surrounding
/// route timing also includes horizon filtering, branch restoration, and lease
/// journal publication, so this breakdown is required to be no larger than the
/// parent scheduling-and-leasing phase rather than exactly equal to it.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticGraphSchedulingTiming {
    pub registration_micros: u64,
    #[serde(default)]
    pub graph_validation_micros: u64,
    pub graph_projection_micros: u64,
    pub replay_fit_micros: u64,
    pub exact_action_ranking_micros: u64,
    pub priority_projection_micros: u64,
    pub graph_content_hash_micros: u64,
    pub expansion_ranking_micros: u64,
    pub leasing_and_validation_micros: u64,
}

impl TacticGraphSchedulingTiming {
    pub fn checked_total_micros(self) -> Option<u64> {
        [
            self.registration_micros,
            self.graph_validation_micros,
            self.graph_projection_micros,
            self.replay_fit_micros,
            self.exact_action_ranking_micros,
            self.priority_projection_micros,
            self.graph_content_hash_micros,
            self.expansion_ranking_micros,
            self.leasing_and_validation_micros,
        ]
        .into_iter()
        .try_fold(0_u64, u64::checked_add)
    }

    pub(crate) fn checked_merge(self, other: Self) -> Option<Self> {
        Some(Self {
            registration_micros: self
                .registration_micros
                .checked_add(other.registration_micros)?,
            graph_validation_micros: self
                .graph_validation_micros
                .checked_add(other.graph_validation_micros)?,
            graph_projection_micros: self
                .graph_projection_micros
                .checked_add(other.graph_projection_micros)?,
            replay_fit_micros: self
                .replay_fit_micros
                .checked_add(other.replay_fit_micros)?,
            exact_action_ranking_micros: self
                .exact_action_ranking_micros
                .checked_add(other.exact_action_ranking_micros)?,
            priority_projection_micros: self
                .priority_projection_micros
                .checked_add(other.priority_projection_micros)?,
            graph_content_hash_micros: self
                .graph_content_hash_micros
                .checked_add(other.graph_content_hash_micros)?,
            expansion_ranking_micros: self
                .expansion_ranking_micros
                .checked_add(other.expansion_ranking_micros)?,
            leasing_and_validation_micros: self
                .leasing_and_validation_micros
                .checked_add(other.leasing_and_validation_micros)?,
        })
    }
}

/// Complete integer evidence for one expansion in the state-local action
/// queue. The queue is retained before leasing mutates the graph.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticScheduledExpansionEvidence {
    pub expansion_sha256: Digest,
    pub source_root_ticks: u64,
    pub source_exact_terminal_ticks_to_go: Option<u64>,
    pub generalized_terminal_support_per_million: Option<u32>,
    pub generalized_conditional_ticks_to_go: Option<u64>,
    pub uncertainty_millionths: u64,
    pub prediction_error_millionths: u64,
    pub completed_visits: u64,
    pub policy_rank: Option<u64>,
    pub global_exploration_priority_rank: u64,
    pub source_queue_rank: u64,
}

/// Durable provenance for the exact scheduler choice used by a native tactic
/// decision. This is deliberately integer-valued so queue identity is stable
/// across platforms.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticSchedulerDecisionTrace {
    pub schema: String,
    pub graph_sha256: Digest,
    pub learner_model_sha256: Digest,
    pub generation: u64,
    pub regime: SearchRegime,
    pub queue_sha256: Digest,
    pub decision_sha256: Digest,
    pub ranked_source_queue: Vec<TacticScheduledExpansionEvidence>,
    pub evaluated_expansion_sha256: Vec<Digest>,
    pub final_selected_expansion_sha256: Digest,
}

impl TacticSchedulerDecisionTrace {
    pub fn validate(&self) -> Result<(), TacticQCampaignError> {
        if self.schema != TACTIC_SCHEDULER_DECISION_SCHEMA_V1
            || self.graph_sha256 == Digest::ZERO
            || self.learner_model_sha256 == Digest::ZERO
            || self.ranked_source_queue.is_empty()
            || self.evaluated_expansion_sha256.is_empty()
            || self.final_selected_expansion_sha256 != self.evaluated_expansion_sha256[0]
            || self.queue_sha256
                != tactic_scheduler_queue_sha256(
                    self.graph_sha256,
                    self.learner_model_sha256,
                    self.generation,
                    self.regime,
                    &self.ranked_source_queue,
                )
            || self.decision_sha256
                != tactic_scheduler_decision_sha256(
                    self.queue_sha256,
                    &self.evaluated_expansion_sha256,
                    self.final_selected_expansion_sha256,
                )
        {
            return Err(TacticQCampaignError::InvalidState(
                "tactic scheduler decision trace is invalid",
            ));
        }
        for (rank, candidate) in self.ranked_source_queue.iter().enumerate() {
            if candidate.expansion_sha256 == Digest::ZERO
                || candidate.source_queue_rank != rank as u64
                || candidate
                    .generalized_terminal_support_per_million
                    .is_some_and(|support| support > 1_000_000)
            {
                return Err(TacticQCampaignError::InvalidState(
                    "tactic scheduler source queue is invalid",
                ));
            }
        }
        let queue = self
            .ranked_source_queue
            .iter()
            .map(|candidate| candidate.expansion_sha256)
            .collect::<BTreeSet<_>>();
        if queue.len() != self.ranked_source_queue.len()
            || self.evaluated_expansion_sha256.len() > self.ranked_source_queue.len()
            || self
                .evaluated_expansion_sha256
                .iter()
                .zip(&self.ranked_source_queue)
                .any(|(selected, candidate)| *selected != candidate.expansion_sha256)
        {
            return Err(TacticQCampaignError::InvalidState(
                "tactic scheduler selection is detached from its queue",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TacticExpansionLease {
    pub expansion_sha256: Digest,
    pub lease_sha256: Digest,
    pub descriptor: OptionActionDescriptor,
    pub kind: TacticExpansionLeaseKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TacticExpansionLeaseKind {
    GraphExploration,
    PolicyEvaluation,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LeasedTacticQProposalBatch {
    pub batch: TacticQProposalBatch,
    pub leases: Vec<TacticExpansionLease>,
    pub scheduler_decision: TacticSchedulerDecisionTrace,
    pub timing: TacticGraphSchedulingTiming,
}

/// Durable proof that native workers received the policy-ranked batch itself.
/// Unlike a graph-scheduler decision, this authority may repeat a completed
/// expansion and must never substitute a different untried action.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticPolicyEvaluationDecisionTrace {
    pub schema: String,
    pub learner_model_sha256: Digest,
    pub generation: u64,
    pub proposal_policy: TacticProposalPolicy,
    pub evaluated_expansion_sha256: Vec<Digest>,
    pub selection_reasons: Vec<TacticSelectionReason>,
    pub decision_sha256: Digest,
}

impl TacticPolicyEvaluationDecisionTrace {
    pub fn new(
        learner_model_sha256: Digest,
        generation: u64,
        proposal_policy: TacticProposalPolicy,
        evaluated_expansion_sha256: Vec<Digest>,
        selection_reasons: Vec<TacticSelectionReason>,
    ) -> Result<Self, TacticQCampaignError> {
        let decision_sha256 = tactic_policy_evaluation_decision_sha256(
            learner_model_sha256,
            generation,
            proposal_policy,
            &evaluated_expansion_sha256,
            &selection_reasons,
        );
        let trace = Self {
            schema: TACTIC_POLICY_EVALUATION_DECISION_SCHEMA_V1.into(),
            learner_model_sha256,
            generation,
            proposal_policy,
            evaluated_expansion_sha256,
            selection_reasons,
            decision_sha256,
        };
        trace.validate()?;
        Ok(trace)
    }

    pub fn validate(&self) -> Result<(), TacticQCampaignError> {
        if self.schema != TACTIC_POLICY_EVALUATION_DECISION_SCHEMA_V1
            || self.learner_model_sha256 == Digest::ZERO
            || self.evaluated_expansion_sha256.is_empty()
            || self.evaluated_expansion_sha256.len() != self.selection_reasons.len()
            || self
                .evaluated_expansion_sha256
                .iter()
                .any(|identity| *identity == Digest::ZERO)
            || self
                .evaluated_expansion_sha256
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len()
                != self.evaluated_expansion_sha256.len()
            || self.decision_sha256
                != tactic_policy_evaluation_decision_sha256(
                    self.learner_model_sha256,
                    self.generation,
                    self.proposal_policy,
                    &self.evaluated_expansion_sha256,
                    &self.selection_reasons,
                )
        {
            return Err(TacticQCampaignError::InvalidState(
                "tactic policy-evaluation decision trace is invalid",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EvaluatedTacticQProposalBatch {
    pub batch: TacticQProposalBatch,
    pub leases: Vec<TacticExpansionLease>,
    pub evaluation_decision: TacticPolicyEvaluationDecisionTrace,
    pub timing: TacticGraphSchedulingTiming,
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
        let receipt = graph.validate_prehashed_restored_state(
            &plan,
            &self.current.snapshot,
            self.current.snapshot_sha256,
        )?;
        Ok(TacticRestorationContract { plan, receipt })
    }

    pub fn graph_scheduled_root_and_frontier(
        &self,
        seed: u64,
        generation: u64,
        acquisition_rank: u64,
        maximum_route_frames: usize,
    ) -> Result<[TacticCampaignBranch; 2], TacticQCampaignError> {
        self.graph_scheduled_root_and_frontier_where(
            seed,
            generation,
            acquisition_rank,
            maximum_route_frames,
            |_, _, _| Ok(true),
        )
    }

    /// Select a graph frontier that has at least one action which is either
    /// unregistered or currently schedulable. Node scheduling alone cannot
    /// infer exhaustion because action surfaces are generated from the live
    /// state and zero registered actions denotes a fresh boundary, not a dead
    /// one.
    pub fn graph_scheduled_root_and_frontier_with_action_surface<AE, A>(
        &self,
        seed: u64,
        generation: u64,
        acquisition_rank: u64,
        maximum_route_frames: usize,
        applicable_actions: &A,
    ) -> Result<[TacticCampaignBranch; 2], TacticQCampaignError>
    where
        AE: fmt::Display,
        A: Fn(&FactSnapshot) -> Result<Vec<OptionActionDescriptor>, AE>,
    {
        self.graph_scheduled_root_and_frontier_where(
            seed,
            generation,
            acquisition_rank,
            maximum_route_frames,
            |graph, node_id, state| {
                let applicable = applicable_actions(state)
                    .map_err(|error| TacticQCampaignError::Features(error.to_string()))?;
                let node = graph
                    .node(node_id)
                    .ok_or(TacticQCampaignError::InvalidState(
                        "scheduled graph node disappeared",
                    ))?;
                Ok(applicable.iter().any(|descriptor| {
                    let registered = node
                        .outgoing_expansions
                        .iter()
                        .filter_map(|identity| graph.expansion(*identity))
                        .find(|expansion| expansion.action == *descriptor);
                    match registered {
                        Some(expansion) => {
                            graph.expansion_is_schedulable(expansion.identity_sha256, generation)
                        }
                        None => true,
                    }
                }))
            },
        )
    }

    fn graph_scheduled_root_and_frontier_where<P>(
        &self,
        seed: u64,
        generation: u64,
        acquisition_rank: u64,
        maximum_route_frames: usize,
        mut is_schedulable: P,
    ) -> Result<[TacticCampaignBranch; 2], TacticQCampaignError>
    where
        P: FnMut(
            &crate::state_graph::StateGraph,
            crate::state_graph::ExactStateId,
            &FactSnapshot,
        ) -> Result<bool, TacticQCampaignError>,
    {
        let graph = self
            .state_graph
            .as_ref()
            .ok_or(TacticQCampaignError::InvalidState(
                "node scheduling requires a bound state graph",
            ))?;
        let root = graph_root_branch(graph)?;
        let terminal_supported = graph.best_terminal_path().is_some();
        let (regime, selected_rank) = graph_node_acquisition(terminal_supported, acquisition_rank);
        let ranked = rank_schedulable_nodes_validated(
            self.validated_state_graph()?,
            regime,
            maximum_route_frames as u64,
            seed,
            generation,
        )?;
        let ranked = ranked
            .into_iter()
            .filter_map(|candidate| {
                let node = graph.node(candidate.node)?;
                match is_schedulable(graph, candidate.node, node.state.as_ref()) {
                    Ok(true) => Some(Ok(candidate)),
                    Ok(false) => None,
                    Err(error) => Some(Err(error)),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        if ranked.is_empty() {
            return Ok([root.clone(), root]);
        }
        // Rank zero is the terminal-support lane. Nonzero ranks are a sealed
        // broad-exploration share: after terminal discovery they deliberately
        // use discovery ordering rather than becoming weaker copies of the
        // incumbent-path optimizer.
        let ranked_len = u64::try_from(ranked.len()).map_err(|_| {
            TacticQCampaignError::InvalidState("scheduled graph node count overflows")
        })?;
        let selected_rank = usize::try_from(selected_rank % ranked_len).map_err(|_| {
            TacticQCampaignError::InvalidState("scheduled graph node rank overflows")
        })?;
        let selected = ranked[selected_rank];
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
            goal_reachability_evidence_available: false,
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
            discovery_spatial_novelty: Some(selected.reachability_novelty),
            novelty_rank: selected_rank as u64,
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
            state: node.state.as_ref().clone(),
            route_tape: route.clone(),
            descriptor: None,
        };
        Ok([root, frontier])
    }

    /// Return an exact preferred branch only while it remains executable and
    /// has an action the graph can currently lease. Environments may expose a
    /// bounded restoration-locality hint, but cannot force exhausted work or
    /// bypass graph lease authority.
    pub(crate) fn exact_schedulable_frontier_branch<AE, A>(
        &self,
        target: crate::state_graph::ExactStateId,
        generation: u64,
        maximum_route_frames: usize,
        terminal_support: bool,
        applicable_actions: &A,
    ) -> Result<Option<TacticCampaignBranch>, TacticQCampaignError>
    where
        AE: fmt::Display,
        A: Fn(&FactSnapshot) -> Result<Vec<OptionActionDescriptor>, AE>,
    {
        let graph = self.state_graph()?;
        let Some(node) = graph.node(target).filter(|node| {
            node.id != graph.root() && node.restoration.executable && !node.terminal
        }) else {
            return Ok(None);
        };
        let Some(route) = graph.route(target.route_checkpoint_sha256) else {
            return Ok(None);
        };
        if route.frames.len() > maximum_route_frames {
            return Ok(None);
        }
        let applicable = applicable_actions(node.state.as_ref())
            .map_err(|error| TacticQCampaignError::Features(error.to_string()))?;
        let schedulable = applicable.iter().any(|descriptor| {
            let registered = node
                .outgoing_expansions
                .iter()
                .filter_map(|identity| graph.expansion(*identity))
                .find(|expansion| expansion.action == *descriptor);
            match registered {
                Some(expansion) => {
                    graph.expansion_is_schedulable(expansion.identity_sha256, generation)
                }
                None => true,
            }
        });
        if !schedulable {
            return Ok(None);
        }
        let branch = if terminal_support {
            self.exact_terminal_frontier_branch(target)?
        } else {
            self.exact_frontier_branch(target)?
        };
        Ok(Some(branch))
    }

    /// A locality hint may accelerate rank-zero optimization only when its
    /// exact route lineage has the current best authenticated total cost. This
    /// permits one bounded cache reuse without allowing restore locality to
    /// turn into a competing acquisition policy.
    pub(crate) fn exact_frontier_matches_best_terminal_total(
        &self,
        target: crate::state_graph::ExactStateId,
    ) -> Result<bool, TacticQCampaignError> {
        let validated = self.validated_state_graph()?;
        let graph = validated.graph();
        let Some(best) = graph.best_terminal_path() else {
            return Ok(false);
        };
        let Some(node) = graph.node(target) else {
            return Ok(false);
        };
        let exact_returns = validated.exact_terminal_returns()?;
        Ok(exact_returns.get(&target).is_some_and(|ticks_to_go| {
            node.root_ticks.saturating_add(*ticks_to_go) == best.root_to_terminal_ticks
        }))
    }

    /// Materialize one exact executable graph node without ranking it against
    /// any observed outcome. Paired terminal-return controls use this to honor
    /// the control target selected at the original source boundary, even after
    /// the policy lineage has produced additional evidence.
    pub(crate) fn exact_frontier_branch(
        &self,
        target: crate::state_graph::ExactStateId,
    ) -> Result<TacticCampaignBranch, TacticQCampaignError> {
        let graph = self.state_graph()?;
        let root = graph
            .node(graph.root())
            .ok_or(TacticQCampaignError::InvalidState(
                "state graph root is absent",
            ))?;
        let node = graph
            .node(target)
            .filter(|node| node.id != graph.root() && node.restoration.executable && !node.terminal)
            .ok_or(TacticQCampaignError::InvalidState(
                "exact paired-control frontier is absent, terminal, or not executable",
            ))?;
        let route = graph.route(target.route_checkpoint_sha256).ok_or(
            TacticQCampaignError::InvalidState("exact paired-control route is absent"),
        )?;
        let root_frames = usize::try_from(root.restoration.route.tape_frames)
            .map_err(|_| TacticQCampaignError::InvalidState("root route frames overflow"))?;
        let replayed_prefix_ticks = route.frames.len().checked_sub(root_frames).ok_or(
            TacticQCampaignError::InvalidState(
                "exact paired-control route precedes its native root",
            ),
        )? as u64;
        Ok(TacticCampaignBranch {
            kind: TacticBranchKind::RetainedFrontier,
            logical_frontier: LogicalTacticFrontierRecord {
                identity_sha256: target.route_checkpoint_sha256,
                state_sha256: target.state_sha256,
                route_frames: route.frames.len() as u64,
                replayed_prefix_ticks,
            },
            restorable_native_checkpoint: None,
            acquisition: None,
            state: node.state.as_ref().clone(),
            route_tape: route.clone(),
            descriptor: None,
        })
    }

    pub(crate) fn exact_terminal_frontier_branch(
        &self,
        target: crate::state_graph::ExactStateId,
    ) -> Result<TacticCampaignBranch, TacticQCampaignError> {
        let graph = self.state_graph()?;
        let node = graph
            .node(target)
            .ok_or(TacticQCampaignError::InvalidState(
                "terminal-support locality target is absent",
            ))?;
        let exact_returns = graph.validated()?.exact_terminal_returns()?;
        let ticks_to_go =
            exact_returns
                .get(&target)
                .copied()
                .ok_or(TacticQCampaignError::InvalidState(
                    "terminal-support locality target lacks an exact return",
                ))?;
        let completed_expansions = node
            .outgoing_expansions
            .iter()
            .filter(|identity| {
                graph.expansion(**identity).is_some_and(|expansion| {
                    matches!(
                        expansion.status,
                        crate::state_graph::ActionExpansionStatus::Completed { .. }
                    )
                })
            })
            .count() as u64;
        let mut branch = self.exact_frontier_branch(target)?;
        let replayed_prefix_ticks = branch.logical_frontier.replayed_prefix_ticks;
        branch.acquisition = Some(TacticFrontierAcquisition {
            expansion_count: completed_expansions,
            terminal: false,
            terminal_value_supported: true,
            achieved_goal_value_supported: false,
            goal_reachability_supported: false,
            goal_reachability_evidence_available: false,
            reward: 0.0,
            best_mean_q: None,
            best_goal_progress_per_tick: None,
            predicted_terminal_ticks_to_go: None,
            predicted_total_terminal_ticks: None,
            exact_terminal_ticks_to_go: Some(ticks_to_go),
            exact_total_terminal_ticks: Some(replayed_prefix_ticks.saturating_add(ticks_to_go)),
            maximum_ensemble_variance: None,
            generalized_nearest_distance: None,
            discovery_spatial_novelty: None,
            novelty_rank: 0,
            replayed_prefix_ticks,
        });
        Ok(branch)
    }

    /// Authorize the exact policy-ranked batch for causal evaluation without
    /// passing it through the unique-expansion search scheduler. Completed
    /// actions remain completed and can receive another deterministic native
    /// observation; untried actions are registered but are not graph-leased.
    pub fn authorize_current_policy_evaluation_batch(
        &mut self,
        mut batch: TacticQProposalBatch,
        eligible: &[OptionActionDescriptor],
        maximum_proposals: usize,
        learner_model_sha256: Digest,
        proposal_policy: TacticProposalPolicy,
    ) -> Result<EvaluatedTacticQProposalBatch, TacticQCampaignError> {
        let mut timing_boundary = Instant::now();
        let mut timing = TacticGraphSchedulingTiming::default();
        if maximum_proposals == 0
            || eligible.is_empty()
            || learner_model_sha256 == Digest::ZERO
            || batch.ranking.learner_snapshot_sha256 != self.current.snapshot_sha256
            || batch.proposals.is_empty()
        {
            return Err(TacticQCampaignError::InvalidState(
                "policy evaluation requires an eligible current-state ranking",
            ));
        }
        batch.proposals.truncate(maximum_proposals);
        let eligible_identities = eligible
            .iter()
            .map(OptionActionDescriptor::content_sha256)
            .collect::<Result<BTreeSet<_>, _>>()?;
        let proposal_identities = batch
            .proposals
            .iter()
            .map(|proposal| proposal.descriptor.content_sha256())
            .collect::<Result<Vec<_>, _>>()?;
        if batch
            .proposals
            .iter()
            .zip(&proposal_identities)
            .any(|(proposal, identity)| {
                proposal.learner_snapshot_sha256 != self.current.snapshot_sha256
                    || proposal.decision_index != self.decision_index
                    || !eligible_identities.contains(identity)
            })
            || proposal_identities
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len()
                != batch.proposals.len()
        {
            return Err(TacticQCampaignError::InvalidState(
                "policy evaluation batch is detached or duplicated",
            ));
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
                "policy evaluation requires a bound state graph",
            ))?;
        let source_node = graph
            .node(source)
            .ok_or(TacticQCampaignError::InvalidState(
                "policy evaluation source is absent from the state graph",
            ))?;
        if source_node.state.as_ref() != &self.current.snapshot
            || source_node.terminal
            || !source_node.restoration.executable
            || graph.route(source_route_checkpoint) != Some(&self.route_tape)
        {
            return Err(TacticQCampaignError::InvalidState(
                "policy evaluation source is not executable",
            ));
        }

        let mut leases = Vec::with_capacity(batch.proposals.len());
        for (slot, proposal) in batch.proposals.iter().enumerate() {
            let expansion_sha256 =
                graph.register_action_expansion(source, proposal.descriptor.clone())?;
            leases.push(TacticExpansionLease {
                expansion_sha256,
                lease_sha256: policy_evaluation_lease_sha256(
                    self,
                    learner_model_sha256,
                    expansion_sha256,
                    slot as u64,
                ),
                descriptor: proposal.descriptor.clone(),
                kind: TacticExpansionLeaseKind::PolicyEvaluation,
            });
        }
        timing.registration_micros = scheduling_lap_micros(&mut timing_boundary);
        self.validated_graph_mutation(&graph)?;
        timing.graph_validation_micros = scheduling_lap_micros(&mut timing_boundary);

        let evaluated_expansion_sha256 = leases
            .iter()
            .map(|lease| lease.expansion_sha256)
            .collect::<Vec<_>>();
        let selection_reasons = batch
            .proposals
            .iter()
            .map(|proposal| proposal.reason)
            .collect::<Vec<_>>();
        let evaluation_decision = TacticPolicyEvaluationDecisionTrace::new(
            learner_model_sha256,
            self.decision_index,
            proposal_policy,
            evaluated_expansion_sha256,
            selection_reasons,
        )?;
        timing.leasing_and_validation_micros = scheduling_lap_micros(&mut timing_boundary);
        self.state_graph = Some(graph);
        Ok(EvaluatedTacticQProposalBatch {
            batch,
            leases,
            evaluation_decision,
            timing,
        })
    }

    /// Register every currently eligible action in the authoritative graph,
    /// rank the resulting untried expansions, and lease the exact batch that
    /// may be sent to native workers.
    pub fn lease_current_parameterized_batch(
        &mut self,
        batch: TacticQProposalBatch,
        eligible: &[OptionActionDescriptor],
        maximum_proposals: usize,
        learner_model_sha256: Digest,
    ) -> Result<LeasedTacticQProposalBatch, TacticQCampaignError> {
        let mut timing_boundary = Instant::now();
        let mut timing = TacticGraphSchedulingTiming::default();
        if maximum_proposals == 0
            || eligible.is_empty()
            || learner_model_sha256 == Digest::ZERO
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
        if source_node.state.as_ref() != &self.current.snapshot
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
        timing.registration_micros = scheduling_lap_micros(&mut timing_boundary);
        let validated_graph = self.validated_graph_mutation(&graph)?;
        timing.graph_validation_micros = scheduling_lap_micros(&mut timing_boundary);
        let hashed_graph = validated_graph.hashed()?;
        timing.graph_content_hash_micros = scheduling_lap_micros(&mut timing_boundary);
        let exact_learner = ExactGraphTableLearner;
        let learner_contract = GraphLearnerContract::default();
        let learning_batch = GraphLearningBatch::from_hashed_graph(hashed_graph)?;
        timing.graph_projection_micros = scheduling_lap_micros(&mut timing_boundary);
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
            let replay = GraphReplayPlan::prepare_projected(
                &learner_contract,
                &learning_batch,
                &policy_relevant_actions,
                self.decision_index,
            )?;
            exact_learner.fit_prepared(replay)?
        };
        timing.replay_fit_micros = scheduling_lap_micros(&mut timing_boundary);
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
        timing.exact_action_ranking_micros = scheduling_lap_micros(&mut timing_boundary);
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
        timing.priority_projection_micros = scheduling_lap_micros(&mut timing_boundary);

        let regime = if graph.best_terminal_path().is_some() {
            SearchRegime::Optimization
        } else {
            SearchRegime::Discovery
        };
        // The learner batch was derived from this exact immutable validated
        // graph and already seals its content identity. Re-encoding the whole
        // graph here would produce the same digest a second time.
        let graph_sha256 = learning_batch.graph_sha256;
        let ranked = rank_schedulable_expansions_validated(
            validated_graph,
            regime,
            self.decision_index,
            &priorities,
        )?;
        let source_queue = ranked
            .into_iter()
            .filter(|entry| {
                entry.source == source && descriptors.contains_key(&entry.expansion_sha256)
            })
            .collect::<Vec<_>>();
        let ranked_source_queue = source_queue
            .iter()
            .enumerate()
            .map(|(source_rank, entry)| TacticScheduledExpansionEvidence {
                expansion_sha256: entry.expansion_sha256,
                source_root_ticks: entry.source_root_ticks,
                source_exact_terminal_ticks_to_go: entry.source_exact_terminal_ticks_to_go,
                generalized_terminal_support_per_million: entry
                    .learned
                    .terminal_support_per_million,
                generalized_conditional_ticks_to_go: entry.generalized_conditional_ticks_to_go,
                uncertainty_millionths: entry.uncertainty_millionths,
                prediction_error_millionths: entry.learned.prediction_error_millionths,
                completed_visits: entry.learned.completed_visits,
                policy_rank: entry.learned.policy_rank,
                global_exploration_priority_rank: entry.exploration_priority_rank,
                source_queue_rank: source_rank as u64,
            })
            .collect::<Vec<_>>();
        let selected = source_queue
            .into_iter()
            .take(maximum_proposals)
            .collect::<Vec<_>>();
        timing.expansion_ranking_micros = scheduling_lap_micros(&mut timing_boundary);
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
                kind: TacticExpansionLeaseKind::GraphExploration,
            });
        }
        let evaluated_expansion_sha256 = leases
            .iter()
            .map(|lease| lease.expansion_sha256)
            .collect::<Vec<_>>();
        let queue_sha256 = tactic_scheduler_queue_sha256(
            graph_sha256,
            learner_model_sha256,
            self.decision_index,
            regime,
            &ranked_source_queue,
        );
        let scheduler_decision = TacticSchedulerDecisionTrace {
            schema: TACTIC_SCHEDULER_DECISION_SCHEMA_V1.into(),
            graph_sha256,
            learner_model_sha256,
            generation: self.decision_index,
            regime,
            queue_sha256,
            decision_sha256: tactic_scheduler_decision_sha256(
                queue_sha256,
                &evaluated_expansion_sha256,
                evaluated_expansion_sha256[0],
            ),
            ranked_source_queue,
            final_selected_expansion_sha256: evaluated_expansion_sha256[0],
            evaluated_expansion_sha256,
        };
        scheduler_decision.validate()?;
        // Registration was validated before ranking. Leasing checks the exact
        // expansion identity, current status, lease identity, and expiry while
        // changing only that expansion's lifecycle status, so another complete
        // graph traversal cannot establish any additional invariant here.
        timing.leasing_and_validation_micros = scheduling_lap_micros(&mut timing_boundary);
        self.state_graph = Some(graph);
        Ok(LeasedTacticQProposalBatch {
            batch: TacticQProposalBatch {
                ranking: batch.ranking,
                proposals,
                goal_reachability_estimates: batch.goal_reachability_estimates,
                goal_reachability_calibration: batch.goal_reachability_calibration,
                terminal_action_calibration: batch.terminal_action_calibration,
            },
            leases,
            scheduler_decision,
            timing,
        })
    }
}

fn scheduling_lap_micros(boundary: &mut Instant) -> u64 {
    let elapsed = boundary.elapsed();
    *boundary = Instant::now();
    u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX)
}

fn graph_node_acquisition(terminal_supported: bool, acquisition_rank: u64) -> (SearchRegime, u64) {
    if terminal_supported && acquisition_rank == 0 {
        (SearchRegime::Optimization, 0)
    } else if terminal_supported {
        (SearchRegime::Discovery, acquisition_rank.saturating_sub(1))
    } else {
        (SearchRegime::Discovery, acquisition_rank)
    }
}

fn tactic_scheduler_queue_sha256(
    graph_sha256: Digest,
    learner_model_sha256: Digest,
    generation: u64,
    regime: SearchRegime,
    queue: &[TacticScheduledExpansionEvidence],
) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(TACTIC_SCHEDULER_DECISION_SCHEMA_V1.as_bytes());
    hasher.update(graph_sha256.0);
    hasher.update(learner_model_sha256.0);
    hasher.update(generation.to_le_bytes());
    hasher.update([match regime {
        SearchRegime::Discovery => 0,
        SearchRegime::Optimization => 1,
    }]);
    hasher.update((queue.len() as u64).to_le_bytes());
    for candidate in queue {
        hasher.update(candidate.expansion_sha256.0);
        hasher.update(candidate.source_root_ticks.to_le_bytes());
        hash_optional_u64(&mut hasher, candidate.source_exact_terminal_ticks_to_go);
        match candidate.generalized_terminal_support_per_million {
            Some(value) => {
                hasher.update([1]);
                hasher.update(value.to_le_bytes());
            }
            None => hasher.update([0]),
        }
        hash_optional_u64(&mut hasher, candidate.generalized_conditional_ticks_to_go);
        hasher.update(candidate.uncertainty_millionths.to_le_bytes());
        hasher.update(candidate.prediction_error_millionths.to_le_bytes());
        hasher.update(candidate.completed_visits.to_le_bytes());
        hash_optional_u64(&mut hasher, candidate.policy_rank);
        hasher.update(candidate.global_exploration_priority_rank.to_le_bytes());
        hasher.update(candidate.source_queue_rank.to_le_bytes());
    }
    Digest(hasher.finalize().into())
}

#[cfg(test)]
mod graph_node_acquisition_tests {
    use super::*;

    #[test]
    fn post_terminal_rank_zero_optimizes_and_other_ranks_explore_broadly() {
        assert_eq!(
            graph_node_acquisition(true, 0),
            (SearchRegime::Optimization, 0)
        );
        assert_eq!(
            graph_node_acquisition(true, 1),
            (SearchRegime::Discovery, 0)
        );
        assert_eq!(
            graph_node_acquisition(true, 4),
            (SearchRegime::Discovery, 3)
        );
        assert_eq!(
            graph_node_acquisition(false, 2),
            (SearchRegime::Discovery, 2)
        );
    }
}

fn tactic_scheduler_decision_sha256(
    queue_sha256: Digest,
    evaluated_expansion_sha256: &[Digest],
    final_selected_expansion_sha256: Digest,
) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(TACTIC_SCHEDULER_DECISION_SCHEMA_V1.as_bytes());
    hasher.update(queue_sha256.0);
    hasher.update((evaluated_expansion_sha256.len() as u64).to_le_bytes());
    for identity in evaluated_expansion_sha256 {
        hasher.update(identity.0);
    }
    hasher.update(final_selected_expansion_sha256.0);
    Digest(hasher.finalize().into())
}

fn hash_optional_u64(hasher: &mut Sha256, value: Option<u64>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            hasher.update(value.to_le_bytes());
        }
        None => hasher.update([0]),
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

fn policy_evaluation_lease_sha256(
    campaign: &TacticQCampaign,
    learner_model_sha256: Digest,
    expansion_sha256: Digest,
    slot: u64,
) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight-tactic-policy-evaluation-lease/v1");
    hasher.update(campaign.execution_authority_sha256.0);
    hasher.update(campaign.root_checkpoint_sha256.0);
    hasher.update(campaign.decision_index.to_le_bytes());
    hasher.update(campaign.episode_group.to_le_bytes());
    hasher.update(learner_model_sha256.0);
    hasher.update(expansion_sha256.0);
    hasher.update(slot.to_le_bytes());
    Digest(hasher.finalize().into())
}

fn tactic_policy_evaluation_decision_sha256(
    learner_model_sha256: Digest,
    generation: u64,
    proposal_policy: TacticProposalPolicy,
    evaluated_expansion_sha256: &[Digest],
    selection_reasons: &[TacticSelectionReason],
) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(TACTIC_POLICY_EVALUATION_DECISION_SCHEMA_V1.as_bytes());
    hasher.update(learner_model_sha256.0);
    hasher.update(generation.to_le_bytes());
    hasher.update([proposal_policy_code(proposal_policy)]);
    hasher.update((evaluated_expansion_sha256.len() as u64).to_le_bytes());
    for (expansion, reason) in evaluated_expansion_sha256.iter().zip(selection_reasons) {
        hasher.update(expansion.0);
        hasher.update([selection_reason_code(*reason)]);
    }
    Digest(hasher.finalize().into())
}

fn proposal_policy_code(policy: TacticProposalPolicy) -> u8 {
    match policy {
        TacticProposalPolicy::Learned => 0,
        TacticProposalPolicy::FrozenPolicy => 1,
        TacticProposalPolicy::RandomValid => 2,
        TacticProposalPolicy::StructuredNonLearning => 3,
    }
}

fn selection_reason_code(reason: TacticSelectionReason) -> u8 {
    match reason {
        TacticSelectionReason::Greedy => 0,
        TacticSelectionReason::Epsilon => 1,
        TacticSelectionReason::UnsupportedBootstrap => 2,
        TacticSelectionReason::BatchUncertainty => 3,
        TacticSelectionReason::BatchValue => 4,
        TacticSelectionReason::BatchCoverage => 5,
        TacticSelectionReason::GeneralizedValue => 6,
        TacticSelectionReason::GoalReachability => 7,
        TacticSelectionReason::TerminalCostRefinement => 8,
        TacticSelectionReason::GraphScheduler => 9,
        TacticSelectionReason::RandomBaseline => 10,
        TacticSelectionReason::StructuredBaseline => 11,
        TacticSelectionReason::BatchDiversity => 12,
        TacticSelectionReason::ExactTerminalReturn => 13,
    }
}
