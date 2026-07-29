use super::*;

pub(super) struct BoundedStalenessReplaySession {
    learner: SharedTacticLearnerAuthority,
    lane: NativeTacticLanePlan,
    maximum_stale_replay_revisions: u64,
    consumed_revision: u64,
    telemetry: NativeTacticReplaySharingTelemetry,
}

impl BoundedStalenessReplaySession {
    pub(super) fn new(
        learner: SharedTacticLearnerAuthority,
        lane: &NativeTacticLanePlan,
        maximum_stale_replay_revisions: u64,
        consumed_revision: u64,
    ) -> Result<Self, NativeTacticRouteRunError> {
        u32::try_from(lane.lane_index).map_err(route_error)?;
        Ok(Self {
            learner,
            lane: lane.clone(),
            maximum_stale_replay_revisions,
            consumed_revision,
            telemetry: NativeTacticReplaySharingTelemetry::default(),
        })
    }

    pub(super) fn refresh_if_required(
        &mut self,
        campaign: &mut TacticQCampaign,
    ) -> Result<Option<Arc<TacticQImmutableLearnerSnapshot>>, NativeTacticRouteRunError> {
        let snapshot = {
            let learner = lock_learner_authority(&self.learner)?;
            let snapshot = learner.snapshot();
            let replay_revision = learner.replay().replay_snapshot().revision;
            let model_replay_lag = checked_model_replay_lag(
                replay_revision,
                snapshot.replay_revision,
                self.maximum_stale_replay_revisions,
            )?;
            self.telemetry.maximum_model_replay_lag_revisions = self
                .telemetry
                .maximum_model_replay_lag_revisions
                .max(model_replay_lag);
            let current_revision = snapshot.replay_revision;
            let observed_staleness = current_revision.saturating_sub(self.consumed_revision);
            self.telemetry.maximum_observed_stale_revisions = self
                .telemetry
                .maximum_observed_stale_revisions
                .max(observed_staleness);
            if !replay_refresh_required(self.consumed_revision, current_revision) {
                return Ok(None);
            }
            snapshot
        };
        let admitted = campaign
            .consume_learner_snapshot_with_exploration_filter(&snapshot, |episode_group| {
                self.lane.owns_episode_group(episode_group)
            })
            .map_err(route_error)?;
        self.consumed_revision = snapshot.replay_revision;
        self.telemetry.refreshes = self.telemetry.refreshes.saturating_add(1);
        self.telemetry.imported_rows = self.telemetry.imported_rows.saturating_add(admitted as u64);
        Ok(Some(snapshot))
    }

    pub(super) fn publish_evaluated(
        &self,
        publisher_decision: u64,
        learner_snapshot_sha256: Digest,
        evaluated: &[EvaluatedRewardedTacticOutcome],
        episode_groups: &[u64],
    ) -> Result<CampaignLearnerPublishResult, NativeTacticRouteRunError> {
        if evaluated.len() != episode_groups.len() {
            return Err(route_message(
                "published tactic replay batch has detached episode lineages",
            ));
        }
        let mut learner = lock_learner_authority(&self.learner)?;
        let mut admitted_rows = 0_u64;
        let mut duplicate_rows = 0_u64;
        for (proposal, episode_group) in evaluated.iter().zip(episode_groups) {
            match learner
                .publish(
                    u32::try_from(self.lane.lane_index).map_err(route_error)?,
                    publisher_decision,
                    learner_snapshot_sha256,
                    &proposal.transition,
                    &proposal.outcome.route_tape,
                    *episode_group,
                )
                .map_err(route_error)?
            {
                TacticReplayAdmissionOutcome::Admitted { .. } => {
                    admitted_rows = admitted_rows.saturating_add(1);
                }
                TacticReplayAdmissionOutcome::Duplicate { .. } => {
                    duplicate_rows = duplicate_rows.saturating_add(1);
                }
            }
        }
        learner.finish_decision(
            admitted_rows,
            duplicate_rows,
            evaluated.iter().any(|proposal| proposal.outcome.terminal),
            self.maximum_stale_replay_revisions,
        )
    }

    pub(super) fn repair_committed(
        &self,
        campaign: &TacticQCampaign,
        trace: &[NativeTacticDecisionTrace],
    ) -> Result<(), NativeTacticRouteRunError> {
        if trace.is_empty() {
            return Ok(());
        }
        let generated = lane_generated_training_corpus(campaign, &self.lane);
        let mut learner = lock_learner_authority(&self.learner)?;
        let (admitted_rows, duplicate_rows) =
            publish_trace_replay(&mut learner, trace, &generated)?;
        learner.finish_decision(
            admitted_rows,
            duplicate_rows,
            trace.iter().any(|decision| decision.terminal),
            self.maximum_stale_replay_revisions,
        )?;
        Ok(())
    }

