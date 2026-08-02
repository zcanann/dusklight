use super::*;

pub(super) type SharedTacticLearnerAuthority = Arc<Mutex<CampaignTacticLearnerAuthority>>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct CampaignLearnerUpdateMetrics {
    pub(super) updates: u64,
    pub(super) update_micros: u64,
    pub(super) snapshots_published: u64,
    pub(super) reconstruction_micros: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct CampaignLearnerPublishResult {
    pub(super) admitted_rows: u64,
    pub(super) duplicate_rows: u64,
    pub(super) update: CampaignLearnerUpdateMetrics,
}

pub(super) struct CampaignLearnerFinalizationSnapshot {
    pub(super) replay_snapshot: TacticReplaySnapshotVersion,
    pub(super) replay_rows: u64,
    pub(super) useful_training_transitions: u64,
    pub(super) censored_training_transitions: u64,
    pub(super) replay_admission: TacticReplayAdmissionMetrics,
    pub(super) learner_metrics: CampaignLearnerUpdateMetrics,
    pub(super) learner_updates: u64,
    pub(super) model_snapshots_published: u64,
    pub(super) latest_snapshot_sha256: Digest,
    pub(super) latest_manifest: TacticQLearnerSnapshot,
}

/// Read-only durable learner authority used after every lane is committed.
/// Macro finalization does not mutate learner state, so interrupted macro work
/// can consume this view without fitting or publishing another model.
pub(super) struct CompletedCampaignLearnerView {
    full: Option<CompletedCampaignLearnerFullView>,
    sealed: Option<NativeTacticLearnerCompletion>,
}

struct CompletedCampaignLearnerFullView {
    replay: TacticReplayControlPlane,
    learner_heads: CampaignLearnerHeadJournal,
    latest_snapshot_sha256: Digest,
    latest_manifest: TacticQLearnerSnapshot,
}

impl CompletedCampaignLearnerView {
    pub(super) fn open(
        output_root: &Path,
        content_store: TacticQContentStore,
        identity: &TacticReplayControlPlaneIdentity,
    ) -> Result<Self, NativeTacticRouteRunError> {
        let completion_path = output_root.join(NATIVE_TACTIC_LEARNER_COMPLETION_FILE);
        if completion_path.is_file() {
            return Ok(Self {
                full: None,
                sealed: Some(NativeTacticLearnerCompletion::read_and_validate(
                    &completion_path,
                    output_root,
                    identity,
                )?),
            });
        }
        let replay = TacticReplayControlPlane::open_with_content_store(
            output_root.join(NATIVE_TACTIC_REPLAY_CONTROL_PLANE_FILE),
            content_store,
            identity,
        )
        .map_err(route_error)?;
        Self::from_replay(replay)
    }

    fn from_replay(replay: TacticReplayControlPlane) -> Result<Self, NativeTacticRouteRunError> {
        let learner_heads = CampaignLearnerHeadJournal::open_existing(&replay)?;
        let latest = learner_heads
            .latest()
            .ok_or_else(|| route_message("completed campaign has no learner head"))?;
        let latest_manifest = replay
            .learner_snapshot(latest.learner_snapshot_sha256)
            .map_err(route_error)?;
        if latest_manifest.training_replay_rows != latest.replay_revision
            || latest_manifest.model_revision != latest.model_revision
            || latest.replay_revision > replay.replay_snapshot().revision
        {
            return Err(route_message(
                "completed campaign learner head is detached from replay",
            ));
        }
        Ok(Self {
            full: Some(CompletedCampaignLearnerFullView {
                replay,
                learner_heads,
                latest_snapshot_sha256: latest.learner_snapshot_sha256,
                latest_manifest,
            }),
            sealed: None,
        })
    }

    pub(super) fn replay_len(&self) -> usize {
        self.full.as_ref().map_or_else(
            || {
                usize::try_from(
                    self.sealed
                        .as_ref()
                        .expect("completed learner view has one authority")
                        .replay_rows(),
                )
                .unwrap_or(usize::MAX)
            },
            |full| full.replay.len(),
        )
    }

    pub(super) fn finalization_snapshot(
        &self,
        goal_distance_feature: usize,
    ) -> Result<CampaignLearnerFinalizationSnapshot, NativeTacticRouteRunError> {
        if let Some(sealed) = &self.sealed {
            return Ok(sealed.finalization_snapshot());
        }
        let full = self
            .full
            .as_ref()
            .ok_or_else(|| route_message("completed learner view has no authority"))?;
        let replay = full.replay.snapshot().map_err(route_error)?;
        Ok(CampaignLearnerFinalizationSnapshot {
            replay_snapshot: replay.version,
            replay_rows: replay.corpus.transitions.len() as u64,
            useful_training_transitions: useful_training_transitions(
                &replay.corpus,
                goal_distance_feature,
            ),
            censored_training_transitions: censored_training_transitions(&replay.corpus),
            replay_admission: full.replay.invocation_metrics(),
            learner_metrics: CampaignLearnerUpdateMetrics::default(),
            learner_updates: full.latest_manifest.model_revision,
            model_snapshots_published: self
                .full
                .as_ref()
                .expect("full learner view")
                .learner_heads
                .snapshot_sha256s()
                .collect::<BTreeSet<_>>()
                .len() as u64,
            latest_snapshot_sha256: full.latest_snapshot_sha256,
            latest_manifest: full.latest_manifest.clone(),
        })
    }
}

/// The single fitted-policy owner for one native tactic route campaign.
///
/// Native lanes publish authenticated transitions to the replay journal and
/// consume `Arc` snapshots from this authority. Only this type calls the model
/// fitter, so worker count cannot multiply identical replay-prefix refits.
pub(super) struct CampaignTacticLearnerAuthority {
    replay: TacticReplayControlPlane,
    learner_heads: CampaignLearnerHeadJournal,
    model_config: OptionValueConfig,
    goal_distance_feature: usize,
    value_treatment: TacticValueTreatment,
    refit_every_decisions: u64,
    completed_decisions: BTreeSet<(u32, u64)>,
    published_snapshot_sha256s: BTreeSet<Digest>,
    latest: Arc<TacticQImmutableLearnerSnapshot>,
    invocation_metrics: CampaignLearnerUpdateMetrics,
}

impl CampaignTacticLearnerAuthority {
    pub(super) fn new(
        replay: TacticReplayControlPlane,
        model_config: OptionValueConfig,
        goal_distance_feature: usize,
        value_treatment: TacticValueTreatment,
        refit_every_decisions: u64,
        maximum_stale_replay_revisions: Option<u64>,
    ) -> Result<Self, NativeTacticRouteRunError> {
        if refit_every_decisions == 0 {
            return Err(route_message(
                "campaign learner refit cadence must be positive",
            ));
        }
        let learner_heads = CampaignLearnerHeadJournal::open_or_create(&replay)?;
        let durable_snapshot = learner_heads
            .latest()
            .map(|head| {
                replay
                    .learner_snapshot(head.learner_snapshot_sha256)
                    .map_err(route_error)
                    .and_then(|snapshot| {
                        if snapshot.training_replay_rows != head.replay_revision
                            || snapshot.model_revision != head.model_revision
                        {
                            return Err(route_message(
                                "durable campaign learner head is detached from its snapshot",
                            ));
                        }
                        Ok((head.learner_snapshot_sha256, snapshot))
                    })
            })
            .transpose()?;
        let admissions = replay.admissions();
        let completed_decisions = admissions
            .iter()
            .filter(|admission| admission.publisher_lane != u32::MAX)
            .map(|admission| (admission.publisher_lane, admission.publisher_decision))
            .collect::<BTreeSet<_>>();
        let admission_snapshot_sha256s = admissions
            .iter()
            .map(|admission| admission.learner_snapshot_sha256)
            .collect::<BTreeSet<_>>();
        let prior_snapshots = admission_snapshot_sha256s
            .iter()
            .map(|sha256| {
                replay
                    .learner_snapshot(*sha256)
                    .map(|snapshot| (*sha256, snapshot))
                    .map_err(route_error)
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|(_, snapshot)| snapshot.kind == TacticQLearnerSnapshotKind::Learned)
            .collect::<Vec<_>>();
        let mut published_snapshot_sha256s = prior_snapshots
            .iter()
            .map(|(sha256, _)| *sha256)
            .collect::<BTreeSet<_>>();
        published_snapshot_sha256s.extend(learner_heads.snapshot_sha256s());
        let prior_snapshot = durable_snapshot.or_else(|| {
            prior_snapshots.into_iter().max_by_key(|(_, snapshot)| {
                (snapshot.model_revision, snapshot.training_replay_rows)
            })
        });
        let (snapshot, invocation_metrics, mut published_snapshot_sha256s) =
            if let Some((expected_sha256, manifest)) = prior_snapshot {
                if manifest.model_config != model_config
                    || manifest.value_treatment != value_treatment
                    || manifest.training_replay_rows > replay.replay_snapshot().revision
                {
                    return Err(route_message(
                        "durable campaign learner snapshot is detached from its execution plan",
                    ));
                }
                let replay_snapshot = replay
                    .snapshot_through(manifest.training_replay_rows)
                    .map_err(route_error)?;
                let started = Instant::now();
                let snapshot =
                    TacticQImmutableLearnerSnapshot::fit_with_prior_goal_reachability_calibration(
                        replay_snapshot.corpus,
                        replay_snapshot.version.revision,
                        manifest.model_revision,
                        model_config.clone(),
                        goal_distance_feature,
                        value_treatment,
                        manifest.goal_reachability_calibration.as_ref(),
                    )
                    .map_err(route_error)?;
                let migrated = manifest.schema != TACTIC_Q_LEARNER_SNAPSHOT_SCHEMA_V4;
                if migrated {
                    let stored_sha256 = replay
                        .publish_learner_snapshot(&snapshot.manifest)
                        .map_err(route_error)?;
                    if stored_sha256 != snapshot.sha256 {
                        return Err(route_message(
                            "migrated campaign learner snapshot store changed its identity",
                        ));
                    }
                    published_snapshot_sha256s.insert(snapshot.sha256);
                } else if snapshot.sha256 != expected_sha256 || snapshot.manifest != manifest {
                    return Err(route_message(
                        "durable campaign learner snapshot cannot be reconstructed exactly",
                    ));
                }
                (
                    snapshot,
                    CampaignLearnerUpdateMetrics {
                        updates: 0,
                        update_micros: 0,
                        snapshots_published: u64::from(migrated),
                        reconstruction_micros: elapsed_micros(started.elapsed()),
                    },
                    published_snapshot_sha256s,
                )
            } else {
                if !replay.is_empty() {
                    return Err(route_message(
                        "campaign replay has no durable learner snapshot authority",
                    ));
                }
                let replay_snapshot = replay.snapshot().map_err(route_error)?;
                let snapshot = TacticQImmutableLearnerSnapshot::fit(
                    replay_snapshot.corpus,
                    replay_snapshot.version.revision,
                    0,
                    model_config.clone(),
                    goal_distance_feature,
                    value_treatment,
                )
                .map_err(route_error)?;
                let stored_sha256 = replay
                    .publish_learner_snapshot(&snapshot.manifest)
                    .map_err(route_error)?;
                if stored_sha256 != snapshot.sha256 {
                    return Err(route_message(
                        "campaign learner snapshot store changed its identity",
                    ));
                }
                let mut published = BTreeSet::new();
                published.insert(snapshot.sha256);
                (
                    snapshot,
                    CampaignLearnerUpdateMetrics {
                        updates: 0,
                        update_micros: 0,
                        snapshots_published: 1,
                        reconstruction_micros: 0,
                    },
                    published,
                )
            };
        published_snapshot_sha256s.insert(snapshot.sha256);
        let mut authority = Self {
            replay,
            learner_heads,
            model_config,
            goal_distance_feature,
            value_treatment,
            refit_every_decisions,
            completed_decisions,
            published_snapshot_sha256s,
            latest: Arc::new(snapshot),
            invocation_metrics,
        };
        authority.publish_latest_head()?;
        if let Some(maximum_stale_replay_revisions) = maximum_stale_replay_revisions {
            authority.restore_missing_update(maximum_stale_replay_revisions)?;
        }
        Ok(authority)
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
        publisher_lane: u32,
        publisher_decision: u64,
        admitted_rows: u64,
        duplicate_rows: u64,
        force_update: bool,
        maximum_stale_replay_revisions: u64,
    ) -> Result<CampaignLearnerPublishResult, NativeTacticRouteRunError> {
        if !self
            .completed_decisions
            .insert((publisher_lane, publisher_decision))
        {
            return Ok(CampaignLearnerPublishResult {
                admitted_rows,
                duplicate_rows,
                update: CampaignLearnerUpdateMetrics::default(),
            });
        }
        let current_revision = self.replay.replay_snapshot().revision;
        let pending_revisions = current_revision.saturating_sub(self.latest.replay_revision);
        let update_required = learner_update_required(
            pending_revisions,
            self.latest.manifest.model_revision,
            self.completed_decisions.len() as u64,
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
                reconstruction_micros: 0,
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
            reconstruction_micros: 0,
        })
    }

    pub(super) fn replay(&self) -> &TacticReplayControlPlane {
        &self.replay
    }

    pub(super) fn contains_transition(
        &self,
        transition: &OptionTransitionSample,
    ) -> Result<bool, NativeTacticRouteRunError> {
        self.replay
            .contains_transition(transition)
            .map_err(route_error)
    }

    pub(super) fn invocation_metrics(&self) -> CampaignLearnerUpdateMetrics {
        self.invocation_metrics
    }

    pub(super) fn total_updates(&self) -> u64 {
        self.latest.manifest.model_revision
    }

    pub(super) fn published_snapshot_count(&self) -> u64 {
        self.published_snapshot_sha256s.len() as u64
    }

    pub(super) fn finalization_snapshot(
        &self,
        goal_distance_feature: usize,
    ) -> Result<CampaignLearnerFinalizationSnapshot, NativeTacticRouteRunError> {
        let replay = self.replay.snapshot().map_err(route_error)?;
        Ok(CampaignLearnerFinalizationSnapshot {
            replay_snapshot: replay.version,
            replay_rows: replay.corpus.transitions.len() as u64,
            useful_training_transitions: useful_training_transitions(
                &replay.corpus,
                goal_distance_feature,
            ),
            censored_training_transitions: censored_training_transitions(&replay.corpus),
            replay_admission: self.replay.invocation_metrics(),
            learner_metrics: self.invocation_metrics(),
            learner_updates: self.total_updates(),
            model_snapshots_published: self.published_snapshot_count(),
            latest_snapshot_sha256: self.latest.sha256,
            latest_manifest: self.latest.manifest.clone(),
        })
    }

    fn restore_missing_update(
        &mut self,
        maximum_stale_replay_revisions: u64,
    ) -> Result<(), NativeTacticRouteRunError> {
        let current_revision = self.replay.replay_snapshot().revision;
        let pending_revisions = current_revision.saturating_sub(self.latest.replay_revision);
        if learner_update_required(
            pending_revisions,
            self.latest.manifest.model_revision,
            self.completed_decisions.len() as u64,
            self.refit_every_decisions,
            false,
            maximum_stale_replay_revisions,
        ) {
            self.fit_current()?;
        }
        Ok(())
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
        let snapshot =
            TacticQImmutableLearnerSnapshot::fit_with_prior_goal_reachability_calibration(
                replay.corpus,
                replay.version.revision,
                model_revision,
                self.model_config.clone(),
                self.goal_distance_feature,
                self.value_treatment,
                self.latest.manifest.goal_reachability_calibration.as_ref(),
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
        self.learner_heads.publish(CampaignLearnerHead {
            learner_snapshot_sha256: snapshot.sha256,
            replay_revision: snapshot.replay_revision,
            model_revision: snapshot.manifest.model_revision,
        })?;
        let snapshot = Arc::new(snapshot);
        self.published_snapshot_sha256s.insert(snapshot.sha256);
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

    fn publish_latest_head(&mut self) -> Result<(), NativeTacticRouteRunError> {
        self.learner_heads.publish(CampaignLearnerHead {
            learner_snapshot_sha256: self.latest.sha256,
            replay_revision: self.latest.replay_revision,
            model_revision: self.latest.manifest.model_revision,
        })?;
        Ok(())
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
            || completed_decisions.is_multiple_of(refit_every_decisions)
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
    use super::*;
    use dusklight_control::game_tactic::{GameTactic, GameTacticPlan};
    use dusklight_control::option_execution::{OptionEndReason, OptionExecution, TapeRange};
    use dusklight_evidence::native_episode_shard::NativeObservationPhase;
    use dusklight_learning::fact_snapshot::FactTerminalReason;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn one_campaign_schedule_batches_updates_and_bounds_staleness() {
        assert!(learner_update_required(1, 0, 1, 4, false, 16));
        assert!(!learner_update_required(3, 1, 3, 4, false, 16));
        assert!(learner_update_required(4, 1, 4, 4, false, 16));
        assert!(learner_update_required(17, 1, 5, 4, false, 16));
        assert!(learner_update_required(1, 1, 5, 4, true, 16));
        assert!(!learner_update_required(0, 1, 8, 4, true, 0));

        // Reopening after decision ten must preserve the revision fitted after
        // decision eight. Process restart is not a refit boundary.
        assert!(!learner_update_required(32, 3, 10, 4, false, 64));
        // If decision twelve committed but its scheduled publication did not,
        // recovery performs exactly that missing cadence update.
        assert!(learner_update_required(64, 3, 12, 4, false, 64));
    }

    #[test]
    fn native_feedback_and_stale_workers_restart_with_the_exact_ranking() {
        let root = std::env::temp_dir().join(format!(
            "dusklight-learner-authority-feedback-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let execution_authority_sha256 = Digest([8; 32]);
        let feature_schema_sha256 = Digest([1; 32]);
        let objective_sha256 = Digest([2; 32]);
        let root_checkpoint_sha256 = Digest([3; 32]);
        let identity = TacticReplayControlPlaneIdentity::new(
            execution_authority_sha256,
            feature_schema_sha256,
            objective_sha256,
            root_checkpoint_sha256,
        )
        .unwrap();
        let journal = root.join("replay.dtrp");
        let objects = root.join("objects");
        let replay =
            TacticReplayControlPlane::create(&journal, &objects, identity.clone()).unwrap();
        let mut authority = CampaignTacticLearnerAuthority::new(
            replay,
            OptionValueConfig::default(),
            0,
            TacticValueTreatment::LocalGeneralizedFittedQKnnV1,
            1,
            Some(4),
        )
        .unwrap();
        let cold_snapshot = authority.snapshot();
        assert_eq!(cold_snapshot.replay_revision, 0);
        assert_eq!(cold_snapshot.manifest.model_revision, 0);

        let catalog = TacticAssetCatalog::new(vec![
            TacticCatalogEntry::new(
                "shield",
                TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield {
                    frames: 1,
                })),
            )
            .unwrap(),
        ])
        .unwrap();
        let description = catalog.entries()[0].description();
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        let native_step = &shard.episodes[0].steps[0];
        let mut before =
            FactSnapshot::from_native_learning(&native_step.pre_input, &[], None, Vec::new())
                .unwrap();
        before.terminal.configured = Some(true);
        before.terminal.reached = Some(false);
        before.terminal.reason = FactTerminalReason::None;
        let range = TapeRange {
            start_frame: before.tape_frame,
            end_frame_exclusive: before.tape_frame + 1,
        };
        let route = InputTape {
            frames: vec![InputFrame::default(); range.end_frame_exclusive as usize],
            ..InputTape::default()
        };
        let execution = OptionExecution::capture(
            description.option.option_id.clone(),
            description.option.option_type.clone(),
            description.option.parameters.clone(),
            description.duration.minimum_ticks,
            description.duration.maximum_ticks,
            description.stopping.termination.clone(),
            description.stopping.cancellation.clone(),
            OptionEndReason::Completed,
            &route,
            range,
        )
        .unwrap();
        let mut next_boundary = native_step.post_simulation.clone();
        next_boundary.phase = NativeObservationPhase::PreInput;
        next_boundary.simulation_tick += 1;
        next_boundary.tape_frame += 1;
        let mut after = FactSnapshot::from_native_learning(
            &next_boundary,
            std::slice::from_ref(&native_step.pre_input),
            Some(&execution),
            Vec::new(),
        )
        .unwrap();
        after.terminal.configured = Some(true);
        after.terminal.reached = Some(true);
        after.terminal.reason = FactTerminalReason::GoalReached;
        after.terminal.first_hit_tick = Some(after.simulation_tick);
        let source_route = InputTape {
            frames: route.frames[..range.start_frame as usize].to_vec(),
            ..route.clone()
        };
        let mut transition = OptionTransitionSample::capture(
            feature_schema_sha256,
            route_checkpoint(root_checkpoint_sha256, &source_route).unwrap(),
            route_checkpoint(root_checkpoint_sha256, &route).unwrap(),
            before.clone(),
            after,
            execution,
            &route,
            5.0,
            true,
            |facts| Ok::<_, &'static str>(vec![facts.tape_frame as f32]),
        )
        .unwrap();
        transition.execution_authority_sha256 = execution_authority_sha256;
        transition.validate().unwrap();

        let registry = FactRegistry::canonical();
        let current =
            LearnerState::build(before.clone(), &registry, &catalog, &[], |_| true).unwrap();
        let mut campaign = TacticQCampaign::new(
            feature_schema_sha256,
            objective_sha256,
            root_checkpoint_sha256,
            11,
            current,
            source_route.clone(),
            OptionValueConfig::default(),
            TacticExplorationConfig {
                seed: 41,
                epsilon_per_million: 0,
            },
        )
        .unwrap();
        campaign
            .bind_execution_authority(execution_authority_sha256)
            .unwrap();
        assert_eq!(
            campaign.consume_learner_snapshot(&cold_snapshot).unwrap(),
            0
        );
        let encode = |facts: &FactSnapshot| Ok::<_, &'static str>(vec![facts.tape_frame as f32]);
        let cold_decision = campaign.decide(&catalog, &[], &encode).unwrap();
        let cold_probe_batch = campaign
            .decide_parameterized_batch(&catalog, &[], Digest([9; 32]), &encode, 1)
            .unwrap();
        assert_eq!(
            cold_decision.selected.reason,
            TacticSelectionReason::UnsupportedBootstrap
        );
        assert!(cold_decision.ranking.values.ranked.is_empty());

        assert!(matches!(
            authority
                .publish(
                    0,
                    0,
                    cold_snapshot.sha256,
                    &transition,
                    &route,
                    campaign.episode_group,
                )
                .unwrap(),
            TacticReplayAdmissionOutcome::Admitted { sequence: 0, .. }
        ));
        let publish = authority.finish_decision(0, 0, 1, 0, true, 4).unwrap();
        assert_eq!(publish.update.updates, 1);
        assert_eq!(publish.update.snapshots_published, 1);
        let learned_snapshot = authority.snapshot();
        assert_eq!(learned_snapshot.replay_revision, 1);
        assert_eq!(learned_snapshot.manifest.model_revision, 1);
        assert_ne!(learned_snapshot.sha256, cold_snapshot.sha256);

        assert_eq!(
            campaign
                .consume_learner_snapshot(&learned_snapshot)
                .unwrap(),
            1
        );
        let learned_decision = campaign.decide(&catalog, &[], &encode).unwrap();
        let learned_probe_batch = campaign
            .decide_parameterized_batch(&catalog, &[], Digest([9; 32]), &encode, 1)
            .unwrap();
        let policy_update_probe = build_policy_update_probe(
            campaign.current.snapshot_sha256,
            &cold_snapshot,
            &learned_snapshot,
            &cold_probe_batch,
            &learned_probe_batch,
        )
        .unwrap();
        assert_eq!(
            policy_update_probe.before_action_surface_sha256,
            policy_update_probe.after_action_surface_sha256
        );
        assert_eq!(
            policy_update_probe.before_selection_reason,
            TacticSelectionReason::UnsupportedBootstrap
        );
        assert_eq!(
            policy_update_probe.after_selection_reason,
            TacticSelectionReason::Greedy
        );
        assert_eq!(
            learned_decision.ranking.learner_snapshot_sha256, campaign.current.snapshot_sha256,
            "the legacy ranking field binds the state snapshot, not the fitted policy"
        );
        assert_eq!(learned_decision.ranking.values.ranked.len(), 1);
        assert!(learned_decision.ranking.values.unsupported.is_empty());
        assert_eq!(
            learned_decision.selected.reason,
            TacticSelectionReason::Greedy
        );

        assert!(
            campaign.consume_learner_snapshot(&cold_snapshot).is_err(),
            "a managed lane must not roll its fitted policy back to an older revision"
        );
        assert_eq!(campaign.model_revision(), 1);

        // A worker may finish evidence selected under an older policy after a
        // newer snapshot has already been published. It remains authenticated
        // off-policy experience and must advance the shared replay and model
        // without making restart forget the intervening learner revision.
        let mut stale_transition = transition.clone();
        stale_transition.execution.option_id = "shield-stale-worker".into();
        stale_transition.value_sample.action.option_id = "shield-stale-worker".into();
        stale_transition.validate().unwrap();
        assert!(matches!(
            authority
                .publish(
                    1,
                    0,
                    cold_snapshot.sha256,
                    &stale_transition,
                    &route,
                    campaign.episode_group + 1,
                )
                .unwrap(),
            TacticReplayAdmissionOutcome::Admitted { sequence: 1, .. }
        ));
        let stale_publish = authority.finish_decision(1, 0, 1, 0, true, 4).unwrap();
        assert_eq!(stale_publish.update.updates, 1);
        let latest_snapshot = authority.snapshot();
        assert_eq!(latest_snapshot.replay_revision, 2);
        assert_eq!(latest_snapshot.manifest.model_revision, 2);
        assert_eq!(
            campaign.consume_learner_snapshot(&latest_snapshot).unwrap(),
            1
        );
        let latest_decision = campaign.decide(&catalog, &[], &encode).unwrap();
        let learned_values = latest_decision.ranking.values.clone();
        let learned_selected = latest_decision.selected.clone();
        let published_snapshot_count = authority.published_snapshot_count();
        assert_eq!(published_snapshot_count, 3);

        drop(campaign);
        drop(authority);
        let replay = TacticReplayControlPlane::open(&journal, &objects, &identity).unwrap();
        let completed_view = CompletedCampaignLearnerView::from_replay(replay).unwrap();
        let completed_snapshot = completed_view.finalization_snapshot(0).unwrap();
        assert_eq!(
            completed_snapshot.learner_metrics,
            CampaignLearnerUpdateMetrics::default()
        );
        assert_eq!(completed_snapshot.replay_snapshot.revision, 2);
        assert_eq!(
            completed_snapshot.latest_snapshot_sha256,
            latest_snapshot.sha256
        );
        assert_eq!(completed_snapshot.latest_manifest, latest_snapshot.manifest);
        assert_eq!(
            completed_snapshot.model_snapshots_published,
            published_snapshot_count
        );
        drop(completed_view);

        let replay = TacticReplayControlPlane::open(&journal, &objects, &identity).unwrap();
        let reopened_authority = CampaignTacticLearnerAuthority::new(
            replay,
            OptionValueConfig::default(),
            0,
            TacticValueTreatment::LocalGeneralizedFittedQKnnV1,
            1,
            Some(4),
        )
        .unwrap();
        let reopened_snapshot = reopened_authority.snapshot();
        assert_eq!(
            reopened_authority.published_snapshot_count(),
            published_snapshot_count
        );
        assert_eq!(reopened_snapshot.sha256, latest_snapshot.sha256);
        assert_eq!(reopened_snapshot.manifest, latest_snapshot.manifest);
        assert_eq!(
            reopened_snapshot.replay_revision,
            latest_snapshot.replay_revision
        );

        let current = LearnerState::build(before, &registry, &catalog, &[], |_| true).unwrap();
        let mut restarted_campaign = TacticQCampaign::new(
            feature_schema_sha256,
            objective_sha256,
            root_checkpoint_sha256,
            11,
            current,
            source_route,
            OptionValueConfig::default(),
            TacticExplorationConfig {
                seed: 41,
                epsilon_per_million: 0,
            },
        )
        .unwrap();
        restarted_campaign
            .bind_execution_authority(execution_authority_sha256)
            .unwrap();
        assert_eq!(
            restarted_campaign
                .consume_learner_snapshot(&reopened_snapshot)
                .unwrap(),
            2
        );
        let restarted_decision = restarted_campaign.decide(&catalog, &[], &encode).unwrap();
        assert_eq!(restarted_decision.ranking.values, learned_values);
        assert_eq!(restarted_decision.selected, learned_selected);

        drop(restarted_campaign);
        drop(reopened_authority);
        fs::remove_dir_all(root).unwrap();
    }
}
