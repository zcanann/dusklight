use super::campaign_schedule::acquisition_rank_for_episode;
use super::*;

pub(super) fn encode_seed_result_manifest(
    result: &NativeTacticSeedResult,
) -> Result<Vec<u8>, NativeTacticRouteRunError> {
    let mut manifest = result.clone();
    manifest.trace.clear();
    encode_bounded_compact_json(
        &manifest,
        MAX_SEED_RESULT_JSON_BYTES,
        "native tactic seed result manifest",
    )
}

pub(super) fn hydrate_seed_result_trace(
    result_path: &Path,
    result: &mut NativeTacticSeedResult,
) -> Result<(), NativeTacticRouteRunError> {
    if !result.trace.is_empty() {
        return Ok(());
    }
    if result.decisions == 0 {
        return Ok(());
    }
    if result.decision_trace_journal.as_deref() != Some(NATIVE_TACTIC_DECISION_JOURNAL_FILE) {
        return Err(route_message(
            "compact tactic seed result has no local binary trace authority",
        ));
    }
    let seed_root = result_path
        .parent()
        .ok_or_else(|| route_message("compact tactic seed result has no seed directory"))?;
    result.trace = read_tactic_decision_journal(seed_root)?;
    if result.trace.len() as u64 != result.decisions {
        return Err(route_message(
            "compact tactic seed result trace journal is incomplete",
        ));
    }
    Ok(())
}

pub(super) fn cancellation_requested(config: &NativeTacticRouteRunConfig<'_>) -> bool {
    config
        .cancellation
        .is_some_and(|cancellation| cancellation.load(Ordering::Acquire))
}

