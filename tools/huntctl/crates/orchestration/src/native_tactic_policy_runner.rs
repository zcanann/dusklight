//! Native execution of one immutable greedy tactic-Q policy from its sealed root.

use crate::native_residual_campaign::NativeResidualExecutionBinding;
use crate::native_suffix_result::NativeTerminalBinding;
use crate::native_suffix_worker::{NativeSuffixWorkerLaunch, NativeSuffixWorkerSession};
use crate::native_tactic_route_runner::{
    NativeTacticGoalTargetReport, goal_conditioned_tactic_runtime, initial_facts, path_text,
    tactic_root_probe_batch, write_new,
};
use crate::native_tactic_worker::{
    NativeTacticWorkerPaths, execute_selected_tactic, tactic_root_checkpoint_sha256,
};
use crate::optimization_request::OptimizationRequest;
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::InputTape;
use dusklight_learning::fact_registry::FactRegistry;
use dusklight_learning::learner_state::LearnerState;
use dusklight_learning::live_tactic_catalog::LiveTacticCatalog;
use dusklight_learning::tactic_exploration::{
    TacticExplorationConfig, TacticSelectionReason, choose_tactic,
};
use dusklight_learning::tactic_frozen_policy::TacticFrozenPolicy;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;

pub const NATIVE_TACTIC_POLICY_REPORT_SCHEMA_V2: &str = "dusklight-native-tactic-policy-report/v2";
const MAX_GREEDY_DECISIONS: u64 = 100_000;

