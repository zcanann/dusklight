//! Frozen-policy, tactic execution, and Q-variant commands.

use super::{
    Digest, FactorizedPolicyOutputSet, GoalConditionedTacticFeatureEncoder, InputTape,
    MAX_LEARN_INPUT_CORPORA, NativeEpisodeShard, NativeFactorizedPolicyBatchConfig,
    NativeFactorizedPolicySuffixBatch, NativeFrozenPolicyReinferenceReport,
    NativeFrozenPolicySuffixBatch, NativeGenericExecutionStrategy, NativeResidualExecutionBinding,
    NativeTacticDemonstrationReport, NativeTacticFaultInjector, NativeTacticLaunchSmokeBundle,
    NativeTacticObservationAudit, NativeTacticPolicyRunConfig,
    NativeTacticPostTerminalControlReport, NativeTacticRestoreLocalityConfig,
    NativeTacticRestoreLocalityReport, NativeTacticRouteDiagnosisReport, NativeTacticRouteReport,
    NativeTacticRouteRunConfig, NativeTacticScratchCampaignAudit,
    NativeTacticScratchComparisonReport, NativeTacticScratchDiscoveryReport,
    NativeTacticScratchEvidenceBundle, NativeTacticThroughputCurveConfig,
    NativeTacticThroughputEvidenceBundle, OptimizationRequest, Sha256, TacticFrozenPolicy,
    TacticProposalPolicy, TacticQCampaign, TacticQFinalResult, TacticQTrainingCorpus,
    audit_native_tactic_fault_recovery, cli, command_conservative_q, flag,
    native_frozen_policy_probe_model, native_tactic_execution_plan, option,
    prove_generalized_tactic_held_out_value, realize_native_frozen_policy_tape, repeated_option,
    required_path, run_native_tactic_policy, run_native_tactic_restore_locality,
    run_native_tactic_route, run_native_tactic_throughput_curve, tactic_macro_registry_identity,
    u64_option, usage_error, usize_option, verify_native_frozen_policy_cold_replay,
    verify_native_frozen_policy_reinference,
};
use serde_json::json;
use sha2::Digest as _;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

