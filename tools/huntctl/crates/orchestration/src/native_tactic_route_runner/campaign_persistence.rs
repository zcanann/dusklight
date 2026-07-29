use super::*;

pub(super) fn cancellation_requested(config: &NativeTacticRouteRunConfig<'_>) -> bool {
    config
        .cancellation
        .is_some_and(|cancellation| cancellation.load(Ordering::Acquire))
}

pub(super) fn pause_tactic_campaign(
    seed_root: &Path,
    campaign: &TacticQCampaign,
) -> Result<PathBuf, NativeTacticRouteRunError> {
    campaign
        .write_checkpoint_with_store(
            &seed_root
                .join("pause-checkpoints")
                .join(format!("decision-{:06}", campaign.decision_index)),
            &tactic_content_store_path(seed_root),
        )
        .map_err(route_error)
}

pub(super) fn seed_performance_root(seed_root: &Path) -> PathBuf {
    seed_root.join("performance")
}

pub(super) fn seed_performance_prefix(decisions: u64) -> String {
    format!("decision-{decisions:06}-attempt-")
}

pub(super) fn seed_performance_exists(
    seed_root: &Path,
    decisions: u64,
) -> Result<bool, NativeTacticRouteRunError> {
    let root = seed_performance_root(seed_root);
    if !root.exists() {
        return Ok(false);
    }
    let prefix = seed_performance_prefix(decisions);
    Ok(fs::read_dir(root).map_err(route_error)?.any(|entry| {
        entry.is_ok_and(|entry| {
            entry
                .file_type()
                .is_ok_and(|kind| kind.is_file() && !kind.is_symlink())
                && entry.file_name().to_string_lossy().starts_with(&prefix)
                && entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "json")
        })
    }))
}

pub(super) fn persist_seed_performance(
    seed_root: &Path,
    decisions: u64,
    timing: &NativeTacticRouteTiming,
    useful_decisions: u64,
    native_restore_accounting: &NativeTacticRestoreAccounting,
) -> Result<(), NativeTacticRouteRunError> {
    let performance = NativeTacticSeedPerformance {
        schema: TACTIC_ROUTE_PERFORMANCE_SCHEMA_V2.into(),
        decisions,
        useful_decisions,
        native_restore_accounting: native_restore_accounting.clone(),
        timing: timing.clone(),
    };
    let root = seed_performance_root(seed_root);
    fs::create_dir_all(&root).map_err(route_error)?;
    for attempt in 0..MAX_ROUTE_ATTEMPTS {
        let path = root.join(format!(
            "{}{attempt:04}.json",
            seed_performance_prefix(decisions)
        ));
        if path.exists() {
            let existing: NativeTacticSeedPerformance = read_bounded_json(&path)?;
            if existing == performance {
                return Ok(());
            }
            continue;
        }
        return write_new(
            &path,
            &serde_json::to_vec_pretty(&performance).map_err(route_error)?,
        );
    }
    Err(route_message(
        "immutable tactic performance checkpoint capacity is exhausted",
    ))
}

pub(super) fn load_seed_performance(
    seed_root: &Path,
    decisions: u64,
) -> Result<NativeTacticSeedPerformance, NativeTacticRouteRunError> {
    let root = seed_performance_root(seed_root);
    if !root.exists() {
        return Ok(NativeTacticSeedPerformance {
            schema: TACTIC_ROUTE_PERFORMANCE_SCHEMA_V2.into(),
            decisions,
            useful_decisions: 0,
            native_restore_accounting: NativeTacticRestoreAccounting::default(),
            timing: NativeTacticRouteTiming::default(),
        });
    }
    let prefix = seed_performance_prefix(decisions);
    let mut paths = fs::read_dir(root)
        .map_err(route_error)?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_type()
                .is_ok_and(|kind| kind.is_file() && !kind.is_symlink())
                && entry.file_name().to_string_lossy().starts_with(&prefix)
                && entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "json")
        })
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    paths.sort();
    let Some(path) = paths.last() else {
        return Ok(NativeTacticSeedPerformance {
            schema: TACTIC_ROUTE_PERFORMANCE_SCHEMA_V2.into(),
            decisions,
            useful_decisions: 0,
            native_restore_accounting: NativeTacticRestoreAccounting::default(),
            timing: NativeTacticRouteTiming::default(),
        });
    };
    let performance: NativeTacticSeedPerformance = read_bounded_json(path)?;
    if performance.schema != TACTIC_ROUTE_PERFORMANCE_SCHEMA_V2
        || performance.decisions != decisions
        || performance.useful_decisions > decisions
    {
        return Err(route_message(
            "paused tactic performance checkpoint is invalid",
        ));
    }
    Ok(performance)
}

