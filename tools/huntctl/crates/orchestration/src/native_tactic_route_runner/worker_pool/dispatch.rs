use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct NativeTacticProposalDispatch {
    pub(super) worker_slot: usize,
    pub(super) proposal_indices: Vec<usize>,
    pub(super) checkpoint_source: Option<NativeTacticCheckpointSource>,
    pub(super) materialize_frontier: bool,
}

impl NativeTacticProposalPool {
    pub(super) fn proposal_dispatches(
        &self,
        proposal_count: usize,
        direct: Option<&CachedTacticFrontier>,
        restoration_present: bool,
        replayed_prefix: usize,
    ) -> Vec<NativeTacticProposalDispatch> {
        let mut dispatches = Vec::<NativeTacticProposalDispatch>::new();
        let balance_direct_owner = direct.is_some() && self.preferred_owner_slot.is_none();
        let mut planned_proposals = vec![0_usize; self.senders.len()];
        if balance_direct_owner {
            planned_proposals[direct.expect("direct owner is present").worker_slot] = 1;
        }
        for proposal_index in (1..proposal_count).chain(std::iter::once(0)) {
            let primary_source = (proposal_index == 0).then_some(direct).flatten();
            let worker_slot = primary_source.map_or_else(
                || {
                    if proposal_index == 0
                        && let Some(owner) = self.preferred_owner_slot
                    {
                        owner
                    } else if balance_direct_owner {
                        self.next_least_loaded_worker(&planned_proposals)
                    } else {
                        self.next_counterfactual_worker(direct.map(|frontier| frontier.worker_slot))
                    }
                },
                |frontier| frontier.worker_slot,
            );
            if !(balance_direct_owner && proposal_index == 0) {
                planned_proposals[worker_slot] += 1;
            }
            if primary_source.is_some()
                && balance_direct_owner
                && let Some(dispatch) = dispatches.iter_mut().find(|dispatch| {
                    dispatch.worker_slot == worker_slot
                        && dispatch.checkpoint_source.is_none()
                        && dispatch.materialize_frontier
                })
            {
                // A live endpoint is single-use. When its owner also executes
                // siblings, append the selected proposal to the owner's
                // portable batch instead of evicting the live source and then
                // replaying the same frontier again. Keeping it last rearms
                // the owner at the selected endpoint.
                dispatch.proposal_indices.push(proposal_index);
                continue;
            }
            let materialize_frontier = requires_frontier_materialization(
                restoration_present,
                replayed_prefix,
                primary_source.is_some(),
            );

            // A process-local direct source belongs only to the selected
            // proposal. Every other proposal assigned to the same worker can
            // share one portable frontier materialization for this decision.
            if primary_source.is_none()
                && let Some(dispatch) = dispatches.iter_mut().find(|dispatch| {
                    dispatch.worker_slot == worker_slot
                        && dispatch.checkpoint_source.is_none()
                        && dispatch.materialize_frontier == materialize_frontier
                })
            {
                dispatch.proposal_indices.push(proposal_index);
                continue;
            }
            dispatches.push(NativeTacticProposalDispatch {
                worker_slot,
                proposal_indices: vec![proposal_index],
                checkpoint_source: primary_source.map(|frontier| frontier.source.clone()),
                materialize_frontier,
            });
        }
        dispatches
    }

    fn next_least_loaded_worker(&self, planned_proposals: &[usize]) -> usize {
        let minimum = planned_proposals.iter().copied().min().unwrap_or(0);
        let start = self.next_worker.fetch_add(1, Ordering::Relaxed) % planned_proposals.len();
        (0..planned_proposals.len())
            .map(|offset| (start + offset) % planned_proposals.len())
            .find(|worker| planned_proposals[*worker] == minimum)
            .expect("a least-loaded tactic worker is always present")
    }
}
