use super::*;

pub(super) type SharedTacticLearnerAuthority = Arc<Mutex<CampaignTacticLearnerAuthority>>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct CampaignLearnerUpdateMetrics {
    pub(super) updates: u64,
    pub(super) update_micros: u64,
    pub(super) snapshots_published: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct CampaignLearnerPublishResult {
    pub(super) admitted_rows: u64,
    pub(super) duplicate_rows: u64,
    pub(super) update: CampaignLearnerUpdateMetrics,
}

/// The single fitted-policy owner for one native tactic route campaign.
///
/// Native lanes publish authenticated transitions to the replay journal and
/// consume `Arc` snapshots from this authority. Only this type calls the model
/// fitter, so worker count cannot multiply identical replay-prefix refits.
pub(super) struct CampaignTacticLearnerAuthority {
    replay: TacticReplayControlPlane,
    model_config: OptionValueConfig,
    goal_distance_feature: usize,
    refit_every_decisions: u64,
    completed_decisions: u64,
    latest: Arc<TacticQImmutableLearnerSnapshot>,
    invocation_metrics: CampaignLearnerUpdateMetrics,
}

impl CampaignTacticLearnerAuthority {
    pub(super) fn new(
        replay: TacticReplayControlPlane,
        model_config: OptionValueConfig,
        goal_distance_feature: usize,
        refit_every_decisions: u64,
    ) -> Result<Self, NativeTacticRouteRunError> {
        if refit_every_decisions == 0 {
            return Err(route_message(
                "campaign learner refit cadence must be positive",
            ));
        }
        let prior_model_revision = replay
            .admissions()
            .into_iter()
            .map(|admission| {
                replay
                    .learner_snapshot(admission.learner_snapshot_sha256)
                    .map(|snapshot| snapshot.model_revision)
                    .map_err(route_error)
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .max()
            .unwrap_or(0);
        let replay_snapshot = replay.snapshot().map_err(route_error)?;
        let has_training = !replay_snapshot.corpus.transitions.is_empty();
        let model_revision = prior_model_revision.saturating_add(u64::from(has_training));
        let started = Instant::now();
        let snapshot = TacticQImmutableLearnerSnapshot::fit(
            replay_snapshot.corpus,
            replay_snapshot.version.revision,
            model_revision,
            model_config.clone(),
            goal_distance_feature,
        )
        .map_err(route_error)?;
        let update_micros = has_training
            .then(|| elapsed_micros(started.elapsed()))
            .unwrap_or(0);
        let stored_sha256 = replay
            .publish_learner_snapshot(&snapshot.manifest)
            .map_err(route_error)?;
        if stored_sha256 != snapshot.sha256 {
            return Err(route_message(
                "campaign learner snapshot store changed its identity",
            ));
        }
        Ok(Self {
            replay,
            model_config,
            goal_distance_feature,
            refit_every_decisions,
            completed_decisions: 0,
            latest: Arc::new(snapshot),
            invocation_metrics: CampaignLearnerUpdateMetrics {
                updates: u64::from(has_training),
                update_micros,
                snapshots_published: 1,
            },
        })
    }

    pub(super) fn snapshot(&self) -> Arc<TacticQImmutableLearnerSnapshot> {
        Arc::clone(&self.latest)
    }

    pub(super) fn snapshot_through(
        &mut self,
        replay_revision: u64,
    ) -> Result<Arc<TacticQImmutableLearnerSnapshot>, NativeTacticRouteRunError> {
        if self.latest.replay_revision == replay_revision {
            return Ok(self.snapshot());
        }
        let replay = self
            .replay
            .snapshot_through(replay_revision)
            .map_err(route_error)?;
        self.fit_and_publish(replay)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn publish(
        &mut self,
        publisher_lane: u32,
        publisher_decision: u64,
        learner_snapshot_sha256: Digest,
        transition: &OptionTransitionSample,
        route: &InputTape,
        episode_group: u64,
    ) -> Result<TacticReplayAdmissionOutcome, NativeTacticRouteRunError> {
        self.replay
            .publish(
                publisher_lane,
                publisher_decision,
                learner_snapshot_sha256,
                transition,
                route,
                episode_group,
            )
            .map_err(route_error)
    }

    pub(super) fn finish_decision(
        &mut self,
        admitted_rows: u64,
        duplicate_rows: u64,
        force_update: bool,
        maximum_stale_replay_revisions: u64,
    ) -> Result<CampaignLearnerPublishResult, NativeTacticRouteRunError> {
        self.completed_decisions = self.completed_decisions.saturating_add(1);
        let current_revision = self.replay.replay_snapshot().revision;
        let pending_revisions = current_revision.saturating_sub(self.latest.replay_revision);
        let update_required = learner_update_required(
            pending_revisions,
            self.latest.manifest.model_revision,
            self.completed_decisions,
            self.refit_every_decisions,
            force_update,
            maximum_stale_replay_revisions,
        );
        let update = if update_required {
            let before = self.invocation_metrics;
            self.fit_current()?;
            CampaignLearnerUpdateMetrics {
                updates: self
                    .invocation_metrics
                    .updates
                    .saturating_sub(before.updates),
                update_micros: self
                    .invocation_metrics
                    .update_micros
                    .saturating_sub(before.update_micros),
                snapshots_published: self
                    .invocation_metrics
                    .snapshots_published
                    .saturating_sub(before.snapshots_published),
            }
        } else {
            CampaignLearnerUpdateMetrics::default()
        };
        Ok(CampaignLearnerPublishResult {
            admitted_rows,
            duplicate_rows,
            update,
        })
    }

    pub(super) fn force_update(
        &mut self,
    ) -> Result<CampaignLearnerUpdateMetrics, NativeTacticRouteRunError> {
        if self.replay.replay_snapshot().revision == self.latest.replay_revision {
            return Ok(CampaignLearnerUpdateMetrics::default());
        }
        let before = self.invocation_metrics;
        self.fit_current()?;
        Ok(CampaignLearnerUpdateMetrics {
            updates: self
                .invocation_metrics
                .updates
                .saturating_sub(before.updates),
            update_micros: self
                .invocation_metrics
                .update_micros
                .saturating_sub(before.update_micros),
            snapshots_published: self
                .invocation_metrics
                .snapshots_published
                .saturating_sub(before.snapshots_published),
        })
    }

    pub(super) fn replay(&self) -> &TacticReplayControlPlane {
        &self.replay
    }

    pub(super) fn invocation_metrics(&self) -> CampaignLearnerUpdateMetrics {
        self.invocation_metrics
    }

    fn fit_current(
        &mut self,
    ) -> Result<Arc<TacticQImmutableLearnerSnapshot>, NativeTacticRouteRunError> {
        let replay = self.replay.snapshot().map_err(route_error)?;
        self.fit_and_publish(replay)
    }

    fn fit_and_publish(
        &mut self,
        replay: TacticReplaySnapshot,
    ) -> Result<Arc<TacticQImmutableLearnerSnapshot>, NativeTacticRouteRunError> {
        let started = Instant::now();
        let model_revision = self.latest.manifest.model_revision.saturating_add(1);
        let snapshot = TacticQImmutableLearnerSnapshot::fit(
            replay.corpus,
            replay.version.revision,
            model_revision,
            self.model_config.clone(),
            self.goal_distance_feature,
        )
        .map_err(route_error)?;
        let stored_sha256 = self
            .replay
            .publish_learner_snapshot(&snapshot.manifest)
            .map_err(route_error)?;
        if stored_sha256 != snapshot.sha256 {
            return Err(route_message(
                "campaign learner snapshot store changed its identity",
            ));
        }
        let snapshot = Arc::new(snapshot);
        self.latest = Arc::clone(&snapshot);
        self.invocation_metrics.updates = self.invocation_metrics.updates.saturating_add(1);
        self.invocation_metrics.snapshots_published = self
            .invocation_metrics
            .snapshots_published
            .saturating_add(1);
        self.invocation_metrics.update_micros = self
            .invocation_metrics
            .update_micros
            .saturating_add(elapsed_micros(started.elapsed()));
        Ok(snapshot)
    }
}

fn learner_update_required(
    pending_revisions: u64,
    model_revision: u64,
    completed_decisions: u64,
    refit_every_decisions: u64,
    force_update: bool,
    maximum_stale_replay_revisions: u64,
) -> bool {
    pending_revisions > 0
        && (force_update
            || model_revision == 0
            || completed_decisions % refit_every_decisions == 0
            || pending_revisions > maximum_stale_replay_revisions)
}

pub(super) fn lock_learner_authority(
    learner: &SharedTacticLearnerAuthority,
) -> Result<std::sync::MutexGuard<'_, CampaignTacticLearnerAuthority>, NativeTacticRouteRunError> {
    learner
        .lock()
        .map_err(|_| route_message("campaign tactic learner authority lock is poisoned"))
}

#[cfg(test)]
mod tests {
    use super::learner_update_required;

    #[test]
    fn one_campaign_schedule_batches_updates_and_bounds_staleness() {
        assert!(learner_update_required(1, 0, 1, 4, false, 16));
        assert!(!learner_update_required(3, 1, 3, 4, false, 16));
        assert!(learner_update_required(4, 1, 4, 4, false, 16));
        assert!(learner_update_required(17, 1, 5, 4, false, 16));
        assert!(learner_update_required(1, 1, 5, 4, true, 16));
        assert!(!learner_update_required(0, 1, 8, 4, true, 0));
    }
}
