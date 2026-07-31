//! Command dispatch and artifact workflows for the core learning baselines.

use super::MAX_LEARN_INPUT_CORPORA;
use crate::cli;
use crate::{flag, option, repeated_option, required_path, u64_option, usage_error, usize_option};
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
use huntctl::learning::tactic_exploration::TacticProposalPolicy;
use huntctl::learning::tactic_features::GoalConditionedTacticFeatureEncoder;
use huntctl::learning::tactic_frozen_policy::TacticFrozenPolicy;
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
use huntctl::search_evaluator::generalized_tactic_evidence::prove_generalized_tactic_held_out_value;
use huntctl::search_evaluator::native_residual_campaign::NativeResidualExecutionBinding;
use huntctl::search_evaluator::native_tactic_policy_runner::{
    NativeTacticPolicyRunConfig, run_native_tactic_policy,
};
use huntctl::search_evaluator::native_tactic_route_runner::{
    NativeTacticCampaignSummary, NativeTacticColdReplayConfig,
    NativeTacticColdReplayEvidenceBundle, NativeTacticDemonstrationReport,
    NativeTacticExecutionPlan, NativeTacticExecutionPlanRequest, NativeTacticFaultInjector,
    NativeTacticFaultRecoveryEvidenceBundle, NativeTacticLaunchSmokeBundle,
    NativeTacticObservationAudit, NativeTacticPlanBudgets, NativeTacticPostTerminalControlReport,
    NativeTacticResourceLimit, NativeTacticRestoreLocalityConfig,
    NativeTacticRestoreLocalityReport, NativeTacticRouteDiagnosisReport, NativeTacticRouteReport,
    NativeTacticRouteRunConfig, NativeTacticScratchCampaignAudit,
    NativeTacticScratchComparisonReport, NativeTacticScratchDiscoveryReport,
    NativeTacticScratchEvidenceBundle, NativeTacticThroughputCurveConfig,
    NativeTacticThroughputCurveRun, NativeTacticThroughputEvidenceBundle,
    NativeTacticThroughputTreatmentBundle, audit_native_tactic_fault_recovery,
    read_and_validate_native_tactic_cold_replay, run_native_tactic_cold_replay,
    run_native_tactic_restore_locality, run_native_tactic_route,
    run_native_tactic_throughput_curve_controlled, tactic_macro_registry_identity,
};
use huntctl::search_evaluator::native_tactic_worker::NativeGenericExecutionStrategy;
use huntctl::search_evaluator::optimization_request::OptimizationRequest;
use huntctl::search_evaluator::tactic_q_campaign::{
    TacticQCampaign, TacticQFinalResult, TacticQTrainingCorpus,
};
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
use sha2::Sha256;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

mod route_plan;
use route_plan::{native_tactic_execution_plan, sealed_plan_shape_conflict};

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

mod baselines;
mod corpora;
mod frozen_and_tactics;
mod native_views;
mod q_training;
mod reachability;
mod tactic_calibration;

fn is_frozen_and_tactic_command(name: &str) -> bool {
    matches!(
        name,
        "verify-frozen-policy-cold-replay"
            | "export-frozen-policy-tape"
            | "verify-frozen-policy"
            | "frozen-policy-probe-model"
            | "frozen-policy-batch"
            | "factorized-policy-batch"
            | "cql"
            | "iql"
            | "ensemble-q"
            | "prioritized-q"
            | "ablate-q"
            | "option-values"
            | "benchmark-tactic-checkpoint-codecs"
            | "freeze-tactic-policy"
            | "execute-tactic-policy"
            | "prove-generalized-tactics"
            | "tactic-route"
            | "project-tactic-route-accounting"
            | "project-tactic-campaign-summary"
            | "validate-tactic-campaign-summary"
            | "prove-tactic-route-cold-replay"
            | "validate-tactic-route-cold-replay"
            | "seal-tactic-cold-replay-bundle"
            | "validate-tactic-cold-replay-bundle"
            | "run-tactic-launch-smoke"
            | "seal-tactic-launch-smoke"
            | "validate-tactic-launch-smoke"
            | "tactic-throughput-curve"
            | "seal-tactic-throughput-curve"
            | "validate-tactic-throughput-curve-bundle"
            | "seal-tactic-throughput-treatment"
            | "validate-tactic-throughput-treatment-bundle"
            | "tactic-restore-locality"
            | "audit-tactic-fault-recovery"
            | "seal-tactic-fault-recovery"
            | "validate-tactic-fault-recovery-bundle"
            | "audit-post-terminal-tactic-controls"
            | "audit-tactic-scratch-campaign"
            | "audit-tactic-observations"
            | "compare-tactic-scratch-campaigns"
            | "diagnose-tactic-terminal-routes"
            | "validate-tactic-scratch-discovery"
            | "validate-tactic-scratch-bundle"
            | "validate-tactic-restore-locality"
    )
}

pub fn command_learn(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some(name) if is_frozen_and_tactic_command(name) => frozen_and_tactics::command(args),
        Some(
            "diff-episodes"
            | "dataset"
            | "extract-trace"
            | "inspect-episode"
            | "inspect-native"
            | "trace-return-restart-writes"
            | "validate-return-restart-write-trace"
            | "native-replay"
            | "auxiliary-dataset"
            | "goal-trajectory-dataset"
            | "inspect-auxiliary",
        ) => corpora::command(args),
        Some(
            "fit-goal-reachability"
            | "evaluate-goal-reachability-negative-controls"
            | "inspect-goal-reachability"
            | "fit-frozen-goal-policy"
            | "inspect-frozen-goal-policy",
        ) => reachability::command(args),
        Some(
            "pretrain-native-encoder"
            | "collision-history"
            | "episode-history"
            | "geometry-view"
            | "surface-graph-view"
            | "room-load-view"
            | "resource-load-view"
            | "actor-view",
        ) => native_views::command(args),
        Some("inspect" | "baseline" | "calibrate") => baselines::command(args),
        Some("double-q" | "fit" | "benchmark") => q_training::command(args),
        Some(
            "calibrate-tactic-value"
            | "cross-calibrate-tactic-value"
            | "compare-tactic-value-controls",
        ) => tactic_calibration::command(args),
        _ => usage_error(),
    }
}

#[cfg(test)]
mod tests {
    use super::is_frozen_and_tactic_command;

    #[test]
    fn retained_tactic_audits_are_reachable_from_the_learn_dispatcher() {
        for command in [
            "audit-tactic-fault-recovery",
            "seal-tactic-fault-recovery",
            "validate-tactic-fault-recovery-bundle",
            "validate-tactic-campaign-summary",
            "project-tactic-campaign-summary",
            "project-tactic-route-accounting",
            "audit-post-terminal-tactic-controls",
            "audit-tactic-scratch-campaign",
            "audit-tactic-observations",
            "compare-tactic-scratch-campaigns",
            "diagnose-tactic-terminal-routes",
            "validate-tactic-scratch-bundle",
            "prove-tactic-route-cold-replay",
            "validate-tactic-route-cold-replay",
            "seal-tactic-cold-replay-bundle",
            "validate-tactic-cold-replay-bundle",
            "run-tactic-launch-smoke",
            "seal-tactic-launch-smoke",
            "validate-tactic-launch-smoke",
            "seal-tactic-throughput-treatment",
            "validate-tactic-throughput-treatment-bundle",
        ] {
            assert!(is_frozen_and_tactic_command(command), "{command}");
        }
    }
}
