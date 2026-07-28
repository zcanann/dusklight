use super::*;

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
    root_checkpoint_sha256: Digest,
    root_tape_ref: StoredContentRef,
    shared_training: &[TacticQTrainingCorpus],
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
            root_checkpoint_sha256,
            root_tape_ref,
            shared_training,
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
    let corpus = if let Some(corpus) = result.generated_training_corpus.as_deref() {
        TacticQTrainingCorpus::read(Path::new(corpus)).map_err(route_error)?
    } else {
        let checkpoint = result.final_checkpoint.as_deref().ok_or_else(|| {
            route_message("completed tactic seed has no generated training corpus or checkpoint")
        })?;
        TacticQCampaign::read_checkpoint(Path::new(checkpoint))
            .and_then(|campaign| {
                campaign.training_corpus_from(result.imported_training_replay_rows)
            })
            .map_err(route_error)?
    };
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
    validate_parameterized_policy_catalog(&proposals.catalog)?;
    proposals.family_schema_sha256 = action_schema_sha256;
    Ok(proposals)
}

pub(super) fn validate_parameterized_policy_catalog(
    catalog: &TacticAssetCatalog,
) -> Result<(), NativeTacticRouteRunError> {
    if let Some(entry) = catalog
        .entries()
        .iter()
        .find(|entry| !entry.option_id().starts_with("family/"))
    {
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
        // Keep candidate generation policy-neutral. The learned policy uses
        // critic uncertainty when it ranks this shared valid catalog; random
        // and structured baselines must see the same action candidates.
        ensemble_uncertainty: None,
        endpoint_novel: prior_occurrences == 1,
        terminal: previous.value_sample.terminal,
    }))
}

pub(super) struct TimedTacticWorker<'a, W> {
    inner: &'a mut W,
    native_elapsed: Duration,
    pending_accounting: NativeTacticRestoreAccounting,
    prior_cache_hits: u64,
    prior_cache_misses: u64,
}

impl<'a, W> TimedTacticWorker<'a, W> {
    fn new(inner: &'a mut W) -> Self {
        Self {
            inner,
            native_elapsed: Duration::ZERO,
            pending_accounting: NativeTacticRestoreAccounting::default(),
            prior_cache_hits: 0,
            prior_cache_misses: 0,
        }
    }