pub(super) fn command(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("verify-frozen-policy-cold-replay") => {
            let learn_args = &args[1..];
            let batch_path = required_path(learn_args, "--result")?;
            let reinference_path = required_path(learn_args, "--reinference")?;
            let source_tape_path = required_path(learn_args, "--source-tape")?;
            let realized_tape_path = required_path(learn_args, "--realized-tape")?;
            let shard_path = required_path(learn_args, "--input")?;
            let live_trace_path = required_path(learn_args, "--live-trace")?;
            let cold_trace_path = required_path(learn_args, "--cold-trace")?;
            let cold_milestone_path = required_path(learn_args, "--cold-milestone")?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "frozen policy cold replay verification output already exists: {}",
                    output.display()
                )
                .into());
            }
            let episode_id =
                option(learn_args, "--episode-id").ok_or("missing required --episode-id ID")?;
            let batch_bytes = fs::read(batch_path)?;
            let reinference: NativeFrozenPolicyReinferenceReport =
                serde_json::from_slice(&fs::read(reinference_path)?)?;
            let source_tape_bytes = fs::read(source_tape_path)?;
            let source_tape = InputTape::decode(&source_tape_bytes)?.tape;
            let realized_tape_bytes = fs::read(realized_tape_path)?;
            let realized_tape = InputTape::decode(&realized_tape_bytes)?.tape;
            let shard = NativeEpisodeShard::read(shard_path)?;
            let live_trace_bytes = fs::read(live_trace_path)?;
            let live_trace = huntctl::trace::decode(&live_trace_bytes)?;
            let cold_trace_bytes = fs::read(cold_trace_path)?;
            let cold_trace = huntctl::trace::decode(&cold_trace_bytes)?;
            let cold_milestone_bytes = fs::read(cold_milestone_path)?;
            let report = verify_native_frozen_policy_cold_replay(
                &batch_bytes,
                &reinference,
                &source_tape,
                &source_tape_bytes,
                &realized_tape,
                &realized_tape_bytes,
                &shard,
                &live_trace,
                &live_trace_bytes,
                &cold_trace,
                &cold_trace_bytes,
                &cold_milestone_bytes,
                &episode_id,
            )?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let mut bytes = serde_json::to_vec_pretty(&report)?;
            bytes.push(b'\n');
            fs::write(&output, bytes)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("export-frozen-policy-tape") => {
            let learn_args = &args[1..];
            let source_path = required_path(learn_args, "--source-tape")?;
            let shard_path = required_path(learn_args, "--input")?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "frozen policy realized tape output already exists: {}",
                    output.display()
                )
                .into());
            }
            let episode_id =
                option(learn_args, "--episode-id").ok_or("missing required --episode-id ID")?;
            let source = InputTape::decode(&fs::read(&source_path)?)?.tape;
            let shard = NativeEpisodeShard::read(&shard_path)?;
            let realized = realize_native_frozen_policy_tape(&source, &shard, &episode_id)?;
            let frame_count = realized.frames.len();
            let bytes = realized.encode()?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &bytes)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": "dusklight-native-frozen-policy-realized-tape/v1",
                    "source_tape": source_path,
                    "episode_shard": shard_path,
                    "episode_id": episode_id,
                    "source_frame": shard.source_frame,
                    "frame_count": frame_count,
                    "tape_sha256": Digest(Sha256::digest(&bytes).into()),
                    "output": output
                }))?
            );
            Ok(())
        }
        Some("verify-frozen-policy") => {
            let learn_args = &args[1..];
            let model_path = required_path(learn_args, "--model")?;
            let shard_path = required_path(learn_args, "--input")?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "frozen policy verification output already exists: {}",
                    output.display()
                )
                .into());
            }
            let objective = option(learn_args, "--objective-sha256")
                .ok_or("missing required --objective-sha256 SHA256")?
                .parse::<Digest>()?;
            let checkpoint = option(learn_args, "--checkpoint-identity")
                .ok_or("missing required --checkpoint-identity VALUE")?;
            let boundary = option(learn_args, "--source-boundary-fingerprint")
                .ok_or("missing required --source-boundary-fingerprint VALUE")?;
            let model_bytes = fs::read(&model_path)?;
            let batch = option(learn_args, "--batch")
                .map(
                    |path| -> Result<NativeFrozenPolicySuffixBatch, Box<dyn Error>> {
                        let batch: NativeFrozenPolicySuffixBatch =
                            serde_json::from_slice(&fs::read(path)?)?;
                        batch.validate(&model_bytes)?;
                        Ok(batch)
                    },
                )
                .transpose()?;
            let shard = NativeEpisodeShard::read(&shard_path)?;
            let report = verify_native_frozen_policy_reinference(
                &model_bytes,
                batch
                    .as_ref()
                    .and_then(|batch| batch.frozen_policy.rollout_exploration.as_ref()),
                &shard,
                objective,
                &checkpoint,
                &boundary,
            )?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let mut bytes = serde_json::to_vec_pretty(&report)?;
            bytes.push(b'\n');
            fs::write(&output, bytes)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("frozen-policy-probe-model") => {
            let learn_args = &args[1..];
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "frozen policy probe model output already exists: {}",
                    output.display()
                )
                .into());
            }
            let objective = option(learn_args, "--objective-sha256")
                .ok_or("missing required --objective-sha256 SHA256")?
                .parse::<Digest>()?;
            let model = native_frozen_policy_probe_model(objective)?;
            let bytes = model.to_bytes()?;
            let parameter_count = model
                .layers
                .iter()
                .map(|layer| layer.weights.len() + layer.biases.len())
                .sum::<usize>();
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &bytes)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": "dusklight-native-frozen-policy-probe/v1",
                    "output": output,
                    "artifact_sha256": model.artifact_sha256()?,
                    "feature_schema_sha256": model.feature_schema_sha256,
                    "action_schema_sha256": model.action_schema_sha256,
                    "objective_sha256": model.objective_sha256,
                    "input_width": model.input_width,
                    "output_width": model.actions.len(),
                    "parameter_count": parameter_count,
                    "byte_count": bytes.len(),
                    "policy": "player-present forward drive plus current-yaw steering",
                    "promotion_authority": false
                }))?
            );
            Ok(())
        }
        Some("frozen-policy-batch") => {
            let learn_args = &args[1..];
            let model = required_path(learn_args, "--model")?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "frozen policy batch output already exists: {}",
                    output.display()
                )
                .into());
            }
            let maximum_ticks = usize_option(learn_args, "--maximum-ticks", 125)?;
            let objective = option(learn_args, "--objective-sha256")
                .ok_or("missing required --objective-sha256 SHA256")?
                .parse::<Digest>()?;
            let canonical_model = fs::canonicalize(&model)?;
            let batch = NativeFrozenPolicySuffixBatch::build(
                &fs::read(&canonical_model)?,
                canonical_model.to_string_lossy().into_owned(),
                objective,
                option(learn_args, "--candidate-id").unwrap_or_else(|| "native-policy".into()),
                NativeFactorizedPolicyBatchConfig {
                    source_frame: usize_option(learn_args, "--source-frame", 440)?,
                    source_boundary_fingerprint: option(
                        learn_args,
                        "--source-boundary-fingerprint",
                    )
                    .ok_or("missing required --source-boundary-fingerprint VALUE")?,
                    checkpoint_validation_ticks: usize_option(
                        learn_args,
                        "--checkpoint-validation-ticks",
                        maximum_ticks.min(8),
                    )?,
                    maximum_ticks,
                    verify_state_hashes: learn_args
                        .iter()
                        .any(|argument| argument == "--verify-state-hashes"),
                },
            )?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let mut encoded = serde_json::to_vec_pretty(&batch)?;
            encoded.push(b'\n');
            fs::write(&output, encoded)?;
            println!(
                "wrote native frozen policy batch ({} ticks) to {}",
                batch.maximum_ticks,
                output.display()
            );
            Ok(())
        }
        Some("factorized-policy-batch") => {
            let learn_args = &args[1..];
            let input = required_path(learn_args, "--input")?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "factorized policy batch output already exists: {}",
                    output.display()
                )
                .into());
            }
            let maximum_ticks = usize_option(learn_args, "--maximum-ticks", 125)?;
            let output_set: FactorizedPolicyOutputSet = serde_json::from_slice(&fs::read(&input)?)?;
            let batch = NativeFactorizedPolicySuffixBatch::build(
                output_set,
                NativeFactorizedPolicyBatchConfig {
                    source_frame: usize_option(learn_args, "--source-frame", 440)?,
                    source_boundary_fingerprint: option(
                        learn_args,
                        "--source-boundary-fingerprint",
                    )
                    .ok_or("missing required --source-boundary-fingerprint VALUE")?,
                    checkpoint_validation_ticks: usize_option(
                        learn_args,
                        "--checkpoint-validation-ticks",
                        maximum_ticks.min(8),
                    )?,
                    maximum_ticks,
                    verify_state_hashes: learn_args
                        .iter()
                        .any(|argument| argument == "--verify-state-hashes"),
                },
            )?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let mut encoded = serde_json::to_vec_pretty(&batch)?;
            encoded.push(b'\n');
            fs::write(&output, encoded)?;
            println!(
                "wrote {} factorized policy candidates ({} ticks each) to {}",
                batch.candidates.len(),
                batch.maximum_ticks,
                output.display()
            );
            Ok(())
        }
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
            let proposal_policy = match proposal_policy_argument.as_deref().unwrap_or("learned") {
                "learned" => TacticProposalPolicy::Learned,
                "random-valid" => TacticProposalPolicy::RandomValid,
                "structured-non-learning" => TacticProposalPolicy::StructuredNonLearning,
                value => {
                    return Err(format!(
                        "unknown tactic proposal policy {value:?}; expected learned, random-valid, or structured-non-learning"
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
            let workers = usize_option(
                learn_args,
                "--workers",
                usize::from(request.execution.workers),
            )?;
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
            let execution_plan = native_tactic_execution_plan(
                learn_args,
                &request,
                &seeds,
                proposal_policy,
                execution_strategy,
                promoted_tactic_registry_sha256,
            )?;
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
            let report = run_native_tactic_throughput_curve(&NativeTacticThroughputCurveConfig {
                repository_root: &repository_root,
                optimization: &request,
                execution: &execution,
                execution_plan: &execution_plan,
                output_root: &output,
                repetitions: usize_option(learn_args, "--repetitions", 2)?.try_into()?,
            })?;
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
            let routes = ["--learned-report", "--scheduler-report", "--random-report"]
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
