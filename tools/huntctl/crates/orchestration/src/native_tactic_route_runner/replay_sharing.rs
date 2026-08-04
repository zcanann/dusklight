use super::*;

#[derive(Clone, Copy, Debug)]
struct CrossLaneCursor {
    next_decision: u64,
    active: bool,
}

struct CrossLaneRound {
    decision: u64,
    participants: Vec<usize>,
    begun: BTreeSet<usize>,
    published: BTreeSet<usize>,
    publishing: Option<usize>,
    snapshot: Option<CrossLanePinnedSnapshot>,
}

#[derive(Clone)]
struct CrossLanePinnedSnapshot {
    snapshot: Arc<TacticQImmutableLearnerSnapshot>,
    replay_revision: u64,
}

struct CrossLaneReplayState {
    expected: BTreeSet<usize>,
    lanes: BTreeMap<usize, CrossLaneCursor>,
    startup_completed: BTreeSet<usize>,
    round: Option<CrossLaneRound>,
    aborted: bool,
}

/// Orders cross-lane replay at decision boundaries without serializing native
/// branch execution. Every active lane in a logical round selects from one
/// pinned learner snapshot; committed outcomes publish in sealed lane order.
pub(super) struct CrossLaneReplayCoordinator {
    learner: SharedTacticLearnerAuthority,
    state: Mutex<CrossLaneReplayState>,
    changed: Condvar,
}

impl CrossLaneReplayCoordinator {
    pub(super) fn new(
        learner: SharedTacticLearnerAuthority,
        lane_indices: &[usize],
    ) -> Result<Arc<Self>, NativeTacticRouteRunError> {
        let expected = lane_indices.iter().copied().collect::<BTreeSet<_>>();
        if expected.len() != lane_indices.len() || expected.len() < 2 {
            return Err(route_message(
                "cross-lane replay requires at least two distinct lanes",
            ));
        }
        Ok(Arc::new(Self {
            learner,
            state: Mutex::new(CrossLaneReplayState {
                expected,
                lanes: BTreeMap::new(),
                startup_completed: BTreeSet::new(),
                round: None,
                aborted: false,
            }),
            changed: Condvar::new(),
        }))
    }

