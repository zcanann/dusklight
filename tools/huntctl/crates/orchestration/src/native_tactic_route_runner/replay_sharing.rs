use super::*;

pub(super) type SharedDecisionRoundCoordinator = Arc<DecisionRoundCoordinator>;

pub(super) struct DecisionRoundCoordinator {
    state: Mutex<DecisionRoundState>,
    changed: Condvar,
}

struct DecisionRoundState {
    lane_order: Vec<u32>,
    decisions_per_lane: u64,
    completed: BTreeSet<(u32, u64)>,
    closed_lanes: BTreeSet<u32>,
    round_snapshot: Option<(u64, Arc<TacticQImmutableLearnerSnapshot>)>,
    aborted: bool,
}

impl DecisionRoundCoordinator {
    pub(super) fn new(
        generation: &NativeTacticGenerationPlan,
        execution_plan: &NativeTacticExecutionPlan,
        learner: &SharedTacticLearnerAuthority,
    ) -> Result<SharedDecisionRoundCoordinator, NativeTacticRouteRunError> {
        let lane_order = generation
            .lane_indices
            .iter()
            .map(|lane| u32::try_from(*lane).map_err(route_error))
            .collect::<Result<Vec<_>, _>>()?;
        if lane_order.len() < 2
            || lane_order.iter().copied().collect::<BTreeSet<_>>().len() != lane_order.len()
        {
            return Err(route_message(
                "decision-round coordinator requires distinct concurrent lanes",
            ));
        }
        let completed = lock_learner_authority(learner)?
            .completed_decisions()
            .iter()
            .copied()
            .filter(|(lane, _)| lane_order.contains(lane))
            .collect::<BTreeSet<_>>();
        let decisions_per_lane = execution_plan.budgets.decisions_per_lane;
        let mut missing_seen = false;
        for decision in 0..decisions_per_lane {
            for lane in &lane_order {
                if completed.contains(&(*lane, decision)) {
                    if missing_seen {
                        return Err(route_message(
                            "durable concurrent replay decisions are not a logical prefix",
                        ));
                    }
                } else {
                    missing_seen = true;
                }
            }
        }
        Ok(Arc::new(Self {
            state: Mutex::new(DecisionRoundState {
                lane_order,
                decisions_per_lane,
                completed,
                closed_lanes: BTreeSet::new(),
                round_snapshot: None,
                aborted: false,
            }),
            changed: Condvar::new(),
        }))
    }

    pub(super) fn snapshot_for_decision(
        &self,
        lane: u32,
        decision: u64,
        learner: &SharedTacticLearnerAuthority,
    ) -> Result<(Arc<TacticQImmutableLearnerSnapshot>, u64), NativeTacticRouteRunError> {
        let mut state = self.lock_state()?;
        self.validate_key(&state, lane, decision)?;
        while !state.aborted && !round_ready(&state, decision) {
            state = self
                .changed
                .wait(state)
                .map_err(|_| route_message("decision-round coordinator lock is poisoned"))?;
        }
        if state.aborted {
            return Err(route_cancelled("concurrent tactic decision round aborted"));
        }
        if let Some((snapshot_decision, snapshot)) = &state.round_snapshot {
            if *snapshot_decision != decision {
                return Err(route_message(
                    "decision-round learner snapshot belongs to another round",
                ));
            }
            let replay_revision = lock_learner_authority(learner)?
                .replay()
                .replay_snapshot()
                .revision;
            return Ok((Arc::clone(snapshot), replay_revision));
        }
        let authority = lock_learner_authority(learner)?;
        let snapshot = authority.snapshot();
        let replay_revision = authority.replay().replay_snapshot().revision;
        state.round_snapshot = Some((decision, Arc::clone(&snapshot)));
        Ok((snapshot, replay_revision))
    }

    pub(super) fn await_publication_turn(
        &self,
        lane: u32,
        decision: u64,
    ) -> Result<(), NativeTacticRouteRunError> {
        let mut state = self.lock_state()?;
        self.validate_key(&state, lane, decision)?;
        while !state.aborted && !publication_turn_ready(&state, lane, decision) {
            state = self
                .changed
                .wait(state)
                .map_err(|_| route_message("decision-round coordinator lock is poisoned"))?;
        }
        if state.aborted {
            return Err(route_cancelled("concurrent tactic decision round aborted"));
        }
        if state.completed.contains(&(lane, decision)) {
            return Err(route_message(
                "concurrent tactic decision was already published",
            ));
        }
        Ok(())
    }

