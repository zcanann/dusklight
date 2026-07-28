//! Goal reachability fitting, controls, inspection, and frozen policy export.

use super::{
    ContentKind, ContentStore, MAX_LEARN_INPUT_CORPORA,
    NATIVE_GOAL_FROZEN_POLICY_MANIFEST_SCHEMA_V3, NATIVE_GOAL_REACHABILITY_MODEL_SCHEMA_V1,
    NATIVE_GOAL_REACHABILITY_NEGATIVE_CONTROL_SCHEMA_V1, NativeEpisodeShard,
    NativeGoalFrozenPolicyConfig, NativeGoalFrozenPolicyExport, NativeGoalFrozenPolicyManifest,
    NativeGoalReachabilityModel, NativeGoalReachabilityNegativeControlReport,
    NativeGoalTrajectoryDataset, goal_reachability_config, option, repeated_option, required_path,
    u64_option, usage_error, usize_option,
};
use serde_json::json;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

pub(super) fn command(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("fit-goal-reachability") => {
            let learn_args = &args[1..];
            let dataset_paths = repeated_option(learn_args, "--dataset");
            let input_paths = repeated_option(learn_args, "--input");
            if dataset_paths.is_empty()
                || dataset_paths.len() > MAX_LEARN_INPUT_CORPORA
                || input_paths.is_empty()
                || input_paths.len() > MAX_LEARN_INPUT_CORPORA
            {
                return Err(format!(
                    "learn fit-goal-reachability requires 1..={MAX_LEARN_INPUT_CORPORA} --dataset DATASET.json and --input EPISODES.dseps"
                )
                .into());
            }
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "native goal reachability model output already exists: {}",
                    output.display()
                )
                .into());
            }
            let datasets = dataset_paths
                .iter()
                .map(|path| -> Result<_, Box<dyn Error>> {
                    let dataset: NativeGoalTrajectoryDataset =
                        serde_json::from_slice(&fs::read(path)?)?;
                    dataset.validate()?;
                    Ok(dataset)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let shards = input_paths
                .iter()
                .map(NativeEpisodeShard::read)
                .collect::<Result<Vec<_>, _>>()?;
            let config = goal_reachability_config(learn_args)?;
            let model = NativeGoalReachabilityModel::fit(&datasets, &shards, config)?;
            let bytes = serde_json::to_vec_pretty(&model)?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &bytes)?;
            let artifact_store = option(learn_args, "--artifact-store")
                .map(PathBuf::from)
                .unwrap_or_else(|| output.parent().unwrap_or(Path::new(".")).join("content"));
            let content_blob =
                ContentStore::initialize(&artifact_store)?.put_bytes(&bytes, ContentKind::Model)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": NATIVE_GOAL_REACHABILITY_MODEL_SCHEMA_V1,
                    "model_sha256": model.model_sha256,
                    "source_dataset_sha256": model.source_dataset_sha256,
                    "source_replay_corpus_sha256": model.source_replay_corpus_sha256,
                    "training_n_step_bootstrap_rows": model.training_n_step_bootstrap_rows,
                    "admission": model.admission,
                    "training": model.training,
                    "validation": model.validation,
                    "test": model.test,
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                }))?
            );
            Ok(())
        }
        Some("evaluate-goal-reachability-negative-controls") => {
            const MAX_CONTROL_INPUT_SHARDS: usize = 4_096;
            let learn_args = &args[1..];
            let dataset_paths = repeated_option(learn_args, "--dataset");
            let input_paths = repeated_option(learn_args, "--input");
            if dataset_paths.is_empty()
                || dataset_paths.len() > MAX_LEARN_INPUT_CORPORA
                || input_paths.is_empty()
                || input_paths.len() > MAX_CONTROL_INPUT_SHARDS
            {
                return Err(format!(
                    "learn evaluate-goal-reachability-negative-controls requires 1..={MAX_LEARN_INPUT_CORPORA} --dataset DATASET.json and 1..={MAX_CONTROL_INPUT_SHARDS} --input EPISODES.dseps"
                )
                .into());
            }
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "goal reachability negative-control output already exists: {}",
                    output.display()
                )
                .into());
            }
            let datasets = dataset_paths
                .iter()
                .map(|path| -> Result<_, Box<dyn Error>> {
                    let dataset: NativeGoalTrajectoryDataset =
                        serde_json::from_slice(&fs::read(path)?)?;
                    dataset.validate()?;
                    Ok(dataset)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let shards = input_paths
                .iter()
                .map(NativeEpisodeShard::read)
                .collect::<Result<Vec<_>, _>>()?;
            let config = goal_reachability_config(learn_args)?;
            let report =
                NativeGoalReachabilityNegativeControlReport::evaluate(&datasets, &shards, config)?;
            let bytes = serde_json::to_vec_pretty(&report)?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &bytes)?;
            let artifact_store = option(learn_args, "--artifact-store")
                .map(PathBuf::from)
                .unwrap_or_else(|| output.parent().unwrap_or(Path::new(".")).join("content"));
            let content_blob =
                ContentStore::initialize(&artifact_store)?.put_bytes(&bytes, ContentKind::Model)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": NATIVE_GOAL_REACHABILITY_NEGATIVE_CONTROL_SCHEMA_V1,
                    "report_sha256": report.report_sha256,
                    "source_dataset_sha256": report.source_dataset_sha256,
                    "source_replay_corpus_sha256": report.source_replay_corpus_sha256,
                    "config": report.config,
                    "baseline": report.baseline,
                    "controls": report.controls,
                    "observation_insufficiency": report.observation_insufficiency,
                    "promotion_authority": report.promotion_authority,
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                }))?
            );
            Ok(())
        }
        Some("inspect-goal-reachability") => {
            let input = required_path(&args[1..], "--input")?;
            let model: NativeGoalReachabilityModel = serde_json::from_slice(&fs::read(&input)?)?;
            model.validate()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": NATIVE_GOAL_REACHABILITY_MODEL_SCHEMA_V1,
                    "model_sha256": model.model_sha256,
                    "input_schema_sha256": model.input_schema_sha256,
                    "input_width": model.input_width,
                    "goal_embedding_width": model.goal_embedding_width,
                    "source_dataset_sha256": model.source_dataset_sha256,
                    "training_n_step_bootstrap_rows": model.training_n_step_bootstrap_rows,
                    "admission": model.admission,
                    "training": model.training,
                    "validation": model.validation,
                    "test": model.test,
                    "promotion_authority": model.promotion_authority,
                }))?
            );
            Ok(())
        }
        Some("fit-frozen-goal-policy") => {
            let learn_args = &args[1..];
            let dataset_path = required_path(learn_args, "--dataset")?;
            let critic_path = required_path(learn_args, "--critic")?;
            let input_paths = repeated_option(learn_args, "--input");
            if input_paths.is_empty() || input_paths.len() > MAX_LEARN_INPUT_CORPORA {
                return Err(format!(
                    "learn fit-frozen-goal-policy requires 1..={MAX_LEARN_INPUT_CORPORA} --input EPISODES.dseps"
                )
                .into());
            }
            let model_output = required_path(learn_args, "--model-output")?;
            let manifest_output = required_path(learn_args, "--manifest-output")?;
            if model_output == manifest_output {
                return Err("goal frozen policy model and manifest outputs must differ".into());
            }
            for output in [&model_output, &manifest_output] {
                if output.exists() {
                    return Err(format!(
                        "goal frozen policy output already exists: {}",
                        output.display()
                    )
                    .into());
                }
            }
            let dataset: NativeGoalTrajectoryDataset =
                serde_json::from_slice(&fs::read(&dataset_path)?)?;
            dataset.validate()?;
            let critic: NativeGoalReachabilityModel =
                serde_json::from_slice(&fs::read(&critic_path)?)?;
            critic.validate()?;
            let shards = input_paths
                .iter()
                .map(NativeEpisodeShard::read)
                .collect::<Result<Vec<_>, _>>()?;
            let defaults = NativeGoalFrozenPolicyConfig::default();
            let parse_f64 = |name: &str, default: f64| -> Result<f64, Box<dyn Error>> {
                option(learn_args, name)
                    .map(|value| value.parse::<f64>().map_err(Into::into))
                    .transpose()
                    .map(|value| value.unwrap_or(default))
            };
            let config = NativeGoalFrozenPolicyConfig {
                epochs: u16::try_from(usize_option(
                    learn_args,
                    "--epochs",
                    usize::from(defaults.epochs),
                )?)
                .map_err(|_| "goal frozen policy epochs exceed u16")?,
                hidden_width: u16::try_from(usize_option(
                    learn_args,
                    "--hidden-width",
                    usize::from(defaults.hidden_width),
                )?)
                .map_err(|_| "goal frozen policy hidden width exceeds u16")?,
                learning_rate: parse_f64("--learning-rate", defaults.learning_rate)?,
                l2_penalty: parse_f64("--l2-penalty", defaults.l2_penalty)?,
                gradient_clip: parse_f64("--gradient-clip", defaults.gradient_clip)?,
                minimum_validation_joint_improvement: parse_f64(
                    "--minimum-validation-joint-improvement",
                    defaults.minimum_validation_joint_improvement,
                )?,
                seed: u64_option(learn_args, "--seed", defaults.seed)?,
            };
            let export = NativeGoalFrozenPolicyExport::fit(&dataset, &shards, &critic, config)?;
            let mut manifest_bytes = serde_json::to_vec_pretty(&export.manifest)?;
            manifest_bytes.push(b'\n');
            for output in [&model_output, &manifest_output] {
                if let Some(parent) = output
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    fs::create_dir_all(parent)?;
                }
            }
            fs::write(&model_output, &export.model_bytes)?;
            fs::write(&manifest_output, &manifest_bytes)?;
            let artifact_store = option(learn_args, "--artifact-store")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    manifest_output
                        .parent()
                        .unwrap_or(Path::new("."))
                        .join("content")
                });
            let store = ContentStore::initialize(&artifact_store)?;
            let model_blob = store.put_bytes(&export.model_bytes, ContentKind::Model)?;
            let manifest_blob = store.put_bytes(&manifest_bytes, ContentKind::Model)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": NATIVE_GOAL_FROZEN_POLICY_MANIFEST_SCHEMA_V3,
                    "manifest_sha256": export.manifest.manifest_sha256,
                    "frozen_artifact_sha256": export.manifest.frozen_artifact_sha256,
                    "source_dataset_sha256": export.manifest.source_dataset_sha256,
                    "source_reachability_model_sha256": export.manifest.source_reachability_model_sha256,
                    "objective_sha256": export.manifest.objective_sha256,
                    "admission": export.manifest.admission,
                    "training": export.manifest.training,
                    "validation": export.manifest.validation,
                    "test": export.manifest.test,
                    "model_output": model_output,
                    "manifest_output": manifest_output,
                    "artifact_store": artifact_store,
                    "model_content_blob": model_blob,
                    "manifest_content_blob": manifest_blob,
                    "promotion_authority": false,
                }))?
            );
            Ok(())
        }
        Some("inspect-frozen-goal-policy") => {
            let learn_args = &args[1..];
            let manifest_path = required_path(learn_args, "--manifest")?;
            let model_path = required_path(learn_args, "--model")?;
            let manifest: NativeGoalFrozenPolicyManifest =
                serde_json::from_slice(&fs::read(&manifest_path)?)?;
            let model_bytes = fs::read(&model_path)?;
            manifest.validate(&model_bytes)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
            Ok(())
        }
        _ => usage_error(),
    }
}
