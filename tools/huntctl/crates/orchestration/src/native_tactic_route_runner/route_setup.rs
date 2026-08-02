//! Route-learning configuration, initial probes, and bounded local I/O.

use super::*;

pub(super) fn read_bounded_json<T: for<'de> Deserialize<'de>>(
    path: &Path,
) -> Result<T, NativeTacticRouteRunError> {
    let metadata = fs::symlink_metadata(path).map_err(route_error)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_RESUME_JSON_BYTES
    {
        return Err(route_message(format!(
            "resumable tactic JSON is invalid or oversized: {}",
            path.display()
        )));
    }
    serde_json::from_slice(&fs::read(path).map_err(route_error)?).map_err(route_error)
}

pub(super) fn reserve_attempt_root(
    output_root: &Path,
) -> Result<PathBuf, NativeTacticRouteRunError> {
    let attempts = output_root.join("attempts");
    fs::create_dir_all(&attempts).map_err(route_error)?;
    for index in 0..MAX_ROUTE_ATTEMPTS {
        let attempt = attempts.join(format!("attempt-{index:04}"));
        match fs::create_dir(&attempt) {
            Ok(()) => return Ok(attempt),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(route_error(error)),
        }
    }
    Err(route_message("tactic route attempt capacity is exhausted"))
}

pub(super) fn tactic_state_trace(
    facts: &FactSnapshot,
) -> Result<NativeTacticStateTrace, NativeTacticRouteRunError> {
    let room = facts.world.room;
    Ok(NativeTacticStateTrace {
        snapshot_sha256: facts.content_sha256().map_err(route_error)?,
        stage: facts.world.stage.clone(),
        room,
        layer: facts.world.layer,
        point: facts.world.point,
        simulation_tick: facts.simulation_tick,
        tape_frame: facts.tape_frame,
        player_position: facts.player.position_f32_bits.map(f32::from_bits),
        player_velocity: facts
            .player
            .velocity_f32_bits
            .map(|bits| bits.map(f32::from_bits)),
        player_procedure: facts.player.procedure,
        player_contacts: facts.player.contacts,
        event_running: facts.event.as_ref().map(|event| event.running),
        event_id: facts.event.as_ref().map(|event| event.event_id),
        terminal_reached: facts.terminal.reached,
        actor_count: facts.actors.len(),
        same_room_actor_count: facts
            .actors
            .iter()
            .filter(|actor| actor.current_room == room)
            .count(),
        recent_option_id: facts
            .recent_option
            .as_ref()
            .map(|option| option.option_id.clone()),
    })
}

pub(super) fn frontier_sampling_round(episode: u64) -> u64 {
    episode.saturating_sub(1 + episode / 4)
}

pub(super) fn initial_probe_batch(
    config: &NativeTacticRouteRunConfig<'_>,
) -> Result<NativeSuffixBatch, NativeTacticRouteRunError> {
    // The root observation is the first pre-input row. Running the entire
    // exploration horizon here produces no additional authority: the
    // persistent worker has already captured the authenticated source
    // checkpoint before it evaluates this candidate, and subsequent batches
    // declare their own bounded horizons.
    let mut batch = tactic_root_probe_batch_with_ticks(config.optimization, config.execution, 1)?;
    let capacity = tactic_checkpoint_cache_capacity_per_worker(
        config.execution_plan.budgets.memory_bytes,
        config.checkpoint_capacity_workers,
    )?;
    attach_root_probe_checkpoint_cache(&mut batch, capacity);
    Ok(batch)
}

pub(super) fn attach_root_probe_checkpoint_cache(batch: &mut NativeSuffixBatch, capacity: usize) {
    batch.schema = NATIVE_CACHED_SUFFIX_BATCH_SCHEMA.into();
    batch.checkpoint_cache = Some(tactic_checkpoint_cache_request(
        None,
        NativeTacticCheckpointRetention::None,
        capacity,
    ));
}
pub(super) fn goal_tactic_maximum_ticks(horizon: u64) -> Result<u32, NativeTacticRouteRunError> {
    let horizon = u32::try_from(horizon).map_err(route_error)?;
    if horizon == 0 {
        return Err(route_message("goal tactic requires a nonzero horizon"));
    }
    // Route-relative seeks are navigation decisions, not whole-route
    // controllers. Reserve room for four reactive decisions so the learner can
    // redirect around contact geometry instead of spending half its horizon on
    // one stalled target.
    Ok((horizon / 4).clamp(1, 40))
}

pub(super) fn goal_route_sequence_maximum_ticks(
    horizon: u64,
) -> Result<u32, NativeTacticRouteRunError> {
    goal_tactic_maximum_ticks(horizon)
}

pub(super) fn route_tactic_reward_spec() -> TacticRewardSpec {
    route_tactic_base_reward_spec()
}

