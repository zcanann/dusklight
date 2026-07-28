use super::*;
use crate::compiled_goal_graph::CompiledGoalGraph;
use crate::factorized_policy_suffix_batch::NativeFactorizedPolicyBatchConfig;
use crate::milestone_dsl::compile_source;
use crate::native_frozen_policy_suffix_batch::NativeFrozenPolicySuffixBatch;
use crate::native_goal_reachability::{NativeGoalReachabilityConfig, NativeGoalReachabilityModel};
use crate::native_goal_trajectory::NativeGoalTrajectoryConfig;
use crate::native_replay_corpus::{NativeReplayCorpus, ReplayEpisodeSource, ReplayExperienceRole};
use dusklight_evidence::native_episode_shard::authored_milestone_objective_identity;

const GOAL_SOURCE: &str = r#"milestones 1.8
milestone reach_goal {
  phase post_sim
  when stage.room == 1
}
"#;

fn graph() -> CompiledGoalGraph {
    let compiled = compile_source(GOAL_SOURCE).unwrap();
    CompiledGoalGraph::from_compiled(&compiled, 0).unwrap()
}

fn sources() -> (
    NativeEpisodeShard,
    NativeGoalTrajectoryDataset,
    NativeGoalReachabilityModel,
) {
    sources_with_failure_role(ReplayExperienceRole::RandomizedCoverage)
}

fn sources_with_failure_role(
    failure_role: ReplayExperienceRole,
) -> (
    NativeEpisodeShard,
    NativeGoalTrajectoryDataset,
    NativeGoalReachabilityModel,
) {
    let mut shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v14.dseps"
    ))
    .unwrap();
    let graph = graph();
    shard.metadata.objective = graph.definition_name.clone();
    shard.metadata.objective_identity = authored_milestone_objective_identity(
        &graph.program_sha256.to_string(),
        &graph.definition_sha256.to_string(),
    )
    .unwrap();
    let success_template = shard
        .episodes
        .iter()
        .find(|episode| episode.success)
        .unwrap()
        .clone();
    let failure_template = shard
        .episodes
        .iter()
        .find(|episode| !episode.success)
        .unwrap()
        .clone();
    shard.episodes = (0..120_u32)
        .map(|episode_index| {
            let success = episode_index % 2 == 0;
            let mut episode = if success {
                success_template.clone()
            } else {
                failure_template.clone()
            };
            episode.id = format!("policy-episode-{episode_index:04}");
            let digest = Sha256::digest(episode.id.as_bytes());
            episode.payload_xxh3_128.copy_from_slice(&digest[..16]);
            episode.success = success;
            episode.ticks_executed = 5;
            episode.first_hit_tick = success.then_some(4);
            let template = episode.steps[0].clone();
            episode.steps = (0..5_u32)
                .map(|step_index| {
                    let positive = step_index % 2 == 0;
                    let mut step = template.clone();
                    step.pre_input.player_position[0] = if positive { 20.0 } else { -20.0 };
                    step.pre_input.player_position[1] = if success { 20.0 } else { -20.0 };
                    step.pre_input.remaining_ticks = 5 - step_index;
                    step.pre_input.state_identity = state_identity(episode_index, step_index, 0);
                    step.post_simulation.state_identity =
                        state_identity(episode_index, step_index, 1);
                    let pad = NativeRawPad {
                        buttons: if positive { 1 } else { 0 },
                        stick_x: if positive { 64 } else { -64 },
                        stick_y: if positive { 32 } else { -32 },
                        substick_x: 0,
                        substick_y: 0,
                        trigger_left: if positive { 200 } else { 10 },
                        trigger_right: 0,
                        analog_a: 0,
                        analog_b: 0,
                        connected: true,
                        error: 0,
                    };
                    step.chosen_pad = pad;
                    step.consumed_pad = pad;
                    step
                })
                .collect();
            episode
        })
        .collect();
    let replay_sources = shard
        .episodes
        .iter()
        .enumerate()
        .map(|(episode_index, episode)| ReplayEpisodeSource {
            shard: &shard,
            episode_index,
            role: if episode.success {
                ReplayExperienceRole::RandomizedCoverage
            } else {
                failure_role
            },
            policy_lineage_sha256: (!episode.success
                && failure_role == ReplayExperienceRole::PolicyRollout)
                .then_some(Digest([9; 32])),
            parent_entry_sha256: None,
        })
        .collect::<Vec<_>>();
    let corpus = NativeReplayCorpus::build(None, &replay_sources).unwrap();
    let dataset = (0..10_000_u64)
        .find_map(|split_seed| {
            let dataset = NativeGoalTrajectoryDataset::build(
                &corpus,
                std::slice::from_ref(&shard),
                &graph,
                NativeGoalTrajectoryConfig {
                    demonstration_mode: DemonstrationMode::BehaviorCloningWarmStart,
                    n_step: 2,
                    discount_millionths: 900_000,
                    training_basis_points: 6_000,
                    validation_basis_points: 2_000,
                    split_seed,
                },
            )
            .unwrap();
            split_has_both(&dataset, AuxiliarySplit::Training)
                .then_some(dataset)
                .filter(|dataset| split_has_both(dataset, AuxiliarySplit::Validation))
                .filter(|dataset| split_has_both(dataset, AuxiliarySplit::Test))
        })
        .expect("a balanced deterministic episode split");
    let reachability = NativeGoalReachabilityModel::fit(
        std::slice::from_ref(&dataset),
        std::slice::from_ref(&shard),
        NativeGoalReachabilityConfig {
            members: 3,
            epochs: 18,
            hidden_width: 4,
            learning_rate: 0.02,
            l2_penalty: 1.0e-6,
            gradient_clip: 5.0,
            minimum_validation_improvement: 0.02,
            maximum_validation_reachability_stddev: 0.5,
            seed: 0x1234_5678_9abc_def0,
        },
    )
    .unwrap();
    assert_eq!(
        reachability.admission,
        NativeGoalReachabilityAdmission::GoalConditionedCandidate
    );
    (shard, dataset, reachability)
}