    fn lock_state(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, CrossLaneReplayState>, NativeTacticRouteRunError> {
        self.state
            .lock()
            .map_err(|_| route_message("cross-lane replay coordinator lock is poisoned"))
    }

    fn wait<'a>(
        &self,
        state: std::sync::MutexGuard<'a, CrossLaneReplayState>,
    ) -> Result<std::sync::MutexGuard<'a, CrossLaneReplayState>, NativeTacticRouteRunError> {
        self.changed
            .wait(state)
            .map_err(|_| route_message("cross-lane replay coordinator lock is poisoned"))
    }

    fn check_active(state: &CrossLaneReplayState) -> Result<(), NativeTacticRouteRunError> {
        if state.aborted {
            Err(route_message("cross-lane replay coordination was aborted"))
        } else {
            Ok(())
        }
    }

    fn register_locked(
        state: &mut CrossLaneReplayState,
        lane_index: usize,
        next_decision: u64,
        active: bool,
    ) -> Result<(), NativeTacticRouteRunError> {
        if !state.expected.contains(&lane_index) {
            return Err(route_message(
                "cross-lane replay registered a lane outside its generation",
            ));
        }
        match state.lanes.get(&lane_index) {
            Some(cursor) if cursor.next_decision == next_decision && cursor.active == active => {
                Ok(())
            }
            Some(_) => Err(route_message(
                "cross-lane replay lane registered with a different cursor",
            )),
            None => {
                state.lanes.insert(
                    lane_index,
                    CrossLaneCursor {
                        next_decision,
                        active,
                    },
                );
                Ok(())
            }
        }
    }

    pub(super) fn complete_unstarted_lane(
        &self,
        lane_index: usize,
        next_decision: u64,
    ) -> Result<(), NativeTacticRouteRunError> {
        let mut state = self.lock_state()?;
        Self::check_active(&state)?;
        Self::register_locked(&mut state, lane_index, next_decision, false)?;
        state.startup_completed.insert(lane_index);
        self.changed.notify_all();
        Ok(())
    }

    pub(super) fn synchronize_startup<F>(
        &self,
        lane_index: usize,
        next_decision: u64,
        repair: F,
    ) -> Result<(), NativeTacticRouteRunError>
    where
        F: FnOnce() -> Result<(), NativeTacticRouteRunError>,
    {
        let mut state = self.lock_state()?;
        Self::check_active(&state)?;
        Self::register_locked(&mut state, lane_index, next_decision, true)?;
        self.changed.notify_all();
        loop {
            Self::check_active(&state)?;
            if state.lanes.len() == state.expected.len() {
                let next = state.expected.iter().copied().find(|candidate| {
                    state
                        .lanes
                        .get(candidate)
                        .is_some_and(|cursor| cursor.active)
                        && !state.startup_completed.contains(candidate)
                });
                if next == Some(lane_index) {
                    drop(state);
                    if let Err(error) = repair() {
                        self.abort();
                        return Err(error);
                    }
                    state = self.lock_state()?;
                    state.startup_completed.insert(lane_index);
                    self.changed.notify_all();
                    return Ok(());
                }
                if state.startup_completed.contains(&lane_index) {
                    return Ok(());
                }
            }
            state = self.wait(state)?;
        }
    }

    fn begin_decision(
        &self,
        lane_index: usize,
        decision: u64,
    ) -> Result<CrossLanePinnedSnapshot, NativeTacticRouteRunError> {
        let mut state = self.lock_state()?;
        loop {
            Self::check_active(&state)?;
            if state.startup_completed.len() != state.expected.len() {
                state = self.wait(state)?;
                continue;
            }
            let cursor = state
                .lanes
                .get(&lane_index)
                .copied()
                .ok_or_else(|| route_message("cross-lane replay lane is not registered"))?;
            if !cursor.active || cursor.next_decision != decision {
                return Err(route_message(
                    "cross-lane replay decision is detached from its lane cursor",
                ));
            }
            if state.round.is_none() {
                let next_decision = state
                    .lanes
                    .values()
                    .filter(|cursor| cursor.active)
                    .map(|cursor| cursor.next_decision)
                    .min()
                    .ok_or_else(|| route_message("cross-lane replay has no active lanes"))?;
                let participants = state
                    .expected
                    .iter()
                    .copied()
                    .filter(|candidate| {
                        state.lanes.get(candidate).is_some_and(|cursor| {
                            cursor.active && cursor.next_decision == next_decision
                        })
                    })
                    .collect::<Vec<_>>();
                state.round = Some(CrossLaneRound {
                    decision: next_decision,
                    participants,
                    begun: BTreeSet::new(),
                    published: BTreeSet::new(),
                    publishing: None,
                    snapshot: None,
                });
            }
            let round = state
                .round
                .as_mut()
                .ok_or_else(|| route_message("cross-lane replay round is absent"))?;
            if round.decision != decision || !round.participants.contains(&lane_index) {
                state = self.wait(state)?;
                continue;
            }
            round.begun.insert(lane_index);
            if round.snapshot.is_none() {
                let learner = lock_learner_authority(&self.learner)?;
                round.snapshot = Some(CrossLanePinnedSnapshot {
                    snapshot: learner.snapshot(),
                    replay_revision: learner.replay().replay_snapshot().revision,
                });
            }
            self.changed.notify_all();
            if round.begun.len() == round.participants.len() {
                return round
                    .snapshot
                    .as_ref()
                    .cloned()
                    .ok_or_else(|| route_message("cross-lane replay snapshot is absent"));
            }
            state = self.wait(state)?;
        }
    }

    pub(super) fn publish_in_order<F>(
        &self,
        lane_index: usize,
        decision: u64,
        publish: F,
    ) -> Result<CampaignLearnerPublishResult, NativeTacticRouteRunError>
    where
        F: FnOnce() -> Result<CampaignLearnerPublishResult, NativeTacticRouteRunError>,
    {
        let mut state = self.lock_state()?;
        loop {
            Self::check_active(&state)?;
            let round = state
                .round
                .as_ref()
                .ok_or_else(|| route_message("cross-lane replay publication has no round"))?;
            if round.decision != decision
                || !round.participants.contains(&lane_index)
                || round.begun.len() != round.participants.len()
            {
                return Err(route_message(
                    "cross-lane replay publication is detached from its decision round",
                ));
            }
            let next_publisher = round
                .participants
                .iter()
                .copied()
                .find(|candidate| !round.published.contains(candidate));
            if next_publisher == Some(lane_index) && round.publishing.is_none() {
                state
                    .round
                    .as_mut()
                    .expect("checked replay round")
                    .publishing = Some(lane_index);
                break;
            }
            state = self.wait(state)?;
        }
        drop(state);
        let result = publish();
        let mut state = self.lock_state()?;
        if result.is_err() {
            state.aborted = true;
            self.changed.notify_all();
            return result;
        }
        let round_complete = {
            let round = state
                .round
                .as_mut()
                .ok_or_else(|| route_message("cross-lane replay publication lost its round"))?;
            if round.publishing != Some(lane_index) || !round.published.insert(lane_index) {
                state.aborted = true;
                self.changed.notify_all();
                return Err(route_message(
                    "cross-lane replay publication cursor changed unexpectedly",
                ));
            }
            round.publishing = None;
            round.published.len() == round.participants.len()
        };
        state
            .lanes
            .get_mut(&lane_index)
            .ok_or_else(|| route_message("cross-lane replay lane disappeared"))?
            .next_decision = decision
            .checked_add(1)
            .ok_or_else(|| route_message("cross-lane replay decision overflowed"))?;
        if round_complete {
            state.round = None;
        }
        self.changed.notify_all();
        result
    }

    pub(super) fn finish_lane(
        &self,
        lane_index: usize,
        next_decision: u64,
    ) -> Result<(), NativeTacticRouteRunError> {
        let mut state = self.lock_state()?;
        Self::check_active(&state)?;
        let cursor = state
            .lanes
            .get(&lane_index)
            .copied()
            .ok_or_else(|| route_message("cross-lane replay lane is not registered"))?;
        if cursor.next_decision != next_decision {
            return Err(route_message(
                "cross-lane replay finished at a detached decision cursor",
            ));
        }
        if let Some(round) = state.round.as_mut()
            && round.participants.contains(&lane_index)
        {
            if round.begun.contains(&lane_index) && !round.published.contains(&lane_index) {
                state.aborted = true;
                self.changed.notify_all();
                return Err(route_message(
                    "cross-lane replay lane finished during an active decision",
                ));
            }
            round
                .participants
                .retain(|candidate| *candidate != lane_index);
            round.begun.remove(&lane_index);
            round.published.remove(&lane_index);
            if round.participants.is_empty() || round.published.len() == round.participants.len() {
                state.round = None;
            }
        }
        state
            .lanes
            .get_mut(&lane_index)
            .expect("checked replay lane")
            .active = false;
        self.changed.notify_all();
        Ok(())
    }

    pub(super) fn abort(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.aborted = true;
            self.changed.notify_all();
        }
    }
}

