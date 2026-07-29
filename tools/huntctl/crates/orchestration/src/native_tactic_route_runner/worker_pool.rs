use super::*;

mod restoration_dispatch;

use restoration_dispatch::{requires_frontier_materialization, validate_restoration_contract};

pub(super) fn launch_tactic_route_worker(
    config: &NativeTacticRouteRunConfig<'_>,
    repository_root: &Path,
    attempt_root: &Path,
    initial_batch: &NativeSuffixBatch,
    terminal: &NativeTerminalBinding,
    card_fixture: &Path,
) -> Result<(NativeSuffixWorkerSession, FactSnapshot, Digest), NativeTacticRouteRunError> {
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
    let (worker, initial) = NativeSuffixWorkerSession::launch_with_prevalidated_files(
        &launch,
        NativeSuffixPrevalidatedFileIdentities {
            executable_sha256: config.execution.executable.sha256,
            game_data_sha256: config.execution.game_data.sha256,
        },
    )
    .map_err(route_error)?;
    let facts = initial_facts(&initial)?;
    let root_checkpoint_sha256 =
        tactic_root_checkpoint_sha256(worker.identity()).map_err(route_error)?;
    Ok((worker, facts, root_checkpoint_sha256))
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
        let result =
            read_completed_seed_result(
                &seed_result_path,
                seed,
                config.execution_plan.budgets.decisions_per_lane,
                config.execution_plan.identity()?,
                config.execution_plan.lanes.get(seed_index).ok_or_else(|| {
                    route_message("tactic seed lane is absent from execution plan")
                })?,
            )?;
        let generated_training = load_generated_training_corpus(&result)?;
        Ok(CompletedNativeTacticSeed {
            result,
            generated_training,
        })
    } else {
        let completion = run_seed(
            config,
            pool,
            registry,
            encoder,
            reward_spec,
            initial_facts,
            route_prefix,
            action_schema_sha256,
            promoted_tactics,
            root_checkpoint_sha256,
            root_tape_ref,
            inherited_learner_snapshot,
            live_learner,
            seed_index,
            seed,
        )?;
        write_new(
            &seed_result_path,
            &serde_json::to_vec_pretty(&completion.result).map_err(route_error)?,
        )?;
        Ok(completion)
    }
}

pub(super) fn load_generated_training_corpus(
    result: &NativeTacticSeedResult,
) -> Result<TacticQTrainingCorpus, NativeTacticRouteRunError> {
    let corpus = TacticQCampaign::read_checkpoint(Path::new(&result.final_checkpoint))
        .and_then(|campaign| campaign.training_corpus_from(result.imported_training_replay_rows))
        .map_err(route_error)?;
    if corpus.execution_authority_sha256 != result.execution_plan_sha256 {
        return Err(route_message(
            "generated tactic training corpus belongs to another execution plan",
        ));
    }
    Ok(corpus)
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
                    == dusklight_learning::tactic_asset::TacticAssetKind::RecordedTape)
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
        .replay
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
        .replay
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
    paths_root: PathBuf,
    response: mpsc::SyncSender<Result<Vec<NativeTacticProposalWork>, NativeTacticRouteRunError>>,
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

