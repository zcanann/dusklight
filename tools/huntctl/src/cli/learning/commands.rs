//! Command dispatch and artifact workflows for the core learning baselines.

use super::MAX_LEARN_INPUT_CORPORA;
use crate::cli;
use crate::{option, repeated_option, required_path, u64_option, usage_error, usize_option};
use huntctl::Digest;
use huntctl::actor_profile_catalog::ActorProfileCatalog;
use huntctl::calibration::calibrate_fitted_q;
use huntctl::content_store::{ContentKind, ContentStore};
use huntctl::dataset::{
    DATASET_SOURCE_SCHEMA_V1, DatasetBuildConfig, DatasetManifest, DatasetSourceDescriptor,
    DatasetSplit,
};
use huntctl::double_q::{ConservativeQ, ConservativeQConfig, DoubleQ, DoubleQConfig};
use huntctl::episode::{EpisodeContext, EpisodeManifest, EpisodeManifestBuild};
use huntctl::fqi::{
    FittedQ, FqiConfig, MAX_FQI_ACTIONS, MAX_FQI_BACKUP_STEPS, MAX_FQI_ITERATIONS,
    MAX_FQI_TRANSITIONS, MAX_FQI_TREE_DEPTH, MAX_FQI_TREES_PER_ACTION, Transition as FqiTransition,
};
use huntctl::learning::batch::load_fqi_batch;
use huntctl::learning::default_tactic_catalog::default_route_tactic_catalog;
use huntctl::learning::factorized_policy_suffix_batch::{
    FactorizedPolicyOutputSet, NativeFactorizedPolicyBatchConfig, NativeFactorizedPolicySuffixBatch,
};
use huntctl::learning::multitask_set_encoder::{
    CompleteSetMultiTaskEncoder, DEFAULT_HISTORY_RECURRENT_WIDTH, MultiTaskSetPooling,
    NativeEncoderChannelFamily, NativeEncoderFeatureSpec, NativeMultiTaskActorCorpus,
    fit_shuffled_auxiliary_control_with_pooling_and_temporal,
};
use huntctl::learning::native_auxiliary_dataset::{
    AuxiliarySplitConfig, NATIVE_AUXILIARY_DATASET_SCHEMA_V2, NativeAuxiliaryDataset,
};
use huntctl::learning::native_frozen_policy_cold_replay::verify_native_frozen_policy_cold_replay;
use huntctl::learning::native_frozen_policy_reinference::{
    NativeFrozenPolicyReinferenceReport, realize_native_frozen_policy_tape,
    verify_native_frozen_policy_reinference,
};
use huntctl::learning::native_frozen_policy_suffix_batch::{
    NativeFrozenPolicySuffixBatch, native_frozen_policy_probe_model,
};
use huntctl::learning::native_goal_frozen_policy::{
    NATIVE_GOAL_FROZEN_POLICY_MANIFEST_SCHEMA_V3, NativeGoalFrozenPolicyConfig,
    NativeGoalFrozenPolicyExport, NativeGoalFrozenPolicyManifest,
};
use huntctl::learning::native_goal_reachability::{
    NATIVE_GOAL_REACHABILITY_MODEL_SCHEMA_V1, NATIVE_GOAL_REACHABILITY_NEGATIVE_CONTROL_SCHEMA_V1,
    NativeGoalReachabilityConfig, NativeGoalReachabilityModel,
    NativeGoalReachabilityNegativeControlReport,
};
use huntctl::learning::native_goal_trajectory::{
    NATIVE_GOAL_TRAJECTORY_DATASET_SCHEMA_V2, NativeGoalTrajectoryConfig,
    NativeGoalTrajectoryDataset,
};
use huntctl::learning::native_replay_corpus::{
    NATIVE_REPLAY_CORPUS_SCHEMA_V1, NativeReplayCorpus, ReplayEpisodeSource, ReplayExperienceRole,
};
use huntctl::learning::trainable_set_encoder::TrainableSetConfig;
use huntctl::low_data_baselines::{
    LocalFeature, LocalReturnConfig, NearestNeighborReturn, TabularAxis, TabularReturn,
    empirical_return_samples,
};
use huntctl::native_actor_view::NativeEpisodeActorView;
use huntctl::native_collision_history::{
    DEFAULT_COLLISION_HISTORY_DEPTH, NativeCollisionHistoryView,
};
use huntctl::native_corpus_inspection::inspect_native_episode_corpus;
use huntctl::native_episode_history::{DEFAULT_EPISODE_HISTORY_DEPTH, NativeEpisodeHistoryView};
use huntctl::native_episode_shard::NativeEpisodeShard;
use huntctl::native_geometry_view::{
    GeometryObservationStatus, NativeEpisodeGeometryView, NativeGeometryViewConfiguration,
};
use huntctl::native_resource_load_view::{
    NativeEpisodeResourceLoadView, ResourceArchiveKind, ResourceLoadOutcome, ResourceLoadSetStatus,
};
use huntctl::native_return_restart_trace::NativeReturnRestartWriteTrace;
use huntctl::native_room_load_view::{
    NativeEpisodeRoomLoadView, RoomLoadSetStatus, RoomSceneSetStatus,
};
use huntctl::native_surface_graph_view::{
    NativeEpisodeSurfaceGraphView, NativeSurfaceGraphViewConfiguration,
    SurfaceGraphObservationStatus,
};
use huntctl::observation_view::{MOVEMENT_STATE_V2_ID, movement_state_v2_spec};
use huntctl::offline_rl::{
    ExploratoryExtractConfig, MOVEMENT_CATEGORICAL_FEATURES_V1, extract_exploratory_from_bytes,
    extract_exploratory_v2_from_bytes, extract_exploratory_v3_from_bytes,
    movement_feature_schema_digest_v1,
};
use huntctl::reward_shaping::{PotentialShapingSpec, REWARD_REPORT_SCHEMA_V1};
use huntctl::search_evaluator::native_residual_campaign::NativeResidualExecutionBinding;
use huntctl::search_evaluator::native_tactic_route_runner::{
    NativeTacticRouteRunConfig, run_native_tactic_route,
};
use huntctl::search_evaluator::optimization_request::OptimizationRequest;
use huntctl::search_evaluator::tactic_q_campaign::TacticQCampaign;
use huntctl::tape::InputTape;
use huntctl::trace_diff::SiblingTraceDiff;
use huntctl::transition_corpus::TransitionCorpus;
use huntctl::transition_evidence::{
    ImmutableEpisodeArtifact, TerminalReasonEvidence, TransitionEvidenceBuild,
    TransitionEvidenceBundle,
};
use huntctl::world_inventory::WorldInventory;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

const NATIVE_REPLAY_SOURCE_SCHEMA_V1: &str = "dusklight-native-replay-source/v1";

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeReplaySourceDescriptor {
    schema: String,
    shard: PathBuf,
    episode_id: String,
    role: ReplayExperienceRole,
    #[serde(default)]
    policy_lineage_sha256: Option<Digest>,
    #[serde(default)]
    parent_entry_sha256: Option<Digest>,
}

fn parse_replay_role(value: &str) -> Result<ReplayExperienceRole, Box<dyn Error>> {
    match value {
        "demonstration" => Ok(ReplayExperienceRole::Demonstration),
        "policy_rollout" => Ok(ReplayExperienceRole::PolicyRollout),
        "randomized_coverage" => Ok(ReplayExperienceRole::RandomizedCoverage),
        "alternate_terminal" => Ok(ReplayExperienceRole::AlternateTerminal),
        _ => Err(
            "replay role must be demonstration, policy_rollout, randomized_coverage, or alternate_terminal"
                .into(),
        ),
    }
}

fn command_conservative_q(learn_args: &[String]) -> Result<(), Box<dyn Error>> {
    let direct_inputs = repeated_option(learn_args, "--input");
    let dataset_path = option(learn_args, "--dataset").map(PathBuf::from);
    if dataset_path.is_some() && !direct_inputs.is_empty() {
        return Err("learn cql accepts either --dataset or --input, not both".into());
    }
    let dataset_manifest: Option<DatasetManifest> = dataset_path
        .as_ref()
        .map(|path| -> Result<_, Box<dyn Error>> {
            let manifest: DatasetManifest = serde_json::from_slice(&fs::read(path)?)?;
            manifest.validate()?;
            Ok(manifest)
        })
        .transpose()?;
    let inputs = if let Some(manifest) = &dataset_manifest {
        manifest
            .entries
            .iter()
            .filter(|entry| entry.split == DatasetSplit::Train)
            .map(|entry| entry.transition_corpus.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
    } else {
        direct_inputs
    };
    let training = load_fqi_batch(&inputs, "CQL training", MAX_LEARN_INPUT_CORPORA)?;
    let expected_corpus_digests = dataset_manifest.as_ref().map(|manifest| {
        manifest
            .entries
            .iter()
            .filter(|entry| entry.split == DatasetSplit::Train)
            .map(|entry| entry.corpus_sha256)
            .collect::<Vec<_>>()
    });
    if expected_corpus_digests
        .as_ref()
        .is_some_and(|expected| expected != &training.corpus_digests)
    {
        return Err("CQL corpus content differs from dataset manifest".into());
    }
    let defaults = ConservativeQConfig::default();
    let config = ConservativeQConfig {
        double_q: DoubleQConfig {
            epochs: usize_option(learn_args, "--epochs", defaults.double_q.epochs)?,
            hidden_width: usize_option(
                learn_args,
                "--hidden-width",
                defaults.double_q.hidden_width,
            )?,
            learning_rate: option(learn_args, "--learning-rate")
                .map(|value| value.parse::<f64>())
                .transpose()?
                .unwrap_or(defaults.double_q.learning_rate),
            discount: option(learn_args, "--discount")
                .map(|value| value.parse::<f64>())
                .transpose()?
                .unwrap_or(defaults.double_q.discount),
            target_sync_steps: usize_option(
                learn_args,
                "--target-sync-steps",
                defaults.double_q.target_sync_steps,
            )?,
            gradient_clip: option(learn_args, "--gradient-clip")
                .map(|value| value.parse::<f64>())
                .transpose()?
                .unwrap_or(defaults.double_q.gradient_clip),
            seed: u64_option(learn_args, "--seed", defaults.double_q.seed)?,
        },
        conservative_weight: option(learn_args, "--conservative-weight")
            .map(|value| value.parse::<f64>())
            .transpose()?
            .unwrap_or(defaults.conservative_weight),
        temperature: option(learn_args, "--temperature")
            .map(|value| value.parse::<f64>())
            .transpose()?
            .unwrap_or(defaults.temperature),
    };
    let action_support = training.transitions.iter().fold(
        BTreeMap::<u32, usize>::new(),
        |mut counts, transition| {
            *counts.entry(transition.action).or_default() += 1;
            counts
        },
    );
    if action_support.len() > MAX_FQI_ACTIONS {
        return Err(format!(
            "CQL supports at most {MAX_FQI_ACTIONS} distinct actions; received {}",
            action_support.len()
        )
        .into());
    }
    let actions = action_support.keys().copied().collect::<Vec<_>>();
    let model = ConservativeQ::fit(
        training.feature_count,
        &actions,
        &training.transitions,
        &config,
    )?;
    let query_index = usize_option(learn_args, "--query-transition", 0)?;
    let query_transition = training
        .transitions
        .get(query_index)
        .ok_or("--query-transition is outside the merged transition batch")?;
    let query_side = option(learn_args, "--query-side").unwrap_or_else(|| "state".into());
    let query_state = match query_side.as_str() {
        "state" => &query_transition.state,
        "next-state" => &query_transition.next_state,
        _ => return Err("--query-side must be state or next-state".into()),
    };
    let ranking = model
        .rank_actions(query_state)?
        .into_iter()
        .map(|estimate| {
            json!({
                "action": estimate.action,
                "mean_q": estimate.mean,
                "critic_a": estimate.critic_a,
                "critic_b": estimate.critic_b,
                "critic_disagreement": estimate.critic_disagreement,
                "support": action_support[&estimate.action],
            })
        })
        .collect::<Vec<_>>();
    let model_output = option(learn_args, "--model-output").map(PathBuf::from);
    let mut model_content_blob = None;
    let mut model_artifact_store = None;
    if let Some(path) = &model_output {
        if path.exists() {
            return Err(format!("CQL model output already exists: {}", path.display()).into());
        }
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        let bytes = model.artifact_bytes(
            training.feature_schema,
            training.action_schema,
            dataset_manifest
                .as_ref()
                .map(|manifest| manifest.dataset_sha256),
            &training.corpus_digests,
            &config,
        )?;
        fs::write(path, &bytes)?;
        let store_path = option(learn_args, "--artifact-store")
            .map(PathBuf::from)
            .unwrap_or_else(|| path.parent().unwrap_or(Path::new(".")).join("content"));
        model_content_blob =
            Some(ContentStore::initialize(&store_path)?.put_bytes(&bytes, ContentKind::Model)?);
        model_artifact_store = Some(store_path);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": "dusklight-conservative-q-ranking/v1",
            "feature_schema": training.feature_schema,
            "action_schema": training.action_schema,
            "input_corpora": inputs,
            "training_corpus_sha256": training.corpus_digests,
            "training_dataset": dataset_path,
            "training_dataset_sha256": dataset_manifest.as_ref().map(|manifest| manifest.dataset_sha256),
            "transition_count": training.transitions.len(),
            "episode_groups": training.episode_groups.iter().copied().collect::<BTreeSet<_>>().len(),
            "query_transition": query_index,
            "query_side": query_side,
            "config": config,
            "gradient_updates": model.gradient_updates(),
            "target_synchronizations": model.target_synchronizations(),
            "conservative_updates": model.conservative_updates(),
            "mean_conservative_gap": model.mean_conservative_gap(),
            "conservative_objective": "temperature_logsumexp_all_actions_minus_observed_action",
            "model_output": model_output,
            "model_artifact_store": model_artifact_store,
            "model_content_blob": model_content_blob,
            "ranking": ranking,
            "promotion_authority": false,
            "limitations": [
                "CQL reduces but does not prove safety for state-local unsupported actions",
                "numeric normalization does not provide categorical embeddings or missingness masks",
                "critic disagreement is not calibrated uncertainty",
                "rankings are proposals and require native predicate and cold replay proof"
            ]
        }))?
    );
    Ok(())
}