type ResumedSeedState = (
    TacticQCampaign,
    Vec<NativeTacticDecisionTrace>,
    BTreeMap<String, u64>,
    u64,
    u64,
);

pub(super) fn resume_seed(
    config: &NativeTacticRouteRunConfig<'_>,
    seed_root: &Path,
    feature_schema_sha256: Digest,
    root_checkpoint_sha256: Digest,
    seed_index: usize,
    seed: u64,
) -> Result<ResumedSeedState, NativeTacticRouteRunError> {
    let lane = config
        .execution_plan
        .lanes
        .get(seed_index)
        .filter(|lane| lane.seed == seed)
        .ok_or_else(|| route_message("tactic seed is detached from its execution-plan lane"))?;
    let (checkpoint_decision, checkpoint_path) = latest_pause_checkpoint(seed_root)?;
    let checkpoint =
        TacticQCampaign::read_checkpoint_payload(&checkpoint_path).map_err(route_error)?;
    let expected_exploration = TacticExplorationConfig {
        seed,
        epsilon_per_million: lane.epsilon_per_million,
    };
    if checkpoint.decision_index != checkpoint_decision
        || checkpoint.decision_index > config.execution_plan.budgets.decisions_per_lane
        || checkpoint.execution_authority_sha256 != config.execution_plan.identity()?
        || checkpoint.feature_schema_sha256 != feature_schema_sha256
        || checkpoint.objective_sha256 != config.optimization.terminal_predicate.definition_sha256
        || checkpoint.root_checkpoint_sha256 != root_checkpoint_sha256
        || checkpoint.model_config != route_option_value_config(config.execution_plan.identity()?)
        || checkpoint.exploration != expected_exploration
    {
        return Err(route_message(
            "paused tactic checkpoint does not match this authenticated run",
        ));
    }
    let campaign = TacticQCampaign::resume_without_model(checkpoint).map_err(route_error)?;
    if campaign.replay.len() as u64 != campaign.decision_index {
        return Err(route_message(
            "paused tactic checkpoint has a detached decision history",
        ));
    }
    let trace = read_resumed_trace(seed_root, campaign.decision_index)?;
    let episode = trace.last().map_or(0, |decision| decision.episode);
    if campaign.episode_group != lane.episode_group(episode)?
        || trace
            .iter()
            .zip(&campaign.replay)
            .any(|(decision, replay)| decision.selected_option_id != replay.execution.option_id)
    {
        return Err(route_message(
            "paused tactic checkpoint and decision trace disagree",
        ));
    }
    let mut selection_counts = BTreeMap::new();
    let mut native_ticks = 0_u64;
    for decision in &trace {
        *selection_counts
            .entry(decision.selected_option_id.clone())
            .or_default() += 1;
        native_ticks = native_ticks
            .checked_add(decision_evaluated_ticks(decision))
            .ok_or_else(|| route_message("resumed native tick count overflowed"))?;
    }
    if native_ticks > config.optimization.budgets.simulated_tick_budget {
        return Err(route_message(
            "paused tactic checkpoint exceeds the simulated tick budget",
        ));
    }
    Ok((campaign, trace, selection_counts, native_ticks, episode))
}