pub(super) fn route_tactic_base_reward_spec() -> TacticRewardSpec {
    TacticRewardSpec {
        schema: TACTIC_REWARD_SPEC_SCHEMA_V2.into(),
        terminal_reward: 100.0,
        // Terminal evidence remains overwhelmingly dominant, while every
        // simulated controller tick has a small explicit cost. This makes the
        // learned value function prefer a shorter terminal route without
        // making necessary collision-avoidance detours worse than failure.
        tick_cost: ROUTE_TACTIC_TICK_COST,
        novelty_reward: 0.0,
        per_tick_discount: 1.0,
        potential: None,
        motion_cost: None,
    }
}

pub(super) fn route_option_value_config(execution_authority_sha256: Digest) -> OptionValueConfig {
    let learner_seed = u64::from_le_bytes(
        execution_authority_sha256.0[..8]
            .try_into()
            .expect("fixed slice"),
    );
    OptionValueConfig {
        fitted_q: FqiConfig {
            iterations: 12,
            trees_per_action: 15,
            max_tree_depth: 8,
            // Keep a mild contraction so zero-reward waypoint holds lose
            // value, without erasing a terminal reached late in the declared
            // discovery horizon.
            discount: ROUTE_TACTIC_VALUE_DISCOUNT,
            seed: 0xd15c_a11d_5eed_f017 ^ learner_seed,
            ..FqiConfig::default()
        },
    }
}

pub(crate) fn tactic_root_probe_batch(
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
) -> Result<NativeSuffixBatch, NativeTacticRouteRunError> {
    let maximum_ticks =
        usize::try_from(optimization.budgets.exploration_horizon_ticks).map_err(route_error)?;
    tactic_root_probe_batch_with_ticks(optimization, execution, maximum_ticks)
}

pub(super) fn tactic_root_probe_batch_with_ticks(
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    maximum_ticks: usize,
) -> Result<NativeSuffixBatch, NativeTacticRouteRunError> {
    if maximum_ticks == 0
        || maximum_ticks
            > usize::try_from(optimization.budgets.exploration_horizon_ticks)
                .map_err(route_error)?
    {
        return Err(route_message(
            "tactic root probe exceeds the exploration horizon",
        ));
    }
    Ok(NativeSuffixBatch {
        schema: NATIVE_SUFFIX_BATCH_SCHEMA.into(),
        source_frame: usize::try_from(optimization.route.source_boundary_index)
            .map_err(route_error)?,
        source_boundary_fingerprint: optimization
            .route
            .native_source_boundary_fingerprint
            .clone(),
        checkpoint_validation: NativeCheckpointValidation {
            kind: "recorded_replay_window".into(),
            ticks: usize::try_from(execution.checkpoint_validation_ticks).map_err(route_error)?,
        },
        maximum_ticks,
        verify_state_hashes: execution.verify_state_hashes,
        checkpoint_cache: None,
        candidates: vec![NativeSuffixCandidate {
            id: "tactic-root-probe".into(),
            actions: vec![MacroAction::PadRun {
                pad: SearchPadState::from(RawPadState::default()),
                frames: u32::try_from(maximum_ticks).map_err(route_error)?,
                imported_owned_ports: None,
                port_one_secondary_pads: None,
            }],
            controller_program_hex: None,
        }],
    })
}

pub(crate) fn initial_facts(
    initial: &ValidatedNativeSuffixBatch,
) -> Result<FactSnapshot, NativeTacticRouteRunError> {
    let shard = NativeEpisodeShard::decode(
        &fs::read(Path::new(&initial.episode_shard_path)).map_err(route_error)?,
    )
    .map_err(route_error)?;
    let episode = shard
        .episodes
        .iter()
        .find(|episode| episode.id == "tactic-root-probe")
        .ok_or_else(|| route_message("initial native shard has no root probe"))?;
    let observation = &episode
        .steps
        .first()
        .ok_or_else(|| route_message("initial native root probe has no step"))?
        .pre_input;
    FactSnapshot::from_native_learning(observation, &[], None, Vec::new()).map_err(route_error)
}

pub(super) fn maximum_demonstration_chunk_ticks(
    horizon: u64,
) -> Result<u32, NativeTacticRouteRunError> {
    Ok(goal_tactic_maximum_ticks(horizon)?
        .min(u32::try_from(TACTIC_INTERMEDIATE_BOUNDARY_STRIDE).map_err(route_error)?))
}

