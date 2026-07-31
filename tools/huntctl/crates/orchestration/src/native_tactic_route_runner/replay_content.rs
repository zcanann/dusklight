use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PersistedTacticReplayRow {
    pub(super) transition: StoredContentRef,
}

pub(super) fn retained_replay_components(
    evaluated: &[EvaluatedRewardedTacticOutcome],
    traces: &[NativeTacticProposalTrace],
    catalog: &TacticAssetCatalog,
    goal_distance_before: f32,
) -> Result<Vec<Option<TacticMacroComponent>>, NativeTacticRouteRunError> {
    evaluated
        .iter()
        .zip(traces)
        .map(|(proposal, trace)| {
            if !trace.retained
                && !trace.terminal
                && trace.reward <= 0.0
                && trace.goal_distance_after >= goal_distance_before
            {
                return Ok(None);
            }
            let entry = catalog
                .entry(&proposal.outcome.selected.descriptor.option_id)
                .filter(|entry| entry.description().option == proposal.outcome.selected.descriptor)
                .ok_or_else(|| {
                    route_message("executed proposal is detached from its executable tactic source")
                })?;
            TacticMacroComponent::from_catalog_entry(entry)
                .map(Some)
                .map_err(route_error)
        })
        .collect()
}

pub(super) fn persist_evaluated_replay_content(
    store: &TacticQContentStore,
    evaluated: &[EvaluatedRewardedTacticOutcome],
) -> Result<Vec<PersistedTacticReplayRow>, NativeTacticRouteRunError> {
    evaluated
        .iter()
        .map(|proposal| {
            let transition = store
                .store_option_transition(&proposal.transition, &proposal.outcome.route_tape)
                .map_err(route_error)?;
            store
                .store_tape(&proposal.outcome.route_tape)
                .map_err(route_error)?;
            Ok(PersistedTacticReplayRow { transition })
        })
        .collect()
}
