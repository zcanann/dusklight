//! Authenticated replay admission for newly promoted tactic macros.

use super::macro_discovery::TacticMacroValidationFrontier;
use super::*;

pub(super) const TACTIC_MACRO_POLICY_EVIDENCE_PUBLISHER_LANE: u32 = u32::MAX - 1;
const TACTIC_MACRO_POLICY_EVIDENCE_DOMAIN: &[u8] = b"dusklight.tactic-macro-policy-evidence/v1\0";

#[derive(Clone)]
pub(super) struct TacticMacroPolicyEvidence {
    pub(super) candidate_sha256: Digest,
    pub(super) frontier_state_sha256: Digest,
    pub(super) transition: OptionTransitionSample,
    pub(super) route: InputTape,
    pub(super) episode_group: u64,
    publisher_decision: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct TacticMacroPolicyEvidencePublication {
    pub(super) admitted_rows: u64,
    pub(super) duplicate_rows: u64,
    pub(super) update: CampaignLearnerUpdateMetrics,
}

pub(super) fn capture_tactic_macro_policy_evidence(
    candidate_sha256: Digest,
    frontier: &TacticMacroValidationFrontier,
    outcome: &NativeTacticWorkerOutcome,
    encoder: &GoalConditionedTacticFeatureEncoder,
    root_checkpoint_sha256: Digest,
    execution_authority_sha256: Digest,
) -> Result<TacticMacroPolicyEvidence, NativeTacticRouteRunError> {
    if candidate_sha256 == Digest::ZERO
        || frontier.state_sha256 == Digest::ZERO
        || execution_authority_sha256 == Digest::ZERO
        || outcome.route_tape.frames.len() < frontier.route_tape.frames.len()
        || outcome.route_tape.frames[..frontier.route_tape.frames.len()]
            != frontier.route_tape.frames
    {
        return Err(route_message(
            "macro policy evidence is detached from its authenticated frontier",
        ));
    }
    let before_features = encoder.encode(&frontier.snapshot).map_err(route_error)?;
    let after_features = encoder.encode(&outcome.next_facts).map_err(route_error)?;
    let reward = route_tactic_reward_spec()
        .evaluate_with_motion(
            encoder.schema_sha256,
            &before_features,
            &after_features,
            outcome.execution.duration.realized_ticks,
            outcome.terminal,
            false,
            outcome
                .next_facts
                .recent_option
                .as_ref()
                .and_then(|option| option.trajectory),
        )
        .map_err(route_error)?;
    let source_checkpoint_sha256 =
        route_checkpoint(root_checkpoint_sha256, &frontier.route_tape).map_err(route_error)?;
    let next_checkpoint_sha256 =
        route_checkpoint(root_checkpoint_sha256, &outcome.route_tape).map_err(route_error)?;
    let mut transition = OptionTransitionSample::capture(
        encoder.schema_sha256,
        source_checkpoint_sha256,
        next_checkpoint_sha256,
        frontier.snapshot.clone(),
        outcome.next_facts.clone(),
        outcome.execution.clone(),
        &outcome.route_tape,
        reward.training_reward,
        outcome.terminal,
        |facts| encoder.encode(facts),
    )
    .map_err(route_error)?;
    transition.execution_authority_sha256 = execution_authority_sha256;
    transition.intermediate_boundaries = outcome.intermediate_boundaries.clone();
    transition.validate().map_err(route_error)?;
    let (episode_group, publisher_decision) = policy_evidence_identity(
        candidate_sha256,
        frontier.state_sha256,
        transition.replay_identity_sha256().map_err(route_error)?,
    );
    Ok(TacticMacroPolicyEvidence {
        candidate_sha256,
        frontier_state_sha256: frontier.state_sha256,
        transition,
        route: outcome.route_tape.clone(),
        episode_group,
        publisher_decision,
    })
}

pub(super) fn publish_tactic_macro_policy_evidence(
    learner: &mut CampaignTacticLearnerAuthority,
    evidence: &[TacticMacroPolicyEvidence],
) -> Result<TacticMacroPolicyEvidencePublication, NativeTacticRouteRunError> {
    if evidence.is_empty() {
        return Ok(TacticMacroPolicyEvidencePublication::default());
    }
    let learner_snapshot_sha256 = learner.snapshot().sha256;
    let mut publication = TacticMacroPolicyEvidencePublication::default();
    let mut identities = BTreeSet::new();
    for row in evidence {
        if row.candidate_sha256 == Digest::ZERO
            || row.frontier_state_sha256 != row.transition.before_state_sha256
            || !row
                .transition
                .value_sample
                .action
                .option_id
                .starts_with("promoted/")
            || !identities.insert((row.candidate_sha256, row.frontier_state_sha256))
        {
            return Err(route_message(
                "macro policy evidence publication contains a detached or duplicate row",
            ));
        }
        match learner.publish(
            TACTIC_MACRO_POLICY_EVIDENCE_PUBLISHER_LANE,
            row.publisher_decision,
            learner_snapshot_sha256,
            &row.transition,
            &row.route,
            row.episode_group,
        )? {
            TacticReplayAdmissionOutcome::Admitted { .. } => {
                publication.admitted_rows = publication.admitted_rows.saturating_add(1);
            }
            TacticReplayAdmissionOutcome::Duplicate { .. } => {
                publication.duplicate_rows = publication.duplicate_rows.saturating_add(1);
            }
        }
    }
    publication.update = learner.force_update()?;
    Ok(publication)
}

fn policy_evidence_identity(
    candidate_sha256: Digest,
    frontier_state_sha256: Digest,
    transition_sha256: Digest,
) -> (u64, u64) {
    let mut hasher = Sha256::new();
    hasher.update(TACTIC_MACRO_POLICY_EVIDENCE_DOMAIN);
    hasher.update(candidate_sha256.0);
    hasher.update(frontier_state_sha256.0);
    hasher.update(transition_sha256.0);
    let digest = hasher.finalize();
    (
        u64::from_le_bytes(digest[..8].try_into().expect("fixed digest prefix")) & (u64::MAX >> 1),
        u64::from_le_bytes(digest[8..16].try_into().expect("fixed digest prefix")),
    )
}
