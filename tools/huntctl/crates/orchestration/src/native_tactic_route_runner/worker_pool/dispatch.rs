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
            let local_direct = primary_source.or_else(|| {
                (balance_direct_owner
                    && direct.is_some_and(|frontier| frontier.worker_slot == worker_slot))
                .then_some(direct)
                .flatten()
            });
            let materialize_frontier = requires_frontier_materialization(
                restoration_present,
                replayed_prefix,
                local_direct.is_some(),
            );

            // A process-local source can anchor every proposal assigned to its
            // owning worker. Other workers share one portable materialization
            // each. The selected proposal is appended last, leaving the owner
            // at the retained live endpoint after its local batch completes.
            let checkpoint_source = local_direct.map(|frontier| frontier.source.clone());
            if let Some(dispatch) = dispatches.iter_mut().find(|dispatch| {
                dispatch.worker_slot == worker_slot
                    && dispatch.checkpoint_source == checkpoint_source
                    && dispatch.materialize_frontier == materialize_frontier
            }) {
                dispatch.proposal_indices.push(proposal_index);
                continue;
            }
            dispatches.push(NativeTacticProposalDispatch {
                worker_slot,
                proposal_indices: vec![proposal_index],
                checkpoint_source,
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