fn goal_reachability_config(
    learn_args: &[String],
) -> Result<NativeGoalReachabilityConfig, Box<dyn Error>> {
    let defaults = NativeGoalReachabilityConfig::default();
    let parse_f64 = |name: &str, default: f64| -> Result<f64, Box<dyn Error>> {
        option(learn_args, name)
            .map(|value| value.parse::<f64>().map_err(Into::into))
            .transpose()
            .map(|value| value.unwrap_or(default))
    };
    Ok(NativeGoalReachabilityConfig {
        members: u8::try_from(usize_option(
            learn_args,
            "--members",
            usize::from(defaults.members),
        )?)
        .map_err(|_| "goal reachability members exceed u8")?,
        epochs: u16::try_from(usize_option(
            learn_args,
            "--epochs",
            usize::from(defaults.epochs),
        )?)
        .map_err(|_| "goal reachability epochs exceed u16")?,
        hidden_width: u16::try_from(usize_option(
            learn_args,
            "--hidden-width",
            usize::from(defaults.hidden_width),
        )?)
        .map_err(|_| "goal reachability hidden width exceeds u16")?,
        learning_rate: parse_f64("--learning-rate", defaults.learning_rate)?,
        l2_penalty: parse_f64("--l2-penalty", defaults.l2_penalty)?,
        gradient_clip: parse_f64("--gradient-clip", defaults.gradient_clip)?,
        minimum_validation_improvement: parse_f64(
            "--minimum-validation-improvement",
            defaults.minimum_validation_improvement,
        )?,
        maximum_validation_reachability_stddev: parse_f64(
            "--maximum-validation-reachability-stddev",
            defaults.maximum_validation_reachability_stddev,
        )?,
        seed: u64_option(learn_args, "--seed", defaults.seed)?,
    })
}

