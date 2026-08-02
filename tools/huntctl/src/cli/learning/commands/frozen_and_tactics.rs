//! Frozen-policy, tactic execution, and Q-variant commands.

use super::{
    Digest, FactorizedPolicyOutputSet, GoalConditionedTacticFeatureEncoder, InputTape,
    MAX_LEARN_INPUT_CORPORA, NATIVE_TACTIC_CAMPAIGN_COMPLETION_FILE, NativeEpisodeShard,
    NativeFactorizedPolicyBatchConfig, NativeFactorizedPolicySuffixBatch,
    NativeFrozenPolicyReinferenceReport, NativeFrozenPolicySuffixBatch,
    NativeGenericExecutionStrategy, NativeResidualExecutionBinding, NativeTacticCampaignCompletion,
    NativeTacticCampaignSummary, NativeTacticColdReplayConfig,
    NativeTacticColdReplayEvidenceBundle, NativeTacticDemonstrationReport,
    NativeTacticExecutionPlan, NativeTacticFaultInjector, NativeTacticFaultRecoveryEvidenceBundle,
    NativeTacticLaunchSmokeBundle, NativeTacticObservationAudit,
    NativeTacticOptimizationHandoffConfig, NativeTacticPolicyRunConfig,
    NativeTacticPostTerminalControlReport, NativeTacticRestoreLocalityConfig,
    NativeTacticRestoreLocalityReport, NativeTacticRouteDiagnosisReport, NativeTacticRouteReport,
    NativeTacticRouteRunConfig, NativeTacticScratchCampaignAudit,
    NativeTacticScratchComparisonReport, NativeTacticScratchDiscoveryReport,
    NativeTacticScratchEvidenceBundle, NativeTacticTerminalEvidenceBundle,
    NativeTacticThroughputCurveConfig, NativeTacticThroughputCurveRun,
    NativeTacticThroughputEvidenceBundle, NativeTacticThroughputTreatmentBundle,
    OptimizationRequest, Sha256, TacticFrozenPolicy, TacticProposalPolicy, TacticQCampaign,
    TacticQFinalResult, TacticQTrainingCorpus, audit_native_tactic_fault_recovery,
    build_native_tactic_optimization_handoff, cli, command_conservative_q, flag,
    native_frozen_policy_probe_model, native_tactic_execution_plan, option,
    prove_generalized_tactic_held_out_value, read_and_validate_native_tactic_cold_replay,
    realize_native_frozen_policy_tape, repeated_option, required_path,
    run_native_tactic_cold_replay, run_native_tactic_policy, run_native_tactic_restore_locality,
    run_native_tactic_route, run_native_tactic_throughput_curve_controlled,
    sealed_plan_shape_conflict, tactic_macro_registry_identity, u64_option, usage_error,
    usize_option, verify_native_frozen_policy_cold_replay, verify_native_frozen_policy_reinference,
};
use huntctl::search_evaluator::native_scratch_learner::{
    NativeScratchRunConfig, run_native_scratch_learner,
};
use serde_json::json;
use sha2::Digest as _;
use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

mod frozen_policy;