pub(super) fn latest_pause_checkpoint(
    seed_root: &Path,
) -> Result<(u64, PathBuf), NativeTacticRouteRunError> {
    let pause_root = seed_root.join("pause-checkpoints");
    let mut checkpoints = Vec::new();
    for entry in fs::read_dir(&pause_root).map_err(|error| {
        route_message(format!(
            "paused tactic checkpoint is unavailable at {}: {error}",
            pause_root.display()
        ))
    })? {
        let entry = entry.map_err(route_error)?;
        let metadata = entry.file_type().map_err(route_error)?;
        if !metadata.is_dir() || metadata.is_symlink() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(decision) = name
            .strip_prefix("decision-")
            .and_then(|value| value.parse::<u64>().ok())
        else {
            continue;
        };
        let mut files = fs::read_dir(entry.path())
            .map_err(route_error)?
            .filter_map(Result::ok)
            .filter(|candidate| {
                candidate
                    .file_type()
                    .is_ok_and(|kind| kind.is_file() && !kind.is_symlink())
                    && candidate
                        .file_name()
                        .to_string_lossy()
                        .starts_with("tactic-q-")
                    && candidate
                        .path()
                        .extension()
                        .is_some_and(|value| value == TACTIC_Q_CHECKPOINT_EXTENSION)
            })
            .map(|candidate| candidate.path())
            .collect::<Vec<_>>();
        if files.len() != 1 {
            return Err(route_message(
                "paused tactic checkpoint directory must contain exactly one checkpoint",
            ));
        }
        checkpoints.push((decision, files.remove(0)));
    }
    checkpoints
        .into_iter()
        .max_by_key(|(decision, _)| *decision)
        .ok_or_else(|| route_message("no resumable paused tactic checkpoint exists"))
}

pub(super) fn read_resumed_trace(
    seed_root: &Path,
    decision_count: u64,
) -> Result<Vec<NativeTacticDecisionTrace>, NativeTacticRouteRunError> {
    let trace = read_tactic_decision_journal(seed_root)?;
    if trace.len() as u64 != decision_count {
        return Err(route_message(
            "paused tactic decision journal does not exactly match its checkpoint",
        ));
    }
    Ok(trace)
}