fn state_identity(episode: u32, step: u32, phase: u8) -> [u8; 16] {
    let mut hasher = Sha256::new();
    hasher.update(episode.to_le_bytes());
    hasher.update(step.to_le_bytes());
    hasher.update([phase]);
    hasher.finalize()[..16].try_into().unwrap()
}

fn split_has_both(dataset: &NativeGoalTrajectoryDataset, split: AuxiliarySplit) -> bool {
    dataset
        .rows
        .iter()
        .any(|row| row.split == split && row.success)
        && dataset
            .rows
            .iter()
            .any(|row| row.split == split && !row.success)
}

fn config() -> NativeGoalFrozenPolicyConfig {
    NativeGoalFrozenPolicyConfig {
        epochs: 80,
        hidden_width: 8,
        learning_rate: 0.01,
        l2_penalty: 1.0e-6,
        gradient_clip: 5.0,
        minimum_validation_joint_improvement: 0.02,
        seed: 0x9988_7766_5544_3322,
    }
}

#[test]
fn trains_and_directly_exports_a_deterministic_native_policy() {
    let (shard, dataset, reachability) = sources();
    let first = NativeGoalFrozenPolicyExport::fit(
        &dataset,
        std::slice::from_ref(&shard),
        &reachability,
        config(),
    )
    .unwrap();
    first.validate().unwrap();
    assert_eq!(
        first.manifest.admission,
        NativeGoalFrozenPolicyAdmission::FrozenPolicyCandidate,
        "{:?}",
        first.manifest.validation
    );
    assert!(first.manifest.validation.joint_error < first.manifest.validation.baseline_joint_error);
    assert!(first.manifest.test.joint_error < first.manifest.test.baseline_joint_error);
    let decoded: NativeGoalFrozenPolicyManifest =
        serde_json::from_slice(&serde_json::to_vec(&first.manifest).unwrap()).unwrap();
    decoded.validate(&first.model_bytes).unwrap();

    let model = FrozenInferenceModel::from_bytes(&first.model_bytes).unwrap();
    let batch = NativeFrozenPolicySuffixBatch::build(
        &first.model_bytes,
        "policy.dsfrozen".into(),
        first.manifest.objective_sha256,
        "trained-goal-policy".into(),
        NativeFactorizedPolicyBatchConfig {
            source_frame: 440,
            source_boundary_fingerprint: "1f849e432274771426236d60fbf7d72f".into(),
            checkpoint_validation_ticks: 2,
            maximum_ticks: 5,
            verify_state_hashes: true,
        },
    )
    .unwrap();
    assert_eq!(
        batch.frozen_policy.model_xxh3_128,
        first.manifest.frozen_model_xxh3_128
    );
    let success = shard
        .episodes
        .iter()
        .find(|episode| episode.success)
        .unwrap();
    let inputs = [0_usize, 1]
        .into_iter()
        .map(|index| {
            encode_native_policy_observation(&success.steps[index].pre_input)
                .unwrap()
                .to_vec()
        })
        .collect::<Vec<_>>();
    let outputs = model.infer_batch(&inputs).unwrap();
    let head = FactorizedPadPolicyHead::default();
    let positive = head.decode(&outputs[0]).unwrap().realized_pad().unwrap();
    let negative = head.decode(&outputs[1]).unwrap().realized_pad().unwrap();
    assert!(positive.stick_x > 0);
    assert!(negative.stick_x < 0);
    assert_eq!(positive.buttons & 1, 1);
    assert_eq!(negative.buttons & 1, 0);

    let second = NativeGoalFrozenPolicyExport::fit(
        &dataset,
        std::slice::from_ref(&shard),
        &reachability,
        config(),
    )
    .unwrap();
    assert_eq!(first, second);
}