pub fn command_learn(args: &[String]) -> Result<(), Box<dyn Error>> {
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
        Some("freeze-tactic-policy") => {
            let learn_args = &args[1..];
            let checkpoint =
                TacticQCampaign::read_checkpoint(&required_path(learn_args, "--checkpoint")?)?;
            let catalog = default_route_tactic_catalog()?;
            let policy = checkpoint.freeze_greedy_policy(catalog.action_schema_sha256())?;
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
            let report = run_native_tactic_route(&NativeTacticRouteRunConfig {
                repository_root: &repository_root,
                optimization: &request,
                execution: &execution,
                output_root: &output,
                exploration_seeds: &seeds,
                decisions_per_seed: u64_option(learn_args, "--decisions-per-seed", 256)?,
                branch_every_decisions: u64_option(learn_args, "--branch-every", 8)?,
                epsilon_per_million: option(learn_args, "--epsilon-per-million")
                    .map(|value| value.parse())
                    .transpose()?
                    .unwrap_or(350_000),
            })?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": report.schema,
                    "report": output.join("report.json"),
                    "successful_seeds": report.successful_seeds,
                    "exploration_seeds": report.exploration_seeds,
                    "total_decisions": report.total_decisions,
                    "total_native_ticks": report.total_native_ticks,
                    "demonstration_transitions": report.demonstration_transitions,
                }))?
            );
            Ok(())
        }
        Some("diff-episodes") => {
            let learn_args = &args[1..];
            let success_trace_path = required_path(learn_args, "--success-trace")?;
            let failure_trace_path = required_path(learn_args, "--failure-trace")?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(
                    format!("trace diff output already exists: {}", output.display()).into(),
                );
            }
            let success_evidence_path = option(learn_args, "--success-evidence").map(PathBuf::from);
            let failure_evidence_path = option(learn_args, "--failure-evidence").map(PathBuf::from);
            if success_evidence_path.is_some() != failure_evidence_path.is_some() {
                return Err(
                    "--success-evidence and --failure-evidence must be supplied together".into(),
                );
            }
            let success_bytes = fs::read(&success_trace_path)?;
            let failure_bytes = fs::read(&failure_trace_path)?;
            let success_trace = huntctl::trace::decode(&success_bytes)?;
            let failure_trace = huntctl::trace::decode(&failure_bytes)?;
            let success_evidence: Option<TransitionEvidenceBundle> = success_evidence_path
                .as_ref()
                .map(|path| fs::read(path).map_err(Box::<dyn Error>::from))
                .transpose()?
                .map(|bytes| serde_json::from_slice(&bytes))
                .transpose()?;
            let failure_evidence: Option<TransitionEvidenceBundle> = failure_evidence_path
                .as_ref()
                .map(|path| fs::read(path).map_err(Box::<dyn Error>::from))
                .transpose()?
                .map(|bytes| serde_json::from_slice(&bytes))
                .transpose()?;
            let report = SiblingTraceDiff::compare(
                &success_trace,
                &success_bytes,
                &failure_trace,
                &failure_bytes,
                success_evidence.as_ref(),
                failure_evidence.as_ref(),
            )?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, serde_json::to_vec_pretty(&report)?)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("dataset") => {
            let learn_args = &args[1..];
            let source_paths = repeated_option(learn_args, "--source");
            if source_paths.is_empty() {
                return Err("learn dataset requires at least one --source SOURCE.json".into());
            }
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!("dataset output already exists: {}", output.display()).into());
            }
            let mut sources = Vec::with_capacity(source_paths.len());
            for source_path in &source_paths {
                let source_path = PathBuf::from(source_path);
                let descriptor: DatasetSourceDescriptor =
                    serde_json::from_slice(&fs::read(&source_path)?)?;
                sources.push(descriptor.load(source_path.parent().unwrap_or(Path::new(".")))?);
            }
            let validation_percent =
                u8::try_from(usize_option(learn_args, "--validation-percent", 10)?)?;
            let test_percent = u8::try_from(usize_option(learn_args, "--test-percent", 10)?)?;
            let manifest = DatasetManifest::build(
                &sources,
                &DatasetBuildConfig {
                    validation_percent,
                    test_percent,
                    withheld_objectives: repeated_option(learn_args, "--withheld-objective")
                        .into_iter()
                        .collect(),
                },
            )?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let bytes = serde_json::to_vec_pretty(&manifest)?;
            fs::write(&output, &bytes)?;
            let artifact_store = option(learn_args, "--artifact-store")
                .map(PathBuf::from)
                .unwrap_or_else(|| output.parent().unwrap_or(Path::new(".")).join("content"));
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::DatasetManifest)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": manifest.schema,
                    "dataset_sha256": manifest.dataset_sha256,
                    "frozen_withheld_sha256": manifest.frozen_withheld_sha256,
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                    "report": manifest.report,
                    "leakage": manifest.leakage,
                    "normalization_schemas": manifest.normalization.len(),
                }))?
            );
            Ok(())
        }
        Some("extract-trace") => {
            let learn_args = &args[1..];
            let trace_path = required_path(learn_args, "--trace")?;
            let tape_path = required_path(learn_args, "--tape")?;
            let episode_context_path = required_path(learn_args, "--episode-context")?;
            let output = required_path(learn_args, "--output")?;
            let start_tape_frame: u64 = option(learn_args, "--start-frame")
                .ok_or("missing required --start-frame N")?
                .parse()?;
            let end_tape_frame: u64 = option(learn_args, "--end-frame")
                .ok_or("missing required --end-frame N")?
                .parse()?;
            let trace_bytes = fs::read(&trace_path)?;
            let tape_bytes = fs::read(&tape_path)?;
            let episode_context: EpisodeContext =
                serde_json::from_slice(&fs::read(&episode_context_path)?)?;
            episode_context.validate()?;
            let episode_digest = if let Some(value) = option(learn_args, "--episode-digest") {
                value.parse::<Digest>()?
            } else {
                let mut hasher = Sha256::new();
                hasher.update(b"dusklight.exploratory-offline-episode/v1\0");
                hasher.update((trace_bytes.len() as u64).to_le_bytes());
                hasher.update(&trace_bytes);
                hasher.update((tape_bytes.len() as u64).to_le_bytes());
                hasher.update(&tape_bytes);
                Digest(hasher.finalize().into())
            };
            let end_is_terminal = learn_args.iter().any(|arg| arg == "--terminal");
            let feature_view =
                option(learn_args, "--view").unwrap_or_else(|| "movement-state/v1".into());
            let action_view =
                option(learn_args, "--action-view").unwrap_or_else(|| "movement-action/v2".into());
            let extract_config = ExploratoryExtractConfig {
                episode_digest,
                start_tape_frame,
                end_tape_frame,
                start_reference: None,
                terminal_reference: None,
                end_is_terminal,
            };
            let corpus = match (feature_view.as_str(), action_view.as_str()) {
                ("movement-state/v1", "movement-action/v2") => {
                    extract_exploratory_from_bytes(&trace_bytes, &tape_bytes, extract_config)?
                }
                (MOVEMENT_STATE_V2_ID, "movement-action/v2") => {
                    extract_exploratory_v2_from_bytes(&trace_bytes, &tape_bytes, extract_config)?
                }
                (MOVEMENT_STATE_V2_ID, "movement-action/v3") => {
                    extract_exploratory_v3_from_bytes(&trace_bytes, &tape_bytes, extract_config)?
                }
                (feature, actions) => {
                    return Err(format!(
                        "unsupported observation/action view pair {feature:?}/{actions:?}; expected movement-state/v1 with movement-action/v2, or {MOVEMENT_STATE_V2_ID} with movement-action/v2 or movement-action/v3"
                    )
                    .into());
                }
            };
            let decoded_trace = huntctl::trace::decode(&trace_bytes)?;
            let decoded_tape = InputTape::decode(&tape_bytes)?.tape;
            let transition_evidence = TransitionEvidenceBundle::build(TransitionEvidenceBuild {
                corpus: &corpus,
                trace: &decoded_trace,
                tape: &decoded_tape,
                trace_sha256: Digest(Sha256::digest(&trace_bytes).into()),
                tape_sha256: Digest(Sha256::digest(&tape_bytes).into()),
                start_tape_frame,
                end_tape_frame,
                terminal_reason: end_is_terminal
                    .then_some(TerminalReasonEvidence::DeclaredExtractionBoundary),
            })?;
            let transition_evidence_bytes = serde_json::to_vec_pretty(&transition_evidence)?;
            let trace_sha256 = Digest(Sha256::digest(&trace_bytes).into());
            let tape_sha256 = Digest(Sha256::digest(&tape_bytes).into());
            let episode_manifest = EpisodeManifest::build(EpisodeManifestBuild {
                context: &episode_context,
                boot: &decoded_tape.boot,
                corpus: &corpus,
                query_view_id: &feature_view,
                tape_sha256,
                trace_sha256,
                transition_evidence_sha256: Digest(
                    Sha256::digest(&transition_evidence_bytes).into(),
                ),
            })?;
            let compression_level: i32 = option(learn_args, "--compression-level")
                .map(|value| value.parse())
                .transpose()?
                .unwrap_or(3);
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let content_digest = corpus.write_zstd_file(&output, compression_level)?;
            let artifact_store = option(learn_args, "--artifact-store")
                .map(PathBuf::from)
                .unwrap_or_else(|| output.parent().unwrap_or(Path::new(".")).join("content"));
            let trace_content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&trace_bytes, ContentKind::GameplayTrace)?;
            let transition_evidence_path =
                PathBuf::from(format!("{}.evidence.json", output.display()));
            fs::write(&transition_evidence_path, transition_evidence_bytes)?;
            let episode_manifest_path = PathBuf::from(format!("{}.episode.json", output.display()));
            fs::write(
                &episode_manifest_path,
                serde_json::to_vec_pretty(&episode_manifest)?,
            )?;
            let dataset_source_path =
                PathBuf::from(format!("{}.dataset-source.json", output.display()));
            fs::write(
                &dataset_source_path,
                serde_json::to_vec_pretty(&DatasetSourceDescriptor {
                    schema: DATASET_SOURCE_SCHEMA_V1.into(),
                    source_id: episode_manifest.episode_sha256.to_string(),
                    episode_manifest: fs::canonicalize(&episode_manifest_path)?,
                    transition_corpus: fs::canonicalize(&output)?,
                    absolute_tape: fs::canonicalize(&tape_path)?,
                    transition_evidence: fs::canonicalize(&transition_evidence_path)?,
                    gameplay_trace: fs::canonicalize(&trace_path)?,
                    route_family: episode_manifest.objective.id.clone(),
                    screenshot_sha256: Vec::new(),
                    checkpoint_sha256: Vec::new(),
                })?,
            )?;
            let observation_spec = if feature_view == MOVEMENT_STATE_V2_ID {
                let spec = movement_state_v2_spec();
                let path = PathBuf::from(format!("{}.observation.json", output.display()));
                fs::write(&path, spec.canonical_bytes()?)?;
                Some(path)
            } else {
                None
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": "dusklight-exploratory-extraction/v1",
                    "authoritative": false,
                    "limitations": [
                        "the batch contains observed behavior, not counterfactual actions",
                        "explicit frame bounds are not native milestone proof",
                        "--terminal records a declared extraction boundary, not inferred objective proof",
                        "the observation view is objective-specific and not a complete process state"
                    ],
                    "trace": trace_path,
                    "trace_content_blob": trace_content_blob,
                    "artifact_store": artifact_store,
                    "tape": tape_path,
                    "output": output,
                    "transition_evidence": transition_evidence_path,
                    "episode_context": episode_context_path,
                    "episode_manifest": episode_manifest_path,
                    "dataset_source": dataset_source_path,
                    "input_identity": episode_manifest.input_identity_sha256,
                    "episode_identity": episode_manifest.episode_sha256,
                    "feature_view": feature_view,
                    "observation_spec": observation_spec,
                    "episode_digest": episode_digest,
                    "content_digest": content_digest,
                    "feature_schema": corpus.feature_schema,
                    "action_schema": corpus.action_schema,
                    "feature_count": corpus.feature_count,
                    "transitions": corpus.transitions.len(),
                    "start_frame": start_tape_frame,
                    "end_frame": end_tape_frame,
                    "terminal": end_is_terminal,
                }))?
            );
            Ok(())
        }
        Some("inspect-episode") => {
            let input = required_path(&args[1..], "--input")?;
            let artifact: ImmutableEpisodeArtifact = serde_json::from_slice(&fs::read(&input)?)?;
            artifact.validate()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": artifact.schema,
                    "content_sha256": artifact.content_sha256,
                    "episode_sha256": artifact.episode_sha256,
                    "objective": artifact.objective,
                    "terminal": artifact.terminal,
                    "terminal_detail": artifact.terminal_detail,
                    "realized_tape_sha256": artifact.realized_tape_sha256,
                    "gameplay_trace_sha256": artifact.gameplay_trace_sha256,
                    "transition_corpus_sha256": artifact.transition_corpus_sha256,
                    "transition_evidence_sha256": artifact.transition_evidence_sha256,
                    "steps": artifact.steps.len(),
                    "lineage": artifact.lineage,
                }))?
            );
            Ok(())
        }
        Some("inspect-native") => {
            let learn_args = &args[1..];
            let inputs = repeated_option(learn_args, "--input");
            if inputs.is_empty() || inputs.len() > MAX_LEARN_INPUT_CORPORA {
                return Err(format!(
                    "learn inspect-native requires 1..={MAX_LEARN_INPUT_CORPORA} --input SHARD"
                )
                .into());
            }
            let shards = inputs
                .iter()
                .map(NativeEpisodeShard::read)
                .collect::<Result<Vec<_>, _>>()?;
            let report = inspect_native_episode_corpus(&shards);
            let bytes = serde_json::to_vec_pretty(&report)?;
            if let Some(output) = option(learn_args, "--output").map(PathBuf::from) {
                if output.exists() {
                    return Err(format!(
                        "native corpus inspection output already exists: {}",
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
                fs::write(output, &bytes)?;
            }
            println!("{}", String::from_utf8(bytes)?);
            Ok(())
        }
        Some("trace-return-restart-writes") => {
            let learn_args = &args[1..];
            let inputs = repeated_option(learn_args, "--input");
            if inputs.is_empty() || inputs.len() > MAX_LEARN_INPUT_CORPORA {
                return Err(format!(
                    "learn trace-return-restart-writes requires 1..={MAX_LEARN_INPUT_CORPORA} --input SHARD"
                )
                .into());
            }
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "return/restart write trace output already exists: {}",
                    output.display()
                )
                .into());
            }
            let shards = inputs
                .iter()
                .map(NativeEpisodeShard::read)
                .collect::<Result<Vec<_>, _>>()?;
            let report = NativeReturnRestartWriteTrace::build(&shards)?;
            let bytes = serde_json::to_vec_pretty(&report)?;
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
                    "schema": report.schema,
                    "content_sha256": report.content_sha256,
                    "source_shards": report.source_shards.len(),
                    "summary": report.summary,
                    "output": output,
                }))?
            );
            Ok(())
        }
        Some("validate-return-restart-write-trace") => {
            let input = required_path(&args[1..], "--input")?;
            let report = NativeReturnRestartWriteTrace::decode(&fs::read(&input)?)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": report.schema,
                    "content_sha256": report.content_sha256,
                    "source_shards": report.source_shards.len(),
                    "summary": report.summary,
                    "input": input,
                }))?
            );
            Ok(())
        }
        Some("native-replay") => {
            let learn_args = &args[1..];
            let source_paths = repeated_option(learn_args, "--source");
            let shard_paths = repeated_option(learn_args, "--input");
            if source_paths.is_empty() == shard_paths.is_empty()
                || source_paths.len().max(shard_paths.len()) > MAX_LEARN_INPUT_CORPORA
            {
                return Err(format!(
                    "learn native-replay requires either 1..={MAX_LEARN_INPUT_CORPORA} --source SOURCE.json or --input EPISODES.dseps with --role ROLE"
                )
                .into());
            }
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "native replay corpus output already exists: {}",
                    output.display()
                )
                .into());
            }
            let previous: Option<NativeReplayCorpus> = option(learn_args, "--previous")
                .map(PathBuf::from)
                .map(|path| -> Result<_, Box<dyn Error>> {
                    let corpus: NativeReplayCorpus = serde_json::from_slice(&fs::read(path)?)?;
                    corpus.validate()?;
                    Ok(corpus)
                })
                .transpose()?;
            let corpus = if !source_paths.is_empty() {
                let mut loaded = Vec::with_capacity(source_paths.len());
                for source_path in source_paths {
                    let descriptor_path = PathBuf::from(source_path);
                    let descriptor: NativeReplaySourceDescriptor =
                        serde_json::from_slice(&fs::read(&descriptor_path)?)?;
                    if descriptor.schema != NATIVE_REPLAY_SOURCE_SCHEMA_V1 {
                        return Err(format!(
                            "native replay source has invalid schema: {}",
                            descriptor_path.display()
                        )
                        .into());
                    }
                    let shard_path = if descriptor.shard.is_absolute() {
                        descriptor.shard.clone()
                    } else {
                        descriptor_path
                            .parent()
                            .unwrap_or(Path::new("."))
                            .join(&descriptor.shard)
                    };
                    loaded.push((descriptor, NativeEpisodeShard::read(shard_path)?));
                }
                let sources = loaded
                    .iter()
                    .map(|(descriptor, shard)| {
                        let episode_index = shard
                            .episodes
                            .iter()
                            .position(|episode| episode.id == descriptor.episode_id)
                            .ok_or_else(|| {
                                format!(
                                    "native replay episode {:?} is absent from shard {}",
                                    descriptor.episode_id, shard.content_sha256
                                )
                            })?;
                        Ok(ReplayEpisodeSource {
                            shard,
                            episode_index,
                            role: descriptor.role,
                            policy_lineage_sha256: descriptor.policy_lineage_sha256,
                            parent_entry_sha256: descriptor.parent_entry_sha256,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                NativeReplayCorpus::build(previous.as_ref(), &sources)?
            } else {
                let role_value = option(learn_args, "--role")
                    .ok_or("native replay shard ingestion requires --role ROLE")?;
                let role = parse_replay_role(&role_value)?;
                let policy_lineage_sha256 = option(learn_args, "--policy-lineage-sha256")
                    .map(|value| value.parse::<Digest>())
                    .transpose()?;
                if (role == ReplayExperienceRole::PolicyRollout) != policy_lineage_sha256.is_some()
                {
                    return Err(
                        "policy_rollout shard ingestion requires exactly one --policy-lineage-sha256"
                            .into(),
                    );
                }
                let shards = shard_paths
                    .iter()
                    .map(NativeEpisodeShard::read)
                    .collect::<Result<Vec<_>, _>>()?;
                let sources = shards
                    .iter()
                    .flat_map(|shard| {
                        (0..shard.episodes.len()).map(move |episode_index| ReplayEpisodeSource {
                            shard,
                            episode_index,
                            role,
                            policy_lineage_sha256,
                            parent_entry_sha256: None,
                        })
                    })
                    .collect::<Vec<_>>();
                NativeReplayCorpus::build(previous.as_ref(), &sources)?
            };
            let bytes = serde_json::to_vec_pretty(&corpus)?;
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
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::NativeReplayCorpus)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": NATIVE_REPLAY_CORPUS_SCHEMA_V1,
                    "generation": corpus.generation,
                    "corpus_sha256": corpus.corpus_sha256,
                    "parent_corpus_sha256": corpus.parent_corpus_sha256,
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                    "report": corpus.report,
                }))?
            );
            Ok(())
        }
        Some("auxiliary-dataset") => {
            let learn_args = &args[1..];
            let corpus_path = required_path(learn_args, "--corpus")?;
            let input_paths = repeated_option(learn_args, "--input");
            if input_paths.is_empty() || input_paths.len() > MAX_LEARN_INPUT_CORPORA {
                return Err(format!(
                    "learn auxiliary-dataset requires 1..={MAX_LEARN_INPUT_CORPORA} --input EPISODES.dseps"
                )
                .into());
            }
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "native auxiliary dataset output already exists: {}",
                    output.display()
                )
                .into());
            }
            let corpus: NativeReplayCorpus = serde_json::from_slice(&fs::read(&corpus_path)?)?;
            corpus.validate()?;
            let shards = input_paths
                .iter()
                .map(NativeEpisodeShard::read)
                .collect::<Result<Vec<_>, _>>()?;
            let defaults = AuxiliarySplitConfig::default();
            let training_basis_points = usize_option(
                learn_args,
                "--training-basis-points",
                usize::from(defaults.training_basis_points),
            )?;
            let validation_basis_points = usize_option(
                learn_args,
                "--validation-basis-points",
                usize::from(defaults.validation_basis_points),
            )?;
            let split_config = AuxiliarySplitConfig {
                training_basis_points: u16::try_from(training_basis_points)
                    .map_err(|_| "training basis points exceed u16")?,
                validation_basis_points: u16::try_from(validation_basis_points)
                    .map_err(|_| "validation basis points exceed u16")?,
                seed: u64_option(learn_args, "--seed", defaults.seed)?,
            };
            let dataset = NativeAuxiliaryDataset::build(&corpus, &shards, split_config)?;
            let bytes = serde_json::to_vec_pretty(&dataset)?;
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
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::NativeAuxiliaryDataset)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": NATIVE_AUXILIARY_DATASET_SCHEMA_V2,
                    "dataset_sha256": dataset.dataset_sha256,
                    "replay_corpus_sha256": dataset.replay_corpus_sha256,
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                    "report": dataset.report,
                }))?
            );
            Ok(())
        }
        Some("goal-trajectory-dataset") => {
            let learn_args = &args[1..];
            let corpus_path = required_path(learn_args, "--corpus")?;
            let input_paths = repeated_option(learn_args, "--input");
            if input_paths.is_empty() || input_paths.len() > MAX_LEARN_INPUT_CORPORA {
                return Err(format!(
                    "learn goal-trajectory-dataset requires 1..={MAX_LEARN_INPUT_CORPORA} --input EPISODES.dseps"
                )
                .into());
            }
            let milestones_path = required_path(learn_args, "--milestones")?;
            let milestone_goal = option(learn_args, "--milestone-goal")
                .ok_or("learn goal-trajectory-dataset requires --milestone-goal NAME")?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "native goal trajectory dataset output already exists: {}",
                    output.display()
                )
                .into());
            }
            let corpus: NativeReplayCorpus = serde_json::from_slice(&fs::read(&corpus_path)?)?;
            corpus.validate()?;
            let shards = input_paths
                .iter()
                .map(NativeEpisodeShard::read)
                .collect::<Result<Vec<_>, _>>()?;
            let milestone_bytes = fs::read(&milestones_path)?;
            let decoded = huntctl::milestone_dsl::decode(&milestone_bytes)?;
            let definition_index = decoded
                .definitions
                .iter()
                .position(|definition| definition.name == milestone_goal)
                .ok_or_else(|| {
                    format!(
                        "compiled milestone definition {milestone_goal:?} does not exist in {}",
                        milestones_path.display()
                    )
                })?;
            let compiled = huntctl::milestone_dsl::CompiledMilestones {
                bytes: milestone_bytes,
                program_sha256: decoded.program_sha256,
                definitions: decoded.definitions,
            };
            let graph = huntctl::learning::compiled_goal_graph::CompiledGoalGraph::from_compiled(
                &compiled,
                definition_index,
            )?;
            let defaults = NativeGoalTrajectoryConfig::default();
            let n_step = usize_option(learn_args, "--n-step", usize::from(defaults.n_step))?;
            let discount_millionths = usize_option(
                learn_args,
                "--discount-millionths",
                defaults.discount_millionths as usize,
            )?;
            let training_basis_points = usize_option(
                learn_args,
                "--training-basis-points",
                usize::from(defaults.training_basis_points),
            )?;
            let validation_basis_points = usize_option(
                learn_args,
                "--validation-basis-points",
                usize::from(defaults.validation_basis_points),
            )?;
            let config = NativeGoalTrajectoryConfig {
                demonstration_mode: defaults.demonstration_mode,
                n_step: u16::try_from(n_step).map_err(|_| "n-step exceeds u16")?,
                discount_millionths: u32::try_from(discount_millionths)
                    .map_err(|_| "discount millionths exceed u32")?,
                training_basis_points: u16::try_from(training_basis_points)
                    .map_err(|_| "training basis points exceed u16")?,
                validation_basis_points: u16::try_from(validation_basis_points)
                    .map_err(|_| "validation basis points exceed u16")?,
                split_seed: u64_option(learn_args, "--seed", defaults.split_seed)?,
            };
            let dataset = NativeGoalTrajectoryDataset::build(&corpus, &shards, &graph, config)?;
            let bytes = serde_json::to_vec_pretty(&dataset)?;
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
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::NativeGoalTrajectoryDataset)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": NATIVE_GOAL_TRAJECTORY_DATASET_SCHEMA_V2,
                    "dataset_sha256": dataset.dataset_sha256,
                    "replay_corpus_sha256": dataset.replay_corpus_sha256,
                    "goal": milestone_goal,
                    "goal_graph_sha256": dataset.goal.graph_sha256,
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                    "report": dataset.report,
                }))?
            );
            Ok(())
        }
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
        Some("inspect-auxiliary") => {
            let input = required_path(&args[1..], "--input")?;
            let dataset: NativeAuxiliaryDataset = serde_json::from_slice(&fs::read(input)?)?;
            dataset.validate()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": NATIVE_AUXILIARY_DATASET_SCHEMA_V2,
                    "dataset_sha256": dataset.dataset_sha256,
                    "replay_corpus_sha256": dataset.replay_corpus_sha256,
                    "report": dataset.report,
                    "split_diagnostics": dataset.split_diagnostics()?,
                }))?
            );
            Ok(())
        }
        Some("pretrain-native-encoder") => {
            let learn_args = &args[1..];
            let dataset_path = required_path(learn_args, "--dataset")?;
            let input = required_path(learn_args, "--input")?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(
                    format!("native encoder output already exists: {}", output.display()).into(),
                );
            }
            let dataset: NativeAuxiliaryDataset =
                serde_json::from_slice(&fs::read(&dataset_path)?)?;
            let shard = NativeEpisodeShard::read(&input)?;
            let source_native_shard_sha256 = shard.content_sha256;
            let excluded = repeated_option(learn_args, "--exclude-family")
                .into_iter()
                .map(|name| {
                    NativeEncoderChannelFamily::parse(&name)
                        .ok_or_else(|| format!("unknown native encoder channel family: {name}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let history_depth = usize_option(learn_args, "--history-depth", 0)?;
            let history_encoding = option(learn_args, "--history-encoding");
            let history_width = option(learn_args, "--history-width")
                .map(|value| value.parse::<usize>())
                .transpose()?;
            let base_feature_spec = NativeEncoderFeatureSpec::excluding(excluded)?;
            let feature_spec = match history_encoding.as_deref() {
                None if history_width.is_none() => {
                    base_feature_spec.with_history_depth(history_depth)?
                }
                Some("stacked") if history_depth > 0 && history_width.is_none() => {
                    base_feature_spec.with_history_depth(history_depth)?
                }
                Some("recurrent-reservoir") if history_depth > 0 => base_feature_spec
                    .with_recurrent_history(
                        history_depth,
                        history_width.unwrap_or(DEFAULT_HISTORY_RECURRENT_WIDTH),
                    )?,
                Some("trainable-gru") if history_depth > 0 => base_feature_spec
                    .with_trainable_history(
                        history_depth,
                        history_width.unwrap_or(DEFAULT_HISTORY_RECURRENT_WIDTH),
                    )?,
                Some("stacked" | "recurrent-reservoir" | "trainable-gru") => {
                    return Err("native encoder history encoding requires --history-depth greater than zero".into());
                }
                Some(name) => {
                    return Err(format!("unknown native encoder history encoding: {name}").into());
                }
                None => {
                    return Err(
                        "--history-width requires --history-encoding recurrent-reservoir or trainable-gru".into(),
                    );
                }
            };
            let pooling = option(learn_args, "--pooling")
                .map(|name| {
                    MultiTaskSetPooling::parse(&name)
                        .ok_or_else(|| format!("unknown native encoder pooling mode: {name}"))
                })
                .transpose()?
                .unwrap_or(MultiTaskSetPooling::MeanMax);
            let corpus =
                NativeMultiTaskActorCorpus::build_with_spec(&dataset, &shard, feature_spec)?;
            let temporal = corpus.feature_spec.temporal_config();
            drop(shard);
            let defaults = TrainableSetConfig::default();
            let config = TrainableSetConfig {
                epochs: usize_option(learn_args, "--epochs", defaults.epochs)?,
                node_hidden_width: usize_option(
                    learn_args,
                    "--node-hidden-width",
                    defaults.node_hidden_width,
                )?,
                head_hidden_width: usize_option(
                    learn_args,
                    "--state-width",
                    defaults.head_hidden_width,
                )?,
                learning_rate: option(learn_args, "--learning-rate")
                    .map(|value| value.parse())
                    .transpose()?
                    .unwrap_or(defaults.learning_rate),
                l2_penalty: option(learn_args, "--l2-penalty")
                    .map(|value| value.parse())
                    .transpose()?
                    .unwrap_or(defaults.l2_penalty),
                gradient_clip: option(learn_args, "--gradient-clip")
                    .map(|value| value.parse())
                    .transpose()?
                    .unwrap_or(defaults.gradient_clip),
                minimum_relative_improvement: option(learn_args, "--minimum-relative-improvement")
                    .map(|value| value.parse())
                    .transpose()?
                    .unwrap_or(defaults.minimum_relative_improvement),
                seed: u64_option(learn_args, "--seed", defaults.seed)?,
                fixed_slot_count: defaults.fixed_slot_count,
            };
            let (report, model) = CompleteSetMultiTaskEncoder::fit_with_pooling_and_temporal(
                corpus.actor_feature_schema_sha256,
                corpus.training_dataset_sha256,
                corpus.validation_dataset_sha256,
                corpus.target_names.clone(),
                &corpus.training,
                &corpus.validation,
                config,
                pooling,
                temporal,
            )?;
            let test_evaluation = model.evaluate(&corpus.test)?;
            let shuffled_target_control = fit_shuffled_auxiliary_control_with_pooling_and_temporal(
                corpus.actor_feature_schema_sha256,
                corpus.target_names.clone(),
                corpus.training,
                corpus.validation_dataset_sha256,
                &corpus.validation,
                &corpus.test,
                config,
                pooling,
                temporal,
            )?;
            let artifact = json!({
                "schema": "dusklight-native-multitask-encoder-artifact/v13",
                "source_auxiliary_dataset_sha256": dataset.dataset_sha256,
                "source_native_shard_sha256": source_native_shard_sha256,
                "actor_feature_schema_sha256": corpus.actor_feature_schema_sha256,
                "feature_spec": corpus.feature_spec,
                "test_dataset_sha256": corpus.test_dataset_sha256,
                "report": report,
                "test_evaluation": test_evaluation,
                "shuffled_target_control": shuffled_target_control,
                "model": model,
            });
            let bytes = serde_json::to_vec_pretty(&artifact)?;
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
                    "schema": artifact["schema"],
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                    "report": artifact["report"],
                    "test_evaluation": artifact["test_evaluation"],
                    "shuffled_target_control": artifact["shuffled_target_control"],
                }))?
            );
            Ok(())
        }
        Some("collision-history") => {
            let learn_args = &args[1..];
            let input = required_path(learn_args, "--input")?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "collision history output already exists: {}",
                    output.display()
                )
                .into());
            }
            let history_depth = usize_option(
                learn_args,
                "--history-depth",
                DEFAULT_COLLISION_HISTORY_DEPTH,
            )?;
            let shard = NativeEpisodeShard::read(&input)?;
            let view = NativeCollisionHistoryView::build(&shard, history_depth)?;
            let bytes = view.canonical_bytes()?;
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
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::NativeCollisionHistory)?;
            let solver_present = view
                .decisions
                .iter()
                .filter(|decision| {
                    view.snapshots[decision.current_snapshot_index as usize]
                        .solver
                        .is_some()
                })
                .count();
            let background_present = view
                .decisions
                .iter()
                .filter(|decision| {
                    view.snapshots[decision.current_snapshot_index as usize]
                        .background
                        .is_some()
                })
                .count();
            let solver_changes = view
                .auxiliary_targets
                .iter()
                .filter(|target| {
                    view.snapshots[target.before_snapshot_index as usize].solver
                        != view.snapshots[target.after_snapshot_index as usize].solver
                })
                .count();
            let background_changes = view
                .auxiliary_targets
                .iter()
                .filter(|target| {
                    view.snapshots[target.before_snapshot_index as usize].background
                        != view.snapshots[target.after_snapshot_index as usize].background
                })
                .count();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": view.schema,
                    "view_sha256": view.view_sha256,
                    "native_shard_sha256": view.native_shard_sha256,
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                    "history_depth": view.history_depth,
                    "snapshots": view.snapshots.len(),
                    "decisions": view.decisions.len(),
                    "auxiliary_targets": view.auxiliary_targets.len(),
                    "solver_present": solver_present,
                    "background_present": background_present,
                    "solver_changes": solver_changes,
                    "background_changes": background_changes,
                }))?
            );
            Ok(())
        }
        Some("episode-history") => {
            let learn_args = &args[1..];
            let input = required_path(learn_args, "--input")?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "episode history output already exists: {}",
                    output.display()
                )
                .into());
            }
            let history_depth =
                usize_option(learn_args, "--history-depth", DEFAULT_EPISODE_HISTORY_DEPTH)?;
            let shard = NativeEpisodeShard::read(&input)?;
            let view = NativeEpisodeHistoryView::build(&shard, history_depth)?;
            let bytes = view.canonical_bytes()?;
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
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::NativeEpisodeHistory)?;
            let populated_decisions = view
                .decisions
                .iter()
                .filter(|decision| !decision.completed_transition_indices.is_empty())
                .count();
            let maximum_realized_depth = view
                .decisions
                .iter()
                .map(|decision| decision.completed_transition_indices.len())
                .max()
                .unwrap_or(0);
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": view.schema,
                    "view_sha256": view.view_sha256,
                    "native_shard_sha256": view.native_shard_sha256,
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                    "history_depth": view.history_depth,
                    "source_observations": view.source_observation_count,
                    "decisions": view.decisions.len(),
                    "transitions": view.transitions.len(),
                    "decisions_with_history": populated_decisions,
                    "maximum_realized_depth": maximum_realized_depth,
                }))?
            );
            Ok(())
        }
        Some("geometry-view") => {
            let learn_args = &args[1..];
            let input = required_path(learn_args, "--input")?;
            let inventory_paths = repeated_option(learn_args, "--world-inventory");
            if inventory_paths.is_empty() || inventory_paths.len() > MAX_LEARN_INPUT_CORPORA {
                return Err(format!(
                    "learn geometry-view requires 1..={MAX_LEARN_INPUT_CORPORA} --world-inventory INVENTORY.json"
                )
                .into());
            }
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(
                    format!("geometry view output already exists: {}", output.display()).into(),
                );
            }
            let defaults = NativeGeometryViewConfiguration::default();
            let configuration = NativeGeometryViewConfiguration {
                maximum_distance: option(learn_args, "--maximum-distance")
                    .map(|value| value.parse())
                    .transpose()?
                    .unwrap_or(defaults.maximum_distance),
                surface_limit: usize_option(learn_args, "--surface-limit", defaults.surface_limit)?,
            };
            let shard = NativeEpisodeShard::read(&input)?;
            let inventories = inventory_paths
                .iter()
                .map(|path| WorldInventory::read_canonical(Path::new(path)))
                .collect::<Result<Vec<_>, _>>()?;
            let view = NativeEpisodeGeometryView::build(&shard, &inventories, configuration)?;
            let bytes = view.canonical_bytes()?;
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
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::NativeGeometryView)?;
            let present = view
                .observations
                .iter()
                .filter(|observation| observation.status == GeometryObservationStatus::Present)
                .count();
            let player_absent = view
                .observations
                .iter()
                .filter(|observation| observation.status == GeometryObservationStatus::PlayerAbsent)
                .count();
            let room_unavailable = view.observations.len() - present - player_absent;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": view.schema,
                    "view_sha256": view.view_sha256,
                    "native_shard_sha256": view.native_shard_sha256,
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                    "worlds": view.worlds.iter().map(|world| json!({
                        "stage": world.stage,
                        "inventory_sha256": world.inventory_sha256,
                        "spatial_index_sha256": world.spatial_index_sha256,
                        "placements": world.placements.len(),
                    })).collect::<Vec<_>>(),
                    "configuration": view.configuration,
                    "observations": view.observations.len(),
                    "present": present,
                    "player_absent": player_absent,
                    "room_unavailable": room_unavailable,
                    "probes": view.observations.iter()
                        .map(|observation| observation.probes.len()).sum::<usize>(),
                }))?
            );
            Ok(())
        }
        Some("surface-graph-view") => {
            let learn_args = &args[1..];
            let input = required_path(learn_args, "--input")?;
            let inventory_paths = repeated_option(learn_args, "--world-inventory");
            if inventory_paths.is_empty() || inventory_paths.len() > MAX_LEARN_INPUT_CORPORA {
                return Err(format!(
                    "learn surface-graph-view requires 1..={MAX_LEARN_INPUT_CORPORA} --world-inventory INVENTORY.json"
                )
                .into());
            }
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "surface graph view output already exists: {}",
                    output.display()
                )
                .into());
            }
            let defaults = NativeSurfaceGraphViewConfiguration::default();
            let configuration = NativeSurfaceGraphViewConfiguration {
                maximum_hops: option(learn_args, "--maximum-hops")
                    .map(|value| value.parse())
                    .transpose()?
                    .unwrap_or(defaults.maximum_hops),
                maximum_nodes: usize_option(learn_args, "--node-limit", defaults.maximum_nodes)?,
            };
            let geometry = NativeEpisodeGeometryView::decode_canonical(&fs::read(&input)?)?;
            let inventories = inventory_paths
                .iter()
                .map(|path| WorldInventory::read_canonical(Path::new(path)))
                .collect::<Result<Vec<_>, _>>()?;
            let view =
                NativeEpisodeSurfaceGraphView::build(&geometry, &inventories, configuration)?;
            let bytes = view.canonical_bytes()?;
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
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::NativeSurfaceGraphView)?;
            let present = view
                .observations
                .iter()
                .filter(|observation| observation.status == SurfaceGraphObservationStatus::Present)
                .count();
            let no_surface_seed = view
                .observations
                .iter()
                .filter(|observation| {
                    observation.status == SurfaceGraphObservationStatus::NoSurfaceSeed
                })
                .count();
            let player_absent = view
                .observations
                .iter()
                .filter(|observation| {
                    observation.status == SurfaceGraphObservationStatus::PlayerAbsent
                })
                .count();
            let room_unavailable =
                view.observations.len() - present - no_surface_seed - player_absent;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": view.schema,
                    "view_sha256": view.view_sha256,
                    "native_geometry_view_sha256": view.native_geometry_view_sha256,
                    "native_shard_sha256": view.native_shard_sha256,
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                    "worlds": view.worlds,
                    "configuration": view.configuration,
                    "observations": view.observations.len(),
                    "present": present,
                    "no_surface_seed": no_surface_seed,
                    "player_absent": player_absent,
                    "room_unavailable": room_unavailable,
                    "reachable_nodes": view.observations.iter()
                        .filter_map(|observation| observation.neighborhood.as_ref())
                        .map(|report| report.reachable_within_hops).sum::<usize>(),
                    "returned_nodes": view.observations.iter()
                        .filter_map(|observation| observation.neighborhood.as_ref())
                        .map(|report| report.returned_nodes).sum::<usize>(),
                    "truncated_neighborhoods": view.observations.iter()
                        .filter_map(|observation| observation.neighborhood.as_ref())
                        .filter(|report| report.truncated).count(),
                }))?
            );
            Ok(())
        }
        Some("room-load-view") => {
            let learn_args = &args[1..];
            let input = required_path(learn_args, "--input")?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(
                    format!("room-load view output already exists: {}", output.display()).into(),
                );
            }
            let shard = NativeEpisodeShard::read(&input)?;
            let view = NativeEpisodeRoomLoadView::build(&shard)?;
            let bytes = view.canonical_bytes()?;
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
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::NativeRoomLoadView)?;
            let present = view
                .observations
                .iter()
                .filter(|observation| observation.status == RoomLoadSetStatus::Present)
                .count();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": view.schema,
                    "view_sha256": view.view_sha256,
                    "native_shard_sha256": view.native_shard_sha256,
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                    "observations": view.observations.len(),
                    "present": present,
                    "room_rows": view.observations.iter()
                        .filter_map(|observation| observation.load.as_ref())
                        .map(|load| load.rooms.len()).sum::<usize>(),
                    "active_room_rows": view.observations.iter()
                        .filter_map(|observation| observation.load.as_ref())
                        .flat_map(|load| &load.rooms)
                        .filter(|room| room.status_flags != 0).count(),
                    "live_room_scenes": view.observations.iter()
                        .filter_map(|observation| observation.load.as_ref())
                        .flat_map(|load| &load.rooms)
                        .filter(|room| room.scene_status == RoomSceneSetStatus::Present).count(),
                }))?
            );
            Ok(())
        }
        Some("resource-load-view") => {
            let learn_args = &args[1..];
            let input = required_path(learn_args, "--input")?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "resource-load view output already exists: {}",
                    output.display()
                )
                .into());
            }
            let shard = NativeEpisodeShard::read(&input)?;
            let view = NativeEpisodeResourceLoadView::build(&shard)?;
            let bytes = view.canonical_bytes()?;
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
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::NativeResourceLoadView)?;
            let archives = view
                .observations
                .iter()
                .filter_map(|observation| observation.loads.as_ref())
                .flat_map(|loads| &loads.archives)
                .collect::<Vec<_>>();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": view.schema,
                    "view_sha256": view.view_sha256,
                    "native_shard_sha256": view.native_shard_sha256,
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                    "observations": view.observations.len(),
                    "present": view.observations.iter()
                        .filter(|observation| observation.status == ResourceLoadSetStatus::Present)
                        .count(),
                    "archive_rows": archives.len(),
                    "object_rows": archives.iter()
                        .filter(|archive| archive.kind == ResourceArchiveKind::Object).count(),
                    "stage_rows": archives.iter()
                        .filter(|archive| archive.kind == ResourceArchiveKind::Stage).count(),
                    "mounting_rows": archives.iter()
                        .filter(|archive| archive.outcome == ResourceLoadOutcome::Mounting).count(),
                    "ready_rows": archives.iter()
                        .filter(|archive| archive.outcome == ResourceLoadOutcome::Ready).count(),
                    "failed_rows": archives.iter()
                        .filter(|archive| archive.outcome == ResourceLoadOutcome::Failed).count(),
                }))?
            );
            Ok(())
        }
        Some("actor-view") => {
            let learn_args = &args[1..];
            let input = required_path(learn_args, "--input")?;
            let catalog_path = required_path(learn_args, "--actor-profile-catalog")?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(
                    format!("actor view output already exists: {}", output.display()).into(),
                );
            }
            let shard = NativeEpisodeShard::read(&input)?;
            let catalog = ActorProfileCatalog::read_canonical(&catalog_path)?;
            let milestones = option(learn_args, "--milestones");
            let milestone_goal = option(learn_args, "--milestone-goal");
            let view = match (milestones, milestone_goal) {
                (None, None) => NativeEpisodeActorView::build(&shard, &catalog)?,
                (Some(program), Some(goal)) => NativeEpisodeActorView::build_for_goal(
                    &shard,
                    &catalog,
                    &fs::read(program)?,
                    &goal,
                )?,
                _ => {
                    return Err(
                        "learn actor-view requires both --milestones and --milestone-goal".into(),
                    );
                }
            };
            let bytes = view.canonical_bytes()?;
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
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::NativeActorView)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": view.schema,
                    "view_sha256": view.view_sha256,
                    "native_shard_sha256": view.native_shard_sha256,
                    "actor_profile_catalog_identity": view.actor_profile_catalog_identity,
                    "actor_profile_catalog_sha256": view.actor_profile_catalog_sha256,
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                    "observations": view.observations.len(),
                    "actor_nodes": view.observations.iter()
                        .map(|observation| observation.actors.len()).sum::<usize>(),
                    "camera_frames": view.observations.iter()
                        .filter(|observation| observation.camera_frame_present).count(),
                    "player_frames": view.observations.iter()
                        .filter(|observation| observation.player_present).count(),
                    "parent_relations": view.observations.iter()
                        .flat_map(|observation| &observation.actors)
                        .filter(|actor| actor.parent_relative_position.is_some()).count(),
                    "goal": view.goal_graph.as_ref().map(|graph| &graph.definition_name),
                    "goal_anchors": view.goal_graph.as_ref()
                        .map_or(0, |graph| graph.spatial_anchors().len()),
                    "resolved_goal_anchor_observations": view.observations.iter()
                        .flat_map(|observation| &observation.goal_anchors)
                        .filter(|anchor| anchor.absolute_position.is_some()).count(),
                }))?
            );
            Ok(())
        }
        Some("inspect") => {
            let corpus = TransitionCorpus::read_zstd_file(required_path(&args[1..], "--input")?)?;
            let mut action_counts = BTreeMap::<u32, usize>::new();
            let mut terminal_transitions = 0_usize;
            for transition in &corpus.transitions {
                *action_counts
                    .entry(transition.action.action_id)
                    .or_default() += 1;
                terminal_transitions += usize::from(transition.terminal);
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": "dusklight-transition-inspection/v1",
                    "content_digest": corpus.content_digest()?,
                    "feature_schema": corpus.feature_schema,
                    "action_schema": corpus.action_schema,
                    "feature_count": corpus.feature_count,
                    "transitions": corpus.transitions.len(),
                    "terminal_transitions": terminal_transitions,
                    "action_counts": action_counts,
                }))?
            );
            Ok(())
        }
        Some("baseline") => {
            let learn_args = &args[1..];
            let inputs = repeated_option(learn_args, "--input");
            if inputs.is_empty() || inputs.len() > MAX_LEARN_INPUT_CORPORA {
                return Err(format!(
                    "learn baseline requires 1..={MAX_LEARN_INPUT_CORPORA} --input corpora"
                )
                .into());
            }
            let method = option(learn_args, "--method")
                .ok_or("learn baseline requires --method nearest-neighbor|tabular")?;
            let discount = option(learn_args, "--discount")
                .map(|value| value.parse::<f32>())
                .transpose()?
                .unwrap_or(1.0);
            let mut feature_schema = None;
            let mut action_schema = None;
            let mut feature_count = None;
            let mut transitions = Vec::new();
            let mut episode_groups = Vec::new();
            let mut next_episode_group = 0_u64;
            for input in &inputs {
                let corpus = TransitionCorpus::read_zstd_file(input)?;
                if feature_schema.is_some_and(|value| value != corpus.feature_schema)
                    || action_schema.is_some_and(|value| value != corpus.action_schema)
                    || feature_count.is_some_and(|value| value != corpus.feature_count)
                {
                    return Err("baseline corpora use incompatible schemas".into());
                }
                feature_schema = Some(corpus.feature_schema);
                action_schema = Some(corpus.action_schema);
                feature_count = Some(corpus.feature_count);
                let mut ended_terminal = false;
                for transition in corpus.transitions {
                    let terminal = transition.terminal;
                    transitions.push(FqiTransition {
                        state: transition.state,
                        action: transition.action.action_id,
                        duration: transition.duration_ticks,
                        reward: transition.reward,
                        next_state: transition.next_state,
                        terminal,
                    });
                    episode_groups.push(next_episode_group);
                    ended_terminal = terminal;
                    if terminal {
                        next_episode_group = next_episode_group
                            .checked_add(1)
                            .ok_or("baseline episode-group count overflowed")?;
                    }
                }
                if !ended_terminal {
                    next_episode_group = next_episode_group
                        .checked_add(1)
                        .ok_or("baseline episode-group count overflowed")?;
                }
            }
            let query_index = usize_option(learn_args, "--query-transition", 0)?;
            let query = transitions
                .get(query_index)
                .ok_or("--query-transition is outside the merged transition batch")?;
            let query_side = option(learn_args, "--query-side").unwrap_or_else(|| "state".into());
            let query_state = match query_side.as_str() {
                "state" => &query.state,
                "next-state" => &query.next_state,
                _ => return Err("--query-side must be state or next-state".into()),
            };
            let samples = empirical_return_samples(&transitions, &episode_groups, discount)?;
            let (ranking, configuration) = match method.as_str() {
                "nearest-neighbor" => {
                    let declared = repeated_option(learn_args, "--feature");
                    let categorical = if feature_schema == Some(movement_feature_schema_digest_v1())
                    {
                        MOVEMENT_CATEGORICAL_FEATURES_V1.to_vec()
                    } else if feature_schema == Some(movement_state_v2_spec().digest()?) {
                        movement_state_v2_spec().categorical_features()
                    } else {
                        Vec::new()
                    };
                    let features = if declared.is_empty() {
                        if categorical.is_empty() {
                            return Err("unknown schema requires repeated --feature INDEX:SCALE:continuous|categorical".into());
                        }
                        (0..feature_count.unwrap() as usize)
                            .map(|index| LocalFeature {
                                index,
                                scale: 1.0,
                                categorical: categorical.contains(&index),
                            })
                            .collect::<Vec<_>>()
                    } else {
                        declared
                            .iter()
                            .map(|value| -> Result<LocalFeature, Box<dyn Error>> {
                                let parts = value.split(':').collect::<Vec<_>>();
                                if parts.len() != 3
                                    || !matches!(parts[2], "continuous" | "categorical")
                                {
                                    return Err(
                                        "--feature syntax is INDEX:SCALE:continuous|categorical"
                                            .into(),
                                    );
                                }
                                Ok(LocalFeature {
                                    index: parts[0].parse()?,
                                    scale: parts[1].parse()?,
                                    categorical: parts[2] == "categorical",
                                })
                            })
                            .collect::<Result<Vec<_>, _>>()?
                    };
                    let neighbors = usize_option(learn_args, "--neighbors", 8)?;
                    let model = NearestNeighborReturn::fit(
                        samples,
                        LocalReturnConfig {
                            neighbors,
                            features: features.clone(),
                        },
                    )?;
                    (
                        model.rank(query_state)?,
                        json!({
                            "neighbors": neighbors,
                            "features": features.iter().map(|feature| json!({
                                "index": feature.index,
                                "scale": feature.scale,
                                "categorical": feature.categorical,
                            })).collect::<Vec<_>>(),
                        }),
                    )
                }
                "tabular" => {
                    let axes = repeated_option(learn_args, "--axis")
                        .iter()
                        .map(|value| -> Result<TabularAxis, Box<dyn Error>> {
                            let parts = value.split(':').collect::<Vec<_>>();
                            if parts.len() != 3 {
                                return Err("--axis syntax is INDEX:ORIGIN:WIDTH".into());
                            }
                            Ok(TabularAxis {
                                index: parts[0].parse()?,
                                origin: parts[1].parse()?,
                                width: parts[2].parse()?,
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let model = TabularReturn::fit(&samples, axes.clone())?;
                    (
                        model.rank(query_state)?,
                        json!({
                            "axes": axes.iter().map(|axis| json!({
                                "index": axis.index,
                                "origin": axis.origin,
                                "width": axis.width,
                            })).collect::<Vec<_>>(),
                        }),
                    )
                }
                _ => return Err("--method must be nearest-neighbor or tabular".into()),
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": "dusklight-low-data-baseline/v1",
                    "method": method,
                    "feature_schema": feature_schema,
                    "action_schema": action_schema,
                    "input_corpora": inputs,
                    "episode_groups": episode_groups.iter().copied().collect::<BTreeSet<_>>().len(),
                    "transitions": transitions.len(),
                    "per_tick_discount": discount,
                    "query_transition": query_index,
                    "query_side": query_side,
                    "configuration": configuration,
                    "ranking": ranking,
                    "limitations": [
                        "empirical observed returns only; no counterfactual inference",
                        "a nonterminal episode end is truncated and receives no cross-episode bootstrap",
                        "rankings are proposal heuristics and require native rollout proof"
                    ]
                }))?
            );
            Ok(())
        }
        Some("calibrate") => {
            let learn_args = &args[1..];
            let dataset_path = option(learn_args, "--dataset").map(PathBuf::from);
            let explicit_training = repeated_option(learn_args, "--training");
            let explicit_held_out = repeated_option(learn_args, "--held-out");
            if dataset_path.is_some()
                == (!explicit_training.is_empty() || !explicit_held_out.is_empty())
            {
                return Err("learn calibrate requires either --dataset or both --training/--held-out corpora".into());
            }
            let mut dataset_identity = None;
            let mut held_out_split = None;
            let mut expected_dataset_corpus_digests = None;
            let (training_paths, held_out_paths) = if let Some(path) = &dataset_path {
                let manifest: DatasetManifest = serde_json::from_slice(&fs::read(path)?)?;
                manifest.validate()?;
                let split = match option(learn_args, "--split")
                    .unwrap_or_else(|| "test".into())
                    .as_str()
                {
                    "validation" => DatasetSplit::Validation,
                    "test" => DatasetSplit::Test,
                    "withheld" => DatasetSplit::Withheld,
                    _ => return Err("--split must be validation, test, or withheld".into()),
                };
                let training = manifest
                    .entries
                    .iter()
                    .filter(|entry| entry.split == DatasetSplit::Train)
                    .map(|entry| entry.transition_corpus.to_string_lossy().into_owned())
                    .collect::<Vec<_>>();
                let held_out = manifest
                    .entries
                    .iter()
                    .filter(|entry| entry.split == split)
                    .map(|entry| entry.transition_corpus.to_string_lossy().into_owned())
                    .collect::<Vec<_>>();
                expected_dataset_corpus_digests = Some((
                    manifest
                        .entries
                        .iter()
                        .filter(|entry| entry.split == DatasetSplit::Train)
                        .map(|entry| entry.corpus_sha256)
                        .collect::<Vec<_>>(),
                    manifest
                        .entries
                        .iter()
                        .filter(|entry| entry.split == split)
                        .map(|entry| entry.corpus_sha256)
                        .collect::<Vec<_>>(),
                ));
                dataset_identity = Some(manifest.dataset_sha256);
                held_out_split = Some(split);
                (training, held_out)
            } else {
                if explicit_training.is_empty() || explicit_held_out.is_empty() {
                    return Err(
                        "explicit calibration requires both --training and --held-out".into(),
                    );
                }
                (explicit_training, explicit_held_out)
            };
            let training_files = training_paths
                .iter()
                .map(fs::canonicalize)
                .collect::<Result<BTreeSet<_>, _>>()?;
            let held_out_files = held_out_paths
                .iter()
                .map(fs::canonicalize)
                .collect::<Result<BTreeSet<_>, _>>()?;
            if !training_files.is_disjoint(&held_out_files) {
                return Err("training and held-out calibration files overlap".into());
            }
            let training = load_fqi_batch(
                &training_paths,
                "calibration training",
                MAX_LEARN_INPUT_CORPORA,
            )?;
            let held_out = load_fqi_batch(
                &held_out_paths,
                "calibration held-out",
                MAX_LEARN_INPUT_CORPORA,
            )?;
            if expected_dataset_corpus_digests.as_ref().is_some_and(
                |(expected_training, expected_held_out)| {
                    expected_training != &training.corpus_digests
                        || expected_held_out != &held_out.corpus_digests
                },
            ) {
                return Err("calibration corpus content differs from dataset manifest".into());
            }
            if training.feature_schema != held_out.feature_schema
                || training.action_schema != held_out.action_schema
                || training.feature_count != held_out.feature_count
                || !training
                    .corpus_digests
                    .iter()
                    .all(|digest| !held_out.corpus_digests.contains(digest))
            {
                return Err(
                    "calibration requires compatible schemas and content-disjoint held-out corpora"
                        .into(),
                );
            }
            let mut config = FqiConfig {
                iterations: usize_option(learn_args, "--iterations", 24)?,
                backup_steps: usize_option(learn_args, "--n-step", 1)?,
                trees_per_action: usize_option(learn_args, "--trees", 31)?,
                max_tree_depth: usize_option(learn_args, "--max-depth", 8)?,
                seed: u64_option(learn_args, "--seed", FqiConfig::default().seed)?,
                discount: option(learn_args, "--discount")
                    .map(|value| value.parse::<f32>())
                    .transpose()?
                    .unwrap_or(FqiConfig::default().discount),
                ..FqiConfig::default()
            };
            if config.iterations == 0
                || config.iterations > MAX_FQI_ITERATIONS
                || config.backup_steps == 0
                || config.backup_steps > MAX_FQI_BACKUP_STEPS
                || config.trees_per_action == 0
                || config.trees_per_action > MAX_FQI_TREES_PER_ACTION
                || config.max_tree_depth > MAX_FQI_TREE_DEPTH
            {
                return Err("invalid bounded calibration FQI configuration".into());
            }
            let declared_categorical = repeated_option(learn_args, "--categorical-feature")
                .into_iter()
                .map(|value| value.parse::<usize>())
                .collect::<Result<Vec<_>, _>>()?;
            let declared_all_continuous = learn_args.iter().any(|arg| arg == "--all-continuous");
            if declared_all_continuous && !declared_categorical.is_empty() {
                return Err(
                    "--all-continuous and --categorical-feature cannot be used together".into(),
                );
            }
            if training.feature_schema == movement_feature_schema_digest_v1() {
                if declared_all_continuous || !declared_categorical.is_empty() {
                    return Err(
                        "the authenticated movement schema owns its categorical feature map".into(),
                    );
                }
                config.categorical_features = MOVEMENT_CATEGORICAL_FEATURES_V1.to_vec();
            } else if training.feature_schema == movement_state_v2_spec().digest()? {
                if declared_all_continuous || !declared_categorical.is_empty() {
                    return Err(
                        "the authenticated movement schema owns its categorical feature map".into(),
                    );
                }
                config.categorical_features = movement_state_v2_spec().categorical_features();
            } else if declared_all_continuous {
                config.categorical_features.clear();
            } else if !declared_categorical.is_empty() {
                config.categorical_features = declared_categorical;
            } else {
                return Err("unknown feature schema: declare --all-continuous or repeat --categorical-feature N".into());
            }
            let actions = training
                .transitions
                .iter()
                .map(|transition| transition.action)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            if actions.is_empty() || actions.len() > MAX_FQI_ACTIONS {
                return Err("calibration training action support is outside bounds".into());
            }
            let model = FittedQ::fit_with_episode_groups(
                training.feature_count,
                &actions,
                &training.transitions,
                &training.episode_groups,
                &config,
            )?;
            let held_out_samples = empirical_return_samples(
                &held_out.transitions,
                &held_out.episode_groups,
                config.discount,
            )?;
            let calibration = calibrate_fitted_q(&model, &held_out_samples)?;
            let output_path = required_path(learn_args, "--output")?;
            if output_path.exists() {
                return Err(format!(
                    "calibration output already exists: {}",
                    output_path.display()
                )
                .into());
            }
            let report = json!({
                "schema": "dusklight-held-out-fqi-calibration/v1",
                "dataset": dataset_path,
                "dataset_sha256": dataset_identity,
                "held_out_split": held_out_split,
                "training_corpora": training_paths,
                "training_corpus_sha256": training.corpus_digests,
                "held_out_corpora": held_out_paths,
                "held_out_corpus_sha256": held_out.corpus_digests,
                "feature_schema": training.feature_schema,
                "action_schema": training.action_schema,
                "training_episode_groups": training.episode_groups.iter().copied().collect::<BTreeSet<_>>().len(),
                "held_out_episode_groups": held_out.episode_groups.iter().copied().collect::<BTreeSet<_>>().len(),
                "config": config,
                "calibration": calibration,
                "promotion_authority": false,
                "limitations": [
                    "exact-state proposal win rate is measured only where held-out actions are comparable",
                    "unsupported held-out actions and proposed actions remain explicit OOD diagnostics",
                    "calibration is analysis evidence and cannot replace native predicate or cold replay proof"
                ]
            });
            if let Some(parent) = output_path
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output_path, serde_json::to_vec_pretty(&report)?)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("double-q") => {
            let learn_args = &args[1..];
            let direct_inputs = repeated_option(learn_args, "--input");
            let dataset_path = option(learn_args, "--dataset").map(PathBuf::from);
            if dataset_path.is_some() && !direct_inputs.is_empty() {
                return Err("learn double-q accepts either --dataset or --input, not both".into());
            }
            let dataset_manifest: Option<DatasetManifest> = dataset_path
                .as_ref()
                .map(|path| -> Result<_, Box<dyn Error>> {
                    let manifest: DatasetManifest = serde_json::from_slice(&fs::read(path)?)?;
                    manifest.validate()?;
                    Ok(manifest)
                })
                .transpose()?;
            let inputs = if let Some(manifest) = &dataset_manifest {
                manifest
                    .entries
                    .iter()
                    .filter(|entry| entry.split == DatasetSplit::Train)
                    .map(|entry| entry.transition_corpus.to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
            } else {
                direct_inputs
            };
            let training = load_fqi_batch(&inputs, "Double-Q training", MAX_LEARN_INPUT_CORPORA)?;
            let expected_corpus_digests = dataset_manifest.as_ref().map(|manifest| {
                manifest
                    .entries
                    .iter()
                    .filter(|entry| entry.split == DatasetSplit::Train)
                    .map(|entry| entry.corpus_sha256)
                    .collect::<Vec<_>>()
            });
            if expected_corpus_digests
                .as_ref()
                .is_some_and(|expected| expected != &training.corpus_digests)
            {
                return Err("Double-Q corpus content differs from dataset manifest".into());
            }
            let config = DoubleQConfig {
                epochs: usize_option(learn_args, "--epochs", DoubleQConfig::default().epochs)?,
                hidden_width: usize_option(
                    learn_args,
                    "--hidden-width",
                    DoubleQConfig::default().hidden_width,
                )?,
                learning_rate: option(learn_args, "--learning-rate")
                    .map(|value| value.parse::<f64>())
                    .transpose()?
                    .unwrap_or(DoubleQConfig::default().learning_rate),
                discount: option(learn_args, "--discount")
                    .map(|value| value.parse::<f64>())
                    .transpose()?
                    .unwrap_or(DoubleQConfig::default().discount),
                target_sync_steps: usize_option(
                    learn_args,
                    "--target-sync-steps",
                    DoubleQConfig::default().target_sync_steps,
                )?,
                gradient_clip: option(learn_args, "--gradient-clip")
                    .map(|value| value.parse::<f64>())
                    .transpose()?
                    .unwrap_or(DoubleQConfig::default().gradient_clip),
                seed: u64_option(learn_args, "--seed", DoubleQConfig::default().seed)?,
            };
            let action_support = training.transitions.iter().fold(
                BTreeMap::<u32, usize>::new(),
                |mut counts, transition| {
                    *counts.entry(transition.action).or_default() += 1;
                    counts
                },
            );
            if action_support.len() > MAX_FQI_ACTIONS {
                return Err(format!(
                    "Double-Q supports at most {MAX_FQI_ACTIONS} distinct actions; received {}",
                    action_support.len()
                )
                .into());
            }
            let actions = action_support.keys().copied().collect::<Vec<_>>();
            let model = DoubleQ::fit(
                training.feature_count,
                &actions,
                &training.transitions,
                &config,
            )?;
            let query_index = usize_option(learn_args, "--query-transition", 0)?;
            let query_transition = training
                .transitions
                .get(query_index)
                .ok_or("--query-transition is outside the merged transition batch")?;
            let query_side = option(learn_args, "--query-side").unwrap_or_else(|| "state".into());
            let query_state = match query_side.as_str() {
                "state" => &query_transition.state,
                "next-state" => &query_transition.next_state,
                _ => return Err("--query-side must be state or next-state".into()),
            };
            let ranking = model
                .rank_actions(query_state)?
                .into_iter()
                .map(|estimate| {
                    json!({
                        "action": estimate.action,
                        "mean_q": estimate.mean,
                        "critic_a": estimate.critic_a,
                        "critic_b": estimate.critic_b,
                        "critic_disagreement": estimate.critic_disagreement,
                        "support": action_support[&estimate.action],
                    })
                })
                .collect::<Vec<_>>();
            let model_output = option(learn_args, "--model-output").map(PathBuf::from);
            let mut model_content_blob = None;
            let mut model_artifact_store = None;
            if let Some(path) = &model_output {
                if path.exists() {
                    return Err(format!(
                        "Double-Q model output already exists: {}",
                        path.display()
                    )
                    .into());
                }
                if let Some(parent) = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    fs::create_dir_all(parent)?;
                }
                let bytes = model.artifact_bytes(
                    training.feature_schema,
                    training.action_schema,
                    dataset_manifest
                        .as_ref()
                        .map(|manifest| manifest.dataset_sha256),
                    &training.corpus_digests,
                    &config,
                )?;
                fs::write(path, &bytes)?;
                let store_path = option(learn_args, "--artifact-store")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| path.parent().unwrap_or(Path::new(".")).join("content"));
                model_content_blob = Some(
                    ContentStore::initialize(&store_path)?.put_bytes(&bytes, ContentKind::Model)?,
                );
                model_artifact_store = Some(store_path);
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": "dusklight-double-q-ranking/v1",
                    "feature_schema": training.feature_schema,
                    "action_schema": training.action_schema,
                    "input_corpora": inputs,
                    "training_corpus_sha256": training.corpus_digests,
                    "training_dataset": dataset_path,
                    "training_dataset_sha256": dataset_manifest.as_ref().map(|manifest| manifest.dataset_sha256),
                    "transition_count": training.transitions.len(),
                    "episode_groups": training.episode_groups.iter().copied().collect::<BTreeSet<_>>().len(),
                    "query_transition": query_index,
                    "query_side": query_side,
                    "config": config,
                    "gradient_updates": model.gradient_updates(),
                    "target_synchronizations": model.target_synchronizations(),
                    "target_evaluation": "online_selects_opposite_frozen_target_evaluates",
                    "sample_order": "deterministic_seeded_epoch_shuffle",
                    "model_output": model_output,
                    "model_artifact_store": model_artifact_store,
                    "model_content_blob": model_content_blob,
                    "ranking": ranking,
                    "promotion_authority": false,
                    "limitations": [
                        "offline Double-Q can overvalue actions outside dataset support; use support diagnostics and the conservative learner",
                        "numeric normalization does not provide categorical embeddings or missingness masks",
                        "critic disagreement is not calibrated uncertainty",
                        "rankings are proposals and require native predicate and cold replay proof"
                    ]
                }))?
            );
            Ok(())
        }
        Some("fit") => {
            let learn_args = &args[1..];
            let direct_inputs = repeated_option(learn_args, "--input");
            let dataset_path = option(learn_args, "--dataset").map(PathBuf::from);
            if dataset_path.is_some() && !direct_inputs.is_empty() {
                return Err("learn fit accepts either --dataset or --input, not both".into());
            }
            let dataset_manifest: Option<DatasetManifest> = dataset_path
                .as_ref()
                .map(|path| -> Result<_, Box<dyn Error>> {
                    let manifest: DatasetManifest = serde_json::from_slice(&fs::read(path)?)?;
                    manifest.validate()?;
                    Ok(manifest)
                })
                .transpose()?;
            let inputs = if let Some(manifest) = &dataset_manifest {
                manifest
                    .entries
                    .iter()
                    .filter(|entry| entry.split == huntctl::dataset::DatasetSplit::Train)
                    .map(|entry| entry.transition_corpus.to_string_lossy().into_owned())
                    .collect()
            } else {
                direct_inputs
            };
            if inputs.is_empty() {
                return Err(
                    "learn fit requires training entries in --dataset or at least one --input FILE"
                        .into(),
                );
            }
            if inputs.len() > MAX_LEARN_INPUT_CORPORA {
                return Err(format!(
                    "learn fit accepts at most {MAX_LEARN_INPUT_CORPORA} input corpora; received {}",
                    inputs.len()
                )
                .into());
            }
            let mut config = FqiConfig {
                iterations: usize_option(learn_args, "--iterations", 24)?,
                backup_steps: usize_option(learn_args, "--n-step", 1)?,
                trees_per_action: usize_option(learn_args, "--trees", 31)?,
                max_tree_depth: usize_option(learn_args, "--max-depth", 8)?,
                seed: u64_option(learn_args, "--seed", FqiConfig::default().seed)?,
                discount: option(learn_args, "--discount")
                    .map(|value| value.parse::<f32>())
                    .transpose()?
                    .unwrap_or(FqiConfig::default().discount),
                ..FqiConfig::default()
            };
            if config.iterations > MAX_FQI_ITERATIONS {
                return Err(format!(
                    "--iterations must not exceed {MAX_FQI_ITERATIONS}; received {}",
                    config.iterations
                )
                .into());
            }
            if config.backup_steps == 0 || config.backup_steps > MAX_FQI_BACKUP_STEPS {
                return Err(format!(
                    "--n-step must be within 1..={MAX_FQI_BACKUP_STEPS}; received {}",
                    config.backup_steps
                )
                .into());
            }
            if config.trees_per_action > MAX_FQI_TREES_PER_ACTION {
                return Err(format!(
                    "--trees must not exceed {MAX_FQI_TREES_PER_ACTION}; received {}",
                    config.trees_per_action
                )
                .into());
            }
            if config.max_tree_depth > MAX_FQI_TREE_DEPTH {
                return Err(format!(
                    "--max-depth must not exceed {MAX_FQI_TREE_DEPTH}; received {}",
                    config.max_tree_depth
                )
                .into());
            }
            let mut feature_schema = None;
            let mut action_schema = None;
            let mut feature_count = None;
            let mut transitions = Vec::new();
            let mut episode_groups = Vec::new();
            let mut next_episode_group = 0_u64;
            let mut training_corpus_sha256 = Vec::new();
            let mut action_support = BTreeMap::<u32, usize>::new();
            let shaping_path = option(learn_args, "--shaping").map(PathBuf::from);
            let shaping_report_path = option(learn_args, "--shaping-report").map(PathBuf::from);
            if shaping_path.is_some() != shaping_report_path.is_some() {
                return Err(
                    "--shaping SPEC.json and --shaping-report REPORT.json must be supplied together"
                        .into(),
                );
            }
            let shaping_spec: Option<PotentialShapingSpec> = if let Some(path) = &shaping_path {
                Some(serde_json::from_slice(&fs::read(path)?)?)
            } else {
                None
            };
            let mut shaping_records = Vec::new();
            for input in &inputs {
                let corpus = TransitionCorpus::read_zstd_file(input)?;
                training_corpus_sha256.push(corpus.content_digest()?);
                if feature_schema.is_some_and(|value| value != corpus.feature_schema)
                    || action_schema.is_some_and(|value| value != corpus.action_schema)
                    || feature_count.is_some_and(|value| value != corpus.feature_count)
                {
                    return Err(
                        "transition corpora use incompatible feature or action schemas".into(),
                    );
                }
                feature_schema = Some(corpus.feature_schema);
                action_schema = Some(corpus.action_schema);
                feature_count = Some(corpus.feature_count);
                if let Some(spec) = &shaping_spec {
                    if spec.feature_schema != corpus.feature_schema {
                        return Err(format!(
                            "shaping feature schema {} does not match corpus feature schema {}",
                            spec.feature_schema, corpus.feature_schema
                        )
                        .into());
                    }
                    spec.validate(corpus.feature_count as usize)?;
                }
                let merged_count = transitions
                    .len()
                    .checked_add(corpus.transitions.len())
                    .ok_or("learn fit merged transition count overflow")?;
                if merged_count > MAX_FQI_TRANSITIONS {
                    return Err(format!(
                        "learn fit accepts at most {MAX_FQI_TRANSITIONS} merged transitions; received at least {merged_count}"
                    )
                    .into());
                }
                transitions.reserve(corpus.transitions.len());
                let mut ended_terminal = false;
                for (transition_index, transition) in corpus.transitions.into_iter().enumerate() {
                    let action = transition.action.action_id;
                    let terminal = transition.terminal;
                    if !action_support.contains_key(&action)
                        && action_support.len() >= MAX_FQI_ACTIONS
                    {
                        return Err(format!(
                            "learn fit accepts at most {MAX_FQI_ACTIONS} distinct actions; encountered action {action} after reaching the limit"
                        )
                        .into());
                    }
                    *action_support.entry(action).or_default() += 1;
                    let reward = if let Some(spec) = &shaping_spec {
                        let breakdown = spec.shape_reward(
                            corpus.feature_count as usize,
                            &transition.state,
                            &transition.next_state,
                            transition.reward,
                            transition.duration_ticks,
                            terminal,
                            config.discount,
                        )?;
                        let training_reward = breakdown.training_reward;
                        shaping_records.push(json!({
                            "input_corpus": input,
                            "transition": transition_index,
                            "source_reference": transition.source.digest,
                            "next_reference": transition.next.digest,
                            "reward": breakdown,
                        }));
                        training_reward
                    } else {
                        transition.reward
                    };
                    transitions.push(FqiTransition {
                        state: transition.state,
                        action,
                        duration: transition.duration_ticks,
                        reward,
                        next_state: transition.next_state,
                        terminal,
                    });
                    episode_groups.push(next_episode_group);
                    ended_terminal = terminal;
                    if terminal {
                        next_episode_group = next_episode_group
                            .checked_add(1)
                            .ok_or("learn fit episode-group count overflowed")?;
                    }
                }
                if !ended_terminal {
                    next_episode_group = next_episode_group
                        .checked_add(1)
                        .ok_or("learn fit episode-group count overflowed")?;
                }
            }
            let declared_categorical = repeated_option(learn_args, "--categorical-feature")
                .into_iter()
                .map(|value| value.parse::<usize>())
                .collect::<Result<Vec<_>, _>>()?;
            let declared_all_continuous = learn_args.iter().any(|arg| arg == "--all-continuous");
            if declared_all_continuous && !declared_categorical.is_empty() {
                return Err(
                    "--all-continuous and --categorical-feature cannot be used together".into(),
                );
            }
            if feature_schema == Some(movement_feature_schema_digest_v1()) {
                if declared_all_continuous || !declared_categorical.is_empty() {
                    return Err(
                        "the authenticated movement schema owns its categorical feature map; do not override it"
                            .into(),
                    );
                }
                config.categorical_features = MOVEMENT_CATEGORICAL_FEATURES_V1.to_vec();
            } else if feature_schema == Some(movement_state_v2_spec().digest()?) {
                if declared_all_continuous || !declared_categorical.is_empty() {
                    return Err(
                        "the authenticated movement schema owns its categorical feature map; do not override it"
                            .into(),
                    );
                }
                config.categorical_features = movement_state_v2_spec().categorical_features();
            } else if declared_all_continuous {
                config.categorical_features.clear();
            } else if !declared_categorical.is_empty() {
                config.categorical_features = declared_categorical;
            } else {
                return Err(
                    "unknown feature schema: declare --all-continuous or repeat --categorical-feature N"
                        .into(),
                );
            }
            let actions: Vec<u32> = action_support.keys().copied().collect();
            let query_index = usize_option(learn_args, "--query-transition", 0)?;
            let query_transition = transitions
                .get(query_index)
                .ok_or("--query-transition is outside the merged transition batch")?;
            let query_side = option(learn_args, "--query-side").unwrap_or_else(|| "state".into());
            let query_state = match query_side.as_str() {
                "state" => query_transition.state.clone(),
                "next-state" => query_transition.next_state.clone(),
                _ => return Err("--query-side must be state or next-state".into()),
            };
            let learned_feature_count =
                feature_count.ok_or("transition corpus has no feature width")? as usize;
            let shaping_identity = shaping_spec
                .as_ref()
                .map(|spec| spec.identity(learned_feature_count))
                .transpose()?;
            if let (Some(spec), Some(path)) = (&shaping_spec, &shaping_report_path) {
                if path.exists() {
                    return Err(format!(
                        "shaping reward report already exists: {}",
                        path.display()
                    )
                    .into());
                }
                if let Some(parent) = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    fs::create_dir_all(parent)?;
                }
                fs::write(
                    path,
                    serde_json::to_vec_pretty(&json!({
                        "schema": REWARD_REPORT_SCHEMA_V1,
                        "shaping_identity": shaping_identity,
                        "shaping_spec": spec,
                        "feature_schema": feature_schema,
                        "action_schema": action_schema,
                        "per_tick_discount": config.discount,
                        "proposal_signal_only": true,
                        "terminal_objective": "unchanged_external_predicate",
                        "input_corpora": &inputs,
                        "transitions": shaping_records,
                    }))?,
                )?;
            }
            let model = FittedQ::fit_with_episode_groups(
                learned_feature_count,
                &actions,
                &transitions,
                &episode_groups,
                &config,
            )?;
            let model_output = option(learn_args, "--model-output").map(PathBuf::from);
            let mut model_content_blob = None;
            let mut model_artifact_store = None;
            if let Some(path) = &model_output {
                if path.exists() {
                    return Err(format!("model output already exists: {}", path.display()).into());
                }
                if let Some(parent) = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    fs::create_dir_all(parent)?;
                }
                let bytes = model.artifact_bytes(
                    feature_schema.ok_or("transition corpus has no feature schema")?,
                    action_schema.ok_or("transition corpus has no action schema")?,
                    dataset_manifest
                        .as_ref()
                        .map(|manifest| manifest.dataset_sha256),
                    &training_corpus_sha256,
                    &config,
                )?;
                fs::write(path, &bytes)?;
                let store_path = option(learn_args, "--artifact-store")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| path.parent().unwrap_or(Path::new(".")).join("content"));
                model_content_blob = Some(
                    ContentStore::initialize(&store_path)?.put_bytes(&bytes, ContentKind::Model)?,
                );
                model_artifact_store = Some(store_path);
            }
            let ranking: Vec<_> = model
                .rank_actions(&query_state)?
                .into_iter()
                .map(|estimate| {
                    json!({
                        "action": estimate.action,
                        "mean_q": estimate.mean,
                        "ensemble_variance": estimate.variance,
                        "support": action_support[&estimate.action],
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": "dusklight-fitted-q-ranking/v1",
                    "feature_schema": feature_schema,
                    "action_schema": action_schema,
                    "input_corpora": inputs,
                    "training_dataset": dataset_path,
                    "training_dataset_sha256": dataset_manifest.as_ref().map(|manifest| manifest.dataset_sha256),
                    "transition_count": transitions.len(),
                    "episode_groups": episode_groups.iter().copied().collect::<BTreeSet<_>>().len(),
                    "bootstrap_unit": model.bootstrap_unit(),
                    "query_transition": query_index,
                    "query_side": query_side,
                    "per_tick_discount": config.discount,
                    "potential_shaping": shaping_identity,
                    "reward_report": shaping_report_path,
                    "model_output": model_output,
                    "model_artifact_store": model_artifact_store,
                    "model_content_blob": model_content_blob,
                    "iterations": config.iterations,
                    "backup_steps": config.backup_steps,
                    "trees_per_action": config.trees_per_action,
                    "categorical_features": config.categorical_features,
                    "seed": config.seed,
                    "ranking": ranking,
                }))?
            );
            Ok(())
        }
        Some("benchmark") => {
            const ADVANCE: u32 = 3;
            const WAIT: u32 = 9;
            let mut transitions = Vec::new();
            for nuisance in [-1.0, 1.0] {
                transitions.extend([
                    FqiTransition {
                        state: vec![0.0, nuisance],
                        action: ADVANCE,
                        duration: 1,
                        reward: 0.0,
                        next_state: vec![1.0, nuisance],
                        terminal: false,
                    },
                    FqiTransition {
                        state: vec![0.0, nuisance],
                        action: WAIT,
                        duration: 1,
                        reward: -1.0,
                        next_state: vec![0.0, nuisance],
                        terminal: false,
                    },
                    FqiTransition {
                        state: vec![1.0, nuisance],
                        action: ADVANCE,
                        duration: 1,
                        reward: 10.0,
                        next_state: vec![2.0, nuisance],
                        terminal: true,
                    },
                    FqiTransition {
                        state: vec![1.0, nuisance],
                        action: WAIT,
                        duration: 1,
                        reward: -1.0,
                        next_state: vec![1.0, nuisance],
                        terminal: false,
                    },
                ]);
            }
            let config = FqiConfig {
                iterations: 16,
                trees_per_action: 7,
                max_tree_depth: 3,
                features_per_split: 2,
                discount: 0.9,
                bootstrap: false,
                ..FqiConfig::default()
            };
            let model = FittedQ::fit(2, &[WAIT, ADVANCE], &transitions, &config)?;
            let held_out = [[0.0, 0.0], [1.0, 0.0]];
            let selected: Vec<u32> = held_out
                .iter()
                .map(|state| model.best_action(state).map(|estimate| estimate.action))
                .collect::<Result<_, _>>()?;
            let passed = selected == [ADVANCE, ADVANCE];
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": "dusklight-fqi-benchmark/v1",
                    "training_transitions": transitions.len(),
                    "held_out_states": held_out,
                    "selected_actions": selected,
                    "expected_actions": [ADVANCE, ADVANCE],
                    "passed": passed,
                }))?
            );
            if !passed {
                return Err("fitted-Q benchmark failed its fixed acceptance threshold".into());
            }
            Ok(())
        }
        _ => usage_error(),
    }
}