    pub(super) fn telemetry(&self) -> NativeTacticReplaySharingTelemetry {
        self.telemetry
    }
}

pub(super) fn build_replay_session(
    execution_plan: &NativeTacticExecutionPlan,
    live_learner: Option<SharedTacticLearnerAuthority>,
    lane: &NativeTacticLanePlan,
    inherited_replay_revision: u64,
) -> Result<Option<BoundedStalenessReplaySession>, NativeTacticRouteRunError> {
    match (live_learner, execution_plan.replay_sharing) {
        (
            Some(replay),
            NativeTacticReplaySharingPlan::BoundedStaleness {
                maximum_stale_replay_revisions,
            },
        ) => BoundedStalenessReplaySession::new(
            replay,
            lane,
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

fn replay_refresh_required(consumed_revision: u64, current_revision: u64) -> bool {
    current_revision > consumed_revision
}

fn checked_model_replay_lag(
    replay_revision: u64,
    model_replay_revision: u64,
    maximum_stale_replay_revisions: u64,
) -> Result<u64, NativeTacticRouteRunError> {
    let lag = replay_revision
        .checked_sub(model_replay_revision)
        .ok_or_else(|| route_message("campaign learner model is ahead of durable replay"))?;
    if lag > maximum_stale_replay_revisions {
        return Err(route_message(
            "campaign learner exceeded its sealed replay-staleness bound",
        ));
    }
    Ok(lag)
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
    learner: &mut CampaignTacticLearnerAuthority,
    demonstration: &NativeTacticDemonstration,
) -> Result<(), NativeTacticRouteRunError> {
    let learner_snapshot = TacticQLearnerSnapshot::from_demonstration(
        &demonstration.corpus,
        route_option_value_config(demonstration.corpus.execution_authority_sha256),
        learner.snapshot().manifest.value_treatment,
    )
    .map_err(route_error)?;
    let learner_snapshot_sha256 = learner
        .replay()
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
        learner
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
    learner.force_update()?;
    Ok(())
}

pub(super) fn publish_completed_seed_replay(
    learner: &mut CampaignTacticLearnerAuthority,
    completion: &CompletedNativeTacticSeed,
) -> Result<(), NativeTacticRouteRunError> {
    publish_trace_replay(
        learner,
        &completion.result.trace,
        &completion.generated_training,
    )
    .map(drop)
}

fn publish_trace_replay(
    learner: &mut CampaignTacticLearnerAuthority,
    trace: &[NativeTacticDecisionTrace],
    generated_training: &TacticQTrainingCorpus,
) -> Result<(u64, u64), NativeTacticRouteRunError> {
    let mut admitted_rows = 0_u64;
    let mut duplicate_rows = 0_u64;
    for ((transition, route), episode_group) in generated_training
        .transitions
        .iter()
        .zip(&generated_training.routes)
        .zip(&generated_training.episode_groups)
    {
        // The same exact transition may be evaluated again at a revisited
        // frontier. Replay retains the first admission, so resume repair must
        // bind it to the first learner decision as well.
        let decision = trace.iter().find(|decision| {
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
            || decision.execution_plan_sha256 != generated_training.execution_authority_sha256
        {
            return Err(route_message(
                "generated replay learner authority is detached",
            ));
        }
        let outcome = learner
            .publish(
                u32::try_from(decision.lane_index).map_err(route_error)?,
                decision.decision_index,
                decision.learner_snapshot_sha256,
                transition,
                route,
                *episode_group,
            )
            .map_err(route_error)?;
        match &outcome {
            TacticReplayAdmissionOutcome::Admitted { .. } => {
                admitted_rows = admitted_rows.saturating_add(1);
            }
            TacticReplayAdmissionOutcome::Duplicate { .. } => {
                duplicate_rows = duplicate_rows.saturating_add(1);
            }
        }
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
    Ok((admitted_rows, duplicate_rows))
}

#[cfg(test)]
mod tests {
    use super::{checked_model_replay_lag, replay_refresh_required};

    #[test]
    fn every_newly_fitted_snapshot_is_consumed_immediately() {
        assert!(!replay_refresh_required(10, 10));
        assert!(replay_refresh_required(10, 11));
        assert!(!replay_refresh_required(11, 10));
    }

    #[test]
    fn fitted_model_replay_lag_is_hard_bounded() {
        assert_eq!(checked_model_replay_lag(12, 10, 2).unwrap(), 2);
        assert!(checked_model_replay_lag(13, 10, 2).is_err());
        assert!(checked_model_replay_lag(9, 10, 2).is_err());
    }
}