fn direct_frontier_eligible(
    direct_restore_enabled: bool,
    worker_count: usize,
    frontier: &CachedTacticFrontier,
) -> bool {
    direct_restore_enabled && frontier.worker_slot < worker_count
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

impl NativeTacticProposalPool {
    pub(super) fn execute_batch(
        &self,
        proposals: &[SelectedTactic],
        proposal_catalog: Arc<dusklight_learning::tactic_asset::TacticAssetCatalog>,
        proposal_blueprints: Arc<Vec<TacticBlueprint>>,
        source_snapshot: &FactSnapshot,
        source_route_tape: &InputTape,
        restoration: Option<&TacticRestorationContract>,
        cached_frontier: Option<&CachedTacticFrontier>,
        retain_primary_checkpoint: bool,
        paths_root: &Path,
    ) -> Result<Vec<NativeTacticProposalWork>, NativeTacticRouteRunError> {
        if self.senders.is_empty() {
            return Err(route_message("native tactic proposal pool is empty"));
        }
        if proposals.is_empty() {
            return Err(route_message("native tactic proposal batch is empty"));
        }
        if let Some(restoration) = restoration {
            validate_restoration_contract(restoration, source_snapshot, source_route_tape)?;
        }
        let primary_retention = primary_checkpoint_retention(retain_primary_checkpoint);
        let replayed_prefix = source_route_tape
            .frames
            .len()
            .checked_sub(self.root_source_frame)
            .ok_or_else(|| route_message("tactic route precedes its authenticated root"))?;
        let direct = (replayed_prefix != 0 && restoration.is_some())
            .then(|| {
                cached_frontier.filter(|frontier| {
                    direct_frontier_eligible(
                        self.direct_restore_enabled,
                        self.senders.len(),
                        frontier,
                    ) && restoration.is_some_and(|contract| {
                        frontier.state_sha256 == contract.plan.expected_state_sha256
                            && frontier.route_frames == source_route_tape.frames.len()
                            && frontier.route_checkpoint_sha256
                                == contract.plan.route.route_checkpoint_sha256
                            && frontier.route_tape_sha256 == contract.plan.route.tape_sha256
                    })
                })
            })
            .flatten();
        let mut responses = Vec::with_capacity(proposals.len());
        for proposal_index in (1..proposals.len()).chain(std::iter::once(0)) {
            let selected = &proposals[proposal_index];
            let (response, receiver) = mpsc::sync_channel(1);
            let primary_source = (proposal_index == 0).then_some(direct).flatten();
            let worker_slot = primary_source.map_or_else(
                || {
                    next_worker_excluding(
                        &self.next_worker,
                        self.senders.len(),
                        direct.map(|frontier| frontier.worker_slot),
                    )
                },
                |frontier| frontier.worker_slot,
            );
            self.senders[worker_slot]
                .send(NativeTacticProposalJob {
                    execution_plan_sha256: self.execution_plan_sha256,
                    proposals: vec![IndexedNativeTacticProposal {
                        proposal_index,
                        selected: selected.clone(),
                    }],
                    proposal_catalog: Arc::clone(&proposal_catalog),
                    proposal_blueprints: Arc::clone(&proposal_blueprints),
                    source_snapshot: source_snapshot.clone(),
                    source_route_tape: source_route_tape.clone(),
                    restoration: restoration.cloned(),
                    checkpoint_source: primary_source.map(|frontier| frontier.source.clone()),
                    materialize_frontier: requires_frontier_materialization(
                        restoration.is_some(),
                        replayed_prefix,
                        primary_source.is_some(),
                    ),
                    primary_retention: if proposal_index == 0 {
                        primary_retention
                    } else {
                        NativeTacticCheckpointRetention::None
                    },
                    execution_strategy: self.execution_strategy,
                    paths_root: paths_root.to_path_buf(),
                    response,
                })
                .map_err(|_| route_message("native tactic proposal pool stopped"))?;
            responses.push((proposal_index, receiver));
        }
        let mut work = responses
            .into_iter()
            .map(|(proposal_index, receiver)| {
                let mut work = receiver
                    .recv()
                    .map_err(|_| route_message("native tactic proposal worker stopped"))??;
                let work = work
                    .pop()
                    .ok_or_else(|| route_message("native tactic proposal result is absent"))?;
                Ok((proposal_index, work))
            })
            .collect::<Result<Vec<_>, NativeTacticRouteRunError>>()?;
        work.sort_by_key(|(proposal_index, _)| *proposal_index);
        Ok(work.into_iter().map(|(_, work)| work).collect())
    }
}

pub(super) struct CapturedTerminalRouteReplay {
    pub(super) corpus: TacticQTrainingCorpus,
    pub(super) route: InputTape,
    pub(super) first_hit_tick: u64,
    pub(super) native_ticks: u64,
    pub(super) wall_micros: u64,
    pub(super) native_simulation_micros: u64,
    pub(super) ipc_and_result_transport_micros: u64,
    pub(super) native_observation_capture_micros: u64,
    pub(super) native_corpus_encoding_micros: u64,
    pub(super) rust_state_extraction_micros: u64,
    pub(super) preparation_micros: u64,
    pub(super) restore_accounting: NativeTacticRestoreAccounting,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn capture_terminal_route_replay(
    config: &NativeTacticRouteRunConfig<'_>,
    pool: &NativeTacticProposalPool,
    encoder: &GoalConditionedTacticFeatureEncoder,
    reward_spec: &TacticRewardSpec,
    recorded_route: &InputTape,
    initial_facts: &FactSnapshot,
    route_prefix: &InputTape,
    root_checkpoint_sha256: Digest,
    chunk_ticks: u32,
    episode_group: u64,
    option_namespace: &str,
    artifact_root: &Path,
) -> Result<CapturedTerminalRouteReplay, NativeTacticRouteRunError> {
    let started = Instant::now();
    let chunks = recorded_demonstration_chunks(
        recorded_route,
        pool.root_source_frame,
        chunk_ticks,
        config.optimization.budgets.exploration_horizon_ticks,
    )?;
    let mut before = initial_facts.clone();
    let mut route = route_prefix.clone();
    let mut transitions = Vec::new();
    let mut routes = Vec::new();
    let mut episode_groups = Vec::new();
    let mut native_ticks = 0_u64;
    let mut native_simulation_micros = 0_u64;
    let mut ipc_and_result_transport_micros = 0_u64;
    let mut native_observation_capture_micros = 0_u64;
    let mut native_corpus_encoding_micros = 0_u64;
    let mut rust_state_extraction_micros = 0_u64;
    let mut preparation_micros = 0_u64;
    let mut restore_accounting = NativeTacticRestoreAccounting::default();
    let mut first_hit_tick = None;

    for (chunk_index, chunk) in chunks.into_iter().enumerate() {
        if cancellation_requested(config) {
            return Err(route_cancelled("native terminal-route replay cancelled"));
        }
        let option_id = format!("{option_namespace}/chunk-{chunk_index:04}");
        let entry = TacticCatalogEntry::new(option_id, TacticAssetSource::RecordedTape(chunk))
            .map_err(route_error)?;
        let catalog = Arc::new(TacticAssetCatalog::new(vec![entry]).map_err(route_error)?);
        let descriptor = catalog
            .option_descriptors()
            .next()
            .cloned()
            .ok_or_else(|| route_message("terminal-route replay catalog is empty"))?;
        let selected = SelectedTactic {
            schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
            learner_snapshot_sha256: before.content_sha256().map_err(route_error)?,
            decision_index: chunk_index as u64,
            descriptor,
            reason: TacticSelectionReason::StructuredBaseline,
            exploration_draw: 0,
        };
        let paths_root = artifact_root
            .join("native")
            .join(format!("chunk-{chunk_index:04}"));
        fs::create_dir_all(&paths_root).map_err(route_error)?;
        let mut work = pool.execute_batch(
            std::slice::from_ref(&selected),
            catalog,
            Arc::new(Vec::new()),
            &before,
            &route,
            None,
            None,
            false,
            &paths_root,
        )?;
        if work.len() != 1 {
            return Err(route_message(
                "terminal-route replay chunk did not produce one native outcome",
            ));
        }
        let work = work.remove(0);
        native_simulation_micros =
            native_simulation_micros.saturating_add(elapsed_micros(work.native_elapsed));
        ipc_and_result_transport_micros =
            ipc_and_result_transport_micros.saturating_add(elapsed_micros(work.ipc_elapsed));
        native_observation_capture_micros = native_observation_capture_micros
            .saturating_add(elapsed_micros(work.observation_capture_elapsed));
        native_corpus_encoding_micros = native_corpus_encoding_micros
            .saturating_add(elapsed_micros(work.corpus_encoding_elapsed));
        rust_state_extraction_micros =
            rust_state_extraction_micros.saturating_add(work.outcome.state_extraction_micros);
        preparation_micros =
            preparation_micros.saturating_add(elapsed_micros(work.preparation_elapsed));
        restore_accounting.merge(&work.restore_accounting);
        restore_accounting.proposal_transitions =
            restore_accounting.proposal_transitions.saturating_add(1);

        let outcome = work.outcome;
        let state = encoder.encode(&before).map_err(route_error)?;
        let next_state = encoder.encode(&outcome.next_facts).map_err(route_error)?;
        let reward = reward_spec
            .evaluate_with_motion(
                encoder.schema_sha256,
                &state,
                &next_state,
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
            route_checkpoint(root_checkpoint_sha256, &route).map_err(route_error)?;
        let next_checkpoint_sha256 =
            route_checkpoint(root_checkpoint_sha256, &outcome.route_tape).map_err(route_error)?;
        let mut transition = OptionTransitionSample::capture(
            encoder.schema_sha256,
            source_checkpoint_sha256,
            next_checkpoint_sha256,
            before,
            outcome.next_facts.clone(),
            outcome.execution.clone(),
            &outcome.route_tape,
            reward.training_reward,
            outcome.terminal,
            |facts| encoder.encode(facts),
        )
        .map_err(route_error)?;
        transition.intermediate_boundaries = outcome.intermediate_boundaries;
        transition.execution_authority_sha256 = config.execution_plan.identity()?;
        transition.validate().map_err(route_error)?;
        native_ticks =
            native_ticks.saturating_add(u64::from(outcome.execution.duration.realized_ticks));
        let outcome_first_hit_tick = outcome
            .terminal
            .then(|| {
                outcome
                    .next_facts
                    .terminal
                    .first_hit_tick
                    .and_then(|tick| tick.checked_sub(initial_facts.simulation_tick))
            })
            .flatten();
        before = outcome.next_facts;
        route = outcome.route_tape;
        transitions.push(transition);
        routes.push(route.clone());
        episode_groups.push(episode_group);
        if outcome.terminal {
            first_hit_tick = outcome_first_hit_tick;
            restore_accounting.useful_transitions =
                restore_accounting.useful_transitions.saturating_add(1);
            restore_accounting.refresh_rates();
            break;
        }
    }

    if transitions
        .last()
        .is_none_or(|transition| !transition.value_sample.terminal)
    {
        return Err(route_message(
            "authenticated recorded route did not reach the terminal",
        ));
    }
    let route_native_ticks = route
        .frames
        .len()
        .checked_sub(pool.root_source_frame)
        .ok_or_else(|| route_message("terminal route precedes its source"))?
        as u64;
    let first_hit_tick = first_hit_tick.ok_or_else(|| {
        route_message("terminal-route replay has no source-relative first-hit tick")
    })?;
    if route_native_ticks != native_ticks || native_ticks != first_hit_tick.saturating_add(1) {
        return Err(route_message(
            "terminal-route replay ticks are detached from its first hit",
        ));
    }
    let expected_end = pool
        .root_source_frame
        .checked_add(usize::try_from(native_ticks).map_err(route_error)?)
        .ok_or_else(|| route_message("terminal-route replay first hit overflows"))?;
    if recorded_route.frames.get(..expected_end) != Some(route.frames.as_slice()) {
        return Err(route_message(
            "native terminal-route replay differs from the authenticated tape",
        ));
    }
    let corpus = TacticQTrainingCorpus {
        execution_authority_sha256: config.execution_plan.identity()?,
        feature_schema_sha256: encoder.schema_sha256,
        objective_sha256: config.optimization.terminal_predicate.definition_sha256,
        root_checkpoint_sha256,
        transitions,
        routes,
        episode_groups,
    };
    validate_training_corpus(&corpus).map_err(route_error)?;
    Ok(CapturedTerminalRouteReplay {
        corpus,
        route,
        first_hit_tick,
        native_ticks,
        wall_micros: elapsed_micros(started.elapsed()),
        native_simulation_micros,
        ipc_and_result_transport_micros,
        native_observation_capture_micros,
        native_corpus_encoding_micros,
        rust_state_extraction_micros,
        preparation_micros,
        restore_accounting,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn load_or_capture_demonstration(
    config: &NativeTacticRouteRunConfig<'_>,
    pool: &NativeTacticProposalPool,
    encoder: &GoalConditionedTacticFeatureEncoder,
    reward_spec: &TacticRewardSpec,
    process_tape: &InputTape,
    initial_facts: &FactSnapshot,
    route_prefix: &InputTape,
    root_checkpoint_sha256: Digest,
) -> Result<Option<NativeTacticDemonstration>, NativeTacticRouteRunError> {
    let corpus_path = config
        .output_root
        .join(NATIVE_TACTIC_DEMONSTRATION_CORPUS_FILE);
    let objects_root = config
        .output_root
        .join(NATIVE_TACTIC_CONTENT_STORE_DIRECTORY);
    let report_path = config
        .output_root
        .join(NATIVE_TACTIC_DEMONSTRATION_REPORT_FILE);

    if report_path.exists() {
        if !config.resume || !corpus_path.exists() {
            return Err(route_message(
                "demonstration evidence exists outside a resumable route run",
            ));
        }
        let report: NativeTacticDemonstrationReport = read_bounded_json(&report_path)?;
        let corpus_bytes = fs::read(&corpus_path).map_err(route_error)?;
        let corpus_sha256 = Digest(Sha256::digest(&corpus_bytes).into());
        let corpus = TacticQTrainingCorpus::read(&corpus_path).map_err(route_error)?;
        let configured_chunk_matches = config
            .execution_plan
            .demonstration_chunk_ticks
            .is_none_or(|ticks| ticks == report.chunk_ticks);
        let demonstrated_route = corpus
            .routes
            .last()
            .ok_or_else(|| route_message("demonstration corpus is empty"))?;
        let expected_route_end = usize::try_from(report.native_ticks)
            .ok()
            .and_then(|ticks| pool.root_source_frame.checked_add(ticks));
        let demonstrated_route_sha256 =
            Digest(Sha256::digest(&demonstrated_route.encode().map_err(route_error)?).into());
        if report.schema != NATIVE_TACTIC_DEMONSTRATION_REPORT_SCHEMA_V1
            || report.optimization_request_sha256 != config.optimization.content_sha256
            || report.execution_binding_sha256 != config.execution.content_sha256
            || report.objective_sha256 != config.optimization.terminal_predicate.definition_sha256
            || report.feature_schema_sha256 != encoder.schema_sha256
            || report.root_checkpoint_sha256 != root_checkpoint_sha256
            || config.execution_plan.proposal_policy != TacticProposalPolicy::Learned
            || report.source_boundary_index != config.optimization.route.source_boundary_index
            || !configured_chunk_matches
            || report.transition_count == 0
            || report.transition_count != corpus.transitions.len() as u64
            || report.transition_count != corpus.routes.len() as u64
            || report.first_hit_tick >= config.optimization.budgets.exploration_horizon_ticks
            || report.native_ticks != report.first_hit_tick.saturating_add(1)
            || report.corpus_path != path_text(&corpus_path)
            || report.corpus_sha256 != corpus_sha256
            || report.demonstrated_route_tape_sha256 != demonstrated_route_sha256
            || expected_route_end.is_none_or(|end| {
                process_tape.frames.get(..end) != Some(demonstrated_route.frames.as_slice())
            })
            || corpus.feature_schema_sha256 != encoder.schema_sha256
            || corpus.objective_sha256 != config.optimization.terminal_predicate.definition_sha256
            || corpus.root_checkpoint_sha256 != root_checkpoint_sha256
            || corpus
                .transitions
                .last()
                .is_none_or(|transition| !transition.value_sample.terminal)
            || !demonstration_corpus_is_attached(
                &corpus,
                pool.root_source_frame,
                &report,
                reward_spec,
            )
        {
            return Err(route_message(
                "resumable demonstration evidence is detached from this route run",
            ));
        }
        return Ok(Some(NativeTacticDemonstration { corpus, report }));
    }

    if corpus_path.exists() {
        return Err(route_message(
            "incomplete demonstration evidence cannot be resumed",
        ));
    }
    let Some(chunk_ticks) = config.execution_plan.demonstration_chunk_ticks else {
        return Ok(None);
    };

    let captured = capture_terminal_route_replay(
        config,
        pool,
        encoder,
        reward_spec,
        process_tape,
        initial_facts,
        route_prefix,
        root_checkpoint_sha256,
        chunk_ticks,
        TACTIC_Q_DEMONSTRATION_EPISODE_GROUP,
        "demonstration",
        &config.output_root.join("demonstration"),
    )?;
    let corpus = captured.corpus;
    let route = captured.route;
    let first_hit_tick = captured.first_hit_tick;
    let native_ticks = captured.native_ticks;
    corpus
        .write(&corpus_path, &objects_root)
        .map_err(route_error)?;
    let corpus_sha256 =
        Digest(Sha256::digest(&fs::read(&corpus_path).map_err(route_error)?).into());
    let demonstrated_route_tape_sha256 =
        Digest(Sha256::digest(&route.encode().map_err(route_error)?).into());
    let report = NativeTacticDemonstrationReport {
        schema: NATIVE_TACTIC_DEMONSTRATION_REPORT_SCHEMA_V1.into(),
        optimization_request_sha256: config.optimization.content_sha256,
        execution_binding_sha256: config.execution.content_sha256,
        objective_sha256: config.optimization.terminal_predicate.definition_sha256,
        feature_schema_sha256: encoder.schema_sha256,
        root_checkpoint_sha256,
        source_boundary_index: config.optimization.route.source_boundary_index,
        chunk_ticks,
        transition_count: corpus.transitions.len() as u64,
        first_hit_tick,
        native_ticks,
        wall_micros: captured.wall_micros,
        native_simulation_micros: captured.native_simulation_micros,
        ipc_and_result_transport_micros: captured.ipc_and_result_transport_micros,
        native_observation_capture_micros: captured.native_observation_capture_micros,
        native_corpus_encoding_micros: captured.native_corpus_encoding_micros,
        rust_state_extraction_micros: captured.rust_state_extraction_micros,
        preparation_micros: captured.preparation_micros,
        restore_accounting: captured.restore_accounting,
        corpus_path: path_text(&corpus_path),
        corpus_sha256,
        demonstrated_route_tape_sha256,
    };
    if !demonstration_corpus_is_attached(&corpus, pool.root_source_frame, &report, reward_spec) {
        return Err(route_message(
            "captured demonstration corpus is internally detached",
        ));
    }
    write_new(
        &report_path,
        &serde_json::to_vec_pretty(&report).map_err(route_error)?,
    )?;
    Ok(Some(NativeTacticDemonstration { corpus, report }))
}

pub(super) fn demonstration_corpus_is_attached(
    corpus: &TacticQTrainingCorpus,
    source_frame: usize,
    report: &NativeTacticDemonstrationReport,
    reward_spec: &TacticRewardSpec,
) -> bool {
    if corpus.transitions.is_empty()
        || corpus.transitions.len() != corpus.routes.len()
        || corpus.transitions.len() != corpus.episode_groups.len()
        || corpus
            .episode_groups
            .iter()
            .any(|group| *group != TACTIC_Q_DEMONSTRATION_EPISODE_GROUP)
    {
        return false;
    }
    let mut previous_after = None;
    let mut previous_checkpoint = None;
    let mut realized_ticks = 0_u64;
    for (index, (transition, route)) in corpus.transitions.iter().zip(&corpus.routes).enumerate() {
        let is_last = index + 1 == corpus.transitions.len();
        let expected_reward = (if transition.value_sample.terminal {
            f64::from(reward_spec.terminal_reward)
        } else {
            0.0
        } - f64::from(reward_spec.tick_cost)
            * f64::from(transition.value_sample.duration_ticks))
            as f32;
        let action = &transition.value_sample.action;
        let button_mask = match action.parameters.get("command_button_mask") {
            Some(OptionParameter::Unsigned(mask)) => *mask as u16,
            _ => return false,
        };
        let has_movement = match action.parameters.get("command_has_movement") {
            Some(OptionParameter::Bool(has_movement)) => *has_movement,
            _ => return false,
        };
        let expected_type = if button_mask & 0x0100 != 0 {
            OptionType::Roll
        } else if has_movement {
            OptionType::Move
        } else if button_mask == 0 {
            OptionType::Neutral
        } else {
            OptionType::Custom("recorded_tape".into())
        };
        if transition.value_sample.terminal != is_last
            || transition.value_sample.reward.to_bits() != expected_reward.to_bits()
            || action.option_id != format!("demonstration/chunk-{index:04}")
            || !matches!(
                action.parameters.get("input_tape_sha256"),
                Some(OptionParameter::Digest(_))
            )
            || action.option_type != expected_type
            || transition.execution.realized_tape_range.start_frame
                != if index == 0 {
                    source_frame as u64
                } else {
                    corpus.transitions[index - 1]
                        .execution
                        .realized_tape_range
                        .end_frame_exclusive
                }
            || transition.execution.realized_tape_range.end_frame_exclusive
                != route.frames.len() as u64
            || previous_after.is_some_and(|after| transition.before_state_sha256 != after)
            || previous_checkpoint
                .is_some_and(|checkpoint| transition.source_checkpoint_sha256 != checkpoint)
        {
            return false;
        }
        realized_ticks =
            realized_ticks.saturating_add(u64::from(transition.value_sample.duration_ticks));
        previous_after = Some(transition.after_state_sha256);
        previous_checkpoint = Some(transition.next_checkpoint_sha256);
    }
    corpus.routes.last().is_some_and(|route| {
        route.frames.len().saturating_sub(source_frame) as u64 == report.native_ticks
    }) && realized_ticks == report.native_ticks
}

pub(super) fn recorded_demonstration_chunks(
    process_tape: &InputTape,
    source_frame: usize,
    chunk_ticks: u32,
    horizon_ticks: u64,
) -> Result<Vec<InputTape>, NativeTacticRouteRunError> {
    let chunk_ticks = usize::try_from(chunk_ticks).map_err(route_error)?;
    let horizon_ticks = usize::try_from(horizon_ticks).map_err(route_error)?;
    if chunk_ticks == 0 || horizon_ticks == 0 {
        return Err(route_message(
            "demonstration chunk and horizon must be nonzero",
        ));
    }
    let suffix = process_tape
        .frames
        .get(source_frame..)
        .ok_or_else(|| route_message("demonstration source exceeds the process tape"))?;
    let bounded = &suffix[..suffix.len().min(horizon_ticks)];
    if bounded.is_empty() {
        return Err(route_message("demonstration process-tape suffix is empty"));
    }
    Ok(bounded
        .chunks(chunk_ticks)
        .map(|frames| InputTape {
            boot: process_tape.boot.clone(),
            tick_rate_numerator: process_tape.tick_rate_numerator,
            tick_rate_denominator: process_tape.tick_rate_denominator,
            frames: frames.to_vec(),
        })
        .collect())
}

pub(super) fn run_tactic_proposal_worker(
    worker_slot: usize,
    mut worker: NativeSuffixWorkerSession,
    receiver: mpsc::Receiver<NativeTacticProposalJob>,
) -> Result<(), NativeTacticRouteRunError> {
    let mut timed_worker = TimedTacticWorker::new(&mut worker);
    loop {
        let job = receiver.recv();
        let Ok(mut job) = job else {
            break;
        };
        let batch_started = Instant::now();
        let native_before_batch = timed_worker.native_elapsed;
        let native_batch_before_batch = timed_worker.native_batch_elapsed;
        let ipc_before_batch = timed_worker.ipc_elapsed;
        let observation_before_batch = timed_worker.observation_capture_elapsed;
        let corpus_before_batch = timed_worker.corpus_encoding_elapsed;
        let Some(first_proposal_index) = job
            .proposals
            .first()
            .map(|proposal| proposal.proposal_index)
        else {
            let _ = job
                .response
                .send(Err(route_message("native tactic proposal job is empty")));
            continue;
        };
        let checkpoint_source = if job.materialize_frontier {
            materialize_job_frontier(
                &mut timed_worker,
                &job,
                first_proposal_index,
                "frontier-source",
                false,
            )
            .map(Some)
        } else {
            Ok(job.checkpoint_source.clone())
        };
        let mut checkpoint_source = match checkpoint_source {
            Ok(source) => source,
            Err(error) => {
                let _ = job.response.send(Err(route_error(error)));
                continue;
            }
        };

        let mut work = Vec::with_capacity(job.proposals.len());
        let mut failed = None;
        let proposals = std::mem::take(&mut job.proposals);
        for (batch_index, proposal) in proposals.into_iter().enumerate() {
            let proposal_root = proposal_artifact_root(&job.paths_root, proposal.proposal_index);
            if let Err(error) = fs::create_dir_all(&proposal_root).map_err(route_error) {
                failed = Some(error);
                break;
            }
            let execution_started = if batch_index == 0 {
                batch_started
            } else {
                Instant::now()
            };
            let native_before = if batch_index == 0 {
                native_before_batch
            } else {
                timed_worker.native_elapsed
            };
            let native_batch_before = if batch_index == 0 {
                native_batch_before_batch
            } else {
                timed_worker.native_batch_elapsed
            };
            let ipc_before = if batch_index == 0 {
                ipc_before_batch
            } else {
                timed_worker.ipc_elapsed
            };
            let observation_before = if batch_index == 0 {
                observation_before_batch
            } else {
                timed_worker.observation_capture_elapsed
            };
            let corpus_before = if batch_index == 0 {
                corpus_before_batch
            } else {
                timed_worker.corpus_encoding_elapsed
            };
            let mut outcome = execute_selected_tactic_with_checkpoint_retention_and_strategy(
                &mut timed_worker,
                &proposal.selected,
                &job.proposal_catalog,
                &job.proposal_blueprints,
                &job.source_snapshot,
                &job.source_route_tape,
                checkpoint_source.as_ref(),
                &NativeTacticWorkerPaths {
                    request: proposal_root.join("request.dsbx"),
                    result: proposal_root.join("result.json"),
                },
                if proposal.proposal_index == 0 {
                    job.primary_retention
                } else {
                    NativeTacticCheckpointRetention::None
                },
                job.execution_strategy,
            );
            if outcome
                .as_ref()
                .is_err_and(NativeTacticWorkerError::is_missing_process_local_checkpoint)
                && batch_index == 0
                && checkpoint_source.is_some()
            {
                checkpoint_source = match materialize_job_frontier(
                    &mut timed_worker,
                    &job,
                    proposal.proposal_index,
                    "frontier-replay-fallback",
                    true,
                ) {
                    Ok(source) => Some(source),
                    Err(error) => {
                        failed = Some(route_error(error));
                        break;
                    }
                };
                let fallback_root = proposal_root.join("after-replay");
                if let Err(error) = fs::create_dir_all(&fallback_root).map_err(route_error) {
                    failed = Some(error);
                    break;
                }
                outcome = execute_selected_tactic_with_checkpoint_retention_and_strategy(
                    &mut timed_worker,
                    &proposal.selected,
                    &job.proposal_catalog,
                    &job.proposal_blueprints,
                    &job.source_snapshot,
                    &job.source_route_tape,
                    checkpoint_source.as_ref(),
                    &NativeTacticWorkerPaths {
                        request: fallback_root.join("request.dsbx"),
                        result: fallback_root.join("result.json"),
                    },
                    if proposal.proposal_index == 0 {
                        job.primary_retention
                    } else {
                        NativeTacticCheckpointRetention::None
                    },
                    job.execution_strategy,
                );
            }
            if outcome.is_ok() && checkpoint_source.is_none() {
                if let Err(error) =
                    timed_worker.record_route_replay(job.source_route_tape.frames.len())
                {
                    failed = Some(route_error(error));
                    break;
                }
            }
            let native_elapsed = timed_worker.native_elapsed.saturating_sub(native_before);
            let native_batch_elapsed = timed_worker
                .native_batch_elapsed
                .saturating_sub(native_batch_before);
            let ipc_elapsed = timed_worker.ipc_elapsed.saturating_sub(ipc_before);
            let observation_capture_elapsed = timed_worker
                .observation_capture_elapsed
                .saturating_sub(observation_before);
            let corpus_encoding_elapsed = timed_worker
                .corpus_encoding_elapsed
                .saturating_sub(corpus_before);
            let restore_accounting = timed_worker.take_accounting();
            match outcome {
                Ok(outcome) => {
                    let elapsed = execution_started.elapsed();
                    let state_extraction_elapsed =
                        Duration::from_micros(outcome.state_extraction_micros);
                    work.push(NativeTacticProposalWork {
                        execution_plan_sha256: job.execution_plan_sha256,
                        worker_slot,
                        outcome,
                        native_elapsed,
                        ipc_elapsed,
                        observation_capture_elapsed,
                        corpus_encoding_elapsed,
                        preparation_elapsed: elapsed
                            .saturating_sub(native_batch_elapsed)
                            .saturating_sub(ipc_elapsed)
                            .saturating_sub(state_extraction_elapsed),
                        restore_accounting,
                    });
                }
                Err(error) => {
                    failed = Some(route_error(error));
                    break;
                }
            }
        }
        let _ = job.response.send(match failed {
            Some(error) => Err(error),
            None => Ok(work),
        });
    }
    drop(timed_worker);
    worker.shutdown().map_err(route_error)
}

fn materialize_job_frontier<W: PersistentTacticBatchWorker>(
    worker: &mut TimedTacticWorker<'_, W>,
    job: &NativeTacticProposalJob,
    proposal_index: usize,
    directory: &str,
    fallback: bool,
) -> Result<NativeTacticCheckpointSource, NativeTacticWorkerError> {
    let native_batch_before = worker.native_batch_elapsed;
    let materialization_root =
        proposal_artifact_root(&job.paths_root, proposal_index).join(directory);
    fs::create_dir_all(&materialization_root)
        .map_err(|error| NativeTacticWorkerError::Io(error.to_string()))?;
    let frontier = materialize_tactic_frontier(
        worker,
        &job.source_snapshot,
        &job.source_route_tape,
        &NativeTacticWorkerPaths {
            request: materialization_root.join("request.dsbx"),
            result: materialization_root.join("result.json"),
        },
    )?;
    let restoration = job
        .restoration
        .as_ref()
        .ok_or(NativeTacticWorkerError::DetachedSelection)?;
    if frontier.observed_state_sha256 != restoration.receipt.observed_state_sha256 {
        return Err(NativeTacticWorkerError::DetachedResult(
            "frontier materialization restoration receipt",
        ));
    }
    let replay_elapsed = worker
        .native_batch_elapsed
        .saturating_sub(native_batch_before);
    worker.record_prefix_materialization(
        job.source_route_tape.frames.len(),
        fallback,
        replay_elapsed,
    )?;
    Ok(frontier.source)
}

fn proposal_artifact_root(paths_root: &Path, proposal_index: usize) -> PathBuf {
    paths_root.join(format!("proposal-{proposal_index:03}"))
}

#[cfg(test)]
#[path = "worker_pool_tests.rs"]
mod tests;