pub(super) struct BoundedStalenessReplaySession {
    learner: SharedTacticLearnerAuthority,
    lane: NativeTacticLanePlan,
    maximum_stale_replay_revisions: u64,
    consumed_revision: u64,
    telemetry: NativeTacticReplaySharingTelemetry,
    cross_lane: Option<Arc<CrossLaneReplayCoordinator>>,
    pinned_snapshot: Option<Arc<TacticQImmutableLearnerSnapshot>>,
    pinned_replay_revision: Option<u64>,
    active_decision: Option<u64>,
    finished: bool,
}

impl BoundedStalenessReplaySession {
    pub(super) fn new(
        learner: SharedTacticLearnerAuthority,
        lane: &NativeTacticLanePlan,
        maximum_stale_replay_revisions: u64,
        consumed_revision: u64,
        cross_lane: Option<Arc<CrossLaneReplayCoordinator>>,
    ) -> Result<Self, NativeTacticRouteRunError> {
        u32::try_from(lane.lane_index).map_err(route_error)?;
        Ok(Self {
            learner,
            lane: lane.clone(),
            maximum_stale_replay_revisions,
            consumed_revision,
            telemetry: NativeTacticReplaySharingTelemetry::default(),
            cross_lane,
            pinned_snapshot: None,
            pinned_replay_revision: None,
            active_decision: None,
            finished: false,
        })
    }