    pub(super) fn complete_publication(
        &self,
        lane: u32,
        decision: u64,
    ) -> Result<(), NativeTacticRouteRunError> {
        let mut state = self.lock_state()?;
        self.validate_key(&state, lane, decision)?;
        if !state.completed.insert((lane, decision)) {
            return Err(route_message(
                "concurrent tactic decision publication was duplicated",
            ));
        }
        if round_complete(&state, decision) {
            state.round_snapshot = None;
        }
        self.changed.notify_all();
        Ok(())
    }

    pub(super) fn is_completed(
        &self,
        lane: u32,
        decision: u64,
    ) -> Result<bool, NativeTacticRouteRunError> {
        let state = self.lock_state()?;
        self.validate_key(&state, lane, decision)?;
        Ok(state.completed.contains(&(lane, decision)))
    }

    pub(super) fn close_lane(
        &self,
        lane: u32,
        completed_decisions: u64,
    ) -> Result<(), NativeTacticRouteRunError> {
        let mut state = self.lock_state()?;
        if !state.lane_order.contains(&lane)
            || completed_decisions > state.decisions_per_lane
            || (0..completed_decisions).any(|decision| !state.completed.contains(&(lane, decision)))
        {
            return Err(route_message(
                "completed concurrent tactic lane is detached from replay publication",
            ));
        }
        state.closed_lanes.insert(lane);
        if let Some((decision, _)) = state.round_snapshot
            && round_complete(&state, decision)
        {
            state.round_snapshot = None;
        }
        self.changed.notify_all();
        Ok(())
    }

    pub(super) fn abort(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.aborted = true;
            self.changed.notify_all();
        }
    }

    fn lock_state(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, DecisionRoundState>, NativeTacticRouteRunError> {
        self.state
            .lock()
            .map_err(|_| route_message("decision-round coordinator lock is poisoned"))
    }

    fn validate_key(
        &self,
        state: &DecisionRoundState,
        lane: u32,
        decision: u64,
    ) -> Result<(), NativeTacticRouteRunError> {
        if !state.lane_order.contains(&lane)
            || state.closed_lanes.contains(&lane)
            || decision >= state.decisions_per_lane
        {
            return Err(route_message("invalid concurrent tactic decision key"));
        }
        Ok(())
    }
}

fn round_ready(state: &DecisionRoundState, decision: u64) -> bool {
    decision == 0
        || state.lane_order.iter().all(|lane| {
            state.closed_lanes.contains(lane) || state.completed.contains(&(*lane, decision - 1))
        })
}

fn publication_turn_ready(state: &DecisionRoundState, lane: u32, decision: u64) -> bool {
    if !round_ready(state, decision) {
        return false;
    }
    state
        .lane_order
        .iter()
        .take_while(|candidate| **candidate != lane)
        .all(|candidate| {
            state.closed_lanes.contains(candidate)
                || state.completed.contains(&(*candidate, decision))
        })
}

fn round_complete(state: &DecisionRoundState, decision: u64) -> bool {
    state.lane_order.iter().all(|lane| {
        state.closed_lanes.contains(lane) || state.completed.contains(&(*lane, decision))
    })
}

pub(super) struct BoundedStalenessReplaySession {
    learner: SharedTacticLearnerAuthority,
    lane: NativeTacticLanePlan,
    maximum_stale_replay_revisions: u64,
    consumed_revision: u64,
    telemetry: NativeTacticReplaySharingTelemetry,
    round_coordinator: Option<SharedDecisionRoundCoordinator>,
}

impl BoundedStalenessReplaySession {
    pub(super) fn new(
        learner: SharedTacticLearnerAuthority,
        lane: &NativeTacticLanePlan,
        maximum_stale_replay_revisions: u64,
        consumed_revision: u64,
        round_coordinator: Option<SharedDecisionRoundCoordinator>,
    ) -> Result<Self, NativeTacticRouteRunError> {
        u32::try_from(lane.lane_index).map_err(route_error)?;
        Ok(Self {
            learner,
            lane: lane.clone(),
            maximum_stale_replay_revisions,
            consumed_revision,
            telemetry: NativeTacticReplaySharingTelemetry::default(),
            round_coordinator,
        })
    }