pub(super) fn command(args: &[String]) -> Result<(), Box<dyn Error>> {
    if frozen_policy::handles(args) {
        return frozen_policy::command(args);
    }
    match args.first().map(String::as_str) {
        Some("cql") => command_conservative_q(&args[1..]),
        Some("iql") => cli::learning::command_iql(&args[1..]),
        Some("ensemble-q") => cli::learning::command_ensemble_q(&args[1..]),
        Some("prioritized-q") => cli::learning::command_prioritized_q(&args[1..]),
        Some("ablate-q") => cli::learning::command_q_ablation(&args[1..]),
        Some("option-values") => cli::learning::command_option_values(&args[1..]),
        Some("benchmark-tactic-checkpoint-codecs") => {
            let learn_args = &args[1..];
            let benchmark = TacticQCampaign::benchmark_checkpoint_serialization(
                &required_path(learn_args, "--legacy-json-checkpoint")?,
                &required_path(learn_args, "--current-checkpoint")?,
                u64_option(learn_args, "--iterations", 100)?,
            )?;
            println!("{}", serde_json::to_string_pretty(&benchmark)?);
            Ok(())
        }
        Some("freeze-tactic-policy") => {
            let learn_args = &args[1..];
            let checkpoint =
                TacticQCampaign::read_checkpoint(&required_path(learn_args, "--checkpoint")?)?;
            let policy = checkpoint.freeze_greedy_policy()?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "frozen tactic policy output already exists: {}",
                    output.display()
                )
                .into());
            }
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let mut bytes = serde_json::to_vec_pretty(&policy)?;
            bytes.push(b'\n');
            fs::write(&output, bytes)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": policy.schema,
                    "policy": output,
                    "content_sha256": policy.content_sha256,
                    "source_campaign_sha256": policy.source_campaign_sha256,
                    "training_rows": policy.training_batch.samples.len(),
                    "exploration": "disabled",
                }))?
            );
            Ok(())
        }
        Some("execute-tactic-policy") => {
            let learn_args = &args[1..];
            let repository_root = fs::canonicalize(
                option(learn_args, "--repository-root")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_dir()?),
            )?;
            let request: OptimizationRequest =
                serde_json::from_slice(&fs::read(required_path(learn_args, "--request")?)?)?;
            let execution: NativeResidualExecutionBinding =
                serde_json::from_slice(&fs::read(required_path(learn_args, "--execution")?)?)?;
            let policy: TacticFrozenPolicy =
                serde_json::from_slice(&fs::read(required_path(learn_args, "--policy")?)?)?;
            let output_argument = required_path(learn_args, "--output")?;
            let output = if output_argument.is_absolute() {
                output_argument
            } else {
                repository_root.join(output_argument)
            };
            let report = run_native_tactic_policy(&NativeTacticPolicyRunConfig {
                repository_root: &repository_root,
                optimization: &request,
                execution: &execution,
                policy: &policy,
                output_root: &output,
                maximum_decisions: u64_option(learn_args, "--maximum-decisions", 256)?,
            })?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": report.schema,
                    "report": output.join("report.json"),
                    "policy_sha256": report.policy_sha256,
                    "exploration_enabled": report.exploration_enabled,
                    "success": report.success,
                    "stop_reason": report.stop_reason,
                    "decisions": report.decisions,
                    "native_ticks": report.native_ticks,
                    "realized_tape": report.realized_tape,
                    "realized_tape_sha256": report.realized_tape_sha256,
                }))?
            );
            Ok(())
        }
        Some("scratch-route") => {
            let learn_args = &args[1..];
            let repository_root = fs::canonicalize(
                option(learn_args, "--repository-root")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_dir()?),
            )?;
            let request: OptimizationRequest =
                serde_json::from_slice(&fs::read(required_path(learn_args, "--request")?)?)?;
            let execution: NativeResidualExecutionBinding =
                serde_json::from_slice(&fs::read(required_path(learn_args, "--execution")?)?)?;
            let output_argument = required_path(learn_args, "--output")?;
            let output = if output_argument.is_absolute() {
                output_argument
            } else {
                repository_root.join(output_argument)
            };
            let maximum_episode_ticks =
                u32::try_from(u64_option(learn_args, "--maximum-episode-ticks", 900)?)?;
            let epsilon_per_million =
                u32::try_from(u64_option(learn_args, "--epsilon-per-million", 200_000)?)?;
            let report = run_native_scratch_learner(&NativeScratchRunConfig {
                repository_root: &repository_root,
                optimization: &request,
                execution: &execution,
                output_root: &output,
                seed: u64_option(learn_args, "--seed", 0)?,
                episodes: u64_option(learn_args, "--episodes", 100)?,
                maximum_episode_ticks,
                epsilon_per_million,
                maximum_wall_time: Duration::from_secs(u64_option(
                    learn_args,
                    "--wall-time-seconds",
                    600,
                )?),
                cold_replay_timeout: Duration::from_secs(u64_option(
                    learn_args,
                    "--cold-replay-timeout-seconds",
                    120,
                )?),
            })?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": report.schema,
                    "report": output.join("report.json"),
                    "completed_episodes": report.completed_episodes,
                    "stop_reason": report.stop_reason,
                    "unique_transitions": report.unique_transitions,
                    "terminal_episodes": report.terminal_episodes,
                    "fastest_selected_ticks": report.fastest_selected_ticks,
                    "learner_updates": report.learner_updates,
                    "changed_choices": report.changed_choices,
                    "native_ticks": report.native_ticks,
                    "native_wall_micros": report.native_wall_micros,
                    "wall_micros": report.wall_micros,
                    "first_terminal_wall_micros": report.first_terminal_wall_micros,
                }))?
            );
            Ok(())
        }
        Some("prove-generalized-tactics") => {
            let learn_args = &args[1..];
            let direct_inputs = repeated_option(learn_args, "--input")
                .into_iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>();
            let campaign_root = option(learn_args, "--campaign-root").map(PathBuf::from);
            if !direct_inputs.is_empty() && campaign_root.is_some() {
                return Err(
                    "generalized tactic evidence accepts --input or --campaign-root, not both"
                        .into(),
                );
            }
            let inputs = if let Some(campaign_root) = campaign_root {
                let mut seed_roots = fs::read_dir(&campaign_root)?
                    .map(|entry| entry.map(|entry| entry.path()))
                    .collect::<Result<Vec<_>, _>>()?;
                seed_roots.retain(|path| {
                    path.is_dir()
                        && path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| name.starts_with("seed-"))
                });
                seed_roots.sort();
                seed_roots
                    .into_iter()
                    .map(|root| root.join("generated-training.dtqc"))
                    .collect()
            } else {
                direct_inputs
            };
            if inputs.is_empty() || inputs.len() > MAX_LEARN_INPUT_CORPORA {
                return Err(format!(
                    "learn prove-generalized-tactics requires 1..={MAX_LEARN_INPUT_CORPORA} corpora"
                )
                .into());
            }
            let corpora = inputs
                .iter()
                .map(|path| TacticQTrainingCorpus::read(path))
                .collect::<Result<Vec<_>, _>>()?;
            let goal_distance_feature =
                GoalConditionedTacticFeatureEncoder::new([0.0; 3])?.goal_distance_feature();
            let report = prove_generalized_tactic_held_out_value(
                &corpora,
                usize_option(learn_args, "--goal-distance-feature", goal_distance_feature)?,
            )?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "generalized tactic evidence output already exists: {}",
                    output.display()
                )
                .into());
            }
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let mut bytes = serde_json::to_vec_pretty(&report)?;
            bytes.push(b'\n');
            fs::write(&output, bytes)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": report.schema,
                    "output": output,
                    "input_corpora": report.input_corpora,
                    "unique_native_transitions": report.unique_native_transitions,
                    "unique_controller_instances": report.unique_controller_instances,
                    "passed": report.passed,
                }))?
            );
            if !report.passed {
                return Err("held-out generalized tactic comparisons did not all pass".into());
            }
            Ok(())
        }
        Some("tactic-route") => {
            let learn_args = &args[1..];
            let repository_root = fs::canonicalize(
                option(learn_args, "--repository-root")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_dir()?),
            )?;
            let request: OptimizationRequest =
                serde_json::from_slice(&fs::read(required_path(learn_args, "--request")?)?)?;
            let execution: NativeResidualExecutionBinding =
                serde_json::from_slice(&fs::read(required_path(learn_args, "--execution")?)?)?;
            let output_argument = required_path(learn_args, "--output")?;
            let output = if output_argument.is_absolute() {
                output_argument
            } else {
                repository_root.join(output_argument)
            };
            let workers = usize_option(
                learn_args,
                "--workers",
                usize::from(request.execution.workers),
            )?;
            let checkpoint_capacity_workers =
                usize_option(learn_args, "--checkpoint-capacity-workers", workers)?;
            let promoted_tactic_registry =
                option(learn_args, "--promoted-tactic-registry").map(PathBuf::from);
            let promoted_tactic_registry_sha256 =
                if let Some(path) = promoted_tactic_registry.as_deref() {
                    let (sha256, promoted_count) = tactic_macro_registry_identity(path)?;
                    if promoted_count == 0 {
                        return Err("promoted tactic registry contains no promoted tactics".into());
                    }
                    Some(sha256)
                } else {
                    None
                };
            let execution_plan = if let Some(plan_path) = option(learn_args, "--plan") {
                if let Some(conflict) = sealed_plan_shape_conflict(learn_args) {
                    return Err(format!(
                        "tactic route --plan cannot be combined with plan-shaping option {conflict}"
                    )
                    .into());
                }
                NativeTacticExecutionPlan::read(Path::new(&plan_path))?
            } else {
                let mut seeds = repeated_option(learn_args, "--seed")
                    .into_iter()
                    .map(|seed| seed.parse::<u64>())
                    .collect::<Result<Vec<_>, _>>()?;
                if seeds.is_empty() {
                    seeds = request.execution.deterministic_seeds.clone();
                }
                seeds.sort_unstable();
                seeds.dedup();
                let proposal_policy_argument = option(learn_args, "--proposal-policy");
                let proposal_policy = match proposal_policy_argument.as_deref().unwrap_or("learned")
                {
                    "learned" => TacticProposalPolicy::Learned,
                    "frozen-policy" => TacticProposalPolicy::FrozenPolicy,
                    "random-valid" => TacticProposalPolicy::RandomValid,
                    "structured-non-learning" => TacticProposalPolicy::StructuredNonLearning,
                    value => {
                        return Err(format!(
                            "unknown tactic proposal policy {value:?}; expected learned, frozen-policy, random-valid, or structured-non-learning"
                            )
                            .into());
                    }
                };
                let execution_strategy = match option(learn_args, "--execution-strategy")
                    .as_deref()
                    .unwrap_or("native-controller")
                {
                    "native-controller" => NativeGenericExecutionStrategy::NativeController,
                    "progressive-audit" => NativeGenericExecutionStrategy::ProgressiveAudit,
                    value => {
                        return Err(format!(
                            "unknown tactic execution strategy {value:?}; expected native-controller or progressive-audit"
                        )
                        .into());
                    }
                };
                native_tactic_execution_plan(
                    learn_args,
                    &request,
                    &seeds,
                    proposal_policy,
                    execution_strategy,
                    promoted_tactic_registry_sha256,
                )?
            };
            let fault_injection = option(learn_args, "--fault-inject")
                .map(|point| {
                    Ok::<_, Box<dyn Error>>(NativeTacticFaultInjector::process_exit(
                        point.parse()?,
                        u64_option(learn_args, "--fault-decision", 0)?,
                    ))
                })
                .transpose()?;
            let report = run_native_tactic_route(&NativeTacticRouteRunConfig {
                repository_root: &repository_root,
                optimization: &request,
                execution: &execution,
                execution_plan: &execution_plan,
                promoted_tactic_registry: promoted_tactic_registry.as_deref(),
                output_root: &output,
                checkpoint_capacity_workers,
                workers,
                cancellation: None,
                fault_injection: fault_injection.as_ref(),
                resume: flag(learn_args, "--resume"),
            })?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": report.schema,
                    "report": output.join("report.json"),
                    "summary": output.join("campaign-summary.json"),
                    "terminal_seeds": report.terminal_seeds,
                    "best_authenticated_tick": report.best_authenticated_tick,
                    "promotion_successful_seeds": report.promotion_successful_seeds,
                    "successful_seeds": report.successful_seeds,
                    "exploration_seeds": report.exploration_seeds,
                    "proposal_policy": report.proposal_policy,
                    "execution_strategy": report.execution_strategy,
                    "total_decisions": report.total_decisions,
                    "total_native_ticks": report.total_native_ticks,
                    "median_time_to_first_terminal_micros":
                        report.median_time_to_first_terminal_micros,
                    "worst_time_to_first_terminal_micros":
                        report.worst_time_to_first_terminal_micros,
                    "demonstration_transitions": report.demonstration_transitions,
                }))?
            );
            Ok(())
        }
        Some("project-tactic-route-accounting") => {
            let learn_args = &args[1..];
            let repository_root = fs::canonicalize(
                option(learn_args, "--repository-root")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_dir()?),
            )?;
            let mut route: NativeTacticRouteReport =
                serde_json::from_slice(&fs::read(required_path(learn_args, "--report")?)?)?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "tactic route accounting output already exists: {}",
                    output.display()
                )
                .into());
            }
            let previous_unique_useful_graph_expansions = route.unique_useful_graph_expansions;
            route.reproject_useful_graph_accounting(&repository_root)?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let mut bytes = serde_json::to_vec_pretty(&route)?;
            bytes.push(b'\n');
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&output)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": route.schema,
                    "previous_unique_useful_graph_expansions":
                        previous_unique_useful_graph_expansions,
                    "unique_useful_graph_expansions": route.unique_useful_graph_expansions,
                    "output": output,
                }))?
            );
            Ok(())
        }
        Some("project-tactic-campaign-summary") => {
            let learn_args = &args[1..];
            let route: NativeTacticRouteReport =
                serde_json::from_slice(&fs::read(required_path(learn_args, "--report")?)?)?;
            let plan = NativeTacticExecutionPlan::read(&required_path(learn_args, "--plan")?)?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "tactic campaign summary output already exists: {}",
                    output.display()
                )
                .into());
            }
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)?;
            }
            let summary = NativeTacticCampaignSummary::build(&route, &plan)?;
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&output)?;
            file.write_all(&summary.to_pretty_json()?)?;
            file.sync_all()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": summary.schema,
                    "content_sha256": summary.content_sha256,
                    "route_report_sha256": summary.route_report_sha256,
                    "output": output,
                }))?
            );
            Ok(())
        }
        Some("validate-tactic-campaign-summary") => {
            let learn_args = &args[1..];
            let route: NativeTacticRouteReport =
                serde_json::from_slice(&fs::read(required_path(learn_args, "--report")?)?)?;
            let plan = NativeTacticExecutionPlan::read(&required_path(learn_args, "--plan")?)?;
            let summary: NativeTacticCampaignSummary =
                serde_json::from_slice(&fs::read(required_path(learn_args, "--summary")?)?)?;
            summary.validate_against(&route, &plan)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": summary.schema,
                    "content_sha256": summary.content_sha256,
                    "route_report_sha256": summary.route_report_sha256,
                    "causal_chain_ready_for_matched_evaluation":
                        summary.causal_chain.causal_chain_ready_for_matched_evaluation,
                    "first_incomplete_link": summary.causal_chain.first_incomplete_link,
                    "passed": true,
                }))?
            );
            Ok(())
        }
        Some("prove-tactic-route-cold-replay") => {
            let learn_args = &args[1..];
            let repository_root = fs::canonicalize(
                option(learn_args, "--repository-root")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_dir()?),
            )?;
            let request: OptimizationRequest =
                serde_json::from_slice(&fs::read(required_path(learn_args, "--request")?)?)?;
            let execution: NativeResidualExecutionBinding =
                serde_json::from_slice(&fs::read(required_path(learn_args, "--execution")?)?)?;
            let execution_plan =
                NativeTacticExecutionPlan::read(&required_path(learn_args, "--plan")?)?;
            let route: NativeTacticRouteReport =
                serde_json::from_slice(&fs::read(required_path(learn_args, "--report")?)?)?;
            let output_argument = required_path(learn_args, "--output")?;
            let output = if output_argument.is_absolute() {
                output_argument
            } else {
                repository_root.join(output_argument)
            };
            let repetitions = u32::try_from(u64_option(learn_args, "--repetitions", 2)?)?;
            let timeout_seconds = u64_option(learn_args, "--timeout-seconds", 120)?;
            let seed = option(learn_args, "--seed")
                .ok_or("prove-tactic-route-cold-replay requires --seed")?
                .parse::<u64>()?;
            let proof = run_native_tactic_cold_replay(&NativeTacticColdReplayConfig {
                repository_root: &repository_root,
                optimization: &request,
                execution: &execution,
                execution_plan: &execution_plan,
                route_report: &route,
                seed,
                maximum_first_hit_tick: u64_option(
                    learn_args,
                    "--maximum-first-hit-tick",
                    request.budgets.exploration_horizon_ticks.saturating_sub(1),
                )?,
                repetitions,
                timeout: Duration::from_secs(timeout_seconds),
                output_root: &output,
            })?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": proof.schema,
                    "proof": output.join("proof.json"),
                    "content_sha256": proof.content_sha256,
                    "seed": proof.seed,
                    "first_hit_tick": proof.first_hit_tick,
                    "controller_tape_sha256": proof.controller_tape.sha256,
                    "repetitions": proof.attempts.len(),
                    "terminal_boundary_fingerprint": proof
                        .attempts
                        .first()
                        .map(|attempt| &attempt.boundary_fingerprint),
                }))?
            );
            Ok(())
        }
        Some("validate-tactic-route-cold-replay") => {
            let learn_args = &args[1..];
            let repository_root = fs::canonicalize(
                option(learn_args, "--repository-root")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_dir()?),
            )?;
            let request: OptimizationRequest =
                serde_json::from_slice(&fs::read(required_path(learn_args, "--request")?)?)?;
            let execution: NativeResidualExecutionBinding =
                serde_json::from_slice(&fs::read(required_path(learn_args, "--execution")?)?)?;
            let execution_plan =
                NativeTacticExecutionPlan::read(&required_path(learn_args, "--plan")?)?;
            let route: NativeTacticRouteReport =
                serde_json::from_slice(&fs::read(required_path(learn_args, "--report")?)?)?;
            let proof = read_and_validate_native_tactic_cold_replay(
                &repository_root,
                &request,
                &execution,
                &execution_plan,
                &route,
                &required_path(learn_args, "--proof-root")?,
            )?;
            println!("{}", serde_json::to_string_pretty(&proof)?);
            Ok(())
        }
        Some("seal-tactic-terminal-bundle") => {
            let learn_args = &args[1..];
            let repository_root = fs::canonicalize(
                option(learn_args, "--repository-root")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_dir()?),
            )?;
            let request_path = required_path(learn_args, "--request")?;
            let execution_path = required_path(learn_args, "--execution")?;
            let route_report_path = required_path(learn_args, "--report")?;
            let request: OptimizationRequest = serde_json::from_slice(&fs::read(&request_path)?)?;
            let execution: NativeResidualExecutionBinding =
                serde_json::from_slice(&fs::read(&execution_path)?)?;
            let route: NativeTacticRouteReport =
                serde_json::from_slice(&fs::read(&route_report_path)?)?;
            let seed = option(learn_args, "--seed")
                .ok_or("seal-tactic-terminal-bundle requires --seed")?
                .parse::<u64>()?;
            let bundle = NativeTacticTerminalEvidenceBundle::build(
                &required_path(learn_args, "--bundle")?,
                &repository_root,
                &request_path,
                &execution_path,
                &route_report_path,
                &request,
                &execution,
                &route,
                seed,
            )?;
            println!("{}", serde_json::to_string_pretty(&bundle)?);
            Ok(())
        }
        Some("validate-tactic-terminal-bundle") => {
            let bundle = NativeTacticTerminalEvidenceBundle::read_and_validate(&required_path(
                &args[1..],
                "--bundle",
            )?)?;
            println!("{}", serde_json::to_string_pretty(&bundle)?);
            Ok(())
        }
        Some("seal-tactic-cold-replay-bundle") => {
            let learn_args = &args[1..];
            let campaign_bundle = option(learn_args, "--campaign-bundle");
            let scratch_bundle = option(learn_args, "--scratch-bundle");
            if campaign_bundle.is_some() == scratch_bundle.is_some() {
                return Err("supply exactly one of --campaign-bundle or --scratch-bundle".into());
            }
            let bundle = if let Some(campaign_bundle) = campaign_bundle {
                NativeTacticColdReplayEvidenceBundle::build_terminal(
                    &required_path(learn_args, "--bundle")?,
                    Path::new(&campaign_bundle),
                    &required_path(learn_args, "--proof-root")?,
                )?
            } else {
                NativeTacticColdReplayEvidenceBundle::build(
                    &required_path(learn_args, "--bundle")?,
                    Path::new(scratch_bundle.as_deref().expect("checked above")),
                    &required_path(learn_args, "--proof-root")?,
                )?
            };
            println!("{}", serde_json::to_string_pretty(&bundle)?);
            Ok(())
        }
        Some("validate-tactic-cold-replay-bundle") => {
            let bundle = NativeTacticColdReplayEvidenceBundle::read_and_validate(&required_path(
                &args[1..],
                "--bundle",
            )?)?;
            println!("{}", serde_json::to_string_pretty(&bundle)?);
            Ok(())
        }
        Some("promote-tactic-terminal-for-optimization") => {
            let learn_args = &args[1..];
            let repository_root = fs::canonicalize(
                option(learn_args, "--repository-root")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_dir()?),
            )?;
            let request_path = option(learn_args, "--request");
            let execution_path = option(learn_args, "--execution");
            if request_path.is_some() != execution_path.is_some() {
                return Err("--request and --execution must be supplied together".into());
            }
            let request: Option<OptimizationRequest> = request_path
                .as_deref()
                .map(fs::read)
                .transpose()?
                .map(|bytes| serde_json::from_slice(&bytes))
                .transpose()?;
            let execution: Option<NativeResidualExecutionBinding> = execution_path
                .as_deref()
                .map(fs::read)
                .transpose()?
                .map(|bytes| serde_json::from_slice(&bytes))
                .transpose()?;
            let bundle_argument = required_path(learn_args, "--bundle")?;
            let bundle = if bundle_argument.is_absolute() {
                bundle_argument
            } else {
                repository_root.join(bundle_argument)
            };
            let output_argument = required_path(learn_args, "--output")?;
            let output = if output_argument.is_absolute() {
                output_argument
            } else {
                repository_root.join(output_argument)
            };
            let request_id = option(learn_args, "--id");
            let handoff =
                build_native_tactic_optimization_handoff(&NativeTacticOptimizationHandoffConfig {
                    repository_root: &repository_root,
                    source_optimization: request.as_ref(),
                    source_execution: execution.as_ref(),
                    cold_replay_bundle_root: &bundle,
                    output_root: &output,
                    request_id: request_id.as_deref(),
                    workers: option(learn_args, "--workers")
                        .map(|workers| workers.parse::<u16>())
                        .transpose()?,
                })?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": handoff.schema,
                    "content_sha256": handoff.content_sha256,
                    "manifest": output.join("handoff.json"),
                    "optimization_request": handoff.optimization_request.path,
                    "execution_binding": handoff.execution_binding.path,
                    "incumbent_tape": handoff.incumbent_tape.path,
                    "first_hit_tick": handoff.first_hit_tick,
                }))?
            );
            Ok(())
        }
        Some("run-tactic-launch-smoke") => {
            let learn_args = &args[1..];
            let repository_root = fs::canonicalize(
                option(learn_args, "--repository-root")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_dir()?),
            )?;
            let request_path = required_path(learn_args, "--request")?;
            let execution_path = required_path(learn_args, "--execution")?;
            let request: OptimizationRequest = serde_json::from_slice(&fs::read(&request_path)?)?;
            let execution: NativeResidualExecutionBinding =
                serde_json::from_slice(&fs::read(&execution_path)?)?;
            let seed = option(learn_args, "--seed")
                .ok_or("run-tactic-launch-smoke requires --seed")?
                .parse::<u64>()?;
            let memory_bytes = u64_option(learn_args, "--memory-bytes", 671_088_640)?;
            let wall_micros = u64_option(learn_args, "--wall-micros", 300_000_000)?;
            let plan_args = vec![
                "--decisions-per-seed".into(),
                "1".into(),
                "--proposals-per-decision".into(),
                "1".into(),
                "--branch-every".into(),
                "1".into(),
                "--refit-every".into(),
                "1".into(),
                "--memory-bytes".into(),
                memory_bytes.to_string(),
                "--wall-micros".into(),
                wall_micros.to_string(),
            ];
            let execution_plan = native_tactic_execution_plan(
                &plan_args,
                &request,
                &[seed],
                TacticProposalPolicy::Learned,
                NativeGenericExecutionStrategy::NativeController,
                None,
            )?;
            let output_argument = required_path(learn_args, "--output")?;
            let output = if output_argument.is_absolute() {
                output_argument
            } else {
                repository_root.join(output_argument)
            };
            let bundle_argument = required_path(learn_args, "--bundle")?;
            let bundle_root = if bundle_argument.is_absolute() {
                bundle_argument
            } else {
                repository_root.join(bundle_argument)
            };
            let report = run_native_tactic_route(&NativeTacticRouteRunConfig {
                repository_root: &repository_root,
                optimization: &request,
                execution: &execution,
                execution_plan: &execution_plan,
                promoted_tactic_registry: None,
                output_root: &output,
                checkpoint_capacity_workers: 1,
                workers: 1,
                cancellation: None,
                fault_injection: None,
                resume: false,
            })?;
            let bundle = NativeTacticLaunchSmokeBundle::build(
                &bundle_root,
                &repository_root,
                &request_path,
                &execution_path,
                &output.join("report.json"),
            )?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": bundle.schema,
                    "report": output.join("report.json"),
                    "report_schema": report.schema,
                    "bundle": bundle_root,
                    "bundle_sha256": bundle.content_sha256,
                    "summary": bundle.summary,
                    "passed": bundle.passed,
                }))?
            );
            Ok(())
        }
        Some("seal-tactic-launch-smoke") => {
            let learn_args = &args[1..];
            let repository_root = fs::canonicalize(
                option(learn_args, "--repository-root")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_dir()?),
            )?;
            let bundle_root = required_path(learn_args, "--bundle")?;
            let bundle = NativeTacticLaunchSmokeBundle::build(
                &bundle_root,
                &repository_root,
                &required_path(learn_args, "--request")?,
                &required_path(learn_args, "--execution")?,
                &required_path(learn_args, "--report")?,
            )?;
            println!("{}", serde_json::to_string_pretty(&bundle)?);
            Ok(())
        }
        Some("validate-tactic-launch-smoke") => {
            let bundle = NativeTacticLaunchSmokeBundle::read_and_validate(&required_path(
                &args[1..],
                "--bundle",
            )?)?;
            println!("{}", serde_json::to_string_pretty(&bundle)?);
            Ok(())
        }
        Some("tactic-throughput-curve") => {
            let learn_args = &args[1..];
            let repository_root = fs::canonicalize(
                option(learn_args, "--repository-root")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_dir()?),
            )?;
            let request_path = required_path(learn_args, "--request")?;
            let execution_path = required_path(learn_args, "--execution")?;
            let request: OptimizationRequest = serde_json::from_slice(&fs::read(&request_path)?)?;
            let execution: NativeResidualExecutionBinding =
                serde_json::from_slice(&fs::read(&execution_path)?)?;
            let output_argument = required_path(learn_args, "--output")?;
            let output = if output_argument.is_absolute() {
                output_argument
            } else {
                repository_root.join(output_argument)
            };
            let mut seeds = repeated_option(learn_args, "--seed")
                .into_iter()
                .map(|seed| seed.parse::<u64>())
                .collect::<Result<Vec<_>, _>>()?;
            if seeds.is_empty() {
                seeds.push(
                    request
                        .execution
                        .deterministic_seeds
                        .first()
                        .copied()
                        .ok_or("optimization request has no deterministic seed")?,
                );
            }
            seeds.sort_unstable();
            seeds.dedup();
            if seeds.len() != 1 {
                return Err(
                    "tactic throughput curve requires exactly one deterministic seed".into(),
                );
            }
            let execution_plan = native_tactic_execution_plan(
                learn_args,
                &request,
                &seeds,
                TacticProposalPolicy::Learned,
                NativeGenericExecutionStrategy::NativeController,
                None,
            )?;
            let stop_after_sample = option(learn_args, "--stop-after-sample")
                .map(|value| value.parse::<u32>())
                .transpose()?;
            if stop_after_sample.is_some() && option(learn_args, "--bundle").is_some() {
                return Err(
                    "tactic throughput curve cannot seal a bundle from a partial run".into(),
                );
            }
            let run = run_native_tactic_throughput_curve_controlled(
                &NativeTacticThroughputCurveConfig {
                    repository_root: &repository_root,
                    optimization: &request,
                    execution: &execution,
                    execution_plan: &execution_plan,
                    output_root: &output,
                    repetitions: usize_option(learn_args, "--repetitions", 2)?.try_into()?,
                    resume: flag(learn_args, "--resume"),
                },
                stop_after_sample,
            )?;
            let report = match run {
                NativeTacticThroughputCurveRun::Complete { report } => *report,
                stopped @ NativeTacticThroughputCurveRun::StoppedAfterSample { .. } => {
                    println!("{}", serde_json::to_string_pretty(&stopped)?);
                    return Ok(());
                }
            };
            let report_path = output.join("throughput-curve.json");
            let bundle = option(learn_args, "--bundle")
                .map(|argument| {
                    let argument = PathBuf::from(argument);
                    let bundle_root = if argument.is_absolute() {
                        argument
                    } else {
                        repository_root.join(argument)
                    };
                    NativeTacticThroughputEvidenceBundle::build(
                        &bundle_root,
                        &repository_root,
                        &request_path,
                        &execution_path,
                        &report_path,
                    )
                    .map(|bundle| {
                        json!({
                            "path": bundle_root,
                            "content_sha256": bundle.content_sha256,
                        })
                    })
                })
                .transpose()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": report.schema,
                    "report": report_path,
                    "bundle": bundle,
                    "fixed_unique_useful_graph_expansions":
                        report.fixed_unique_useful_graph_expansions,
                    "long_work_exercised": report.long_work_exercised,
                    "curve": report.curve,
                    "passed": report.passed,
                }))?
            );
            if !report.passed {
                return Err("native tactic fixed-work throughput curve did not pass".into());
            }
            Ok(())
        }
        Some("seal-tactic-throughput-curve") => {
            let learn_args = &args[1..];
            let repository_root = fs::canonicalize(
                option(learn_args, "--repository-root")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_dir()?),
            )?;
            let bundle_argument = required_path(learn_args, "--bundle")?;
            let bundle_root = if bundle_argument.is_absolute() {
                bundle_argument
            } else {
                repository_root.join(bundle_argument)
            };
            let bundle = NativeTacticThroughputEvidenceBundle::build(
                &bundle_root,
                &repository_root,
                &required_path(learn_args, "--request")?,
                &required_path(learn_args, "--execution")?,
                &required_path(learn_args, "--report")?,
            )?;
            println!("{}", serde_json::to_string_pretty(&bundle)?);
            Ok(())
        }
        Some("validate-tactic-throughput-curve-bundle") => {
            let bundle = NativeTacticThroughputEvidenceBundle::read_and_validate(&required_path(
                &args[1..],
                "--bundle",
            )?)?;
            println!("{}", serde_json::to_string_pretty(&bundle)?);
            Ok(())
        }
        Some("seal-tactic-throughput-treatment") => {
            let learn_args = &args[1..];
            let repository_root = fs::canonicalize(
                option(learn_args, "--repository-root")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_dir()?),
            )?;
            let resolve = |path: PathBuf| {
                if path.is_absolute() {
                    path
                } else {
                    repository_root.join(path)
                }
            };
            let bundle = NativeTacticThroughputTreatmentBundle::build(
                &resolve(required_path(learn_args, "--bundle")?),
                &repository_root,
                &resolve(required_path(learn_args, "--control-bundle")?),
                option(learn_args, "--control-sample-ordinal")
                    .ok_or("throughput treatment requires --control-sample-ordinal")?
                    .parse::<u32>()?,
                &required_path(learn_args, "--treatment-report")?,
            )?;
            println!("{}", serde_json::to_string_pretty(&bundle)?);
            Ok(())
        }
        Some("validate-tactic-throughput-treatment-bundle") => {
            let bundle = NativeTacticThroughputTreatmentBundle::read_and_validate(&required_path(
                &args[1..],
                "--bundle",
            )?)?;
            println!("{}", serde_json::to_string_pretty(&bundle)?);
            Ok(())
        }
        Some("tactic-restore-locality") => {
            let learn_args = &args[1..];
            let repository_root = fs::canonicalize(
                option(learn_args, "--repository-root")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_dir()?),
            )?;
            let request: OptimizationRequest =
                serde_json::from_slice(&fs::read(required_path(learn_args, "--request")?)?)?;
            let execution: NativeResidualExecutionBinding =
                serde_json::from_slice(&fs::read(required_path(learn_args, "--execution")?)?)?;
            let output_argument = required_path(learn_args, "--output")?;
            let output = if output_argument.is_absolute() {
                output_argument
            } else {
                repository_root.join(output_argument)
            };
            let mut seeds = repeated_option(learn_args, "--seed")
                .into_iter()
                .map(|seed| seed.parse::<u64>())
                .collect::<Result<Vec<_>, _>>()?;
            if seeds.is_empty() {
                seeds.push(
                    request
                        .execution
                        .deterministic_seeds
                        .first()
                        .copied()
                        .ok_or("optimization request has no deterministic seed")?,
                );
            }
            seeds.sort_unstable();
            seeds.dedup();
            if seeds.len() != 1 {
                return Err(
                    "tactic restore locality requires exactly one deterministic seed".into(),
                );
            }
            let execution_plan = native_tactic_execution_plan(
                learn_args,
                &request,
                &seeds,
                TacticProposalPolicy::Learned,
                NativeGenericExecutionStrategy::NativeController,
                None,
            )?;
            let report = run_native_tactic_restore_locality(&NativeTacticRestoreLocalityConfig {
                repository_root: &repository_root,
                optimization: &request,
                execution: &execution,
                execution_plan: &execution_plan,
                output_root: &output,
                workers: usize_option(
                    learn_args,
                    "--workers",
                    usize::from(request.execution.workers),
                )?,
                repetitions: usize_option(learn_args, "--repetitions", 2)?.try_into()?,
            })?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": report.schema,
                    "report": output.join("restore-locality.json"),
                    "fixed_unique_useful_graph_expansions":
                        report.fixed_unique_useful_graph_expansions,
                    "pairs": report.pairs,
                    "passed": report.passed,
                }))?
            );
            if !report.passed {
                return Err("native tactic restore locality benchmark did not pass".into());
            }
            Ok(())
        }
        Some("validate-tactic-restore-locality") => {
            let report: NativeTacticRestoreLocalityReport =
                serde_json::from_slice(&fs::read(required_path(&args[1..], "--report")?)?)?;
            report.validate()?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("validate-tactic-scratch-discovery") => {
            let learn_args = &args[1..];
            let repository_root = fs::canonicalize(
                option(learn_args, "--repository-root")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_dir()?),
            )?;
            let request_path = required_path(learn_args, "--request")?;
            let execution_path = required_path(learn_args, "--execution")?;
            let route_report_path = required_path(learn_args, "--report")?;
            let request: OptimizationRequest = serde_json::from_slice(&fs::read(&request_path)?)?;
            let execution: NativeResidualExecutionBinding =
                serde_json::from_slice(&fs::read(&execution_path)?)?;
            let route: NativeTacticRouteReport =
                serde_json::from_slice(&fs::read(&route_report_path)?)?;
            let output = required_path(learn_args, "--output")?;
            let bundle_root = required_path(learn_args, "--bundle")?;
            if output.exists() {
                return Err(format!(
                    "scratch discovery validation output already exists: {}",
                    output.display()
                )
                .into());
            }
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let report = NativeTacticScratchDiscoveryReport::build(&request, &route)?;
            let bundle = NativeTacticScratchEvidenceBundle::build(
                &bundle_root,
                &repository_root,
                &request_path,
                &execution_path,
                &route_report_path,
                &request,
                &execution,
                &route,
                &report,
            )?;
            fs::write(&output, report.to_pretty_json()?)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "acceptance": report,
                    "bundle": bundle_root,
                    "bundle_sha256": bundle.content_sha256,
                }))?
            );
            if !report.passed {
                return Err("native tactic scratch discovery did not pass".into());
            }
            Ok(())
        }
        Some("validate-tactic-scratch-bundle") => {
            let bundle_root = required_path(&args[1..], "--bundle")?;
            let bundle = NativeTacticScratchEvidenceBundle::read_and_validate(&bundle_root)?;
            println!("{}", serde_json::to_string_pretty(&bundle)?);
            Ok(())
        }
        Some("audit-tactic-fault-recovery") => {
            let learn_args = &args[1..];
            let audit = audit_native_tactic_fault_recovery(
                &required_path(learn_args, "--control-report")?,
                &required_path(learn_args, "--recovered-report")?,
                &required_path(learn_args, "--output")?,
            )?;
            println!("{}", serde_json::to_string_pretty(&audit)?);
            if !audit.passed {
                return Err("native tactic fault recovery did not preserve exact work".into());
            }
            Ok(())
        }
        Some("seal-tactic-fault-recovery") => {
            let learn_args = &args[1..];
            let repository_root = fs::canonicalize(
                option(learn_args, "--repository-root")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_dir()?),
            )?;
            let bundle_argument = required_path(learn_args, "--bundle")?;
            let bundle_root = if bundle_argument.is_absolute() {
                bundle_argument
            } else {
                repository_root.join(bundle_argument)
            };
            let bundle = NativeTacticFaultRecoveryEvidenceBundle::build(
                &bundle_root,
                &repository_root,
                &required_path(learn_args, "--request")?,
                &required_path(learn_args, "--execution")?,
                &required_path(learn_args, "--control-report")?,
                &required_path(learn_args, "--recovered-report")?,
            )?;
            println!("{}", serde_json::to_string_pretty(&bundle)?);
            Ok(())
        }
        Some("validate-tactic-fault-recovery-bundle") => {
            let bundle = NativeTacticFaultRecoveryEvidenceBundle::read_and_validate(
                &required_path(&args[1..], "--bundle")?,
            )?;
            println!("{}", serde_json::to_string_pretty(&bundle)?);
            Ok(())
        }
        Some("audit-tactic-scratch-campaign") => {
            let learn_args = &args[1..];
            let repository_root = fs::canonicalize(
                option(learn_args, "--repository-root")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_dir()?),
            )?;
            let route: NativeTacticRouteReport =
                serde_json::from_slice(&fs::read(required_path(learn_args, "--report")?)?)?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "scratch campaign audit output already exists: {}",
                    output.display()
                )
                .into());
            }
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let audit = NativeTacticScratchCampaignAudit::build(&repository_root, &route)?;
            fs::write(&output, audit.to_pretty_json()?)?;
            println!("{}", serde_json::to_string_pretty(&audit)?);
            Ok(())
        }
        Some("audit-post-terminal-tactic-controls") => {
            let learn_args = &args[1..];
            let repository_root = fs::canonicalize(
                option(learn_args, "--repository-root")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_dir()?),
            )?;
            let route: NativeTacticRouteReport =
                serde_json::from_slice(&fs::read(required_path(learn_args, "--report")?)?)?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "post-terminal tactic control output already exists: {}",
                    output.display()
                )
                .into());
            }
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let report = NativeTacticPostTerminalControlReport::build(&repository_root, &route)?;
            fs::write(&output, report.to_pretty_json()?)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("audit-tactic-observations") => {
            let learn_args = &args[1..];
            let request: OptimizationRequest =
                serde_json::from_slice(&fs::read(required_path(learn_args, "--request")?)?)?;
            let route: NativeTacticRouteReport =
                serde_json::from_slice(&fs::read(required_path(learn_args, "--report")?)?)?;
            let corpus_paths = repeated_option(learn_args, "--input")
                .into_iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>();
            if corpus_paths.is_empty() || corpus_paths.len() > MAX_LEARN_INPUT_CORPORA {
                return Err(format!(
                    "observation audit requires 1..={MAX_LEARN_INPUT_CORPORA} --input corpora"
                )
                .into());
            }
            let corpora = corpus_paths
                .iter()
                .map(|path| {
                    let bytes = fs::read(path)?;
                    Ok::<_, Box<dyn Error>>((
                        Digest(Sha256::digest(&bytes).into()),
                        TacticQTrainingCorpus::read(path)?,
                    ))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "observation audit output already exists: {}",
                    output.display()
                )
                .into());
            }
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let audit = NativeTacticObservationAudit::build(&request, &route, &corpora)?;
            fs::write(&output, audit.to_pretty_json()?)?;
            println!("{}", serde_json::to_string_pretty(&audit)?);
            if !audit.passed {
                return Err("native tactic observation coverage is incomplete".into());
            }
            Ok(())
        }
        Some("compare-tactic-scratch-campaigns") => {
            let learn_args = &args[1..];
            let repository_root = fs::canonicalize(
                option(learn_args, "--repository-root")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_dir()?),
            )?;
            let routes = ["--learned-report", "--frozen-report", "--random-report"]
                .into_iter()
                .map(|argument| {
                    Ok::<_, Box<dyn Error>>(serde_json::from_slice::<NativeTacticRouteReport>(
                        &fs::read(required_path(learn_args, argument)?)?,
                    )?)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "scratch comparison output already exists: {}",
                    output.display()
                )
                .into());
            }
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let report = NativeTacticScratchComparisonReport::build(&repository_root, routes)?;
            fs::write(&output, report.to_pretty_json()?)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("validate-tactic-scratch-comparison") => {
            let learn_args = &args[1..];
            let report: NativeTacticScratchComparisonReport =
                serde_json::from_slice(&fs::read(required_path(learn_args, "--report")?)?)?;
            report.validate()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": report.schema,
                    "content_sha256": report.content_sha256,
                    "seeds": report.seeds,
                    "workers": report.workers,
                    "cells": report.cells.len(),
                }))?
            );
            Ok(())
        }
        Some("validate-tactic-campaign-completion") => {
            let learn_args = &args[1..];
            let report_path = fs::canonicalize(required_path(learn_args, "--report")?)?;
            let output_root = report_path
                .parent()
                .ok_or("tactic route report has no campaign directory")?;
            let summary_path = output_root.join("campaign-summary.json");
            let completion_path = output_root.join(NATIVE_TACTIC_CAMPAIGN_COMPLETION_FILE);
            let report: NativeTacticRouteReport = serde_json::from_slice(&fs::read(&report_path)?)?;
            let completion = NativeTacticCampaignCompletion::read(&completion_path)?;
            completion.validate_files(&report_path, &summary_path)?;
            if completion.execution_plan_sha256 != report.execution_plan_sha256
                || completion.route_cutoff_wall_micros != report.timing.wall_micros
            {
                return Err("campaign completion marker differs from its route report".into());
            }
            println!("{}", serde_json::to_string_pretty(&completion)?);
            Ok(())
        }
        Some("diagnose-tactic-terminal-routes") => {
            let learn_args = &args[1..];
            let repository_root = fs::canonicalize(
                option(learn_args, "--repository-root")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_dir()?),
            )?;
            let scratch: NativeTacticRouteReport =
                serde_json::from_slice(&fs::read(required_path(learn_args, "--scratch-report")?)?)?;
            let demonstration: NativeTacticDemonstrationReport = serde_json::from_slice(
                &fs::read(required_path(learn_args, "--demonstration-report")?)?,
            )?;
            let corpus_path = required_path(learn_args, "--demonstration-corpus")?;
            let corpus_bytes = fs::read(&corpus_path)?;
            if Digest(Sha256::digest(&corpus_bytes).into()) != demonstration.corpus_sha256 {
                return Err("demonstration corpus bytes differ from its report".into());
            }
            let corpus = TacticQTrainingCorpus::read(&corpus_path)?;
            let mut terminal_results = Vec::new();
            for seed in scratch.seeds.iter().filter(|seed| seed.terminal_discovered) {
                let declared = seed
                    .best_terminal_result
                    .as_deref()
                    .ok_or("terminal seed lacks its best terminal result")?;
                let candidate = PathBuf::from(declared);
                let candidate = if candidate.is_absolute() {
                    candidate
                } else {
                    repository_root.join(candidate)
                };
                let resolved = fs::canonicalize(candidate)?;
                if !resolved.starts_with(&repository_root) || !resolved.is_file() {
                    return Err("terminal result is outside the repository or absent".into());
                }
                terminal_results.push((seed.seed, TacticQFinalResult::read(&resolved)?));
            }
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "route diagnosis output already exists: {}",
                    output.display()
                )
                .into());
            }
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let report = NativeTacticRouteDiagnosisReport::build(
                &scratch,
                &demonstration,
                &corpus,
                terminal_results,
            )?;
            fs::write(&output, report.to_pretty_json()?)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        _ => usage_error(),
    }
}
