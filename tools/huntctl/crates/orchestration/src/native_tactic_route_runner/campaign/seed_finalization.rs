//! Final projection and durable publication of one completed tactic seed.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn finalize_seed(
    config: &NativeTacticRouteRunConfig<'_>,
    lane: &NativeTacticLanePlan,
    execution_plan_sha256: Digest,
    seed: u64,
    seed_root: &Path,
    retained_success_root: &Path,
    decision_content_store: &TacticQContentStore,
    invocation_started: Instant,
    prior_wall_micros: u64,
    prior_model_update_micros: u64,
    mut campaign: TacticQCampaign,
    trace: Vec<NativeTacticDecisionTrace>,
    selection_counts: BTreeMap<String, u64>,
    native_ticks: u64,
    episode: u64,
    useful_decisions: u64,
    native_restore_accounting: NativeTacticRestoreAccounting,
    mut timing: NativeTacticRouteTiming,
    mut best_success: Option<TacticQFinalResult>,
    replay_session: Option<BoundedStalenessReplaySession>,
    lease_ledger: NativeTacticLeaseLedger,
) -> Result<CompletedNativeTacticSeed, NativeTacticRouteRunError> {
    let finalization_started_micros = elapsed_micros(invocation_started.elapsed());
    let finalization_top_baseline = ExclusiveTopTimingSnapshot::capture(&timing);
    let wall_budget_reached =
        config.execution_plan.budgets.wall_micros.reached(
            prior_wall_micros.saturating_add(elapsed_micros(invocation_started.elapsed())),
        ) && campaign.decision_index < config.execution_plan.budgets.decisions_per_lane
            && native_ticks < config.optimization.budgets.simulated_tick_budget
            && !config
                .execution_plan
                .budgets
                .native_ticks
                .reached(native_ticks);
    let mut stop_reasons = Vec::new();
    if campaign.decision_index >= config.execution_plan.budgets.decisions_per_lane {
        stop_reasons.push(NativeTacticSeedStopReason::DecisionBudgetReached);
    }
    if native_ticks >= config.optimization.budgets.simulated_tick_budget {
        stop_reasons.push(NativeTacticSeedStopReason::SimulatedTickBudgetReached);
    }
    if config
        .execution_plan
        .budgets
        .native_ticks
        .reached(native_ticks)
    {
        stop_reasons.push(NativeTacticSeedStopReason::NativeTickBudgetReached);
    }
    if config
        .execution_plan
        .budgets
        .wall_micros
        .reached(prior_wall_micros.saturating_add(elapsed_micros(invocation_started.elapsed())))
    {
        stop_reasons.push(NativeTacticSeedStopReason::WallBudgetReached);
    }
    if stop_reasons.is_empty() {
        return Err(route_message(
            "native tactic route stopped without exhausting a sealed budget",
        ));
    }
    let final_persistence_started = Instant::now();
    compact_tactic_decision_journal(&seed_root)?;
    synchronize_graph_terminal_result(
        &retained_success_root,
        campaign.decision_index,
        &campaign,
        &mut best_success,
    )?;
    let best_graph_terminal = campaign
        .best_graph_terminal_path()
        .map_err(route_error)?
        .cloned();
    if let Some(result) = best_success.as_ref() {
        if !campaign
            .final_result_matches_graph_terminal(result)
            .map_err(route_error)?
        {
            return Err(route_message(
                "retained terminal artifact drifted from state graph authority",
            ));
        }
    }
    let best_authenticated_tick = best_graph_terminal
        .as_ref()
        .and_then(|path| path.root_to_terminal_ticks.checked_sub(1));
    let success = best_authenticated_tick
        .is_some_and(|tick| tick < config.optimization.budgets.promotion_before_tick);
    let generated_training = lane_generated_training_corpus(&campaign, lane);
    let imported_training_replay_rows = campaign
        .training_replay_len()
        .saturating_sub(generated_training.transitions.len());
    let final_checkpoint_commit = campaign
        .write_checkpoint_with_content_store(
            &seed_root.join("final-checkpoint"),
            &decision_content_store,
        )
        .map_err(route_error)?;
    let final_checkpoint_path = final_checkpoint_commit.path;
    // The write above validates and commits the exact in-memory graph. Project
    // report fields from that authority instead of reconstructing the whole
    // graph from disk here. Completed-result admission independently reloads
    // the checkpoint and verifies these fields before reuse.
    let final_state_graph = campaign.state_graph().map_err(route_error)?;
    let unique_useful_graph_expansions =
        u64::try_from(final_state_graph.completed_executable_expansion_count())
            .map_err(route_error)?;
    let useful_graph_expansion_set_sha256 =
        final_state_graph.completed_executable_expansion_set_sha256();
    let state_graph_sha256 = final_state_graph.content_sha256().map_err(route_error)?;
    let graph_metrics = tactic_graph_metrics(
        final_state_graph,
        state_graph_sha256,
        &trace,
        lease_ledger.accounting()?,
    )?;
    let mut useful_graph_expansions = CampaignUsefulGraphExpansionSet::default();
    useful_graph_expansions.include_graph(final_state_graph);
    let root_facts = final_state_graph
        .node(final_state_graph.root())
        .ok_or_else(|| route_message("completed tactic seed graph has no root state"))?
        .state
        .as_ref()
        .clone();
    let terminal_discovered = best_graph_terminal.is_some();
    let (best_terminal_tape, best_terminal_result) = if let Some(result) = best_success.as_ref() {
        let retained_candidate_started = Instant::now();
        let tape_path = seed_root.join("best-terminal.tape");
        write_new(
            &tape_path,
            &result.route_tape.encode().map_err(route_error)?,
        )?;
        let result_path = seed_root.join("best-terminal-result.dtqz");
        result.write(&result_path).map_err(route_error)?;
        timing.retained_candidate_artifact_micros = timing
            .retained_candidate_artifact_micros
            .saturating_add(elapsed_micros(retained_candidate_started.elapsed()));
        (Some(path_text(&tape_path)), Some(path_text(&result_path)))
    } else {
        (None, None)
    };
    let (successful_tape, final_result) = if success {
        (best_terminal_tape.clone(), best_terminal_result.clone())
    } else {
        (None, None)
    };
    let final_persistence_micros = elapsed_micros(final_persistence_started.elapsed());
    record_persistence_timing(
        &mut timing,
        NativeTacticPersistenceTiming {
            finalization_micros: final_persistence_micros,
            ..NativeTacticPersistenceTiming::default()
        },
    )?;
    if trace.len() as u64 != campaign.decision_index {
        return Err(route_message(
            "in-memory tactic trace is detached from the completed campaign",
        ));
    }
    let replay_sharing = replay_session
        .as_ref()
        .map(BoundedStalenessReplaySession::telemetry)
        .unwrap_or_default();
    let generated_training_rows = generated_training.transitions.len() as u64;
    let duplicate_training_transitions = native_restore_accounting
        .proposal_transitions
        .saturating_sub(generated_training_rows);
    let censored_training_transitions = censored_training_transitions(&generated_training);
    let first_terminal = trace.iter().find(|decision| {
        decision
            .proposal_batch
            .iter()
            .any(|proposal| proposal.terminal)
    });
    let completed_invocation_micros = elapsed_micros(invocation_started.elapsed());
    let invocation_model_update_micros = timing
        .model_update_micros
        .checked_sub(prior_model_update_micros)
        .ok_or_else(|| route_message("native tactic invocation model timing regressed"))?;
    let finalization_wall_micros = completed_invocation_micros
        .checked_sub(finalization_started_micros)
        .ok_or_else(|| route_message("native tactic finalization clock regressed"))?;
    let finalization_known_top_micros = finalization_top_baseline.checked_delta_total(&timing)?;
    let seed_finalization_micros = finalization_wall_micros
        .checked_sub(finalization_known_top_micros)
        .ok_or_else(|| route_message("native tactic seed finalization timing is detached"))?;
    record_orchestration_detail(
        &mut timing,
        OrchestrationPhase::SeedFinalization,
        seed_finalization_micros,
    )?;
    record_orchestration_total(&mut timing, seed_finalization_micros)?;
    timing.wall_micros = prior_wall_micros
        .checked_add(completed_invocation_micros)
        .ok_or_else(|| route_message("native tactic seed wall timing overflowed"))?;
    if timing.orchestration_breakdown.is_some() {
        let accounted_micros = [
            timing.tactic_execution_micros,
            timing.model_update_micros,
            timing.evidence_projection_micros,
            timing.persistence_micros,
            timing.orchestration_micros,
        ]
        .into_iter()
        .try_fold(0_u64, u64::checked_add)
        .ok_or_else(|| route_message("native tactic seed phase timing overflowed"))?;
        let timing_boundary_micros = timing
            .wall_micros
            .checked_sub(accounted_micros)
            .ok_or_else(|| route_message("native tactic seed phases exceed seed wall"))?;
        record_orchestration_detail(
            &mut timing,
            OrchestrationPhase::TimingBoundary,
            timing_boundary_micros,
        )?;
        record_orchestration_total(&mut timing, timing_boundary_micros)?;
    }
    if timing.orchestration_breakdown.is_some() && !timing.seed_wall_attribution_is_exact() {
        return Err(route_message(
            "native tactic seed phases do not reconcile to seed wall",
        ));
    }
    timing.useful_decisions_per_second_millionths =
        per_second_millionths(useful_decisions, timing.wall_micros);
    timing.native_ticks_per_second_millionths =
        per_second_millionths(native_ticks, timing.wall_micros);
    timing.episodes_per_second_millionths =
        per_second_millionths(episode.saturating_add(1), timing.wall_micros);
    let result = NativeTacticSeedResult {
        execution_plan_sha256,
        seed,
        terminal_discovered,
        best_authenticated_tick,
        first_terminal_decision_index: first_terminal.map(|decision| decision.decision_index),
        time_to_first_terminal_micros: first_terminal
            .map(|decision| decision.cumulative_wall_micros),
        wall_budget_reached,
        stop_reasons,
        success,
        decisions: campaign.decision_index,
        episodes: episode + 1,
        native_ticks,
        replay_rows: campaign.replay().len(),
        training_replay_rows: campaign.training_replay_len(),
        imported_training_replay_rows,
        duplicate_training_transitions,
        censored_training_transitions,
        learner_updates: 0,
        replay_sharing,
        visited_states: campaign.visited_state_count(),
        useful_decisions,
        unique_useful_graph_expansions,
        native_restore_accounting,
        timing,
        selection_counts,
        diagnostics: None,
        final_checkpoint: path_text(&final_checkpoint_path),
        state_graph_sha256,
        useful_graph_expansion_set_sha256,
        graph_metrics: Some(graph_metrics),
        best_terminal_state_sha256: best_graph_terminal
            .as_ref()
            .map(|path| path.terminal.state_sha256),
        best_terminal_route_checkpoint_sha256: best_graph_terminal
            .as_ref()
            .map(|path| path.route_checkpoint_sha256),
        best_terminal_tape,
        best_terminal_result,
        successful_tape,
        final_result,
        trace,
    };
    Ok(CompletedNativeTacticSeed {
        completion_projection: Some(NativeTacticSeedCompletionProjection {
            final_checkpoint_content_sha256: final_checkpoint_commit.content_sha256,
            feature_schema_sha256: campaign.feature_schema_sha256,
            objective_sha256: campaign.objective_sha256,
            root_checkpoint_sha256: campaign.root_checkpoint_sha256,
            root_facts,
            useful_graph_expansion_identities: useful_graph_expansions.identities(),
        }),
        result,
        generated_training,
        invocation_wall_micros: completed_invocation_micros,
        invocation_model_update_micros,
    })
}