pub(super) fn validate_config(
    config: &NativeTacticRouteRunConfig<'_>,
) -> Result<(), NativeTacticRouteRunError> {
    config.execution_plan.validate()?;
    validate_unassisted_discovery_horizon(config)?;
    tactic_checkpoint_cache_capacity_per_worker(
        config.execution_plan.budgets.memory_bytes,
        config.checkpoint_capacity_workers,
    )?;
    let maximum_demonstration_chunk_ticks =
        maximum_demonstration_chunk_ticks(config.optimization.budgets.exploration_horizon_ticks)?;
    if !valid_worker_capacity_counts(config.workers, config.checkpoint_capacity_workers)
        || config.execution_plan.budgets.decisions_per_lane > MAX_ROUTE_DECISIONS
        || config
            .execution_plan
            .demonstration_chunk_ticks
            .is_some_and(|ticks| ticks == 0 || ticks > maximum_demonstration_chunk_ticks)
        || !planned_decisions_fit_candidate_budget(
            config.execution_plan.budgets.decisions_per_lane,
            config.execution_plan.seeds.len(),
            config.optimization.budgets.candidate_budget,
        )
        || config
            .execution_plan
            .promoted_tactic_registry_sha256
            .is_some()
            != config.promoted_tactic_registry.is_some()
        || config.fault_injection.is_some_and(|fault| {
            fault.decision_index() >= config.execution_plan.budgets.decisions_per_lane
        })
    {
        return Err(route_message(
            "native tactic route configuration is invalid",
        ));
    }
    Ok(())
}

pub(super) fn valid_worker_capacity_counts(
    workers: usize,
    checkpoint_capacity_workers: usize,
) -> bool {
    workers > 0
        && workers <= MAX_ROUTE_WORKERS
        && checkpoint_capacity_workers >= workers
        && checkpoint_capacity_workers <= MAX_ROUTE_WORKERS
}

pub(super) fn planned_decisions_fit_candidate_budget(
    decisions_per_lane: u64,
    lane_count: usize,
    candidate_budget: u64,
) -> bool {
    u64::try_from(lane_count)
        .ok()
        .and_then(|lanes| decisions_per_lane.checked_mul(lanes))
        .is_some_and(|total| total <= candidate_budget)
}

pub(super) fn tactic_checkpoint_cache_capacity_per_worker(
    memory_bytes: NativeTacticResourceLimit,
    workers: usize,
) -> Result<usize, NativeTacticRouteRunError> {
    let workers = u64::try_from(workers).map_err(route_error)?;
    if workers == 0 {
        return Err(route_message(
            "native tactic checkpoint cache requires at least one worker",
        ));
    }
    let capacity = match memory_bytes {
        NativeTacticResourceLimit::Bounded(total) => total / workers,
        NativeTacticResourceLimit::Unbounded => TACTIC_CHECKPOINT_CACHE_BYTES as u64,
    }
    .min(TACTIC_CHECKPOINT_CACHE_BYTES as u64);
    if capacity == 0 {
        return Err(route_message(
            "native tactic memory budget cannot provide every worker a checkpoint cache",
        ));
    }
    usize::try_from(capacity).map_err(route_error)
}

pub(super) fn validate_unassisted_discovery_horizon(
    config: &NativeTacticRouteRunConfig<'_>,
) -> Result<(), NativeTacticRouteRunError> {
    let plan = config.execution_plan;
    let required = unassisted_discovery_horizon_requirement(
        config.optimization.campaign_class,
        plan.proposal_policy,
        plan.demonstration_chunk_ticks.is_some(),
        plan.promoted_tactic_registry_sha256.is_some(),
        config.optimization.budgets.promotion_before_tick,
    )
    .map_err(route_message)?;
    let Some(minimum) = required else {
        return Ok(());
    };
    if config.optimization.budgets.exploration_horizon_ticks < minimum {
        return Err(route_message(format!(
            "unassisted learned tactic routing requires at least {minimum} discovery ticks; \
             promotion and terminal discovery horizons are separate authority"
        )));
    }
    Ok(())
}

pub(super) fn unassisted_discovery_horizon_requirement(
    campaign_class: CampaignClass,
    proposal_policy: TacticProposalPolicy,
    has_demonstration: bool,
    has_promoted_tactics: bool,
    promotion_before_tick: u64,
) -> Result<Option<u64>, &'static str> {
    let unassisted_learning = proposal_policy == TacticProposalPolicy::Learned
        && !has_demonstration
        && !has_promoted_tactics;
    if !unassisted_learning {
        return Ok(None);
    }
    if campaign_class != CampaignClass::FromScratchDiscovery {
        return Err("unassisted learned tactic routing requires a from_scratch_discovery request");
    }
    minimum_discovery_horizon_ticks(promotion_before_tick)
        .map(Some)
        .ok_or("minimum discovery horizon overflowed")
}

pub(super) fn selected_tactic_fits_horizon(
    suffix_ticks: u64,
    selected_maximum_ticks: u32,
    horizon: u64,
) -> bool {
    suffix_ticks.saturating_add(u64::from(selected_maximum_ticks)) <= horizon
}
