use super::*;

pub(super) fn deterministic_generation_barrier_revision(
    replay: &TacticReplayControlPlane,
    generation: &NativeTacticGenerationPlan,
) -> Result<u64, NativeTacticRouteRunError> {
    let lanes = generation
        .lane_indices
        .iter()
        .map(|lane| u32::try_from(*lane).map_err(route_error))
        .collect::<Result<BTreeSet<_>, _>>()?;
    Ok(replay
        .admissions()
        .into_iter()
        .find(|admission| lanes.contains(&admission.publisher_lane))
        .map_or(replay.replay_snapshot().revision, |admission| {
            admission.sequence
        }))
}

pub(super) fn publish_demonstration_replay(
    replay: &mut TacticReplayControlPlane,
    demonstration: &NativeTacticDemonstration,
) -> Result<(), NativeTacticRouteRunError> {
    for (decision, ((transition, route), episode_group)) in demonstration
        .corpus
        .transitions
        .iter()
        .zip(&demonstration.corpus.routes)
        .zip(&demonstration.corpus.episode_groups)
        .enumerate()
    {
        replay
            .publish(
                u32::MAX,
                decision as u64,
                demonstration.report.corpus_sha256,
                transition,
                route,
                *episode_group,
            )
            .map_err(route_error)?;
    }
    Ok(())
}

pub(super) fn publish_completed_seed_replay(
    replay: &mut TacticReplayControlPlane,
    completion: &CompletedNativeTacticSeed,
) -> Result<(), NativeTacticRouteRunError> {
    for ((transition, route), episode_group) in completion
        .generated_training
        .transitions
        .iter()
        .zip(&completion.generated_training.routes)
        .zip(&completion.generated_training.episode_groups)
    {
        let matching = completion
            .result
            .trace
            .iter()
            .filter(|decision| {
                decision.before.snapshot_sha256 == transition.before_state_sha256
                    && decision.proposal_batch.iter().any(|proposal| {
                        proposal.option_id == transition.value_sample.action.option_id
                            && proposal.emitted_tape_sha256
                                == transition.value_sample.realized_tape_sha256
                            && proposal.after_snapshot_sha256 == transition.after_state_sha256
                            && proposal.terminal == transition.value_sample.terminal
                    })
            })
            .collect::<Vec<_>>();
        let [decision] = matching.as_slice() else {
            return Err(route_message(
                "generated replay row does not name exactly one learner decision",
            ));
        };
        if decision.learner_snapshot_sha256 == Digest::ZERO
            || decision.execution_plan_sha256
                != completion.generated_training.execution_authority_sha256
        {
            return Err(route_message(
                "generated replay learner authority is detached",
            ));
        }
        let outcome = replay
            .publish(
                u32::try_from(decision.lane_index).map_err(route_error)?,
                decision.decision_index,
                decision.learner_snapshot_sha256,
                transition,
                route,
                *episode_group,
            )
            .map_err(route_error)?;
        if matches!(
            outcome,
            TacticReplayAdmissionOutcome::Duplicate {
                transition_identity_sha256,
                ..
            } if transition_identity_sha256 != transition.replay_identity_sha256().map_err(route_error)?
        ) {
            return Err(route_message(
                "replay service deduplicated a different transition",
            ));
        }
    }
    Ok(())
}