#[derive(Clone, Debug)]
pub struct NativeTacticPolicyRunConfig<'a> {
    pub repository_root: &'a Path,
    pub optimization: &'a OptimizationRequest,
    pub execution: &'a NativeResidualExecutionBinding,
    pub policy: &'a TacticFrozenPolicy,
    pub output_root: &'a Path,
    pub maximum_decisions: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticPolicyReport {
    pub schema: String,
    pub report_sha256: Digest,
    pub policy_sha256: Digest,
    pub source_campaign_sha256: Digest,
    pub optimization_request_sha256: Digest,
    pub execution_binding_sha256: Digest,
    pub objective_sha256: Digest,
    pub root_checkpoint_sha256: Digest,
    pub root_state_sha256: Digest,
    pub feature_schema_sha256: Digest,
    pub action_universe_sha256: Digest,
    pub goal_target: NativeTacticGoalTargetReport,
    pub exploration_enabled: bool,
    pub success: bool,
    pub stop_reason: String,
    pub decisions: u64,
    pub native_ticks: u64,
    pub terminal_state_sha256: Digest,
    pub realized_tape_sha256: Digest,
    pub realized_tape: String,
    pub trace: Vec<NativeTacticPolicyDecision>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticPolicyDecision {
    pub decision_index: u64,
    pub source_state_sha256: Digest,
    pub selected_option_id: String,
    pub selection_reason: TacticSelectionReason,
    pub selected_q: f64,
    pub ensemble_variance: f64,
    pub applicable_tactics: usize,
    pub unsupported_tactics: usize,
    pub realized_ticks: u32,
    pub next_state_sha256: Digest,
    pub terminal: bool,
}

pub fn run_native_tactic_policy(
    config: &NativeTacticPolicyRunConfig<'_>,
) -> Result<NativeTacticPolicyReport, NativeTacticPolicyRunError> {
    validate_config(config)?;
    let root = config
        .repository_root
        .canonicalize()
        .map_err(policy_error)?;
    config.policy.validate().map_err(policy_error)?;
    config
        .execution
        .validate_files(&root, config.optimization)
        .map_err(policy_error)?;
    if config.output_root.exists() {
        return Err(policy_message(format!(
            "native tactic policy output already exists: {}",
            config.output_root.display()
        )));
    }
    fs::create_dir_all(config.output_root).map_err(policy_error)?;

    let registry = FactRegistry::canonical();
    if config.policy.objective_sha256 != config.optimization.terminal_predicate.definition_sha256 {
        return Err(policy_message(
            "frozen tactic policy differs from the requested objective",
        ));
    }
    let model = config.policy.reconstruct_model().map_err(policy_error)?;
    let process_tape_bytes =
        fs::read(root.join(&config.execution.process_boot_tape.path)).map_err(policy_error)?;
    let process_tape = InputTape::decode(&process_tape_bytes)
        .map_err(policy_error)?
        .tape;
    let source_frame =
        usize::try_from(config.optimization.route.source_boundary_index).map_err(policy_error)?;
    let mut route_tape = InputTape {
        boot: process_tape.boot.clone(),
        tick_rate_numerator: process_tape.tick_rate_numerator,
        tick_rate_denominator: process_tape.tick_rate_denominator,
        frames: process_tape
            .frames
            .get(..source_frame)
            .ok_or_else(|| policy_message("source frame is beyond the process tape"))?
            .to_vec(),
    };
    route_tape.validate().map_err(policy_error)?;

    let initial_batch =
        tactic_root_probe_batch(config.optimization, config.execution).map_err(policy_error)?;
    let initial_root = config.output_root.join("initial");
    fs::create_dir_all(&initial_root).map_err(policy_error)?;
    let initial_batch_path = initial_root.join("request.json");
    write_new(
        &initial_batch_path,
        &serde_json::to_vec_pretty(&initial_batch).map_err(policy_error)?,
    )
    .map_err(policy_error)?;
    let launch = NativeSuffixWorkerLaunch {
        executable: root.join(&config.execution.executable.path),
        game_data: root.join(&config.execution.game_data.path),
        input_tape: root.join(&config.execution.process_boot_tape.path),
        milestone_program: root.join(&config.execution.milestone_program.path),
        card_fixture: config
            .execution
            .card_fixture_root(&root, config.optimization)
            .map_err(policy_error)?,
        card_fixture_sha256: config.execution.card_fixture_manifest.sha256,
        working_directory: root,
        state_root: config.output_root.join("native-state"),
        world_context_sha256: config.execution.world_context.sha256,
        terminal: NativeTerminalBinding {
            goal: config.optimization.terminal_predicate.goal.clone(),
            program_sha256: config.optimization.terminal_predicate.program_sha256,
            definition_sha256: config.optimization.terminal_predicate.definition_sha256,
        },
        initial_batch: initial_batch_path,
        initial_result: initial_root.join("result.json"),
        initial_winner_tape: None,
    };
    let (mut worker, initial) = NativeSuffixWorkerSession::launch(&launch).map_err(policy_error)?;
    let initial_snapshot = initial_facts(&initial).map_err(policy_error)?;
    let tactic_runtime = goal_conditioned_tactic_runtime(
        &launch.working_directory,
        config.optimization,
        config.execution,
        &initial_snapshot,
    )
    .map_err(policy_error)?;
    if config.policy.feature_schema_sha256 != tactic_runtime.encoder.schema_sha256
        || config.policy.action_universe_sha256 != tactic_runtime.catalog.action_schema_sha256()
    {
        let _ = worker.shutdown();
        return Err(policy_message(
            "frozen tactic policy differs from the goal-conditioned features or catalog",
        ));
    }
    let catalog = tactic_runtime.catalog;
    let encoder = tactic_runtime.encoder;
    let goal_target = tactic_runtime.report;
    let initial_sha256 = initial_snapshot.content_sha256().map_err(policy_error)?;
    let root_checkpoint_sha256 =
        tactic_root_checkpoint_sha256(worker.identity()).map_err(policy_error)?;
    if initial_snapshot.tape_frame != config.optimization.route.source_boundary_index
        || initial_snapshot.terminal.reached != Some(false)
        || initial_sha256 != config.policy.root_state_sha256
    {
        let _ = worker.shutdown();
        return Err(policy_message(format!(
            "native policy root state differs from the sealed campaign root: expected state {}, got {}; expected frame {}, got {}; terminal {:?}",
            config.policy.root_state_sha256,
            initial_sha256,
            config.optimization.route.source_boundary_index,
            initial_snapshot.tape_frame,
            initial_snapshot.terminal.reached,
        )));
    }
    if root_checkpoint_sha256 != config.policy.root_checkpoint_sha256 {
        let _ = worker.shutdown();
        return Err(policy_message(format!(
            "native policy execution identity differs from the sealed campaign root: expected {}, got {}",
            config.policy.root_checkpoint_sha256, root_checkpoint_sha256,
        )));
    }

    let run = (|| {
        let mut state = LearnerState::build(initial_snapshot, &registry, &catalog, &[], |_| true)
            .map_err(policy_error)?;
        let mut trace = Vec::new();
        let mut decisions = 0_u64;
        let mut native_ticks = 0_u64;
        let horizon = config.optimization.budgets.exploration_horizon_ticks;
        let stop_reason = loop {
            if state.snapshot.terminal.reached == Some(true) {
                break "terminal";
            }
            if decisions >= config.maximum_decisions {
                break "decision_limit";
            }
            if native_ticks >= config.optimization.budgets.simulated_tick_budget {
                break "tick_budget";
            }
            let live = LiveTacticCatalog::build(&state, &catalog, &[]).map_err(policy_error)?;
            let features = encoder.encode(&state.snapshot).map_err(policy_error)?;
            let ranking = live.rank(&model, &features).map_err(policy_error)?;
            let selected = choose_tactic(
                &ranking,
                decisions,
                TacticExplorationConfig {
                    seed: 0,
                    epsilon_per_million: 0,
                },
            )
            .map_err(policy_error)?;
            let ranked = ranking
                .values
                .ranked
                .iter()
                .find(|ranked| ranked.descriptor == selected.descriptor)
                .ok_or_else(|| {
                    policy_message(
                        "frozen policy has no trained value for an applicable greedy tactic",
                    )
                })?;
            if selected.reason != TacticSelectionReason::Greedy {
                return Err(policy_message(
                    "frozen policy attempted a non-greedy tactic selection",
                ));
            }
            let maximum_tactic_ticks = ranking
                .choices
                .iter()
                .find(|choice| choice.descriptor == selected.descriptor)
                .map(|choice| u64::from(choice.duration.maximum_ticks))
                .ok_or_else(|| policy_message("greedy tactic is absent from the live catalog"))?;
            let suffix_ticks = route_tape.frames.len().saturating_sub(source_frame) as u64;
            if suffix_ticks.saturating_add(maximum_tactic_ticks) > horizon {
                break "horizon";
            }
            let source_state_sha256 = state.snapshot_sha256;
            let decision_root = config
                .output_root
                .join("native")
                .join(format!("decision-{decisions:06}"));
            let outcome = execute_selected_tactic(
                &mut worker,
                &selected,
                &catalog,
                &[],
                &state.snapshot,
                &route_tape,
                &NativeTacticWorkerPaths {
                    request: decision_root.join("request.json"),
                    result: decision_root.join("result.json"),
                },
            )
            .map_err(policy_error)?;
            native_ticks = native_ticks
                .checked_add(u64::from(outcome.execution.duration.realized_ticks))
                .ok_or_else(|| policy_message("native tactic tick count overflowed"))?;
            let next_state_sha256 = outcome.next_facts.content_sha256().map_err(policy_error)?;
            trace.push(NativeTacticPolicyDecision {
                decision_index: decisions,
                source_state_sha256,
                selected_option_id: selected.descriptor.option_id.clone(),
                selection_reason: selected.reason,
                selected_q: ranked.mean_q,
                ensemble_variance: ranked.ensemble_variance,
                applicable_tactics: ranking.choices.len(),
                unsupported_tactics: ranking.values.unsupported.len(),
                realized_ticks: outcome.execution.duration.realized_ticks,
                next_state_sha256,
                terminal: outcome.terminal,
            });
            route_tape = outcome.route_tape;
            state = LearnerState::build(outcome.next_facts, &registry, &catalog, &[], |_| true)
                .map_err(policy_error)?;
            decisions += 1;
        };
        Ok::<_, NativeTacticPolicyRunError>((state, trace, decisions, native_ticks, stop_reason))
    })();
    let shutdown = worker.shutdown().map_err(policy_error);
    let (state, trace, decisions, native_ticks, stop_reason) = run?;
    shutdown?;

    let tape_path = config.output_root.join("realized.tape");
    let tape_bytes = route_tape.encode().map_err(policy_error)?;
    write_new(&tape_path, &tape_bytes).map_err(policy_error)?;
    let mut report = NativeTacticPolicyReport {
        schema: NATIVE_TACTIC_POLICY_REPORT_SCHEMA_V2.into(),
        report_sha256: Digest::ZERO,
        policy_sha256: config.policy.content_sha256,
        source_campaign_sha256: config.policy.source_campaign_sha256,
        optimization_request_sha256: config.optimization.content_sha256,
        execution_binding_sha256: config.execution.content_sha256,
        objective_sha256: config.policy.objective_sha256,
        root_checkpoint_sha256: config.policy.root_checkpoint_sha256,
        root_state_sha256: config.policy.root_state_sha256,
        feature_schema_sha256: config.policy.feature_schema_sha256,
        action_universe_sha256: config.policy.action_universe_sha256,
        goal_target,
        exploration_enabled: false,
        success: state.snapshot.terminal.reached == Some(true),
        stop_reason: stop_reason.into(),
        decisions,
        native_ticks,
        terminal_state_sha256: state.snapshot_sha256,
        realized_tape_sha256: sha256(&tape_bytes),
        realized_tape: path_text(&tape_path),
        trace,
    };
    report.report_sha256 = report_identity(&report)?;
    write_new(
        &config.output_root.join("report.json"),
        &serde_json::to_vec_pretty(&report).map_err(policy_error)?,
    )
    .map_err(policy_error)?;
    Ok(report)
}

fn validate_config(
    config: &NativeTacticPolicyRunConfig<'_>,
) -> Result<(), NativeTacticPolicyRunError> {
    if config.maximum_decisions == 0
        || config.maximum_decisions > MAX_GREEDY_DECISIONS
        || config.maximum_decisions > config.optimization.budgets.candidate_budget
    {
        return Err(policy_message(
            "native tactic policy configuration is invalid",
        ));
    }
    Ok(())
}

fn report_identity(
    report: &NativeTacticPolicyReport,
) -> Result<Digest, NativeTacticPolicyRunError> {
    let mut canonical = report.clone();
    canonical.report_sha256 = Digest::ZERO;
    Ok(sha256(
        &serde_json::to_vec(&canonical).map_err(policy_error)?,
    ))
}

fn sha256(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeTacticPolicyRunError(String);

impl fmt::Display for NativeTacticPolicyRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeTacticPolicyRunError {}

fn policy_message(message: impl Into<String>) -> NativeTacticPolicyRunError {
    NativeTacticPolicyRunError(message.into())
}

fn policy_error(error: impl fmt::Display) -> NativeTacticPolicyRunError {
    policy_message(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_identity_changes_with_tape_evidence() {
        let mut report = NativeTacticPolicyReport {
            schema: NATIVE_TACTIC_POLICY_REPORT_SCHEMA_V2.into(),
            report_sha256: Digest::ZERO,
            policy_sha256: Digest([1; 32]),
            source_campaign_sha256: Digest([2; 32]),
            optimization_request_sha256: Digest([3; 32]),
            execution_binding_sha256: Digest([4; 32]),
            objective_sha256: Digest([5; 32]),
            root_checkpoint_sha256: Digest([6; 32]),
            root_state_sha256: Digest([7; 32]),
            feature_schema_sha256: Digest([8; 32]),
            action_universe_sha256: Digest([9; 32]),
            goal_target: NativeTacticGoalTargetReport {
                source_stage: "F_SP103".into(),
                source_room: 1,
                destination_stage: "F_SP104".into(),
                destination_room: 1,
                destination_point: 0,
                coordinate: [1.0, 2.0, 3.0],
                source_coordinate: [4.0, 5.0, 6.0],
                tactic_targets: vec![[1.0, 2.0, 3.0]],
                supporting_load_triggers: 1,
                source_inventory_sha256: Digest([13; 32]),
                authored_route_coordinates_used: false,
            },
            exploration_enabled: false,
            success: false,
            stop_reason: "decision_limit".into(),
            decisions: 1,
            native_ticks: 2,
            terminal_state_sha256: Digest([10; 32]),
            realized_tape_sha256: Digest([11; 32]),
            realized_tape: "realized.tape".into(),
            trace: Vec::new(),
        };
        let before = report_identity(&report).unwrap();
        report.realized_tape_sha256 = Digest([12; 32]);
        assert_ne!(before, report_identity(&report).unwrap());
    }
}