#[test]
fn authenticated_policy_failures_change_the_next_frozen_objective() {
    let (baseline_shard, baseline_dataset, baseline_reachability) = sources();
    let baseline = NativeGoalFrozenPolicyExport::fit(
        &baseline_dataset,
        std::slice::from_ref(&baseline_shard),
        &baseline_reachability,
        config(),
    )
    .unwrap();
    assert_eq!(baseline.manifest.training_failed_rows, 0);

    let (contrast_shard, contrast_dataset, contrast_reachability) =
        sources_with_failure_role(ReplayExperienceRole::PolicyRollout);
    let contrast = NativeGoalFrozenPolicyExport::fit(
        &contrast_dataset,
        std::slice::from_ref(&contrast_shard),
        &contrast_reachability,
        config(),
    )
    .unwrap();
    contrast.validate().unwrap();
    assert!(contrast.manifest.training_failed_rows > 0);
    assert_eq!(contrast.manifest.failure_contrast_strength, 0.01);
    assert_eq!(contrast.manifest.failure_continuous_margin, 0.10);
    assert_eq!(contrast.manifest.failure_button_probability_margin, 0.10);
    assert_eq!(
        contrast.manifest.admission,
        NativeGoalFrozenPolicyAdmission::FrozenPolicyCandidate,
        "{:?}",
        contrast.manifest.validation
    );
    assert_ne!(baseline.model_bytes, contrast.model_bytes);
    assert_ne!(
        baseline.manifest.frozen_artifact_sha256,
        contrast.manifest.frozen_artifact_sha256
    );
}

#[test]
fn failed_action_contrast_is_directional_and_stops_at_its_margin() {
    assert!(bounded_continuous_contrast_gradient(0.5, 0.5) > 0.0);
    assert!(bounded_continuous_contrast_gradient(-0.25, -0.25) < 0.0);
    assert_eq!(bounded_continuous_contrast_gradient(0.7, 0.5), 0.0);

    let near_pressed = logit(0.95);
    let near_released = logit(0.05);
    assert!(bounded_button_contrast_gradient(near_pressed, 1.0) > 0.0);
    assert!(bounded_button_contrast_gradient(near_released, 0.0) < 0.0);
    assert_eq!(bounded_button_contrast_gradient(logit(0.8), 1.0), 0.0);
    assert_eq!(bounded_button_contrast_gradient(logit(0.2), 0.0), 0.0);
}

#[test]
fn replay_only_demonstrations_train_no_policy_action_targets() {
    let (shard, mut dataset, _) = sources();
    for row in &mut dataset.rows {
        if row.success {
            row.role = ReplayExperienceRole::Demonstration;
        }
    }
    dataset.config.demonstration_mode = DemonstrationMode::ReplayOnly;
    assert!(materialize(&dataset, std::slice::from_ref(&shard)).is_err());

    dataset.config.demonstration_mode = DemonstrationMode::BehaviorCloningWarmStart;
    assert!(!materialize(&dataset, &[shard]).unwrap().is_empty());
}

#[test]
fn source_manifest_and_frozen_byte_tampering_fail_closed() {
    let (shard, dataset, reachability) = sources();
    let export = NativeGoalFrozenPolicyExport::fit(
        &dataset,
        std::slice::from_ref(&shard),
        &reachability,
        config(),
    )
    .unwrap();

    let mut detached = shard.clone();
    detached.episodes[0].steps[0].pre_input.state_identity[0] ^= 1;
    assert!(
        NativeGoalFrozenPolicyExport::fit(
            &dataset,
            std::slice::from_ref(&detached),
            &reachability,
            config(),
        )
        .is_err()
    );

    let mut bytes = export.model_bytes.clone();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    assert!(export.manifest.validate(&bytes).is_err());

    let mut manifest = export.manifest;
    manifest.promotion_authority = true;
    manifest.manifest_sha256 = manifest.digest().unwrap();
    assert!(manifest.validate(&export.model_bytes).is_err());
}
