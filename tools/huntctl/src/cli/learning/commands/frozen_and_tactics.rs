//! Frozen-policy, tactic execution, and Q-variant commands.

use super::{
    Digest, FactorizedPolicyOutputSet, GoalConditionedTacticFeatureEncoder, InputTape,
    MAX_LEARN_INPUT_CORPORA, NativeEpisodeShard, NativeFactorizedPolicyBatchConfig,
    NativeFactorizedPolicySuffixBatch, NativeFrozenPolicyReinferenceReport,
    NativeFrozenPolicySuffixBatch, NativeGenericExecutionStrategy, NativeResidualExecutionBinding,
    NativeTacticPolicyRunConfig, NativeTacticRouteRunConfig, OptimizationRequest, Sha256,
    TacticFrozenPolicy, TacticProposalPolicy, TacticQCampaign, TacticQTrainingCorpus, cli,
    command_conservative_q, flag, native_frozen_policy_probe_model, native_tactic_execution_plan,
    option, prove_generalized_tactic_held_out_value, realize_native_frozen_policy_tape,
    repeated_option, required_path, run_native_tactic_policy, run_native_tactic_route,
    tactic_macro_registry_identity, u64_option, usage_error, usize_option,
    verify_native_frozen_policy_cold_replay, verify_native_frozen_policy_reinference,
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
                seeds = vec![1, 2, 3];
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
            let report = run_native_tactic_route(&NativeTacticRouteRunConfig {
                repository_root: &repository_root,
                optimization: &request,
                execution: &execution,
                execution_plan: &execution_plan,
                promoted_tactic_registry: promoted_tactic_registry.as_deref(),
                output_root: &output,
                workers,
                cancellation: None,
                resume: flag(learn_args, "--resume"),
            })?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": report.schema,
                    "report": output.join("report.json"),
                    "successful_seeds": report.successful_seeds,
                    "exploration_seeds": report.exploration_seeds,
                    "proposal_policy": report.proposal_policy,
                    "execution_strategy": report.execution_strategy,
                    "total_decisions": report.total_decisions,
                    "total_native_ticks": report.total_native_ticks,
                    "demonstration_transitions": report.demonstration_transitions,
                }))?
            );
            Ok(())
        }
        _ => usage_error(),
    }
}
