//! Frozen-policy construction and verification commands.

use super::*;

pub(super) fn handles(args: &[String]) -> bool {
    matches!(
        args.first().map(String::as_str),
        Some(
            "verify-frozen-policy-cold-replay"
                | "export-frozen-policy-tape"
                | "verify-frozen-policy"
                | "frozen-policy-probe-model"
                | "frozen-policy-batch"
                | "factorized-policy-batch"
        )
    )
}

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
        _ => unreachable!("frozen-policy command dispatch was not checked"),
    }
}
