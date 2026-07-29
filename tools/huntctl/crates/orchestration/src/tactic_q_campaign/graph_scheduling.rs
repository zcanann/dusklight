use super::*;
use crate::scheduler::{LearnedExpansionPriority, SearchRegime, rank_schedulable_expansions};

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

impl TacticQCampaign {
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
        let mut priorities = BTreeMap::new();
        for descriptor in eligible {
            let expansion_sha256 = graph.register_action_expansion(source, descriptor.clone())?;
            descriptors.insert(expansion_sha256, descriptor.clone());
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
                expansion_sha256,
                LearnedExpansionPriority {
                    policy_rank,
                    uncertainty_millionths: ranked
                        .map(|entry| variance_millionths(entry.ensemble_variance))
                        .transpose()?
                        .unwrap_or(u64::MAX),
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