pub(super) fn read_completed_seed_result(
    path: &Path,
    seed: u64,
    decisions_per_seed: u64,
    execution_plan_sha256: Digest,
    lane: &NativeTacticLanePlan,
) -> Result<NativeTacticSeedResult, NativeTacticRouteRunError> {
    let result: NativeTacticSeedResult = read_bounded_json(path)?;
    if result.execution_plan_sha256 != execution_plan_sha256
        || result.seed != seed
        || result.decisions > decisions_per_seed
        || result.useful_decisions > result.decisions
        || result.terminal_discovered != result.best_authenticated_tick.is_some()
        || result.terminal_discovered != result.best_terminal_state_sha256.is_some()
        || result.terminal_discovered != result.best_terminal_route_checkpoint_sha256.is_some()
        || result.terminal_discovered != result.best_terminal_tape.is_some()
        || result.terminal_discovered != result.best_terminal_result.is_some()
        || (result.success && !result.terminal_discovered)
        || result.success != result.successful_tape.is_some()
        || result.success != result.final_result.is_some()
        || result.final_checkpoint.is_empty()
        || result.state_graph_sha256 == Digest::ZERO
        || (!result.terminal_discovered && result.timing.retained_candidate_artifact_micros != 0)
        || result.trace.len() as u64 != result.decisions
        || result.trace.iter().enumerate().any(|(index, decision)| {
            decision.execution_plan_sha256 != execution_plan_sha256
                || decision.decision_index != index as u64
                || decision.learner_snapshot_sha256 == Digest::ZERO
                || decision.replay_generation != lane.generation_index as u64
                || decision.lane_index != lane.lane_index
                || decision.lane_role != Some(lane.role)
                || decision.acquisition_rank != lane.acquisition.rank(decision.decision_index)
                || decision.frontier_identity == Digest::ZERO
                || decision.restore_source.is_none()
                || decision.result_admission_schema != NATIVE_TACTIC_RESULT_ADMISSION_SCHEMA_V1
                || decision
                    .proposal_batch
                    .iter()
                    .any(|proposal| proposal.execution_plan_sha256 != execution_plan_sha256)
        })
        || result.native_ticks
            != result
                .trace
                .iter()
                .map(decision_evaluated_ticks)
                .sum::<u64>()
    {
        return Err(route_message(
            "completed tactic seed result is invalid or belongs to another run",
        ));
    }
    let checkpoint = TacticQCampaign::read_checkpoint_payload(Path::new(&result.final_checkpoint))
        .map_err(route_error)?;
    let graph_sha256 = checkpoint
        .state_graph
        .content_sha256()
        .map_err(route_error)?;
    let best_graph_terminal = checkpoint.state_graph.best_terminal_path();
    if checkpoint.execution_authority_sha256 != execution_plan_sha256
        || checkpoint.decision_index != result.decisions
        || checkpoint.replay.len() != result.replay_rows
        || checkpoint.training_replay.len() != result.training_replay_rows
        || checkpoint.state_graph.node_count() != result.visited_states
        || graph_sha256 != result.state_graph_sha256
        || best_graph_terminal.is_some() != result.terminal_discovered
        || best_graph_terminal.map(|path| path.terminal.state_sha256)
            != result.best_terminal_state_sha256
        || best_graph_terminal.map(|path| path.route_checkpoint_sha256)
            != result.best_terminal_route_checkpoint_sha256
        || best_graph_terminal.and_then(|path| path.root_to_terminal_ticks.checked_sub(1))
            != result.best_authenticated_tick
    {
        return Err(route_message(
            "completed tactic seed report is detached from its authoritative state graph",
        ));
    }
    if let (Some(result_path), Some(tape_path), Some(first_hit_tick)) = (
        result.best_terminal_result.as_deref(),
        result.best_terminal_tape.as_deref(),
        result.best_authenticated_tick,
    ) {
        let source_frame = result
            .trace
            .first()
            .map(|decision| decision.before.tape_frame)
            .ok_or_else(|| route_message("terminal seed result has no source decision"))?;
        let terminal_result =
            TacticQFinalResult::read(Path::new(result_path)).map_err(route_error)?;
        let tape = InputTape::decode(&fs::read(tape_path).map_err(route_error)?)
            .map_err(route_error)?
            .tape;
        let best_graph_terminal = best_graph_terminal.ok_or_else(|| {
            route_message("terminal artifacts exist without a graph-selected terminal")
        })?;
        if terminal_result.execution_authority_sha256 != execution_plan_sha256
            || terminal_result.objective_sha256 != checkpoint.objective_sha256
            || terminal_result.root_checkpoint_sha256 != checkpoint.root_checkpoint_sha256
            || terminal_result.route_tape != tape
            || terminal_result.terminal_state_sha256 != best_graph_terminal.terminal.state_sha256
            || route_checkpoint(checkpoint.root_checkpoint_sha256, &tape).map_err(route_error)?
                != best_graph_terminal.route_checkpoint_sha256
            || checkpoint
                .state_graph
                .route(best_graph_terminal.route_checkpoint_sha256)
                != Some(&tape)
            || authenticated_first_hit_tick(&terminal_result, source_frame) != Some(first_hit_tick)
        {
            return Err(route_message(
                "completed tactic best-terminal artifacts belong to another execution plan",
            ));
        }
    }
    if let (Some(final_path), Some(tape_path)) = (
        result.final_result.as_deref(),
        result.successful_tape.as_deref(),
    ) {
        let final_result = TacticQFinalResult::read(Path::new(final_path)).map_err(route_error)?;
        let tape = InputTape::decode(&fs::read(tape_path).map_err(route_error)?)
            .map_err(route_error)?
            .tape;
        if final_result.execution_authority_sha256 != execution_plan_sha256
            || final_result.route_tape != tape
        {
            return Err(route_message(
                "completed tactic terminal artifacts belong to another execution plan",
            ));
        }
    }
    Ok(result)
}