pub(super) fn load_seed_performance(
    seed_root: &Path,
    decisions: u64,
) -> Result<NativeTacticSeedPerformance, NativeTacticRouteRunError> {
    let performance = load_tactic_recovery_point(seed_root, decisions)?.performance;
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
    let trace = read_tactic_decision_journal(seed_root)?;
    let checkpoint_decision = trace.len() as u64;
    let recovery = load_tactic_recovery_point(seed_root, checkpoint_decision)?;
    let checkpoint_path = recovery.checkpoint_path;
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
    if campaign.replay().len() as u64 != campaign.decision_index {
        return Err(route_message(
            "paused tactic checkpoint has a detached decision history",
        ));
    }
    if trace.len() as u64 != campaign.decision_index {
        return Err(route_message(
            "native tactic recovery decision journal does not match its checkpoint",
        ));
    }
    let episode = trace.last().map_or(0, |decision| decision.episode);
    if campaign.episode_group != lane.episode_group(episode)?
        || trace
            .iter()
            .zip(campaign.replay())
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

pub(super) fn read_completed_seed_result(
    path: &Path,
    seed: u64,
    decisions_per_seed: u64,
    execution_plan_sha256: Digest,
    lane: &NativeTacticLanePlan,
    imported_demonstration: bool,
) -> Result<NativeTacticSeedResult, NativeTacticRouteRunError> {
    read_completed_seed(
        path,
        seed,
        decisions_per_seed,
        execution_plan_sha256,
        lane,
        imported_demonstration,
    )
    .map(|completed| completed.result)
}

pub(super) struct ValidatedCompletedNativeTacticSeed {
    pub(super) result: NativeTacticSeedResult,
    pub(super) checkpoint: TacticQCampaignCheckpoint,
}

pub(super) struct ValidatedCompletedSeedPreflight {
    pub(super) result: NativeTacticSeedResult,
    pub(super) root_facts: FactSnapshot,
    pub(super) root_checkpoint_sha256: Digest,
    pub(super) feature_schema_sha256: Digest,
    pub(super) objective_sha256: Digest,
    pub(super) useful_graph_expansions: CampaignUsefulGraphExpansionSet,
}

pub(super) fn read_completed_seed_preflight(
    path: &Path,
    seed: u64,
    decisions_per_seed: u64,
    execution_plan_sha256: Digest,
    lane: &NativeTacticLanePlan,
    imported_demonstration: bool,
) -> Result<ValidatedCompletedSeedPreflight, NativeTacticRouteRunError> {
    let seed_root = path
        .parent()
        .ok_or_else(|| route_message("completed tactic seed result has no seed root"))?;
    let completion_path = seed_root.join(NATIVE_TACTIC_SEED_COMPLETION_FILE);
    if completion_path.is_file() {
        let metadata = fs::symlink_metadata(path).map_err(route_error)?;
        if !metadata.file_type().is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() > MAX_RESUME_JSON_BYTES
        {
            return Err(route_message("completed tactic seed result is oversized"));
        }
        let result_bytes = fs::read(path).map_err(route_error)?;
        let mut result: NativeTacticSeedResult =
            serde_json::from_slice(&result_bytes).map_err(route_error)?;
        hydrate_seed_result_trace(path, &mut result)?;
        validate_completed_seed_result(
            &result,
            seed,
            decisions_per_seed,
            execution_plan_sha256,
            lane,
            imported_demonstration,
        )?;
        let completion = NativeTacticSeedCompletion::read_and_validate(
            &completion_path,
            seed_root,
            &result,
            &result_bytes,
        )?;
        validate_completed_terminal_artifacts(
            &result,
            execution_plan_sha256,
            completion.objective_sha256(),
            completion.root_checkpoint_sha256(),
        )?;
        return Ok(ValidatedCompletedSeedPreflight {
            result,
            root_facts: completion.root_facts().clone(),
            root_checkpoint_sha256: completion.root_checkpoint_sha256(),
            feature_schema_sha256: completion.feature_schema_sha256(),
            objective_sha256: completion.objective_sha256(),
            useful_graph_expansions: completion.useful_graph_expansions()?,
        });
    }

    let completed = read_completed_seed(
        path,
        seed,
        decisions_per_seed,
        execution_plan_sha256,
        lane,
        imported_demonstration,
    )?;
    let root_facts = completed
        .checkpoint
        .state_graph
        .node(completed.checkpoint.state_graph.root())
        .ok_or_else(|| route_message("completed seed graph has no root state"))?
        .state
        .as_ref()
        .clone();
    let mut useful_graph_expansions = CampaignUsefulGraphExpansionSet::default();
    useful_graph_expansions.include_graph(&completed.checkpoint.state_graph);
    Ok(ValidatedCompletedSeedPreflight {
        result: completed.result,
        root_facts,
        root_checkpoint_sha256: completed.checkpoint.root_checkpoint_sha256,
        feature_schema_sha256: completed.checkpoint.feature_schema_sha256,
        objective_sha256: completed.checkpoint.objective_sha256,
        useful_graph_expansions,
    })
}

pub(super) fn read_completed_seed(
    path: &Path,
    seed: u64,
    decisions_per_seed: u64,
    execution_plan_sha256: Digest,
    lane: &NativeTacticLanePlan,
    imported_demonstration: bool,
) -> Result<ValidatedCompletedNativeTacticSeed, NativeTacticRouteRunError> {
    let mut result: NativeTacticSeedResult = read_bounded_json(path)?;
    hydrate_seed_result_trace(path, &mut result)?;
    validate_completed_seed_result(
        &result,
        seed,
        decisions_per_seed,
        execution_plan_sha256,
        lane,
        imported_demonstration,
    )?;
    let checkpoint = TacticQCampaign::read_checkpoint_payload(Path::new(&result.final_checkpoint))
        .map_err(route_error)?;
    validate_completed_seed_against_checkpoint(result, checkpoint, execution_plan_sha256)
}

fn validate_completed_seed_result(
    result: &NativeTacticSeedResult,
    seed: u64,
    decisions_per_seed: u64,
    execution_plan_sha256: Digest,
    lane: &NativeTacticLanePlan,
    imported_demonstration: bool,
) -> Result<(), NativeTacticRouteRunError> {
    let first_terminal = result.trace.iter().find(|decision| {
        decision
            .proposal_batch
            .iter()
            .any(|proposal| proposal.terminal)
    });
    let has_durable_wall_timing = result
        .trace
        .iter()
        .any(|decision| decision.cumulative_wall_micros != 0)
        || result.first_terminal_decision_index.is_some()
        || result.time_to_first_terminal_micros.is_some();
    let has_graph_expansion_timeline = result
        .trace
        .iter()
        .any(|decision| decision.completed_executable_graph_expansions != 0);
    let first_authenticated_tick = result
        .trace
        .iter()
        .find(|decision| decision.best_authenticated_tick_after_decision.is_some());
    let has_authenticated_tick_timeline = first_authenticated_tick.is_some();
    let authenticated_terminal_origin_matches = authenticated_terminal_origin_matches(
        imported_demonstration || result.imported_training_replay_rows != 0,
        first_authenticated_tick.map(|decision| decision.decision_index),
        result.first_terminal_decision_index,
    );
    macro_rules! require_seed_result {
        ($condition:expr, $reason:literal) => {
            if !($condition) {
                return Err(route_message(concat!(
                    "completed tactic seed result is invalid: ",
                    $reason
                )));
            }
        };
    }

    require_seed_result!(
        result.execution_plan_sha256 == execution_plan_sha256,
        "execution-plan identity mismatch"
    );
    require_seed_result!(result.seed == seed, "seed identity mismatch");
    require_seed_result!(
        result.decisions <= decisions_per_seed,
        "decision budget exceeded"
    );
    require_seed_result!(
        result.useful_decisions <= result.decisions,
        "useful decision count exceeded total decisions"
    );
    require_seed_result!(
        result.terminal_discovered == result.best_authenticated_tick.is_some()
            && result.terminal_discovered == result.best_terminal_state_sha256.is_some()
            && result.terminal_discovered == result.best_terminal_route_checkpoint_sha256.is_some()
            && result.terminal_discovered == result.best_terminal_tape.is_some()
            && result.terminal_discovered == result.best_terminal_result.is_some(),
        "terminal artifact presence is inconsistent"
    );
    require_seed_result!(
        !result.success || result.terminal_discovered,
        "success lacks a terminal route"
    );
    require_seed_result!(
        result.success == result.successful_tape.is_some()
            && result.success == result.final_result.is_some(),
        "successful output artifact presence is inconsistent"
    );
    require_seed_result!(
        !result.final_checkpoint.is_empty(),
        "final checkpoint is absent"
    );
    require_seed_result!(
        result.state_graph_sha256 != Digest::ZERO,
        "state graph identity is absent"
    );
    require_seed_result!(
        result.terminal_discovered || result.timing.retained_candidate_artifact_micros == 0,
        "nonterminal seed reports retained terminal artifact time"
    );
    require_seed_result!(
        result.trace.len() as u64 == result.decisions,
        "durable trace length differs from decision count"
    );
    require_seed_result!(
        !has_graph_expansion_timeline
            || (result
                .trace
                .iter()
                .all(|decision| decision.completed_executable_graph_expansions != 0)
                && !result.trace.windows(2).any(|pair| {
                    pair[0].completed_executable_graph_expansions
                        > pair[1].completed_executable_graph_expansions
                })
                && result
                    .trace
                    .last()
                    .map(|decision| decision.completed_executable_graph_expansions)
                    == Some(result.unique_useful_graph_expansions)),
        "graph expansion timeline is invalid"
    );
    require_seed_result!(
        !has_authenticated_tick_timeline || authenticated_terminal_origin_matches,
        "authenticated terminal origin is inconsistent"
    );
    require_seed_result!(
        !has_authenticated_tick_timeline
            || !result.trace.windows(2).any(|pair| {
                match (
                    pair[0].best_authenticated_tick_after_decision,
                    pair[1].best_authenticated_tick_after_decision,
                ) {
                    (Some(_), None) => true,
                    (Some(previous), Some(next)) => next > previous,
                    _ => false,
                }
            }),
        "authenticated terminal quality regresses"
    );
    require_seed_result!(
        !has_authenticated_tick_timeline
            || result
                .trace
                .last()
                .and_then(|decision| decision.best_authenticated_tick_after_decision)
                == result.best_authenticated_tick,
        "authenticated terminal timeline disagrees with final best tick"
    );
    require_seed_result!(
        !has_durable_wall_timing
            || (result.first_terminal_decision_index
                == first_terminal.map(|decision| decision.decision_index)
                && result.time_to_first_terminal_micros
                    == first_terminal.map(|decision| decision.cumulative_wall_micros)
                && !result
                    .trace
                    .windows(2)
                    .any(|pair| pair[0].cumulative_wall_micros > pair[1].cumulative_wall_micros)),
        "wall-time timeline is invalid"
    );
    require_seed_result!(
        !result.trace.iter().enumerate().any(|(index, decision)| {
            decision.execution_plan_sha256 != execution_plan_sha256
                || decision.decision_index != index as u64
                || decision.learner_snapshot_sha256 == Digest::ZERO
                || decision.replay_generation != lane.generation_index as u64
                || decision.lane_index != lane.lane_index
                || decision.lane_role != Some(lane.role)
                || (decision.acquisition_rank != lane.acquisition.rank(decision.decision_index)
                    && decision.acquisition_rank != lane.acquisition.rank(decision.episode)
                    && decision.acquisition_rank
                        != acquisition_rank_for_episode(
                            lane.acquisition,
                            decision.episode,
                            imported_demonstration
                                || result.trace[..index]
                                    .iter()
                                    .any(|previous| previous.terminal),
                            &result.trace[..index],
                        )
                    && decision.acquisition_rank != 0)
                || decision.frontier_identity == Digest::ZERO
                || decision.restore_source.is_none()
                || decision.result_admission_schema != NATIVE_TACTIC_RESULT_ADMISSION_SCHEMA_V1
                || decision
                    .proposal_batch
                    .iter()
                    .any(|proposal| proposal.execution_plan_sha256 != execution_plan_sha256)
        }),
        "a decision is detached from its lane or execution authority"
    );
    require_seed_result!(
        result.native_ticks
            == result
                .trace
                .iter()
                .map(decision_evaluated_ticks)
                .sum::<u64>(),
        "native tick total differs from durable decisions"
    );
    Ok(())
}

fn validate_completed_seed_against_checkpoint(
    result: NativeTacticSeedResult,
    checkpoint: TacticQCampaignCheckpoint,
    execution_plan_sha256: Digest,
) -> Result<ValidatedCompletedNativeTacticSeed, NativeTacticRouteRunError> {
    let graph_sha256 = checkpoint
        .state_graph
        .content_sha256()
        .map_err(route_error)?;
    let useful_graph_expansions = u64::try_from(
        checkpoint
            .state_graph
            .completed_executable_expansion_count(),
    )
    .map_err(route_error)?;
    let useful_graph_expansion_set_sha256 = checkpoint
        .state_graph
        .completed_executable_expansion_set_sha256();
    let seed_root = Path::new(&result.final_checkpoint)
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| route_message("completed tactic seed checkpoint has no seed root"))?;
    if !seed_root.join(NATIVE_TACTIC_LEASE_JOURNAL_FILE).is_file() {
        return Err(route_message(
            "completed tactic seed has no durable lease journal",
        ));
    }
    let lease_accounting = NativeTacticLeaseLedger::open(seed_root)?.accounting()?;
    let expected_graph_metrics = tactic_graph_metrics(
        &checkpoint.state_graph,
        graph_sha256,
        &result.trace,
        lease_accounting,
    )?;
    let best_graph_terminal = checkpoint.state_graph.best_terminal_path();
    if checkpoint.execution_authority_sha256 != execution_plan_sha256
        || checkpoint.decision_index != result.decisions
        || checkpoint.replay.len() != result.replay_rows
        || checkpoint.training_replay.len() != result.training_replay_rows
        || checkpoint.state_graph.node_count() != result.visited_states
        || graph_sha256 != result.state_graph_sha256
        || useful_graph_expansions != result.unique_useful_graph_expansions
        || useful_graph_expansion_set_sha256 == Digest::ZERO
        || useful_graph_expansion_set_sha256 != result.useful_graph_expansion_set_sha256
        || best_graph_terminal.is_some() != result.terminal_discovered
        || best_graph_terminal.map(|path| path.terminal.state_sha256)
            != result.best_terminal_state_sha256
        || best_graph_terminal.map(|path| path.route_checkpoint_sha256)
            != result.best_terminal_route_checkpoint_sha256
        || best_graph_terminal.and_then(|path| path.root_to_terminal_ticks.checked_sub(1))
            != result.best_authenticated_tick
        || result
            .graph_metrics
            .as_ref()
            .is_some_and(|metrics| metrics != &expected_graph_metrics)
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
    Ok(ValidatedCompletedNativeTacticSeed { result, checkpoint })
}