    /// Return a newer immutable learner publication without mutating the lane.
    /// This lets the coordinator evaluate the same native state and legal
    /// action surface immediately before and after consuming the publication.
    pub(super) fn pending_snapshot(
        &mut self,
        decision: u64,
    ) -> Result<Option<Arc<TacticQImmutableLearnerSnapshot>>, NativeTacticRouteRunError> {
        {
            let (snapshot, replay_revision) = if let Some(coordinator) = &self.round_coordinator {
                coordinator.snapshot_for_decision(
                    u32::try_from(self.lane.lane_index).map_err(route_error)?,
                    decision,
                    &self.learner,
                )?
            } else {
                let learner = lock_learner_authority(&self.learner)?;
                (
                    learner.snapshot(),
                    learner.replay().replay_snapshot().revision,
                )
            };
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
            Ok(Some(snapshot))
        }
    }

    pub(super) fn consume_snapshot(
        &mut self,
        campaign: &mut TacticQCampaign,
        snapshot: &Arc<TacticQImmutableLearnerSnapshot>,
    ) -> Result<(), NativeTacticRouteRunError> {
        if snapshot.replay_revision <= self.consumed_revision {
            return Err(route_message(
                "campaign lane learner refresh did not advance replay revision",
            ));
        }
        let admitted = campaign
            .consume_learner_snapshot_with_exploration_filter(&snapshot, |episode_group| {
                self.lane.owns_episode_group(episode_group)
            })
            .map_err(route_error)?;
        self.consumed_revision = snapshot.replay_revision;
        self.telemetry.refreshes = self.telemetry.refreshes.saturating_add(1);
        self.telemetry.imported_rows = self.telemetry.imported_rows.saturating_add(admitted as u64);
        Ok(())
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
        let publisher_lane = u32::try_from(self.lane.lane_index).map_err(route_error)?;
        if let Some(coordinator) = &self.round_coordinator {
            coordinator.await_publication_turn(publisher_lane, publisher_decision)?;
        }
        let mut learner = lock_learner_authority(&self.learner)?;
        let mut admitted_rows = 0_u64;
        let mut duplicate_rows = 0_u64;
        for (proposal, episode_group) in evaluated.iter().zip(episode_groups) {
            match learner
                .publish(
                    publisher_lane,
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
        let result = learner.finish_decision(
            publisher_lane,
            publisher_decision,
            admitted_rows,
            duplicate_rows,
            evaluated.iter().any(|proposal| proposal.outcome.terminal),
            self.maximum_stale_replay_revisions,
        )?;
        drop(learner);
        if let Some(coordinator) = &self.round_coordinator {
            coordinator.complete_publication(publisher_lane, publisher_decision)?;
        }
        Ok(result)
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
        if let Some(coordinator) = &self.round_coordinator {
            for decision in trace {
                let publisher_lane = u32::try_from(decision.lane_index).map_err(route_error)?;
                if coordinator.is_completed(publisher_lane, decision.decision_index)? {
                    continue;
                }
                coordinator.await_publication_turn(publisher_lane, decision.decision_index)?;
                let mut learner = lock_learner_authority(&self.learner)?;
                let (admitted_rows, duplicate_rows) =
                    publish_trace_decision(&mut learner, trace, &generated, decision)?;
                learner.finish_decision(
                    publisher_lane,
                    decision.decision_index,
                    admitted_rows,
                    duplicate_rows,
                    decision.terminal,
                    self.maximum_stale_replay_revisions,
                )?;
                drop(learner);
                coordinator.complete_publication(publisher_lane, decision.decision_index)?;
            }
            return Ok(());
        }
        let mut learner = lock_learner_authority(&self.learner)?;
        publish_trace_replay(&mut learner, trace, &generated)?;
        for decision in trace {
            learner.finish_decision(
                u32::try_from(decision.lane_index).map_err(route_error)?,
                decision.decision_index,
                0,
                0,
                decision.terminal,
                self.maximum_stale_replay_revisions,
            )?;
        }
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
    round_coordinator: Option<SharedDecisionRoundCoordinator>,
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
            round_coordinator,
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
        let decision = trace_decision_for_transition(trace, transition);
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
        if learner.contains_transition(transition)? {
            duplicate_rows = duplicate_rows.saturating_add(1);
            continue;
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

fn publish_trace_decision(
    learner: &mut CampaignTacticLearnerAuthority,
    trace: &[NativeTacticDecisionTrace],
    generated_training: &TacticQTrainingCorpus,
    target: &NativeTacticDecisionTrace,
) -> Result<(u64, u64), NativeTacticRouteRunError> {
    let mut admitted_rows = 0_u64;
    let mut duplicate_rows = 0_u64;
    for ((transition, route), episode_group) in generated_training
        .transitions
        .iter()
        .zip(&generated_training.routes)
        .zip(&generated_training.episode_groups)
    {
        let decision = trace_decision_for_transition(trace, transition).ok_or_else(|| {
            route_message("generated replay row does not name a learner decision")
        })?;
        if decision.lane_index != target.lane_index
            || decision.decision_index != target.decision_index
        {
            continue;
        }
        if decision.learner_snapshot_sha256 == Digest::ZERO
            || decision.execution_plan_sha256 != generated_training.execution_authority_sha256
        {
            return Err(route_message(
                "generated replay learner authority is detached",
            ));
        }
        if learner.contains_transition(transition)? {
            duplicate_rows = duplicate_rows.saturating_add(1);
            continue;
        }
        match learner.publish(
            u32::try_from(decision.lane_index).map_err(route_error)?,
            decision.decision_index,
            decision.learner_snapshot_sha256,
            transition,
            route,
            *episode_group,
        )? {
            TacticReplayAdmissionOutcome::Admitted { .. } => {
                admitted_rows = admitted_rows.saturating_add(1);
            }
            TacticReplayAdmissionOutcome::Duplicate { .. } => {
                duplicate_rows = duplicate_rows.saturating_add(1);
            }
        }
    }
    Ok((admitted_rows, duplicate_rows))
}

fn trace_decision_for_transition<'a>(
    trace: &'a [NativeTacticDecisionTrace],
    transition: &OptionTransitionSample,
) -> Option<&'a NativeTacticDecisionTrace> {
    // The same exact transition may be evaluated again at a revisited
    // frontier. Replay retains the first admission, so resume repair must
    // bind it to the first learner decision as well.
    trace.iter().find(|decision| {
        decision.before.snapshot_sha256 == transition.before_state_sha256
            && decision.proposal_batch.iter().any(|proposal| {
                proposal.option_id == transition.value_sample.action.option_id
                    && proposal.emitted_tape_sha256 == transition.value_sample.realized_tape_sha256
                    && proposal.after_snapshot_sha256 == transition.after_state_sha256
                    && proposal.terminal == transition.value_sample.terminal
            })
    })
}

#[cfg(test)]
mod tests {
    use super::{
        DecisionRoundState, checked_model_replay_lag, publication_turn_ready, replay_refresh_required,
        round_complete, round_ready,
    };
    use std::collections::BTreeSet;

    fn round_state() -> DecisionRoundState {
        DecisionRoundState {
            lane_order: vec![0, 1],
            decisions_per_lane: 2,
            completed: BTreeSet::new(),
            closed_lanes: BTreeSet::new(),
            round_snapshot: None,
            aborted: false,
        }
    }

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

    #[test]
    fn concurrent_decisions_publish_lane_order_then_open_the_next_round() {
        let mut state = round_state();
        assert!(round_ready(&state, 0));
        assert!(publication_turn_ready(&state, 0, 0));
        assert!(!publication_turn_ready(&state, 1, 0));
        assert!(!round_ready(&state, 1));

        state.completed.insert((0, 0));
        assert!(publication_turn_ready(&state, 1, 0));
        assert!(!round_complete(&state, 0));
        state.completed.insert((1, 0));
        assert!(round_complete(&state, 0));
        assert!(round_ready(&state, 1));
        assert!(publication_turn_ready(&state, 0, 1));
        assert!(!publication_turn_ready(&state, 1, 1));
    }

    #[test]
    fn a_closed_short_lane_does_not_deadlock_later_rounds() {
        let mut state = round_state();
        state.completed.insert((0, 0));
        state.completed.insert((1, 0));
        state.closed_lanes.insert(1);
        assert!(round_ready(&state, 1));
        assert!(publication_turn_ready(&state, 0, 1));
        state.completed.insert((0, 1));
        assert!(round_complete(&state, 1));
    }
}