    fn take_accounting(&mut self) -> NativeTacticRestoreAccounting {
        let mut accounting = std::mem::take(&mut self.pending_accounting);
        accounting.refresh_rates();
        accounting
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
        self.pending_accounting.restore_micros =
            self.pending_accounting.restore_micros.saturating_add(
                batch
                    .restore_micros
                    .iter()
                    .fold(0_u64, |total, micros| total.saturating_add(*micros)),
            );
        let Some(cache) = batch.checkpoint_cache.as_ref() else {
            return Err(NativeTacticWorkerError::DetachedResult(
                "tactic batch cache accounting",
            ));
        };
        let direct_restore = match cache.source_kind.as_str() {
            "authenticated_root_restore" => {
                self.pending_accounting.authenticated_root_restore_requests = self
                    .pending_accounting
                    .authenticated_root_restore_requests
                    .saturating_add(1);
                false
            }
            "direct_process_local_restore" => {
                self.pending_accounting
                    .direct_process_local_restore_requests = self
                    .pending_accounting
                    .direct_process_local_restore_requests
                    .saturating_add(1);
                true
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
        self.prior_cache_hits = cache.hits;
        self.prior_cache_misses = cache.misses;
        if cache_misses != 0 || direct_restore != (cache_hits != 0) {
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
    ) -> Result<ValidatedNativeSuffixBatch, NativeTacticWorkerError> {
        let started = Instant::now();
        let response = self.inner.run_tactic_batch(request, result);
        self.native_elapsed = self.native_elapsed.saturating_add(started.elapsed());
        match response {
            Ok(batch) => {
                self.record_batch(&batch)?;
                Ok(batch)
            }
            Err(error) => Err(error),
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
    checkpoint_source: Option<NativeTacticCheckpointSource>,
    materialize_frontier: bool,
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
}

impl NativeTacticProposalPool {
    pub(super) fn execute_batch(
        &self,
        proposals: &[SelectedTactic],
        proposal_catalog: Arc<dusklight_learning::tactic_asset::TacticAssetCatalog>,
        proposal_blueprints: Arc<Vec<TacticBlueprint>>,
        source_snapshot: &FactSnapshot,
        source_route_tape: &InputTape,
        cached_frontier: Option<&CachedTacticFrontier>,
        paths_root: &Path,
    ) -> Result<Vec<NativeTacticProposalWork>, NativeTacticRouteRunError> {
        if self.senders.is_empty() {
            return Err(route_message("native tactic proposal pool is empty"));
        }
        if proposals.is_empty() {
            return Err(route_message("native tactic proposal batch is empty"));
        }
        let replayed_prefix = source_route_tape
            .frames
            .len()
            .checked_sub(self.root_source_frame)
            .ok_or_else(|| route_message("tactic route precedes its authenticated root"))?;
        if replayed_prefix != 0 {
            let direct = cached_frontier.filter(|frontier| {
                self.direct_restore_enabled && frontier.worker_slot < self.senders.len()
            });
            let worker_slot = direct.map_or_else(
                || self.next_worker.fetch_add(1, Ordering::Relaxed) % self.senders.len(),
                |frontier| frontier.worker_slot,
            );
            let (response, receiver) = mpsc::sync_channel(1);
            self.senders[worker_slot]
                .send(NativeTacticProposalJob {
                    execution_plan_sha256: self.execution_plan_sha256,
                    proposals: proposals
                        .iter()
                        .cloned()
                        .enumerate()
                        .map(|(proposal_index, selected)| IndexedNativeTacticProposal {
                            proposal_index,
                            selected,
                        })
                        .collect(),
                    proposal_catalog,
                    proposal_blueprints,
                    source_snapshot: source_snapshot.clone(),
                    source_route_tape: source_route_tape.clone(),
                    checkpoint_source: direct.map(|frontier| frontier.source.clone()),
                    materialize_frontier: direct.is_none(),
                    execution_strategy: self.execution_strategy,
                    paths_root: paths_root.to_path_buf(),
                    response,
                })
                .map_err(|_| route_message("native tactic proposal pool stopped"))?;
            return receiver
                .recv()
                .map_err(|_| route_message("native tactic proposal worker stopped"))?;
        }

        let mut responses = Vec::with_capacity(proposals.len());
        for (proposal_index, selected) in proposals.iter().enumerate() {
            let (response, receiver) = mpsc::sync_channel(1);
            let worker_slot = self.next_worker.fetch_add(1, Ordering::Relaxed) % self.senders.len();
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
                    checkpoint_source: None,
                    materialize_frontier: false,
                    execution_strategy: self.execution_strategy,
                    paths_root: paths_root.to_path_buf(),
                    response,
                })
                .map_err(|_| route_message("native tactic proposal pool stopped"))?;
            responses.push(receiver);
        }
        responses
            .into_iter()
            .map(|receiver| {
                let mut work = receiver
                    .recv()
                    .map_err(|_| route_message("native tactic proposal worker stopped"))??;
                work.pop()
                    .ok_or_else(|| route_message("native tactic proposal result is absent"))
            })
            .collect()
    }
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

    let started = Instant::now();
    let chunks = recorded_demonstration_chunks(
        process_tape,
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
    let mut preparation_micros = 0_u64;
    let mut restore_accounting = NativeTacticRestoreAccounting::default();
    let mut first_hit_tick = None;

    for (chunk_index, chunk) in chunks.into_iter().enumerate() {
        if cancellation_requested(config) {
            return Err(route_cancelled(
                "native tactic demonstration capture cancelled",
            ));
        }
        let option_id = format!("demonstration/chunk-{chunk_index:04}");
        let entry = TacticCatalogEntry::new(option_id, TacticAssetSource::RecordedTape(chunk))
            .map_err(route_error)?;
        let catalog = Arc::new(TacticAssetCatalog::new(vec![entry]).map_err(route_error)?);
        let descriptor = catalog
            .option_descriptors()
            .next()
            .cloned()
            .ok_or_else(|| route_message("demonstration catalog is empty"))?;
        let selected = SelectedTactic {
            schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
            learner_snapshot_sha256: before.content_sha256().map_err(route_error)?,
            decision_index: chunk_index as u64,
            descriptor,
            reason: TacticSelectionReason::StructuredBaseline,
            exploration_draw: 0,
        };
        let paths_root = config
            .output_root
            .join("demonstration")
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
            &paths_root,
        )?;
        if work.len() != 1 {
            return Err(route_message(
                "demonstration chunk did not produce one native outcome",
            ));
        }
        let work = work.remove(0);
        native_simulation_micros =
            native_simulation_micros.saturating_add(elapsed_micros(work.native_elapsed));
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
        episode_groups.push(TACTIC_Q_DEMONSTRATION_EPISODE_GROUP);
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
            "authenticated process-tape demonstration did not reach the terminal",
        ));
    }
    let route_native_ticks = route
        .frames
        .len()
        .checked_sub(pool.root_source_frame)
        .ok_or_else(|| route_message("demonstration route precedes its source"))?
        as u64;
    let first_hit_tick = first_hit_tick.ok_or_else(|| {
        route_message("demonstration terminal has no source-relative first-hit tick")
    })?;
    if route_native_ticks != native_ticks || native_ticks != first_hit_tick.saturating_add(1) {
        return Err(route_message(
            "demonstration native ticks are detached from its first hit",
        ));
    }
    let expected_end = pool
        .root_source_frame
        .checked_add(usize::try_from(native_ticks).map_err(route_error)?)
        .ok_or_else(|| route_message("demonstration first hit overflows"))?;
    if process_tape.frames.get(..expected_end) != Some(route.frames.as_slice()) {
        return Err(route_message(
            "native demonstration differs from the authenticated process tape",
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
        wall_micros: elapsed_micros(started.elapsed()),
        native_simulation_micros,
        preparation_micros,
        restore_accounting,
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
        let Ok(job) = job else {
            break;
        };
        let batch_started = Instant::now();
        let native_before_materialization = timed_worker.native_elapsed;
        let checkpoint_source = if job.materialize_frontier {
            let materialization_root = job.paths_root.join("frontier-source");
            fs::create_dir_all(&materialization_root)
                .map_err(route_error)
                .and_then(|_| {
                    materialize_tactic_frontier(
                        &mut timed_worker,
                        &job.source_snapshot,
                        &job.source_route_tape,
                        &NativeTacticWorkerPaths {
                            request: materialization_root.join("request.json"),
                            result: materialization_root.join("result.json"),
                        },
                    )
                    .map(|frontier| Some(frontier.source))
                    .map_err(route_error)
                })
        } else {
            Ok(job.checkpoint_source)
        };
        let checkpoint_source = match checkpoint_source {
            Ok(source) => source,
            Err(error) => {
                let _ = job.response.send(Err(error));
                continue;
            }
        };
        let materialization_native_elapsed = timed_worker
            .native_elapsed
            .saturating_sub(native_before_materialization);
        let materialization_elapsed = batch_started.elapsed();
        let materialization_preparation_elapsed =
            materialization_elapsed.saturating_sub(materialization_native_elapsed);
        let mut materialization_accounting = timed_worker.take_accounting();
        if job.materialize_frontier {
            materialization_accounting.prefix_materializations = materialization_accounting
                .prefix_materializations
                .saturating_add(1);
            let source_frame = usize::try_from(timed_worker.identity().source_frame)
                .map_err(|_| route_message("native tactic source frame exceeds platform limits"))?;
            let replayed_prefix_ticks = u64::try_from(
                job.source_route_tape
                    .frames
                    .len()
                    .saturating_sub(source_frame),
            )
            .map_err(|_| route_message("replayed tactic prefix exceeds report limits"))?;
            materialization_accounting.replayed_prefix_ticks = materialization_accounting
                .replayed_prefix_ticks
                .saturating_add(replayed_prefix_ticks);
            materialization_accounting.refresh_rates();
        }

        let mut work = Vec::with_capacity(job.proposals.len());
        let mut failed = None;
        for (batch_index, proposal) in job.proposals.into_iter().enumerate() {
            let proposal_root = job
                .paths_root
                .join(format!("proposal-{:03}", proposal.proposal_index));
            if let Err(error) = fs::create_dir_all(&proposal_root).map_err(route_error) {
                failed = Some(error);
                break;
            }
            let execution_started = Instant::now();
            let native_before = timed_worker.native_elapsed;
            let outcome = execute_selected_tactic_with_checkpoint_retention_and_strategy(
                &mut timed_worker,
                &proposal.selected,
                &job.proposal_catalog,
                &job.proposal_blueprints,
                &job.source_snapshot,
                &job.source_route_tape,
                checkpoint_source.as_ref(),
                &NativeTacticWorkerPaths {
                    request: proposal_root.join("request.json"),
                    result: proposal_root.join("result.json"),
                },
                false,
                job.execution_strategy,
            )
            .map_err(route_error);
            let native_elapsed = timed_worker.native_elapsed.saturating_sub(native_before);
            let mut restore_accounting = timed_worker.take_accounting();
            if batch_index == 0 {
                restore_accounting.merge(&materialization_accounting);
            }
            match outcome {
                Ok(outcome) => {
                    let elapsed = execution_started.elapsed();
                    work.push(NativeTacticProposalWork {
                        execution_plan_sha256: job.execution_plan_sha256,
                        worker_slot,
                        outcome,
                        native_elapsed: native_elapsed.saturating_add(if batch_index == 0 {
                            materialization_native_elapsed
                        } else {
                            Duration::ZERO
                        }),
                        preparation_elapsed: elapsed.saturating_sub(native_elapsed).saturating_add(
                            if batch_index == 0 {
                                materialization_preparation_elapsed
                            } else {
                                Duration::ZERO
                            },
                        ),
                        restore_accounting,
                    });
                }
                Err(error) => {
                    failed = Some(error);
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
