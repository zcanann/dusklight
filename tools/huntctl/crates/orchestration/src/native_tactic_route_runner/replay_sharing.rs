use super::*;

pub(super) type SharedTacticReplayControlPlane = Arc<Mutex<TacticReplayControlPlane>>;

pub(super) fn lock_replay_control_plane(
    replay: &SharedTacticReplayControlPlane,
) -> Result<std::sync::MutexGuard<'_, TacticReplayControlPlane>, NativeTacticRouteRunError> {
    replay
        .lock()
        .map_err(|_| route_message("tactic replay control plane lock is poisoned"))
}

pub(super) struct BoundedStalenessReplaySession {
    replay: SharedTacticReplayControlPlane,
    publisher_lane: u32,
    maximum_stale_replay_revisions: u64,
    consumed_revision: u64,
    telemetry: NativeTacticReplaySharingTelemetry,
}

impl BoundedStalenessReplaySession {
    pub(super) fn new(
        replay: SharedTacticReplayControlPlane,
        publisher_lane: usize,
        maximum_stale_replay_revisions: u64,
        consumed_revision: u64,
    ) -> Result<Self, NativeTacticRouteRunError> {
        Ok(Self {
            replay,
            publisher_lane: u32::try_from(publisher_lane).map_err(route_error)?,
            maximum_stale_replay_revisions,
            consumed_revision,
            telemetry: NativeTacticReplaySharingTelemetry::default(),
        })
    }

    pub(super) fn refresh_if_required(
        &mut self,
        campaign: &mut TacticQCampaign,
    ) -> Result<usize, NativeTacticRouteRunError> {
        let snapshot = {
            let replay = lock_replay_control_plane(&self.replay)?;
            let current_revision = replay.replay_snapshot().revision;
            let observed_staleness = current_revision.saturating_sub(self.consumed_revision);
            self.telemetry.maximum_observed_stale_revisions = self
                .telemetry
                .maximum_observed_stale_revisions
                .max(observed_staleness);
            if !replay_refresh_required(
                self.consumed_revision,
                current_revision,
                self.maximum_stale_replay_revisions,
            ) {
                return Ok(0);
            }
            replay
                .snapshot_from(self.consumed_revision)
                .map_err(route_error)?
        };
        let admitted = campaign
            .import_training_corpora(std::slice::from_ref(&snapshot.corpus))
            .map_err(route_error)?;
        self.consumed_revision = snapshot.version.revision;
        self.telemetry.refreshes = self.telemetry.refreshes.saturating_add(1);
        self.telemetry.imported_rows = self.telemetry.imported_rows.saturating_add(admitted as u64);
        Ok(admitted)
    }

    pub(super) fn publish_evaluated(
        &self,
        publisher_decision: u64,
        learner_snapshot_sha256: Digest,
        evaluated: &[EvaluatedRewardedTacticOutcome],
        episode_groups: &[u64],
    ) -> Result<(), NativeTacticRouteRunError> {
        if evaluated.len() != episode_groups.len() {
            return Err(route_message(
                "published tactic replay batch has detached episode lineages",
            ));
        }
        let mut replay = lock_replay_control_plane(&self.replay)?;
        for (proposal, episode_group) in evaluated.iter().zip(episode_groups) {
            replay
                .publish(
                    self.publisher_lane,
                    publisher_decision,
                    learner_snapshot_sha256,
                    &proposal.transition,
                    &proposal.outcome.route_tape,
                    *episode_group,
                )
                .map_err(route_error)?;
        }
        Ok(())
    }

    pub(super) fn telemetry(&self) -> NativeTacticReplaySharingTelemetry {
        self.telemetry
    }
}

pub(super) fn build_replay_session(
    execution_plan: &NativeTacticExecutionPlan,
    live_replay: Option<SharedTacticReplayControlPlane>,
    lane: &NativeTacticLanePlan,
    inherited_replay_revision: u64,
) -> Result<Option<BoundedStalenessReplaySession>, NativeTacticRouteRunError> {
    match (live_replay, execution_plan.replay_sharing) {
        (
            Some(replay),
            NativeTacticReplaySharingPlan::BoundedStaleness {
                maximum_stale_replay_revisions,
            },
        ) => BoundedStalenessReplaySession::new(
            replay,
            lane.lane_index,
            maximum_stale_replay_revisions,
            inherited_replay_revision,
        )
        .map(Some),
        (None, NativeTacticReplaySharingPlan::GenerationBarrier) => Ok(None),
        _ => Err(route_message(
            "tactic replay sharing session does not match its execution plan",
        )),
    }
}

pub(super) fn lane_generated_training_corpus(
    campaign: &TacticQCampaign,
    lane: &NativeTacticLanePlan,
) -> TacticQTrainingCorpus {
    let all = campaign.training_corpus();
    let mut generated = TacticQTrainingCorpus {
        execution_authority_sha256: all.execution_authority_sha256,
        feature_schema_sha256: all.feature_schema_sha256,
        objective_sha256: all.objective_sha256,
        root_checkpoint_sha256: all.root_checkpoint_sha256,
        transitions: Vec::new(),
        routes: Vec::new(),
        episode_groups: Vec::new(),
    };
    for ((transition, route), episode_group) in all
        .transitions
        .into_iter()
        .zip(all.routes)
        .zip(all.episode_groups)
    {
        if lane.owns_episode_group(episode_group) {
            generated.transitions.push(transition);
            generated.routes.push(route);
            generated.episode_groups.push(episode_group);
        }
    }
    generated
}

fn replay_refresh_required(
    consumed_revision: u64,
    current_revision: u64,
    maximum_stale_replay_revisions: u64,
) -> bool {
    current_revision.saturating_sub(consumed_revision) > maximum_stale_replay_revisions
}

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
    let learner_snapshot = TacticQLearnerSnapshot::from_demonstration(
        &demonstration.corpus,
        route_option_value_config(demonstration.corpus.execution_authority_sha256),
    )
    .map_err(route_error)?;
    let learner_snapshot_sha256 = replay
        .publish_learner_snapshot(&learner_snapshot)
        .map_err(route_error)?;
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
                learner_snapshot_sha256,
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
        // The same exact transition may be evaluated again at a revisited
        // frontier. Replay retains the first admission, so resume repair must
        // bind it to the first learner decision as well.
        let decision = completion.result.trace.iter().find(|decision| {
            decision.before.snapshot_sha256 == transition.before_state_sha256
                && decision.proposal_batch.iter().any(|proposal| {
                    proposal.option_id == transition.value_sample.action.option_id
                        && proposal.emitted_tape_sha256
                            == transition.value_sample.realized_tape_sha256
                        && proposal.after_snapshot_sha256 == transition.after_state_sha256
                        && proposal.terminal == transition.value_sample.terminal
                })
        });
        let Some(decision) = decision else {
            return Err(route_message(
                "generated replay row does not name a learner decision",
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

#[cfg(test)]
mod tests {
    use super::replay_refresh_required;

    #[test]
    fn bounded_staleness_refreshes_after_the_declared_revision_distance() {
        assert!(!replay_refresh_required(10, 10, 0));
        assert!(replay_refresh_required(10, 11, 0));
        assert!(!replay_refresh_required(10, 14, 4));
        assert!(replay_refresh_required(10, 15, 4));
    }
}