    pub(super) fn synchronize_startup(
        &self,
        next_decision: u64,
        recovered: Option<(&TacticQCampaign, &[NativeTacticDecisionTrace])>,
    ) -> Result<(), NativeTacticRouteRunError> {
        let repair = || match recovered {
            Some((campaign, trace)) => self.repair_committed(campaign, trace),
            None => Ok(()),
        };
        match &self.cross_lane {
            Some(coordinator) => {
                coordinator.synchronize_startup(self.lane.lane_index, next_decision, repair)
            }
            None => repair(),
        }
    }

    pub(super) fn begin_decision(
        &mut self,
        decision: u64,
    ) -> Result<(), NativeTacticRouteRunError> {
        if self.active_decision.is_some()
            || self.pinned_snapshot.is_some()
            || self.pinned_replay_revision.is_some()
        {
            return Err(route_message(
                "campaign replay began a decision before publishing the prior decision",
            ));
        }
        if let Some(coordinator) = &self.cross_lane {
            let pinned = coordinator.begin_decision(self.lane.lane_index, decision)?;
            self.pinned_snapshot = Some(pinned.snapshot);
            self.pinned_replay_revision = Some(pinned.replay_revision);
            self.active_decision = Some(decision);
        }
        Ok(())
    }

    /// Return a newer immutable learner publication without mutating the lane.
    /// This lets the coordinator evaluate the same native state and legal
    /// action surface immediately before and after consuming the publication.
    pub(super) fn pending_snapshot(
        &mut self,
    ) -> Result<Option<Arc<TacticQImmutableLearnerSnapshot>>, NativeTacticRouteRunError> {
        {
            let learner = lock_learner_authority(&self.learner)?;
            let snapshot = self
                .pinned_snapshot
                .as_ref()
                .map(Arc::clone)
                .unwrap_or_else(|| learner.snapshot());
            let replay_revision = self
                .pinned_replay_revision
                .unwrap_or_else(|| learner.replay().replay_snapshot().revision);
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
        &mut self,
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
        if self.cross_lane.is_some() && self.active_decision != Some(publisher_decision) {
            return Err(route_message(
                "campaign replay publication is detached from its active decision",
            ));
        }
        let publish = || {
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
                u32::try_from(self.lane.lane_index).map_err(route_error)?,
                publisher_decision,
                admitted_rows,
                duplicate_rows,
                evaluated.iter().any(|proposal| proposal.outcome.terminal),
                self.maximum_stale_replay_revisions,
            )
        };
        let result = match &self.cross_lane {
            Some(coordinator) => {
                coordinator.publish_in_order(self.lane.lane_index, publisher_decision, publish)
            }
            None => publish(),
        }?;
        self.pinned_snapshot = None;
        self.pinned_replay_revision = None;
        self.active_decision = None;
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

    pub(super) fn finish(&mut self, next_decision: u64) -> Result<(), NativeTacticRouteRunError> {
        if self.active_decision.is_some()
            || self.pinned_snapshot.is_some()
            || self.pinned_replay_revision.is_some()
        {
            return Err(route_message(
                "campaign replay finished with an unpublished decision",
            ));
        }
        if let Some(coordinator) = &self.cross_lane {
            coordinator.finish_lane(self.lane.lane_index, next_decision)?;
        }
        self.finished = true;
        Ok(())
    }
}

impl Drop for BoundedStalenessReplaySession {
    fn drop(&mut self) {
        if !self.finished
            && let Some(coordinator) = &self.cross_lane
        {
            coordinator.abort();
        }
    }
}

pub(super) fn build_replay_session(
    execution_plan: &NativeTacticExecutionPlan,
    live_learner: Option<SharedTacticLearnerAuthority>,
    lane: &NativeTacticLanePlan,
    inherited_replay_revision: u64,
    cross_lane: Option<Arc<CrossLaneReplayCoordinator>>,
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
            cross_lane,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

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
    fn cross_lane_round_pins_one_snapshot_and_publishes_in_lane_order() {
        let root = std::env::temp_dir().join(format!(
            "dusklight-cross-lane-replay-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let identity = TacticReplayControlPlaneIdentity::new(
            Digest([1; 32]),
            Digest([2; 32]),
            Digest([3; 32]),
            Digest([4; 32]),
        )
        .unwrap();
        let replay = TacticReplayControlPlane::create(
            &root.join("replay.dtrp"),
            &root.join("objects"),
            identity,
        )
        .unwrap();
        let learner = Arc::new(Mutex::new(
            CampaignTacticLearnerAuthority::new(
                replay,
                OptionValueConfig::default(),
                0,
                TacticValueTreatment::LocalGeneralizedFittedQKnnV1,
                1,
                Some(4),
            )
            .unwrap(),
        ));
        let coordinator = CrossLaneReplayCoordinator::new(learner, &[0, 1]).unwrap();
        let startup_order = Arc::new(Mutex::new(Vec::new()));
        let publication_order = Arc::new(Mutex::new(Vec::new()));
        let (lane_one_ready, lane_one_arrived) = mpsc::channel();

        let (lane_zero_snapshot, lane_one_snapshot) = std::thread::scope(|scope| {
            let lane_one_coordinator = Arc::clone(&coordinator);
            let lane_one_startup = Arc::clone(&startup_order);
            let lane_one_publication = Arc::clone(&publication_order);
            let lane_one = scope.spawn(move || {
                lane_one_coordinator.synchronize_startup(1, 0, || {
                    lane_one_startup.lock().unwrap().push(1);
                    Ok(())
                })?;
                let snapshot = lane_one_coordinator.begin_decision(1, 0)?;
                lane_one_ready.send(()).unwrap();
                lane_one_coordinator.publish_in_order(1, 0, || {
                    lane_one_publication.lock().unwrap().push(1);
                    Ok(CampaignLearnerPublishResult {
                        admitted_rows: 0,
                        duplicate_rows: 0,
                        update: CampaignLearnerUpdateMetrics::default(),
                    })
                })?;
                lane_one_coordinator.finish_lane(1, 1)?;
                Ok::<_, NativeTacticRouteRunError>(snapshot.snapshot.sha256)
            });

            let lane_zero_coordinator = Arc::clone(&coordinator);
            let lane_zero_startup = Arc::clone(&startup_order);
            let lane_zero_publication = Arc::clone(&publication_order);
            let lane_zero = scope.spawn(move || {
                lane_zero_coordinator.synchronize_startup(0, 0, || {
                    lane_zero_startup.lock().unwrap().push(0);
                    Ok(())
                })?;
                let snapshot = lane_zero_coordinator.begin_decision(0, 0)?;
                lane_one_arrived.recv().unwrap();
                lane_zero_coordinator.publish_in_order(0, 0, || {
                    lane_zero_publication.lock().unwrap().push(0);
                    Ok(CampaignLearnerPublishResult {
                        admitted_rows: 0,
                        duplicate_rows: 0,
                        update: CampaignLearnerUpdateMetrics::default(),
                    })
                })?;
                let _second_round = lane_zero_coordinator.begin_decision(0, 1)?;
                lane_zero_coordinator.publish_in_order(0, 1, || {
                    lane_zero_publication.lock().unwrap().push(0);
                    Ok(CampaignLearnerPublishResult {
                        admitted_rows: 0,
                        duplicate_rows: 0,
                        update: CampaignLearnerUpdateMetrics::default(),
                    })
                })?;
                lane_zero_coordinator.finish_lane(0, 2)?;
                Ok::<_, NativeTacticRouteRunError>(snapshot.snapshot.sha256)
            });

            (
                lane_zero.join().unwrap().unwrap(),
                lane_one.join().unwrap().unwrap(),
            )
        });

        assert_eq!(*startup_order.lock().unwrap(), vec![0, 1]);
        assert_eq!(*publication_order.lock().unwrap(), vec![0, 1, 0]);
        assert_eq!(lane_zero_snapshot, lane_one_snapshot);
        fs::remove_dir_all(root).unwrap();
    }
}
