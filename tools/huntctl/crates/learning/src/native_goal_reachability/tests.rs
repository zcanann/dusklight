use super::*;
use crate::compiled_goal_graph::CompiledGoalGraph;
use crate::milestone_dsl::compile_source;
use crate::native_goal_trajectory::NativeGoalTrajectoryConfig;
use crate::native_replay_corpus::{NativeReplayCorpus, ReplayEpisodeSource, ReplayExperienceRole};
use crate::semantic_goal_input::SemanticGoalInput;
use dusklight_evidence::native_episode_shard::authored_milestone_objective_identity;

const GOAL_SOURCE: &str = r#"milestones 1.8
milestone reach_goal {
  phase post_sim
  when stage.room == 1
}
"#;

fn graph(source: &str, name: &str) -> CompiledGoalGraph {
    let compiled = compile_source(source).unwrap();
    let index = compiled
        .definitions
        .iter()
        .position(|definition| definition.name == name)
        .unwrap();
    CompiledGoalGraph::from_compiled(&compiled, index).unwrap()
}

fn balanced_sources() -> (
    NativeEpisodeShard,
    CompiledGoalGraph,
    NativeReplayCorpus,
    NativeGoalTrajectoryDataset,
) {
    let mut shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v14.dseps"
    ))
    .unwrap();
    let graph = graph(GOAL_SOURCE, "reach_goal");
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
            episode.id = format!("episode-{episode_index:04}");
            let digest = Sha256::digest(episode.id.as_bytes());
            episode.payload_xxh3_128.copy_from_slice(&digest[..16]);
            episode.success = success;
            episode.ticks_executed = 5;
            episode.first_hit_tick = success.then_some(4);
            let template = episode.steps[0].clone();
            episode.steps = (0..5_u32)
                .map(|step_index| {
                    let mut step = template.clone();
                    step.pre_input.player_position[0] = if success { 20.0 } else { -20.0 };
                    step.pre_input.remaining_ticks = 5 - step_index;
                    step.pre_input.state_identity = state_identity(episode_index, step_index, 0);
                    step.post_simulation.state_identity =
                        state_identity(episode_index, step_index, 1);
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
        .map(|(episode_index, _)| ReplayEpisodeSource {
            shard: &shard,
            episode_index,
            role: ReplayExperienceRole::RandomizedCoverage,
            policy_lineage_sha256: None,
            parent_entry_sha256: None,
        })
        .collect::<Vec<_>>();
    let corpus = NativeReplayCorpus::build(None, &replay_sources).unwrap();
    let dataset = (0..10_000_u64)
        .find_map(|split_seed| {
            let config = NativeGoalTrajectoryConfig {
                demonstration_mode: DemonstrationMode::BehaviorCloningWarmStart,
                n_step: 2,
                discount_millionths: 900_000,
                training_basis_points: 6_000,
                validation_basis_points: 2_000,
                split_seed,
            };
            let dataset = NativeGoalTrajectoryDataset::build(
                &corpus,
                std::slice::from_ref(&shard),
                &graph,
                config,
            )
            .unwrap();
            split_has_both(&dataset, AuxiliarySplit::Training)
                .then_some(dataset)
                .filter(|dataset| split_has_both(dataset, AuxiliarySplit::Validation))
                .filter(|dataset| split_has_both(dataset, AuxiliarySplit::Test))
        })
        .expect("a balanced deterministic episode split");
    (shard, graph, corpus, dataset)
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

fn fit_config() -> NativeGoalReachabilityConfig {
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
    }
}

#[test]
fn ensemble_fits_real_outcomes_n_step_returns_and_held_out_splits() {
    let (shard, graph, _, dataset) = balanced_sources();
    let first = NativeGoalReachabilityModel::fit(
        std::slice::from_ref(&dataset),
        std::slice::from_ref(&shard),
        fit_config(),
    )
    .unwrap();
    first.validate().unwrap();
    let decoded: NativeGoalReachabilityModel =
        serde_json::from_slice(&serde_json::to_vec(&first).unwrap()).unwrap();
    assert_eq!(decoded.digest().unwrap(), decoded.model_sha256);
    decoded.validate().unwrap();
    assert_eq!(decoded.model_sha256, first.model_sha256);
    assert!(first.training_n_step_bootstrap_rows > 0);
    assert_eq!(
        first.admission,
        NativeGoalReachabilityAdmission::GoalConditionedCandidate,
        "{:?}",
        first.validation
    );
    assert!(first.validation.reachability_brier < first.validation.baseline_reachability_brier);
    assert!(
        first.validation.discounted_return_rmse < first.validation.baseline_discounted_return_rmse
    );
    assert!(
        first.validation.successful_time_mae_ticks
            < first.validation.baseline_successful_time_mae_ticks
    );
    assert!(
        first.validation.discounted_tick_cost_mae
            < first.validation.baseline_discounted_tick_cost_mae
    );
    let second = NativeGoalReachabilityModel::fit(
        std::slice::from_ref(&dataset),
        std::slice::from_ref(&shard),
        fit_config(),
    )
    .unwrap();
    assert_eq!(first, second);

    let goal = SemanticGoalInput::from_graph(&graph).unwrap();
    let success = &shard
        .episodes
        .iter()
        .find(|episode| episode.success)
        .unwrap()
        .steps[0]
        .pre_input;
    let failure = &shard
        .episodes
        .iter()
        .find(|episode| !episode.success)
        .unwrap()
        .steps[0]
        .pre_input;
    let success_estimate = first.estimate(success, &goal).unwrap();
    let failure_estimate = first.estimate(failure, &goal).unwrap();
    assert!(success_estimate.reachability_probability > failure_estimate.reachability_probability);
    assert!(
        success_estimate.discounted_terminal_return > failure_estimate.discounted_terminal_return
    );
}

#[test]
fn semantic_embedding_excludes_provenance_and_split_leakage_fails_closed() {
    let single = graph(GOAL_SOURCE, "reach_goal");
    let multi = graph(
        r#"milestones 1.8
milestone unrelated {
  phase pre_input
  when stage.room == 9
}
milestone reach_goal {
  phase post_sim
  when stage.room == 1
}
"#,
        "reach_goal",
    );
    assert_ne!(single.program_sha256, multi.program_sha256);
    let single_input = SemanticGoalInput::from_graph(&single).unwrap();
    let multi_input = SemanticGoalInput::from_graph(&multi).unwrap();
    assert_ne!(single_input.input_sha256, multi_input.input_sha256);
    assert_eq!(
        goal_embedding(&single_input).unwrap(),
        goal_embedding(&multi_input).unwrap()
    );
    let changed = graph(
        r#"milestones 1.8
milestone reach_goal {
  phase post_sim
  when stage.room == 2
}
"#,
        "reach_goal",
    );
    assert_ne!(
        goal_embedding(&single_input).unwrap(),
        goal_embedding(&SemanticGoalInput::from_graph(&changed).unwrap()).unwrap()
    );

    let (shard, graph, corpus, dataset) = balanced_sources();
    let crossed = NativeGoalTrajectoryDataset::build(
        &corpus,
        std::slice::from_ref(&shard),
        &graph,
        NativeGoalTrajectoryConfig {
            split_seed: dataset.config.split_seed.wrapping_add(1),
            ..dataset.config
        },
    )
    .unwrap();
    assert!(
        NativeGoalReachabilityModel::fit(
            &[dataset, crossed],
            std::slice::from_ref(&shard),
            fit_config(),
        )
        .is_err()
    );
}

#[test]
fn source_and_resealed_model_tampering_fail_closed() {
    let (shard, _, _, dataset) = balanced_sources();
    let model = NativeGoalReachabilityModel::fit(
        std::slice::from_ref(&dataset),
        std::slice::from_ref(&shard),
        fit_config(),
    )
    .unwrap();
    let mut detached = shard.clone();
    detached.episodes[0].steps[0].pre_input.state_identity[0] ^= 1;
    assert!(
        NativeGoalReachabilityModel::fit(
            std::slice::from_ref(&dataset),
            std::slice::from_ref(&detached),
            fit_config(),
        )
        .is_err()
    );

    let mut tampered = model;
    tampered.admission = NativeGoalReachabilityAdmission::RetainTrainingMeanBaseline;
    tampered.model_sha256 = tampered.digest().unwrap();
    assert!(tampered.validate().is_err());
}

#[test]
fn negative_controls_are_equal_budget_sealed_and_expose_missing_observations() {
    let (shard, _, _, dataset) = balanced_sources();
    let config = NativeGoalReachabilityConfig {
        members: 2,
        epochs: 3,
        hidden_width: 4,
        minimum_validation_improvement: 0.0,
        maximum_validation_reachability_stddev: 0.5,
        ..fit_config()
    };
    let first = NativeGoalReachabilityNegativeControlReport::evaluate(
        std::slice::from_ref(&dataset),
        std::slice::from_ref(&shard),
        config,
    )
    .unwrap();
    first.validate().unwrap();
    let decoded: NativeGoalReachabilityNegativeControlReport =
        serde_json::from_slice(&serde_json::to_vec(&first).unwrap()).unwrap();
    decoded.validate().unwrap();
    assert_eq!(decoded, first);
    assert_eq!(first.controls.len(), 6);
    assert!(first.controls.iter().all(|control| {
        control.training.rows == first.baseline.training.rows
            && control.validation.rows == first.baseline.validation.rows
            && control.test.rows == first.baseline.test.rows
    }));

    let shuffled = &first.controls[0];
    assert_eq!(
        shuffled.control,
        Some(NativeGoalReachabilityNegativeControl::ShuffledOutcomes)
    );
    assert!(shuffled.changed_training_target_rows > 0);
    assert_eq!(shuffled.changed_input_cells, 0);

    for control in [&first.controls[3], &first.controls[5]] {
        assert_eq!(
            control.representation,
            NegativeControlRepresentation::NotRepresented
        );
        assert_eq!(control.changed_input_cells, 0);
        assert_eq!(control.model_sha256, first.baseline.model_sha256);
    }
    assert_eq!(first.observation_insufficiency.len(), 4);

    let second = NativeGoalReachabilityNegativeControlReport::evaluate(
        std::slice::from_ref(&dataset),
        std::slice::from_ref(&shard),
        config,
    )
    .unwrap();
    assert_eq!(first, second);

    let mut tampered = first;
    tampered.controls[3].changed_input_cells = 1;
    tampered.report_sha256 = tampered.digest().unwrap();
    assert!(tampered.validate().is_err());
}