fn validate_completed_terminal_artifacts(
    result: &NativeTacticSeedResult,
    execution_plan_sha256: Digest,
    objective_sha256: Digest,
    root_checkpoint_sha256: Digest,
) -> Result<(), NativeTacticRouteRunError> {
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
        if terminal_result.execution_authority_sha256 != execution_plan_sha256
            || terminal_result.objective_sha256 != objective_sha256
            || terminal_result.root_checkpoint_sha256 != root_checkpoint_sha256
            || terminal_result.route_tape != tape
            || Some(terminal_result.terminal_state_sha256) != result.best_terminal_state_sha256
            || route_checkpoint(root_checkpoint_sha256, &tape).map_err(route_error)?
                != result
                    .best_terminal_route_checkpoint_sha256
                    .ok_or_else(|| route_message("terminal route checkpoint is absent"))?
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
            || final_result.objective_sha256 != objective_sha256
            || final_result.root_checkpoint_sha256 != root_checkpoint_sha256
            || final_result.route_tape != tape
        {
            return Err(route_message(
                "completed tactic terminal artifacts belong to another execution plan",
            ));
        }
    }
    Ok(())
}

fn authenticated_terminal_origin_matches(
    inherited_terminal_support: bool,
    first_authenticated_decision: Option<u64>,
    first_terminal_proposal_decision: Option<u64>,
) -> bool {
    match first_authenticated_decision {
        None => true,
        Some(0) if inherited_terminal_support => true,
        Some(decision) => Some(decision) == first_terminal_proposal_decision,
    }
}

#[cfg(test)]
mod tests {
    use super::authenticated_terminal_origin_matches;

    #[test]
    fn imported_terminal_support_precedes_native_proposal_discovery() {
        assert!(authenticated_terminal_origin_matches(true, Some(0), None));
        assert!(authenticated_terminal_origin_matches(
            true,
            Some(0),
            Some(17)
        ));
    }

    #[test]
    fn unassisted_terminal_timeline_still_names_its_discovery_decision() {
        assert!(authenticated_terminal_origin_matches(
            false,
            Some(17),
            Some(17)
        ));
        assert!(!authenticated_terminal_origin_matches(false, Some(0), None));
        assert!(!authenticated_terminal_origin_matches(
            false,
            Some(16),
            Some(17)
        ));
    }
}
