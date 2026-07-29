use super::*;

impl NativeTacticProposalPool {
    pub(in crate::native_tactic_route_runner) fn with_lane_owner_partition(
        mut self,
        execution_plan: &NativeTacticExecutionPlan,
    ) -> Self {
        let concurrent_lanes = execution_plan
            .generations
            .iter()
            .map(|generation| generation.lane_indices.len())
            .max()
            .unwrap_or(0);
        self.dedicated_owner_slots = dedicated_owner_slot_count(
            self.senders.len(),
            concurrent_lanes,
            execution_plan.proposal_width_per_decision,
            self.direct_restore_enabled,
        );
        self
    }

    pub(in crate::native_tactic_route_runner) fn for_lane(
        &self,
        generation_lane_index: usize,
    ) -> Result<Self, NativeTacticRouteRunError> {
        let mut lane = self.clone();
        if self.dedicated_owner_slots > 0 {
            if generation_lane_index >= self.dedicated_owner_slots {
                return Err(route_message(
                    "native tactic lane exceeds its dedicated worker partition",
                ));
            }
            lane.preferred_owner_slot = Some(generation_lane_index);
        }
        Ok(lane)
    }

    pub(super) fn direct_frontier_eligible(&self, frontier: &CachedTacticFrontier) -> bool {
        self.direct_restore_enabled
            && frontier.worker_slot < self.senders.len()
            && self
                .preferred_owner_slot
                .is_none_or(|owner| frontier.worker_slot == owner)
    }

    pub(super) fn next_counterfactual_worker(&self, excluded: Option<usize>) -> usize {
        if self.preferred_owner_slot.is_some() {
            let sibling_workers = self.senders.len() - self.dedicated_owner_slots;
            return self.dedicated_owner_slots
                + self.next_worker.fetch_add(1, Ordering::Relaxed) % sibling_workers;
        }
        next_worker_excluding(&self.next_worker, self.senders.len(), excluded)
    }

    pub(in crate::native_tactic_route_runner) fn execute_batch(
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
        self.execute_batch_with_dispatch_hook(
            proposals,
            proposal_catalog,
            proposal_blueprints,
            source_snapshot,
            source_route_tape,
            restoration,
            cached_frontier,
            retain_primary_checkpoint,
            paths_root,
            || Ok(()),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::native_tactic_route_runner) fn execute_batch_with_dispatch_hook<F>(
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
        after_dispatch: F,
    ) -> Result<Vec<NativeTacticProposalWork>, NativeTacticRouteRunError>
    where
        F: FnOnce() -> Result<(), NativeTacticRouteRunError>,
    {
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
                    self.direct_frontier_eligible(frontier)
                        && restoration.is_some_and(|contract| {
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
            let (execution_started, execution_started_receiver) = mpsc::sync_channel(1);
            let primary_source = (proposal_index == 0).then_some(direct).flatten();
            let worker_slot = primary_source.map_or_else(
                || {
                    if proposal_index == 0
                        && let Some(owner) = self.preferred_owner_slot
                    {
                        owner
                    } else {
                        self.next_counterfactual_worker(direct.map(|frontier| frontier.worker_slot))
                    }
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
                    checkpoint_cache_capacity_bytes: self.checkpoint_cache_capacity_bytes,
                    paths_root: paths_root.to_path_buf(),
                    execution_started,
                    response,
                })
                .map_err(|_| route_message("native tactic proposal pool stopped"))?;
            responses.push((proposal_index, execution_started_receiver, receiver));
        }
        for (_, execution_started, _) in &responses {
            execution_started
                .recv()
                .map_err(|_| route_message("native tactic proposal stopped before execution"))?;
        }
        after_dispatch()?;
        let mut work = responses
            .into_iter()
            .map(|(proposal_index, _, receiver)| {
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
pub(in crate::native_tactic_route_runner) fn capture_terminal_route_replay(
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
pub(in crate::native_tactic_route_runner) fn load_or_capture_demonstration(
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

pub(in crate::native_tactic_route_runner) fn demonstration_corpus_is_attached(
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

pub(in crate::native_tactic_route_runner) fn recorded_demonstration_chunks(
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

pub(in crate::native_tactic_route_runner) fn run_tactic_proposal_worker(
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
        let _ = job.execution_started.send(());
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
                job.checkpoint_cache_capacity_bytes,
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
                    job.checkpoint_cache_capacity_bytes,
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
    let frontier = materialize_tactic_frontier_with_cache_capacity(
        worker,
        &job.source_snapshot,
        &job.source_route_tape,
        &NativeTacticWorkerPaths {
            request: materialization_root.join("request.dsbx"),
            result: materialization_root.join("result.json"),
        },
        job.checkpoint_cache_capacity_bytes,
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

pub(super) fn proposal_artifact_root(paths_root: &Path, proposal_index: usize) -> PathBuf {
    paths_root.join(format!("proposal-{proposal_index:03}"))
}
