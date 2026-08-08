use super::*;

pub(super) struct PolicyUpdateProbeContext<'a> {
    pub(super) catalog: &'a TacticAssetCatalog,
    pub(super) blueprints: &'a [TacticBlueprint],
    pub(super) action_schema_sha256: Digest,
    pub(super) encoder: &'a GoalConditionedTacticFeatureEncoder,
    pub(super) maximum_proposals: usize,
    pub(super) acquisition_partition: u64,
    pub(super) proposal_policy: TacticProposalPolicy,
    pub(super) force_exploration: bool,
    /// Expansion availability is part of the fixed action surface. Reusing a
    /// repeatable evaluation mask for an exploration preview would falsely
    /// attribute graph-lifecycle changes to the learner publication.
    pub(super) lease_mode: TacticQOnlineLeaseMode,
}

/// Same-state, same-action-surface policy reassessment across one published
/// learner update. Unlike adjacent-decision comparisons, this isolates the
/// learner snapshot as the only input allowed to change.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticPolicyUpdateProbe {
    pub state_sha256: Digest,
    pub before_action_surface_sha256: Digest,
    pub after_action_surface_sha256: Digest,
    pub before_learner_snapshot_sha256: Digest,
    pub after_learner_snapshot_sha256: Digest,
    pub before_replay_rows: u64,
    pub after_replay_rows: u64,
    pub before_model_revision: u64,
    pub after_model_revision: u64,
    pub before_selected_option_id: String,
    pub after_selected_option_id: String,
    pub before_selection_reason: TacticSelectionReason,
    pub after_selection_reason: TacticSelectionReason,
    pub selected_action_changed: bool,
}

impl NativeTacticPolicyUpdateProbe {
    pub fn validate(&self) -> Result<(), NativeTacticRouteRunError> {
        if self.state_sha256 == Digest::ZERO
            || self.before_action_surface_sha256 == Digest::ZERO
            || self.before_action_surface_sha256 != self.after_action_surface_sha256
            || self.before_learner_snapshot_sha256 == Digest::ZERO
            || self.after_learner_snapshot_sha256 == Digest::ZERO
            || self.before_learner_snapshot_sha256 == self.after_learner_snapshot_sha256
            || self.after_replay_rows <= self.before_replay_rows
            || self.after_model_revision <= self.before_model_revision
            || self.before_selected_option_id.is_empty()
            || self.after_selected_option_id.is_empty()
            || self.selected_action_changed
                != (self.before_selected_option_id != self.after_selected_option_id)
        {
            return Err(route_message(
                "native tactic policy-update probe is invalid",
            ));
        }
        Ok(())
    }
}

pub(super) fn build_policy_update_probe(
    state_sha256: Digest,
    before_snapshot: &TacticQImmutableLearnerSnapshot,
    after_snapshot: &TacticQImmutableLearnerSnapshot,
    before: &TacticQProposalBatch,
    after: &TacticQProposalBatch,
) -> Result<NativeTacticPolicyUpdateProbe, NativeTacticRouteRunError> {
    let before_selected = before
        .proposals
        .first()
        .ok_or_else(|| route_message("pre-update policy probe has no selected action"))?;
    let after_selected = after
        .proposals
        .first()
        .ok_or_else(|| route_message("post-update policy probe has no selected action"))?;
    let before_action_surface_sha256 = action_surface_sha256(before)?;
    let after_action_surface_sha256 = action_surface_sha256(after)?;
    let probe = NativeTacticPolicyUpdateProbe {
        state_sha256,
        before_action_surface_sha256,
        after_action_surface_sha256,
        before_learner_snapshot_sha256: before_snapshot.sha256,
        after_learner_snapshot_sha256: after_snapshot.sha256,
        before_replay_rows: before_snapshot.replay_revision,
        after_replay_rows: after_snapshot.replay_revision,
        before_model_revision: before_snapshot.manifest.model_revision,
        after_model_revision: after_snapshot.manifest.model_revision,
        before_selected_option_id: before_selected.descriptor.option_id.clone(),
        after_selected_option_id: after_selected.descriptor.option_id.clone(),
        before_selection_reason: before_selected.reason,
        after_selection_reason: after_selected.reason,
        selected_action_changed: before_selected.descriptor.option_id
            != after_selected.descriptor.option_id,
    };
    probe.validate()?;
    Ok(probe)
}

pub(super) fn consume_policy_update_with_probe(
    session: &mut BoundedStalenessReplaySession,
    campaign: &mut TacticQCampaign,
    before_snapshot: &TacticQImmutableLearnerSnapshot,
    after_snapshot: &Arc<TacticQImmutableLearnerSnapshot>,
    before_batch: Option<&TacticQProposalBatch>,
    context: PolicyUpdateProbeContext<'_>,
) -> Result<NativeTacticPolicyUpdateProbe, NativeTacticRouteRunError> {
    let encode = |facts: &FactSnapshot| context.encoder.encode(facts);
    let generated_before = if before_batch.is_none() {
        Some(
            campaign
                .decide_parameterized_batch_with_policy_and_lease_mode(
                    context.catalog,
                    context.blueprints,
                    context.action_schema_sha256,
                    &encode,
                    context.maximum_proposals,
                    context.acquisition_partition,
                    context.proposal_policy,
                    Some(context.encoder.goal_distance_feature()),
                    context.force_exploration,
                    context.lease_mode,
                )
                .map_err(route_error)?,
        )
    } else {
        None
    };
    let before = before_batch
        .or(generated_before.as_ref())
        .ok_or_else(|| route_message("policy-update probe has no pre-update decision"))?;
    let state_sha256 = campaign.current.snapshot_sha256;
    session.consume_snapshot(campaign, after_snapshot)?;
    let after = campaign
        .decide_parameterized_batch_with_policy_and_lease_mode(
            context.catalog,
            context.blueprints,
            context.action_schema_sha256,
            &encode,
            context.maximum_proposals,
            context.acquisition_partition,
            context.proposal_policy,
            Some(context.encoder.goal_distance_feature()),
            context.force_exploration,
            context.lease_mode,
        )
        .map_err(route_error)?;
    build_policy_update_probe(
        state_sha256,
        before_snapshot,
        after_snapshot,
        before,
        &after,
    )
}

fn action_surface_sha256(
    batch: &TacticQProposalBatch,
) -> Result<Digest, NativeTacticRouteRunError> {
    let bytes = serde_cbor::to_vec(&batch.ranking.choices).map_err(route_error)?;
    Ok(Digest(Sha256::digest(bytes).into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_rejects_a_changed_action_surface() {
        let probe = NativeTacticPolicyUpdateProbe {
            state_sha256: Digest([1; 32]),
            before_action_surface_sha256: Digest([2; 32]),
            after_action_surface_sha256: Digest([3; 32]),
            before_learner_snapshot_sha256: Digest([4; 32]),
            after_learner_snapshot_sha256: Digest([5; 32]),
            before_replay_rows: 1,
            after_replay_rows: 2,
            before_model_revision: 1,
            after_model_revision: 2,
            before_selected_option_id: "before".into(),
            after_selected_option_id: "after".into(),
            before_selection_reason: TacticSelectionReason::Greedy,
            after_selection_reason: TacticSelectionReason::Greedy,
            selected_action_changed: true,
        };
        assert!(probe.validate().is_err());
    }
}
