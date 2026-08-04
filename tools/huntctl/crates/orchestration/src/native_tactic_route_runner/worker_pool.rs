use super::*;

mod dispatch;
mod restoration_dispatch;

use restoration_dispatch::{requires_frontier_materialization, validate_restoration_contract};

pub(super) fn launch_tactic_route_worker(
    config: &NativeTacticRouteRunConfig<'_>,
    repository_root: &Path,
    attempt_root: &Path,
    initial_batch: &NativeSuffixBatch,
    terminal: &NativeTerminalBinding,
    card_fixture: &Path,
) -> Result<(NativeSuffixWorkerSession, FactSnapshot, Digest, u64), NativeTacticRouteRunError> {
    let initial_root = attempt_root.join("initial");
    fs::create_dir_all(&initial_root).map_err(route_error)?;
    let initial_batch_path = initial_root.join("request.json");
    write_new(
        &initial_batch_path,
        &serde_json::to_vec_pretty(initial_batch).map_err(route_error)?,
    )?;
    let launch = NativeSuffixWorkerLaunch {
        executable: repository_root.join(&config.execution.executable.path),
        game_data: repository_root.join(&config.execution.game_data.path),
        input_tape: repository_root.join(&config.execution.process_boot_tape.path),
        milestone_program: repository_root.join(&config.execution.milestone_program.path),
        card_fixture: card_fixture.to_path_buf(),
        card_fixture_sha256: config.execution.card_fixture_manifest.sha256,
        working_directory: repository_root.to_path_buf(),
        state_root: attempt_root.join("native-state"),
        world_context_sha256: config.execution.world_context.sha256,
        terminal: terminal.clone(),
        initial_batch: initial_batch_path,
        initial_result: initial_root.join("result.json"),
        initial_winner_tape: None,
    };
    let (mut worker, initial) = NativeSuffixWorkerSession::launch_compact_with_prevalidated_files(
        &launch,
        NativeSuffixPrevalidatedFileIdentities {
            executable_sha256: config.execution.executable.sha256,
            game_data_sha256: config.execution.game_data.sha256,
        },
    )
    .map_err(route_error)?;
    write_new(
        &attempt_root.join(NATIVE_TACTIC_WORKER_HELLO_FILE),
        &serde_json::to_vec_pretty(worker.hello()).map_err(route_error)?,
    )?;
    let facts = initial_facts(&initial)?;
    let root_checkpoint_sha256 =
        tactic_root_checkpoint_sha256(worker.identity()).map_err(route_error)?;
    worker.suspend_process().map_err(route_error)?;
    Ok((
        worker,
        facts,
        root_checkpoint_sha256,
        initial.checkpoint_bytes,
    ))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_seed_coordinator(
    config: &NativeTacticRouteRunConfig<'_>,
    pool: &NativeTacticProposalPool,
    registry: &FactRegistry,
    encoder: &GoalConditionedTacticFeatureEncoder,
    reward_spec: &TacticRewardSpec,
    initial_facts: &FactSnapshot,
    route_prefix: &InputTape,
    action_schema_sha256: Digest,
    promoted_tactics: &[ImportedPromotedTactic],
    root_checkpoint_sha256: Digest,
    root_tape_ref: StoredContentRef,
    content_store: TacticQContentStore,
    inherited_learner_snapshot: Arc<TacticQImmutableLearnerSnapshot>,
    live_learner: Option<SharedTacticLearnerAuthority>,
    seed_index: usize,
    seed: u64,
) -> Result<CompletedNativeTacticSeed, NativeTacticRouteRunError> {
    let seed_root = config
        .output_root
        .join(format!("seed-{seed_index:03}-{seed}"));
    let seed_result_path = seed_root.join("seed-result.json");
    if seed_result_path.exists() {
        if !config.resume {
            return Err(route_message("unexpected pre-existing tactic seed result"));
        }
        let lane = config
            .execution_plan
            .lanes
            .get(seed_index)
            .ok_or_else(|| route_message("tactic seed lane is absent from execution plan"))?;
        let completed = read_completed_seed(
            &seed_result_path,
            seed,
            config.execution_plan.budgets.decisions_per_lane,
            config.execution_plan.identity()?,
            lane,
            config.execution_plan.demonstration_chunk_ticks.is_some(),
        )?;
        let generated_training =
            load_generated_training_corpus(&completed.result, lane, &completed.checkpoint)?;
        Ok(CompletedNativeTacticSeed {
            result: completed.result,
            generated_training,
            completion_projection: None,
            invocation_wall_micros: 0,
            invocation_model_update_micros: 0,
        })
    } else {
        let lane_pool = pool.for_lane(
            config
                .execution_plan
                .lanes
                .get(seed_index)
                .ok_or_else(|| route_message("tactic seed lane is absent from execution plan"))?
                .generation_lane_index,
        )?;
        let completion = run_seed(
            config,
            &lane_pool,
            registry,
            encoder,
            reward_spec,
            initial_facts,
            route_prefix,
            action_schema_sha256,
            promoted_tactics,
            root_checkpoint_sha256,
            root_tape_ref,
            content_store,
            inherited_learner_snapshot,
            live_learner,
            seed_index,
            seed,
        )?;
        let result_bytes = serde_json::to_vec_pretty(&completion.result).map_err(route_error)?;
        write_new(&seed_result_path, &result_bytes)?;
        let projection = completion.completion_projection.as_ref().ok_or_else(|| {
            route_message("newly completed tactic seed has no completion projection")
        })?;
        publish_seed_completion(&seed_root, &completion.result, &result_bytes, projection)?;
        Ok(completion)
    }
}

pub(super) fn load_generated_training_corpus(
    result: &NativeTacticSeedResult,
    lane: &NativeTacticLanePlan,
    checkpoint: &TacticQCampaignCheckpoint,
) -> Result<TacticQTrainingCorpus, NativeTacticRouteRunError> {
    let corpus = TacticQTrainingCorpus {
        execution_authority_sha256: checkpoint.execution_authority_sha256,
        feature_schema_sha256: checkpoint.feature_schema_sha256,
        objective_sha256: checkpoint.objective_sha256,
        root_checkpoint_sha256: checkpoint.root_checkpoint_sha256,
        transitions: checkpoint.training_replay.clone(),
        routes: checkpoint.training_replay_routes.clone(),
        episode_groups: checkpoint.training_episode_groups.clone(),
    };
    if corpus.execution_authority_sha256 != result.execution_plan_sha256 {
        return Err(route_message(
            "generated tactic training corpus belongs to another execution plan",
        ));
    }
    let expected_rows = result
        .training_replay_rows
        .checked_sub(result.imported_training_replay_rows)
        .ok_or_else(|| route_message("generated tactic replay accounting underflowed"))?;
    let indices = generated_training_row_indices(&corpus.episode_groups, lane, expected_rows)?;
    Ok(TacticQTrainingCorpus {
        execution_authority_sha256: corpus.execution_authority_sha256,
        feature_schema_sha256: corpus.feature_schema_sha256,
        objective_sha256: corpus.objective_sha256,
        root_checkpoint_sha256: corpus.root_checkpoint_sha256,
        transitions: indices
            .iter()
            .map(|index| corpus.transitions[*index].clone())
            .collect(),
        routes: indices
            .iter()
            .map(|index| corpus.routes[*index].clone())
            .collect(),
        episode_groups: indices
            .iter()
            .map(|index| corpus.episode_groups[*index])
            .collect(),
    })
}

fn generated_training_row_indices(
    episode_groups: &[u64],
    lane: &NativeTacticLanePlan,
    expected_rows: usize,
) -> Result<Vec<usize>, NativeTacticRouteRunError> {
    let indices = episode_groups
        .iter()
        .enumerate()
        .filter_map(|(index, episode_group)| {
            lane.owns_episode_group(*episode_group).then_some(index)
        })
        .collect::<Vec<_>>();
    if indices.len() != expected_rows {
        return Err(route_message(
            "generated tactic replay rows do not match lane-owned accounting",
        ));
    }
    Ok(indices)
}

pub(super) fn parameterized_catalog_for_state(
    seed: u64,
    decision_index: u64,
    state: &FactSnapshot,
    encoder: &GoalConditionedTacticFeatureEncoder,
    maximum_ticks: u32,
    feedback: Option<ParameterizedTacticFeedback>,
    action_schema_sha256: Digest,
) -> Result<ParameterizedTacticProposalCatalog, NativeTacticRouteRunError> {
    parameterized_catalog_for_state_with_promoted(
        seed,
        decision_index,
        state,
        encoder,
        maximum_ticks,
        feedback,
        action_schema_sha256,
        &[],
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn parameterized_catalog_for_state_with_promoted(
    seed: u64,
    decision_index: u64,
    state: &FactSnapshot,
    encoder: &GoalConditionedTacticFeatureEncoder,
    maximum_ticks: u32,
    feedback: Option<ParameterizedTacticFeedback>,
    action_schema_sha256: Digest,
    promoted_tactics: &[ImportedPromotedTactic],
) -> Result<ParameterizedTacticProposalCatalog, NativeTacticRouteRunError> {
    let mut proposals = propose_parameterized_tactics(ParameterizedTacticProposalContext {
        seed,
        decision_index,
        state_sha256: state.content_sha256().map_err(route_error)?,
        player_position: state.player.position_f32_bits.map(f32::from_bits),
        camera_yaw_radians: state.player.camera_yaw_radians_f32_bits.map(f32::from_bits),
        prompted_action_available: state
            .player
            .action_state
            .is_some_and(|action| action.do_status != 0),
        front_roll_prompt_available: state.player.action_state.is_some_and(|action| {
            action.do_status == dusklight_learning::fact_snapshot::FRONT_ROLL_DO_STATUS
        }),
        goal_coordinate: encoder.target_coordinate_f32_bits.map(f32::from_bits),
        maximum_ticks,
        feedback,
    })
    .map_err(route_error)?;
    if !promoted_tactics.is_empty() {
        let mut entries = proposals.catalog.entries().to_vec();
        let goal_distance =
            encoder.encode(state).map_err(route_error)?[encoder.goal_distance_feature()];
        entries.extend(
            promoted_tactics
                .iter()
                .filter(|promoted| {
                    promoted.condition.matches(
                        &state.world.stage,
                        state.world.room,
                        state.player.procedure,
                        state.player.contacts,
                        goal_distance,
                        TACTIC_MACRO_ENTRY_GOAL_DISTANCE_PADDING,
                    )
                })
                .map(|promoted| promoted.entry.clone()),
        );
        proposals.catalog = TacticAssetCatalog::new(entries).map_err(route_error)?;
    }
    validate_parameterized_policy_catalog(&proposals.catalog)?;
    proposals.family_schema_sha256 = action_schema_sha256;
    Ok(proposals)
}

pub(super) fn validate_parameterized_policy_catalog(
    catalog: &TacticAssetCatalog,
) -> Result<(), NativeTacticRouteRunError> {
    if let Some(entry) = catalog.entries().iter().find(|entry| {
        !entry.option_id().starts_with("family/")
            && !(entry.option_id().starts_with("promoted/")
                && entry.description().kind
                    == dusklight_learning::tactic_asset::TacticAssetKind::GuardedRecordedTape)
    }) {
        return Err(route_message(format!(
            "parameterized policy catalog contains non-atomic authored action {:?}",
            entry.option_id()
        )));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn applicable_parameterized_descriptors_for_state(
    campaign: &TacticQCampaign,
    registry: &FactRegistry,
    seed: u64,
    decision_index: u64,
    state: &FactSnapshot,
    encoder: &GoalConditionedTacticFeatureEncoder,
    maximum_ticks: u32,
    action_schema_sha256: Digest,
) -> Result<Vec<OptionActionDescriptor>, NativeTacticRouteRunError> {
    let proposals = parameterized_catalog_for_state(
        seed,
        decision_index,
        state,
        encoder,
        maximum_ticks,
        parameterized_feedback_for_state(campaign, state, encoder)?,
        action_schema_sha256,
    )?;
    let learner = LearnerState::build(
        state.clone(),
        registry,
        &proposals.catalog,
        &proposals.blueprints,
        |_| true,
    )
    .map_err(route_error)?;
    let descriptors = learner
        .action_mask
        .into_iter()
        .filter(|choice| choice.applicable)
        .map(|choice| choice.descriptor)
        .collect::<Vec<_>>();
    if descriptors.is_empty() {
        return Err(route_message(
            "frontier proposal catalog has no applicable executable tactics",
        ));
    }
    Ok(descriptors)
}

pub(super) fn parameterized_feedback_for_state(
    campaign: &TacticQCampaign,
    state: &FactSnapshot,
    encoder: &GoalConditionedTacticFeatureEncoder,
) -> Result<Option<ParameterizedTacticFeedback>, NativeTacticRouteRunError> {
    let state_sha256 = state.content_sha256().map_err(route_error)?;
    let Some(previous) = campaign
        .replay()
        .iter()
        .rev()
        .find(|transition| transition.after_state_sha256 == state_sha256)
    else {
        return Ok(None);
    };
    let goal = encoder.target_coordinate_f32_bits.map(f32::from_bits);
    let before = previous.before.player.position_f32_bits.map(f32::from_bits);
    let after = previous.after.player.position_f32_bits.map(f32::from_bits);
    let prior_occurrences = campaign
        .replay()
        .iter()
        .filter(|transition| transition.after_state_sha256 == state_sha256)
        .count();
    Ok(Some(ParameterizedTacticFeedback {
        previous_reward: previous.value_sample.reward,
        goal_progress: planar_distance(before, goal) - planar_distance(after, goal),
        ensemble_uncertainty: None,
        endpoint_novel: prior_occurrences == 1,
        terminal: previous.value_sample.terminal,
    }))
}

pub(super) struct TimedTacticWorker<'a, W> {
    inner: &'a mut W,
    native_elapsed: Duration,
    native_batch_elapsed: Duration,
    ipc_elapsed: Duration,
    observation_capture_elapsed: Duration,
    corpus_encoding_elapsed: Duration,
    pending_accounting: NativeTacticRestoreAccounting,
    prior_cache_hits: u64,
    prior_cache_misses: u64,
    prior_cache_evictions: u64,
}

impl<'a, W> TimedTacticWorker<'a, W> {
    fn new(inner: &'a mut W) -> Self {
        Self {
            inner,
            native_elapsed: Duration::ZERO,
            native_batch_elapsed: Duration::ZERO,
            ipc_elapsed: Duration::ZERO,
            observation_capture_elapsed: Duration::ZERO,
            corpus_encoding_elapsed: Duration::ZERO,
            pending_accounting: NativeTacticRestoreAccounting::default(),
            prior_cache_hits: 0,
            prior_cache_misses: 0,
            prior_cache_evictions: 0,
        }
    }

    fn take_accounting(&mut self) -> NativeTacticRestoreAccounting {
        let mut accounting = std::mem::take(&mut self.pending_accounting);
        accounting.refresh_rates();
        accounting
    }

    fn record_prefix_materialization(
        &mut self,
        route_frames: usize,
        fallback: bool,
        replay_elapsed: Duration,
    ) -> Result<(), NativeTacticWorkerError>
    where
        W: PersistentTacticBatchWorker,
    {
        let source_frame = usize::try_from(self.identity().source_frame)
            .map_err(|_| NativeTacticWorkerError::InvalidDuration)?;
        let replayed_prefix_ticks = u64::try_from(route_frames.saturating_sub(source_frame))
            .map_err(|_| NativeTacticWorkerError::InvalidDuration)?;
        self.pending_accounting.prefix_materializations = self
            .pending_accounting
            .prefix_materializations
            .saturating_add(1);
        self.pending_accounting.replayed_prefix_ticks = self
            .pending_accounting
            .replayed_prefix_ticks
            .saturating_add(replayed_prefix_ticks);
        self.pending_accounting.replay_restore_micros = self
            .pending_accounting
            .replay_restore_micros
            .saturating_add(elapsed_micros(replay_elapsed));
        if fallback {
            self.pending_accounting.direct_restore_fallback_replays = self
                .pending_accounting
                .direct_restore_fallback_replays
                .saturating_add(1);
        }
        self.pending_accounting.refresh_rates();
        Ok(())
    }

    fn record_route_replay(&mut self, route_frames: usize) -> Result<(), NativeTacticWorkerError>
    where
        W: PersistentTacticBatchWorker,
    {
        let source_frame = usize::try_from(self.identity().source_frame)
            .map_err(|_| NativeTacticWorkerError::InvalidDuration)?;
        let replayed_prefix_ticks = u64::try_from(route_frames.saturating_sub(source_frame))
            .map_err(|_| NativeTacticWorkerError::InvalidDuration)?;
        self.pending_accounting.replayed_prefix_ticks = self
            .pending_accounting
            .replayed_prefix_ticks
            .saturating_add(replayed_prefix_ticks);
        self.pending_accounting.refresh_rates();
        Ok(())
    }

    fn record_batch(
        &mut self,
        batch: &ValidatedNativeSuffixBatch,
    ) -> Result<(), NativeTacticWorkerError> {
        self.pending_accounting.native_requests =
            self.pending_accounting.native_requests.saturating_add(1);
        self.pending_accounting.restore_samples = self
            .pending_accounting
            .restore_samples
            .saturating_add(batch.restore_micros.len() as u64);
        let batch_restore_micros = batch
            .restore_micros
            .iter()
            .fold(0_u64, |total, micros| total.saturating_add(*micros));
        self.pending_accounting.restore_micros = self
            .pending_accounting
            .restore_micros
            .saturating_add(batch_restore_micros);
        let Some(cache) = batch.checkpoint_cache.as_ref() else {
            return Err(NativeTacticWorkerError::DetachedResult(
                "tactic batch cache accounting",
            ));
        };
        let expected_cache_hit = match cache.source_kind.as_str() {
            "authenticated_root_restore" => {
                self.pending_accounting.authenticated_root_restore_requests = self
                    .pending_accounting
                    .authenticated_root_restore_requests
                    .saturating_add(1);
                self.pending_accounting.authenticated_root_restore_micros = self
                    .pending_accounting
                    .authenticated_root_restore_micros
                    .saturating_add(batch_restore_micros);
                false
            }
            "direct_process_local_restore" => {
                self.pending_accounting
                    .direct_process_local_restore_requests = self
                    .pending_accounting
                    .direct_process_local_restore_requests
                    .saturating_add(1);
                self.pending_accounting.direct_process_local_restore_micros = self
                    .pending_accounting
                    .direct_process_local_restore_micros
                    .saturating_add(batch_restore_micros);
                true
            }
            "direct_process_local_continuation" => {
                self.pending_accounting
                    .direct_process_local_continuation_requests = self
                    .pending_accounting
                    .direct_process_local_continuation_requests
                    .saturating_add(1);
                self.pending_accounting.direct_process_local_restore_micros = self
                    .pending_accounting
                    .direct_process_local_restore_micros
                    .saturating_add(batch_restore_micros);
                false
            }
            _ => {
                return Err(NativeTacticWorkerError::DetachedResult(
                    "tactic batch restore kind",
                ));
            }
        };
        let cache_hits = cache.hits.checked_sub(self.prior_cache_hits).ok_or(
            NativeTacticWorkerError::DetachedResult("tactic cache hit counter regressed"),
        )?;
        let cache_misses = cache.misses.checked_sub(self.prior_cache_misses).ok_or(
            NativeTacticWorkerError::DetachedResult("tactic cache miss counter regressed"),
        )?;
        let cache_evictions = cache
            .evictions
            .checked_sub(self.prior_cache_evictions)
            .ok_or(NativeTacticWorkerError::DetachedResult(
                "tactic cache eviction counter regressed",
            ))?;
        self.prior_cache_hits = cache.hits;
        self.prior_cache_misses = cache.misses;
        self.prior_cache_evictions = cache.evictions;
        if cache_misses != 0 || expected_cache_hit != (cache_hits != 0) {
            return Err(NativeTacticWorkerError::DetachedResult(
                "tactic cache lookup accounting",
            ));
        }
        self.pending_accounting.cache_hits = self
            .pending_accounting
            .cache_hits
            .saturating_add(cache_hits);
        self.pending_accounting.cache_misses = self
            .pending_accounting
            .cache_misses
            .saturating_add(cache_misses);
        self.pending_accounting.cache_evictions = self
            .pending_accounting
            .cache_evictions
            .saturating_add(cache_evictions);
        self.pending_accounting.checkpoint_capture_attempts = self
            .pending_accounting
            .checkpoint_capture_attempts
            .saturating_add(cache.batch_capture_attempts);
        self.pending_accounting.checkpoint_capture_successes = self
            .pending_accounting
            .checkpoint_capture_successes
            .saturating_add(cache.batch_capture_successes);
        self.pending_accounting.checkpoint_capture_micros = self
            .pending_accounting
            .checkpoint_capture_micros
            .saturating_add(cache.batch_capture_micros);
        self.pending_accounting.checkpoint_image_reuse_attempts = self
            .pending_accounting
            .checkpoint_image_reuse_attempts
            .saturating_add(cache.batch_image_reuse_attempts);
        self.pending_accounting.checkpoint_image_reuse_successes = self
            .pending_accounting
            .checkpoint_image_reuse_successes
            .saturating_add(cache.batch_image_reuse_successes);
        self.pending_accounting.live_endpoint_retention_attempts = self
            .pending_accounting
            .live_endpoint_retention_attempts
            .saturating_add(cache.batch_live_retention_attempts);
        self.pending_accounting.live_endpoint_retention_successes = self
            .pending_accounting
            .live_endpoint_retention_successes
            .saturating_add(cache.batch_live_retention_successes);
        self.pending_accounting.live_endpoint_retention_nanos = self
            .pending_accounting
            .live_endpoint_retention_nanos
            .saturating_add(cache.batch_live_retention_nanos);
        self.pending_accounting.peak_resident_entries = self
            .pending_accounting
            .peak_resident_entries
            .max(cache.resident_entries);
        self.pending_accounting.peak_resident_bytes = self
            .pending_accounting
            .peak_resident_bytes
            .max(cache.resident_bytes);
        self.pending_accounting.peak_resident_checkpoint_bytes = self
            .pending_accounting
            .peak_resident_checkpoint_bytes
            .max(cache.resident_checkpoint_bytes);
        self.pending_accounting.peak_resident_host_snapshot_bytes = self
            .pending_accounting
            .peak_resident_host_snapshot_bytes
            .max(cache.resident_host_snapshot_bytes);
        self.pending_accounting.peak_live_endpoint_entries = self
            .pending_accounting
            .peak_live_endpoint_entries
            .max(cache.live_endpoint_resident_entries);
        self.pending_accounting
            .peak_live_endpoint_host_snapshot_bytes = self
            .pending_accounting
            .peak_live_endpoint_host_snapshot_bytes
            .max(cache.live_endpoint_resident_host_snapshot_bytes);
        Ok(())
    }
}

impl TimedTacticWorker<'_, NativeSuffixWorkerSession> {
    fn suspend_process(&mut self) -> Result<(), NativeTacticRouteRunError> {
        self.inner.suspend_process().map_err(route_error)
    }

    fn resume_process(&mut self) -> Result<(), NativeTacticRouteRunError> {
        self.inner.resume_process().map_err(route_error)
    }
}

impl<W: PersistentTacticBatchWorker> PersistentTacticBatchWorker for TimedTacticWorker<'_, W> {
    fn identity(&self) -> &crate::native_suffix_worker::NativeSuffixWorkerIdentity {
        self.inner.identity()
    }

    fn run_tactic_batch(
        &mut self,
        request: &Path,
        result: &Path,
        batch: &dusklight_search::suffix_batch::NativeSuffixBatch,
    ) -> Result<ValidatedNativeSuffixBatch, NativeTacticWorkerError> {
        let started = Instant::now();
        let response = self.inner.run_tactic_batch(request, result, batch);
        let round_trip = started.elapsed();
        match response {
            Ok(batch) => {
                let batch_wall = Duration::from_micros(batch.timing.batch_wall_micros);
                self.native_batch_elapsed = self.native_batch_elapsed.saturating_add(batch_wall);
                self.native_elapsed = self
                    .native_elapsed
                    .saturating_add(Duration::from_micros(batch.timing.simulation_micros));
                self.ipc_elapsed = self
                    .ipc_elapsed
                    .saturating_add(round_trip.saturating_sub(batch_wall));
                self.observation_capture_elapsed =
                    self.observation_capture_elapsed
                        .saturating_add(Duration::from_micros(
                            batch.timing.observation_capture_micros,
                        ));
                self.corpus_encoding_elapsed = self
                    .corpus_encoding_elapsed
                    .saturating_add(Duration::from_micros(batch.timing.corpus_encoding_micros));
                self.record_batch(&batch)?;
                Ok(batch)
            }
            Err(error) => {
                self.ipc_elapsed = self.ipc_elapsed.saturating_add(round_trip);
                if error.is_missing_process_local_checkpoint() {
                    self.prior_cache_misses = self.prior_cache_misses.saturating_add(1);
                    self.pending_accounting.cache_misses =
                        self.pending_accounting.cache_misses.saturating_add(1);
                    self.pending_accounting.refresh_rates();
                }
                Err(error)
            }
        }
    }
}

pub(super) struct NativeTacticProposalJob {
    execution_plan_sha256: Digest,
    proposals: Vec<IndexedNativeTacticProposal>,
    proposal_catalog: Arc<dusklight_learning::tactic_asset::TacticAssetCatalog>,
    proposal_blueprints: Arc<Vec<TacticBlueprint>>,
    source_snapshot: FactSnapshot,
    source_route_tape: InputTape,
    restoration: Option<TacticRestorationContract>,
    checkpoint_source: Option<NativeTacticCheckpointSource>,
    materialize_frontier: bool,
    primary_retention: NativeTacticCheckpointRetention,
    execution_strategy: NativeGenericExecutionStrategy,
    checkpoint_cache_capacity_bytes: usize,
    paths_root: PathBuf,
    queued_at: Instant,
    execution_started: mpsc::SyncSender<()>,
    response: mpsc::SyncSender<Result<Vec<NativeTacticProposalWork>, NativeTacticRouteRunError>>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct NativeTacticWorkerShutdownMetrics {
    pub(super) proposal_jobs: u64,
    pub(super) native_process_cpu_micros: Option<u64>,
    pub(super) worker_capacity_micros: u64,
    pub(super) worker_busy_micros: u64,
    pub(super) proposal_queue_wait_micros: u64,
}

pub(super) struct IndexedNativeTacticProposal {
    proposal_index: usize,
    selected: SelectedTactic,
}

pub(super) struct NativeTacticProposalWork {
    pub(super) execution_plan_sha256: Digest,
    pub(super) worker_slot: usize,
    pub(super) outcome: NativeTacticWorkerOutcome,
    pub(super) native_elapsed: Duration,
    pub(super) ipc_elapsed: Duration,
    pub(super) observation_capture_elapsed: Duration,
    pub(super) corpus_encoding_elapsed: Duration,
    pub(super) preparation_elapsed: Duration,
    pub(super) restore_accounting: NativeTacticRestoreAccounting,
}

#[derive(Clone)]
pub(super) struct NativeTacticProposalPool {
    pub(super) senders: Arc<Vec<mpsc::Sender<NativeTacticProposalJob>>>,
    pub(super) next_worker: Arc<AtomicUsize>,
    pub(super) direct_restore_enabled: bool,
    pub(super) root_source_frame: usize,
    pub(super) execution_strategy: NativeGenericExecutionStrategy,
    pub(super) execution_plan_sha256: Digest,
    pub(super) checkpoint_cache_capacity_bytes: usize,
    /// A sufficiently wide multi-lane fleet dedicates one stable worker to
    /// each concurrently executing lane. Counterfactual siblings use only
    /// the remaining workers, so they cannot consume another lane's live
    /// endpoint between decisions.
    pub(super) dedicated_owner_slots: usize,
    pub(super) preferred_owner_slot: Option<usize>,
}

#[derive(Clone, Debug)]
pub(super) struct CachedTacticFrontier {
    pub(super) worker_slot: usize,
    pub(super) source: NativeTacticCheckpointSource,
    pub(super) state_sha256: Digest,
    pub(super) route_frames: usize,
    pub(super) route_checkpoint_sha256: Digest,
    pub(super) route_tape_sha256: Digest,
}

fn primary_checkpoint_retention(retain: bool) -> NativeTacticCheckpointRetention {
    if !retain {
        NativeTacticCheckpointRetention::None
    } else {
        NativeTacticCheckpointRetention::LiveEndpoint
    }
}

fn next_worker_excluding(
    next_worker: &AtomicUsize,
    worker_count: usize,
    excluded: Option<usize>,
) -> usize {
    loop {
        let worker = next_worker.fetch_add(1, Ordering::Relaxed) % worker_count;
        if worker_count == 1 || Some(worker) != excluded {
            return worker;
        }
    }
}

fn dedicated_owner_slot_count(
    worker_count: usize,
    concurrent_lanes: usize,
    proposal_width: usize,
    direct_restore_enabled: bool,
) -> usize {
    let required_workers = concurrent_lanes.saturating_mul(proposal_width);
    if direct_restore_enabled
        && concurrent_lanes > 0
        && proposal_width > 0
        && worker_count >= required_workers
    {
        concurrent_lanes
    } else {
        0
    }
}

mod pool;
#[cfg(test)]
use pool::proposal_artifact_root;
#[cfg(test)]
pub(super) use pool::recorded_demonstration_chunks;
pub(super) use pool::{
    load_existing_demonstration, load_or_capture_demonstration, run_tactic_proposal_worker,
};

#[cfg(test)]
#[path = "worker_pool_tests.rs"]
mod tests;
