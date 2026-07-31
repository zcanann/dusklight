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
        for proposal_index in (1..proposal_count).chain(std::iter::once(0)) {
            let primary_source = (proposal_index == 0).then_some(direct).flatten();
            let worker_slot = primary_source.map_or_else(
                || {
                    if proposal_index == 0
                        && let Some(owner) = self.preferred_owner_slot
                    {
                        owner
                    } else {
                        self.next_counterfactual_worker(direct.map(|frontier| frontier.worker_slot))
                    }
                },
                |frontier| frontier.worker_slot,
            );
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
}
