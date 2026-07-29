use super::*;
use dusklight_automation_contracts::tape::{InputFrame, RawPadState};
use dusklight_control::game_tactic::{GameTactic, GameTacticPlan};
use dusklight_control::option_execution::{OptionCondition, OptionEndReason, TapeRange};
use dusklight_evidence::native_episode_shard::{NativeEpisodeShard, NativeObservationPhase};
use dusklight_learning::parameterized_tactic_proposals::{
    ParameterizedTacticProposalContext, parameterized_tactic_family_schema_sha256,
    propose_parameterized_tactics,
};
use dusklight_learning::reward_shaping::{
    POTENTIAL_SHAPING_SCHEMA_V1, PotentialShapingSpec, PotentialTerm, TACTIC_REWARD_SPEC_SCHEMA_V1,
};
use dusklight_learning::tactic_asset::{TacticAssetSource, TacticCatalogEntry};
use dusklight_learning::tactic_exploration::TacticSelectionReason;
use std::fs;

#[test]
fn seeded_frontier_rotation_visits_every_eligible_cell_before_repeating() {
    let choice_count = 35;
    let visited = (0..choice_count as u64)
        .map(|round| seeded_frontier_index(104_729, round, choice_count))
        .collect::<BTreeSet<_>>();
    assert_eq!(visited, (0..choice_count).collect());
    assert_eq!(
        seeded_frontier_index(104_729, choice_count as u64, choice_count),
        seeded_frontier_index(104_729, 0, choice_count)
    );
}

#[test]
fn frontier_learning_value_precedes_the_last_edges_immediate_cost() {
    let valuable = TacticFrontierAcquisition {
        expansion_count: 0,
        terminal: false,
        terminal_value_supported: true,
        achieved_goal_value_supported: false,
        goal_reachability_supported: false,
        reward: -0.4,
        best_mean_q: Some(10.0),
        best_goal_progress_per_tick: None,
        predicted_terminal_ticks_to_go: None,
        predicted_total_terminal_ticks: None,
        exact_terminal_ticks_to_go: None,
        exact_total_terminal_ticks: None,
        maximum_ensemble_variance: None,
        generalized_nearest_distance: Some(0.1),
        discovery_spatial_novelty: None,
        novelty_rank: 1,
        replayed_prefix_ticks: 40,
    };
    let cheap_dead_end = TacticFrontierAcquisition {
        reward: -0.04,
        best_mean_q: Some(1.0),
        replayed_prefix_ticks: 4,
        ..valuable.clone()
    };

    assert_eq!(
        compare_frontier_acquisition(&valuable, &cheap_dead_end),
        std::cmp::Ordering::Less
    );
}

#[test]
fn frontier_learning_value_precedes_coverage_count() {
    let valuable = TacticFrontierAcquisition {
        expansion_count: 3,
        terminal: false,
        terminal_value_supported: true,
        achieved_goal_value_supported: false,
        goal_reachability_supported: false,
        reward: -0.4,
        best_mean_q: Some(10.0),
        best_goal_progress_per_tick: None,
        predicted_terminal_ticks_to_go: None,
        predicted_total_terminal_ticks: None,
        exact_terminal_ticks_to_go: None,
        exact_total_terminal_ticks: None,
        maximum_ensemble_variance: None,
        generalized_nearest_distance: Some(0.1),
        discovery_spatial_novelty: None,
        novelty_rank: 1,
        replayed_prefix_ticks: 40,
    };
    let fresh_dead_end = TacticFrontierAcquisition {
        expansion_count: 0,
        best_mean_q: Some(1.0),
        ..valuable.clone()
    };

    assert_eq!(
        compare_frontier_acquisition(&valuable, &fresh_dead_end),
        std::cmp::Ordering::Less
    );
}

#[test]
fn cold_start_frontier_coverage_precedes_unsupported_sparse_return() {
    let cheap_shallow = TacticFrontierAcquisition {
        expansion_count: 1,
        terminal: false,
        terminal_value_supported: false,
        achieved_goal_value_supported: false,
        goal_reachability_supported: false,
        reward: -0.04,
        best_mean_q: Some(10.0),
        best_goal_progress_per_tick: None,
        predicted_terminal_ticks_to_go: None,
        predicted_total_terminal_ticks: None,
        exact_terminal_ticks_to_go: None,
        exact_total_terminal_ticks: None,
        maximum_ensemble_variance: Some(0.1),
        generalized_nearest_distance: Some(0.1),
        discovery_spatial_novelty: None,
        novelty_rank: 0,
        replayed_prefix_ticks: 4,
    };
    let fresh_semantic_frontier = TacticFrontierAcquisition {
        expansion_count: 0,
        reward: -0.4,
        best_mean_q: Some(1.0),
        maximum_ensemble_variance: Some(1.0),
        generalized_nearest_distance: Some(1.0),
        novelty_rank: 1,
        replayed_prefix_ticks: 40,
        ..cheap_shallow.clone()
    };

    assert_eq!(
        compare_frontier_acquisition(&fresh_semantic_frontier, &cheap_shallow),
        std::cmp::Ordering::Less
    );
}

#[test]
fn terminal_supported_prediction_precedes_unsupported_q_estimate() {
    let supported = TacticFrontierAcquisition {
        expansion_count: 2,
        terminal: false,
        terminal_value_supported: true,
        achieved_goal_value_supported: false,
        goal_reachability_supported: false,
        reward: -0.4,
        best_mean_q: Some(2.0),
        best_goal_progress_per_tick: None,
        predicted_terminal_ticks_to_go: Some(80.0),
        predicted_total_terminal_ticks: Some(120.0),
        exact_terminal_ticks_to_go: None,
        exact_total_terminal_ticks: None,
        maximum_ensemble_variance: None,
        generalized_nearest_distance: Some(0.1),
        discovery_spatial_novelty: None,
        novelty_rank: 1,
        replayed_prefix_ticks: 40,
    };
    let unsupported = TacticFrontierAcquisition {
        expansion_count: 0,
        best_mean_q: Some(99.0),
        predicted_terminal_ticks_to_go: None,
        predicted_total_terminal_ticks: None,
        ..supported.clone()
    };

    assert_eq!(
        compare_frontier_acquisition(&supported, &unsupported),
        std::cmp::Ordering::Less
    );
}

#[test]
fn exact_terminal_path_precedes_an_optimistic_generalized_prediction() {
    let exact = TacticFrontierAcquisition {
        expansion_count: 1,
        terminal: false,
        terminal_value_supported: true,
        achieved_goal_value_supported: false,
        goal_reachability_supported: false,
        reward: -0.4,
        best_mean_q: Some(90.0),
        best_goal_progress_per_tick: None,
        predicted_terminal_ticks_to_go: Some(180.0),
        predicted_total_terminal_ticks: Some(196.0),
        exact_terminal_ticks_to_go: Some(180),
        exact_total_terminal_ticks: Some(196),
        maximum_ensemble_variance: None,
        generalized_nearest_distance: Some(0.1),
        discovery_spatial_novelty: None,
        novelty_rank: 1,
        replayed_prefix_ticks: 16,
    };
    let optimistic = TacticFrontierAcquisition {
        expansion_count: 0,
        best_mean_q: Some(99.0),
        predicted_terminal_ticks_to_go: Some(23.0),
        predicted_total_terminal_ticks: Some(63.0),
        exact_terminal_ticks_to_go: None,
        exact_total_terminal_ticks: None,
        replayed_prefix_ticks: 40,
        ..exact.clone()
    };

    assert_eq!(
        compare_frontier_acquisition(&exact, &optimistic),
        std::cmp::Ordering::Less
    );
}

#[test]
fn frontier_terminal_cost_includes_the_replayed_prefix() {
    let earlier = TacticFrontierAcquisition {
        expansion_count: 0,
        terminal: false,
        terminal_value_supported: true,
        achieved_goal_value_supported: false,
        goal_reachability_supported: false,
        reward: -0.4,
        best_mean_q: Some(99.0),
        best_goal_progress_per_tick: None,
        predicted_terminal_ticks_to_go: Some(84.0),
        predicted_total_terminal_ticks: Some(124.0),
        exact_terminal_ticks_to_go: None,
        exact_total_terminal_ticks: None,
        maximum_ensemble_variance: None,
        generalized_nearest_distance: Some(0.1),
        discovery_spatial_novelty: None,
        novelty_rank: 1,
        replayed_prefix_ticks: 40,
    };
    let late = TacticFrontierAcquisition {
        best_mean_q: Some(99.98),
        predicted_terminal_ticks_to_go: Some(2.0),
        predicted_total_terminal_ticks: Some(126.0),
        replayed_prefix_ticks: 124,
        ..earlier.clone()
    };

    assert_eq!(
        compare_frontier_acquisition(&earlier, &late),
        std::cmp::Ordering::Less
    );
}

#[test]
fn equal_terminal_cost_prefers_the_less_expanded_frontier() {
    let fresh = TacticFrontierAcquisition {
        expansion_count: 0,
        terminal: false,
        terminal_value_supported: true,
        achieved_goal_value_supported: false,
        goal_reachability_supported: false,
        reward: -0.4,
        best_mean_q: Some(99.0),
        best_goal_progress_per_tick: None,
        predicted_terminal_ticks_to_go: Some(86.0),
        predicted_total_terminal_ticks: Some(126.0),
        exact_terminal_ticks_to_go: None,
        exact_total_terminal_ticks: None,
        maximum_ensemble_variance: None,
        generalized_nearest_distance: Some(0.1),
        discovery_spatial_novelty: None,
        novelty_rank: 1,
        replayed_prefix_ticks: 40,
    };
    let expanded = TacticFrontierAcquisition {
        expansion_count: 1,
        best_mean_q: Some(99.98),
        predicted_terminal_ticks_to_go: Some(2.0),
        replayed_prefix_ticks: 124,
        ..fresh.clone()
    };

    assert_eq!(
        compare_frontier_acquisition(&fresh, &expanded),
        std::cmp::Ordering::Less
    );
}

#[test]
fn goal_reachability_ranks_equally_fresh_cold_start_frontiers() {
    let learned = TacticFrontierAcquisition {
        expansion_count: 0,
        terminal: false,
        terminal_value_supported: false,
        achieved_goal_value_supported: false,
        goal_reachability_supported: true,
        reward: -0.4,
        best_mean_q: None,
        best_goal_progress_per_tick: Some(40.0),
        predicted_terminal_ticks_to_go: None,
        predicted_total_terminal_ticks: None,
        exact_terminal_ticks_to_go: None,
        exact_total_terminal_ticks: None,
        maximum_ensemble_variance: None,
        generalized_nearest_distance: Some(0.2),
        discovery_spatial_novelty: None,
        novelty_rank: 8,
        replayed_prefix_ticks: 40,
    };
    let novel_but_slow = TacticFrontierAcquisition {
        best_goal_progress_per_tick: Some(8.0),
        generalized_nearest_distance: Some(1.0),
        novelty_rank: 0,
        ..learned.clone()
    };

    assert_eq!(
        compare_frontier_acquisition(&learned, &novel_but_slow),
        std::cmp::Ordering::Less
    );
}

#[test]
fn novelty_identity_ignores_bookkeeping_and_micro_motion_but_not_new_cells() {
    let shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v28.dseps"
    ))
    .unwrap();
    let original = FactSnapshot::from_native_learning(
        &shard.episodes[0].steps[0].pre_input,
        &[],
        None,
        vec![],
    )
    .unwrap();
    let mut later_observation = original.clone();
    later_observation.boundary_index += 9;
    later_observation.simulation_tick += 9;
    later_observation.tape_frame += 9;
    later_observation.state_identity = [0x5a; 16];

    assert_ne!(
        original.content_sha256().unwrap(),
        later_observation.content_sha256().unwrap()
    );
    assert_eq!(
        semantic_state_digest(&original).unwrap(),
        semantic_state_digest(&later_observation).unwrap()
    );

    let mut moved = later_observation;
    let mut position = moved.player.position_f32_bits;
    let original_x = f32::from_bits(position[0]);
    position[0] = ((original_x / 256.0).floor() * 256.0 + 128.0).to_bits();
    moved.player.position_f32_bits = position;
    assert_ne!(
        semantic_state_digest(&original).unwrap(),
        semantic_state_digest(&moved).unwrap()
    );
    assert_eq!(
        tactic_state_descriptor(&original, false),
        tactic_state_descriptor(&moved, false)
    );

    let mut new_cell = moved;
    let mut position = new_cell.player.position_f32_bits;
    position[0] = (f32::from_bits(position[0]) + 512.0).to_bits();
    new_cell.player.position_f32_bits = position;
    assert_ne!(
        tactic_state_descriptor(&original, false),
        tactic_state_descriptor(&new_cell, false)
    );
}

#[test]
fn parameterized_batch_uses_family_instances_absent_from_the_state_catalog() {
    let shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v28.dseps"
    ))
    .unwrap();
    let before = FactSnapshot::from_native_learning(
        &shard.episodes[0].steps[0].pre_input,
        &[],
        None,
        Vec::new(),
    )
    .unwrap();
    let bootstrap = TacticAssetCatalog::new(vec![
        TacticCatalogEntry::new(
            "shield",
            TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield { frames: 1 })),
        )
        .unwrap(),
    ])
    .unwrap();
    let current = LearnerState::build(
        before.clone(),
        &FactRegistry::canonical(),
        &bootstrap,
        &[],
        |_| true,
    )
    .unwrap();
    let campaign = TacticQCampaign::new(
        Digest([1; 32]),
        Digest([2; 32]),
        Digest([3; 32]),
        0,
        current,
        InputTape {
            frames: vec![InputFrame::default(); before.tape_frame as usize],
            ..InputTape::default()
        },
        OptionValueConfig::default(),
        TacticExplorationConfig {
            seed: 17,
            epsilon_per_million: 0,
        },
    )
    .unwrap();
    let proposals = propose_parameterized_tactics(ParameterizedTacticProposalContext {
        seed: 17,
        decision_index: 0,
        state_sha256: campaign.current.snapshot_sha256,
        player_position: before.player.position_f32_bits.map(f32::from_bits),
        camera_yaw_radians: before
            .player
            .camera_yaw_radians_f32_bits
            .map(f32::from_bits),
        goal_coordinate: [100.0, 20.0, -50.0],
        maximum_ticks: 40,
        feedback: None,
    })
    .unwrap();
    let batch = campaign
        .decide_parameterized_batch(
            &proposals.catalog,
            &proposals.blueprints,
            parameterized_tactic_family_schema_sha256(),
            &|_: &FactSnapshot| Ok::<_, &'static str>(vec![0.0]),
            32,
        )
        .unwrap();

    assert_eq!(
        batch.ranking.action_universe_sha256,
        parameterized_tactic_family_schema_sha256()
    );
    assert!(batch.proposals.len() > 4);
    assert!(
        batch
            .proposals
            .iter()
            .all(|proposal| { proposal.descriptor.option_id.starts_with("family/") })
    );
    assert!(batch.proposals.iter().all(|proposal| {
        proposals
            .catalog
            .prepare_execution(&proposal.descriptor.option_id)
            .is_ok()
    }));
    assert!(
        batch
            .proposals
            .iter()
            .all(|proposal| proposal.descriptor.option_id != "shield")
    );
    assert!(batch.proposals.iter().any(|proposal| {
        proposal.descriptor.option_type == dusklight_control::option_execution::OptionType::Roll
    }));

    let mut choices = batch.ranking.choices.clone();
    let excluded = choices[0].descriptor.clone();
    choices[0].applicable = false;
    let tried = choices[1].descriptor.option_id.as_str();
    let untried = applicable_untried_descriptors(&choices, &BTreeSet::from([tried]));
    assert!(!untried.contains(&excluded));
    assert!(
        !untried
            .iter()
            .any(|descriptor| descriptor.option_id == tried)
    );
    assert!(untried.iter().all(|descriptor| {
        choices
            .iter()
            .any(|choice| choice.applicable && choice.descriptor == *descriptor)
    }));
}

#[test]
fn cold_start_retains_refits_and_ranks_the_next_boundary() {
    let shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v28.dseps"
    ))
    .unwrap();
    let native_step = &shard.episodes[0].steps[0];
    let before =
        FactSnapshot::from_native_learning(&native_step.pre_input, &[], None, Vec::new()).unwrap();
    let catalog = TacticAssetCatalog::new(vec![
        TacticCatalogEntry::new(
            "shield",
            TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield { frames: 1 })),
        )
        .unwrap(),
    ])
    .unwrap();
    let registry = FactRegistry::canonical();
    let current = LearnerState::build(before.clone(), &registry, &catalog, &[], |_| true).unwrap();
    let route_prefix = InputTape {
        frames: vec![InputFrame::default(); before.tape_frame as usize],
        ..InputTape::default()
    };
    let root_checkpoint_sha256 = Digest([7; 32]);
    let mut campaign = TacticQCampaign::new(
        Digest([1; 32]),
        Digest([2; 32]),
        root_checkpoint_sha256,
        11,
        current,
        route_prefix.clone(),
        OptionValueConfig::default(),
        TacticExplorationConfig {
            seed: 41,
            epsilon_per_million: 0,
        },
    )
    .unwrap();
    let execution_authority_sha256 = Digest([6; 32]);
    campaign
        .bind_execution_authority(execution_authority_sha256)
        .unwrap();
    let cold_snapshot = campaign.learner_snapshot().unwrap();
    assert_eq!(cold_snapshot.training_replay_rows, 0);
    assert!(cold_snapshot.model_sha256.is_none());
    let cold_snapshot_sha256 = cold_snapshot.content_sha256().unwrap();
    let encode = |facts: &FactSnapshot| Ok::<_, &'static str>(vec![facts.tape_frame as f32]);

    let decision = campaign.decide(&catalog, &[], &encode).unwrap();
    assert_eq!(
        decision.selected.reason,
        TacticSelectionReason::UnsupportedBootstrap
    );
    assert!(decision.ranking.values.ranked.is_empty());
    assert_eq!(decision.ranking.values.unsupported.len(), 1);

    let mut frame = InputFrame {
        owned_ports: 1,
        ..InputFrame::default()
    };
    frame.pads[0] = RawPadState {
        buttons: native_step.chosen_pad.buttons,
        stick_x: native_step.chosen_pad.stick_x,
        stick_y: native_step.chosen_pad.stick_y,
        substick_x: native_step.chosen_pad.substick_x,
        substick_y: native_step.chosen_pad.substick_y,
        trigger_left: native_step.chosen_pad.trigger_left,
        trigger_right: native_step.chosen_pad.trigger_right,
        analog_a: native_step.chosen_pad.analog_a,
        analog_b: native_step.chosen_pad.analog_b,
        connected: native_step.chosen_pad.connected,
        error: native_step.chosen_pad.error,
    };
    let mut route_tape = route_prefix;
    route_tape.frames.push(frame);
    let execution = OptionExecution::capture(
        decision.selected.descriptor.option_id.clone(),
        decision.selected.descriptor.option_type.clone(),
        decision.selected.descriptor.parameters.clone(),
        1,
        1,
        OptionCondition::DurationElapsed,
        Vec::new(),
        OptionEndReason::Completed,
        &route_tape,
        TapeRange {
            start_frame: before.tape_frame,
            end_frame_exclusive: before.tape_frame + 1,
        },
    )
    .unwrap();
    let mut next_boundary = native_step.post_simulation.clone();
    next_boundary.phase = NativeObservationPhase::PreInput;
    next_boundary.simulation_tick += 1;
    next_boundary.tape_frame += 1;
    let after = FactSnapshot::from_native_learning(
        &next_boundary,
        std::slice::from_ref(&native_step.pre_input),
        Some(&execution),
        Vec::new(),
    )
    .unwrap();
    let terminal = after.terminal.reached.unwrap();
    let outcome = NativeTacticWorkerOutcome {
        schema: crate::native_tactic_worker::NATIVE_TACTIC_WORKER_OUTCOME_SCHEMA_V2.into(),
        source_checkpoint_sha256: root_checkpoint_sha256,
        checkpoint_identity: "fixture-checkpoint".into(),
        episode_shard_sha256: shard.content_sha256,
        selected: decision.selected.clone(),
        execution,
        native_queries: Vec::new(),
        route_tape,
        next_facts: after,
        state_extraction_micros: 1,
        intermediate_boundaries: Vec::new(),
        terminal,
        retained_native_checkpoint: None,
        retained_native_boundary_fingerprint: None,
    };
    let reward_spec = TacticRewardSpec {
        schema: TACTIC_REWARD_SPEC_SCHEMA_V1.into(),
        terminal_reward: 5.0,
        tick_cost: 0.25,
        novelty_reward: 1.0,
        per_tick_discount: 0.9,
        potential: Some(PotentialShapingSpec {
            schema: POTENTIAL_SHAPING_SCHEMA_V1.into(),
            feature_schema: Digest([1; 32]),
            terms: vec![PotentialTerm::CorridorProgress {
                name: "tape-progress".into(),
                feature: 0,
                start: before.tape_frame as f32,
                end: before.tape_frame as f32 + 1.0,
                weight: 2.0,
                unavailable_value: None,
            }],
        }),
        motion_cost: None,
    };
    let evaluated = campaign
        .evaluate_rewarded_outcome(outcome.clone(), &encode, &reward_spec)
        .unwrap();
    let mut scheduled = TacticQCampaign::resume(campaign.checkpoint().unwrap()).unwrap();
    let leased = scheduled
        .lease_current_parameterized_batch(
            TacticQProposalBatch {
                ranking: decision.ranking.clone(),
                proposals: vec![decision.selected.clone()],
                goal_reachability_estimates: Vec::new(),
            },
            std::slice::from_ref(&decision.selected.descriptor),
            1,
        )
        .unwrap();
    assert_eq!(leased.batch.proposals, vec![decision.selected.clone()]);
    assert_eq!(leased.leases.len(), 1);
    let leased_expansion = leased.leases[0].expansion_sha256;
    let mut restarted_scheduled = TacticQCampaign::resume(scheduled.checkpoint().unwrap()).unwrap();
    assert!(matches!(
        restarted_scheduled
            .state_graph
            .as_ref()
            .unwrap()
            .expansion(leased_expansion)
            .unwrap()
            .status,
        crate::state_graph::ActionExpansionStatus::Leased { .. }
    ));
    assert!(
        restarted_scheduled
            .lease_current_parameterized_batch(
                TacticQProposalBatch {
                    ranking: decision.ranking.clone(),
                    proposals: vec![decision.selected.clone()],
                    goal_reachability_estimates: Vec::new(),
                },
                std::slice::from_ref(&decision.selected.descriptor),
                1,
            )
            .is_err()
    );
    assert_eq!(
        restarted_scheduled
            .admit_leased_evaluated_replay(
                std::slice::from_ref(&evaluated),
                &[restarted_scheduled.episode_group],
                &leased.leases,
            )
            .unwrap(),
        1
    );
    assert!(matches!(
        restarted_scheduled
            .state_graph
            .as_ref()
            .unwrap()
            .expansion(leased_expansion)
            .unwrap()
            .status,
        crate::state_graph::ActionExpansionStatus::Completed { .. }
    ));
    assert_eq!(campaign.decision_index, 0);
    assert!(campaign.replay.is_empty());
    let episode_group = campaign.episode_group;
    assert!(
        campaign
            .admit_evaluated_replay(
                &[evaluated.clone(), evaluated.clone()],
                &[episode_group, episode_group],
            )
            .is_err()
    );
    assert_eq!(
        campaign
            .admit_evaluated_replay(
                &[evaluated.clone(), evaluated.clone()],
                &[episode_group, episode_group + 1],
            )
            .unwrap(),
        1
    );
    assert_eq!(campaign.training_replay_len(), 1);
    let retained = campaign
        .retain_and_refit_rewarded(
            decision,
            outcome,
            &catalog,
            &[],
            &registry,
            &encode,
            |_| true,
            &reward_spec,
            true,
        )
        .unwrap();

    assert_eq!(evaluated.transition, retained.step.transition);
    assert_eq!(evaluated.reward, retained.reward);
    assert_eq!(retained.step.replay_rows, 1);
    assert_eq!(retained.reward.terminal_observed, terminal);
    assert!(!retained.reward.endpoint_novel);
    assert_eq!(retained.reward.tick_cost_component, -0.25);
    assert_eq!(retained.reward.novelty_component, 0.0);
    assert!(retained.reward.potential.is_some());
    assert!(retained.reward.terminal_objective_unchanged);
    assert!(!retained.reward.promotion_authority);
    assert_eq!(campaign.replay.len(), 1);
    assert_eq!(campaign.training_replay_len(), 1);
    assert_eq!(campaign.episode_groups, vec![11]);
    assert!(
        campaign.model().is_none(),
        "an open first episode must not fit a closed option return"
    );
    assert_eq!(campaign.current.snapshot.tape_frame, before.tape_frame + 1);
    assert_eq!(
        campaign.route_tape.frames.len() as u64,
        campaign.current.snapshot.tape_frame
    );
    assert_eq!(campaign.visited_state_count(), 2);

    let expected_fitted_snapshot = campaign.learner_snapshot().unwrap();
    let checkpoint = campaign.checkpoint().unwrap();
    assert_eq!(checkpoint.schema, TACTIC_Q_CHECKPOINT_SCHEMA_V5);
    assert_eq!(
        checkpoint.execution_authority_sha256,
        execution_authority_sha256
    );
    assert_eq!(checkpoint.training_replay.len(), 1);
    assert_eq!(checkpoint.state_graph.expansion_count(), 1);
    assert_eq!(checkpoint.state_graph.completed_transitions().count(), 1);
    let restored = TacticQCampaign::resume(checkpoint.clone()).unwrap();
    assert_eq!(
        restored.execution_authority_sha256,
        execution_authority_sha256
    );
    assert_eq!(restored.decision_index, campaign.decision_index);
    assert_eq!(restored.training_replay_len(), 1);
    assert_eq!(restored.route_tape, campaign.route_tape);
    assert_eq!(restored.replay, campaign.replay);
    assert_eq!(restored.replay_routes, campaign.replay_routes);
    assert!(restored.model().is_none());
    let fitted_snapshot = restored.learner_snapshot().unwrap();
    assert_eq!(fitted_snapshot, expected_fitted_snapshot);
    assert_eq!(fitted_snapshot.training_replay_rows, 1);
    assert!(fitted_snapshot.model_sha256.is_none());
    assert_ne!(
        fitted_snapshot.content_sha256().unwrap(),
        cold_snapshot_sha256
    );
    let corpus = campaign.training_corpus();
    assert_eq!(
        corpus.execution_authority_sha256,
        execution_authority_sha256
    );
    let immutable_snapshot = TacticQImmutableLearnerSnapshot::fit(
        corpus.clone(),
        corpus.transitions.len() as u64,
        7,
        OptionValueConfig::default(),
        0,
        TacticValueTreatment::LocalGeneralizedFittedQKnnV1,
    )
    .unwrap();
    let mut treatment_corpus = corpus.clone();
    let mut alternate = treatment_corpus.transitions[0].clone();
    alternate.execution.option_id = "shield-alternate".into();
    alternate.value_sample.action.option_id = alternate.execution.option_id.clone();
    treatment_corpus.transitions.push(alternate);
    treatment_corpus
        .routes
        .push(treatment_corpus.routes[0].clone());
    treatment_corpus
        .episode_groups
        .push(treatment_corpus.episode_groups[0] + 1);
    let local_treatment_snapshot = TacticQImmutableLearnerSnapshot::fit(
        treatment_corpus.clone(),
        treatment_corpus.transitions.len() as u64,
        7,
        OptionValueConfig::default(),
        0,
        TacticValueTreatment::LocalGeneralizedFittedQKnnV1,
    )
    .unwrap();
    let continuous_snapshot = TacticQImmutableLearnerSnapshot::fit(
        treatment_corpus.clone(),
        treatment_corpus.transitions.len() as u64,
        7,
        OptionValueConfig::default(),
        0,
        TacticValueTreatment::ContinuousFittedQForestV1,
    )
    .unwrap();
    assert_eq!(
        local_treatment_snapshot.manifest.value_treatment,
        TacticValueTreatment::LocalGeneralizedFittedQKnnV1
    );
    assert_eq!(
        continuous_snapshot.manifest.value_treatment,
        TacticValueTreatment::ContinuousFittedQForestV1
    );
    // These are two open nonterminal episode ends. They remain available as
    // replay evidence, but neither value treatment may invent a closed return.
    assert!(local_treatment_snapshot.generalized_model.is_none());
    assert!(local_treatment_snapshot.continuous_model.is_none());
    assert!(continuous_snapshot.generalized_model.is_none());
    assert!(continuous_snapshot.continuous_model.is_none());
    assert_ne!(local_treatment_snapshot.sha256, continuous_snapshot.sha256);
    let mut snapshot_consumer = TacticQCampaign::resume_without_model(checkpoint.clone()).unwrap();
    assert!(snapshot_consumer.model().is_none());
    assert_eq!(
        snapshot_consumer
            .consume_learner_snapshot(&immutable_snapshot)
            .unwrap(),
        0
    );
    assert!(snapshot_consumer.model().is_none());
    assert_eq!(snapshot_consumer.model_revision(), 7);
    assert!(snapshot_consumer.campaign_learner_authority_managed);
    // A lane can admit local rows before the campaign-owned fitter publishes
    // its next immutable snapshot. Proposal selection must keep exploring
    // instead of assuming local replay count implies a shared model exists.
    snapshot_consumer
        .training_replay
        .push(checkpoint.training_replay[0].clone());
    assert_eq!(snapshot_consumer.training_replay_len(), 2);
    assert!(snapshot_consumer.generalized_model(0).unwrap().is_none());
    snapshot_consumer
        .decide_parameterized_batch_with_policy::<&'static str, _>(
            &catalog,
            &[],
            Digest([9; 32]),
            &encode,
            1,
            0,
            TacticProposalPolicy::Learned,
            Some(0),
            false,
        )
        .unwrap();
    assert_eq!(
        immutable_snapshot.sha256,
        immutable_snapshot.manifest.content_sha256().unwrap()
    );
    let corpus_root = std::env::temp_dir().join(format!(
        "dusklight-tactic-training-corpus-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&corpus_root);
    let corpus_path = corpus_root.join("seed-000-41/generated-training.dtqc");
    corpus
        .write(&corpus_path, &corpus_root.join("objects"))
        .unwrap();
    assert_eq!(TacticQTrainingCorpus::read(&corpus_path).unwrap(), corpus);
    let mut tampered = fs::read(&corpus_path).unwrap();
    *tampered.last_mut().unwrap() ^= 0x01;
    let tampered_path = corpus_root.join("tampered.dtqc");
    fs::write(&tampered_path, tampered).unwrap();
    assert!(TacticQTrainingCorpus::read(&tampered_path).is_err());
    fs::remove_dir_all(corpus_root).unwrap();
    let fresh_current =
        LearnerState::build(before.clone(), &registry, &catalog, &[], |_| true).unwrap();
    let mut fresh_episode = TacticQCampaign::new(
        Digest([1; 32]),
        Digest([2; 32]),
        root_checkpoint_sha256,
        99,
        fresh_current,
        tape_prefix(&campaign.replay_routes[0], before.tape_frame as usize),
        OptionValueConfig::default(),
        TacticExplorationConfig {
            seed: 43,
            epsilon_per_million: 0,
        },
    )
    .unwrap();
    fresh_episode
        .bind_execution_authority(execution_authority_sha256)
        .unwrap();
    assert!(fresh_episode.model().is_none());
    assert_eq!(
        fresh_episode
            .import_training_corpora(std::slice::from_ref(&corpus))
            .unwrap(),
        1
    );
    assert!(fresh_episode.model().is_none());
    assert!(fresh_episode.replay.is_empty());
    assert_eq!(fresh_episode.training_replay_len(), 1);
    assert_eq!(fresh_episode.frontier_archive().unwrap().tactic_len(), 1);
    let filtered_current =
        LearnerState::build(before.clone(), &registry, &catalog, &[], |_| true).unwrap();
    let mut filtered_episode = TacticQCampaign::new(
        Digest([1; 32]),
        Digest([2; 32]),
        root_checkpoint_sha256,
        199,
        filtered_current,
        tape_prefix(&campaign.replay_routes[0], before.tape_frame as usize),
        OptionValueConfig::default(),
        TacticExplorationConfig {
            seed: 44,
            epsilon_per_million: 0,
        },
    )
    .unwrap();
    filtered_episode
        .bind_execution_authority(execution_authority_sha256)
        .unwrap();
    assert_eq!(
        filtered_episode
            .consume_learner_snapshot_with_exploration_filter(&immutable_snapshot, |_| false)
            .unwrap(),
        1
    );
    assert_eq!(filtered_episode.training_replay_len(), 1);
    assert!(filtered_episode.model().is_none());
    assert_eq!(filtered_episode.frontier_archive().unwrap().tactic_len(), 0);
    assert_eq!(filtered_episode.visited_state_count(), 2);
    assert_eq!(
        fresh_episode
            .import_training_corpora(std::slice::from_ref(&corpus))
            .unwrap(),
        0
    );
    let mut detached = corpus.clone();
    detached.root_checkpoint_sha256 = Digest([9; 32]);
    assert!(
        fresh_episode
            .import_training_corpora(std::slice::from_ref(&detached))
            .is_err()
    );
    assert_eq!(fresh_episode.training_replay_len(), 1);
    let mut foreign_authority = corpus.clone();
    foreign_authority.execution_authority_sha256 = Digest([8; 32]);
    for transition in &mut foreign_authority.transitions {
        transition.execution_authority_sha256 = Digest([8; 32]);
    }
    assert!(
        fresh_episode
            .import_training_corpora(std::slice::from_ref(&foreign_authority))
            .is_err()
    );
    let policy = restored.freeze_greedy_policy().unwrap();
    assert_eq!(
        policy.execution_authority_sha256,
        execution_authority_sha256
    );
    policy
        .validate_execution_authority(execution_authority_sha256)
        .unwrap();
    assert!(
        policy
            .validate_execution_authority(Digest([8; 32]))
            .is_err()
    );
    assert_eq!(
        policy.action_universe_sha256,
        catalog.action_schema_sha256()
    );
    let archive = restored.frontier_archive().unwrap();
    assert_eq!(archive.tactic_len(), 1);
    let graph = restored.graph().unwrap();
    assert_eq!(graph.nodes.len(), 2);
    assert_eq!(graph.edges.len(), 1);
    assert!(graph.root_connected);
    assert_eq!(
        graph.root_checkpoint_sha256,
        campaign.replay[0].source_checkpoint_sha256
    );
    let root_node = graph
        .nodes
        .iter()
        .find(|node| node.checkpoint_sha256 == graph.root_checkpoint_sha256)
        .unwrap();
    assert_eq!(root_node.route_tape.frames.len() as u64, before.tape_frame);
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| { node.checkpoint_sha256 == campaign.replay[0].next_checkpoint_sha256 })
    );
    let projection = restored.graph_projection().unwrap();
    assert_eq!(projection.nodes.len(), 2);
    assert_eq!(projection.edges.len(), 1);
    assert_eq!(projection.edges[0].edge_index, 0);
    assert_eq!(projection.frontier_cells, 1);
    assert!(projection.root_connected);
    assert!(
        projection
            .nodes
            .iter()
            .any(|node| node.current && node.retained_frontier)
    );
    let projection_json = serde_json::to_vec(&projection).unwrap();
    assert!(
        !projection_json
            .windows(10)
            .any(|bytes| bytes == b"route_tape")
    );
    assert!(projection_json.len() < 4_096);
    let mut equivalent_pad_projection = campaign.replay[0].clone();
    equivalent_pad_projection
        .after
        .recent_option
        .as_mut()
        .unwrap()
        .option_id = "equivalent-pad-tactic".into();
    equivalent_pad_projection.after_state_sha256 =
        equivalent_pad_projection.after.content_sha256().unwrap();
    equivalent_pad_projection.value_sample.after_state_sha256 =
        equivalent_pad_projection.after_state_sha256;
    let mut equivalent_graph = TacticQCampaign::resume(campaign.checkpoint().unwrap()).unwrap();
    equivalent_graph.replay.push(equivalent_pad_projection);
    equivalent_graph
        .replay_routes
        .push(campaign.replay_routes[0].clone());
    equivalent_graph.episode_groups.push(77);
    let graph = equivalent_graph.graph().unwrap();
    assert_eq!(graph.nodes.len(), 2);
    assert!(graph.root_connected);
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| { node.checkpoint_sha256 == campaign.replay[0].next_checkpoint_sha256 })
            .count(),
        1
    );
    let diagnostics = restored.diagnostics().unwrap();
    assert_eq!(diagnostics.unique_selected_actions, 1);
    assert!(!diagnostics.zero_diversity_selection);
    assert!(!diagnostics.repeated_identical_compositions);
    assert!(!diagnostics.no_progress_loop);
    assert!(!diagnostics.frontier_lost_root_connectivity);
    let mut stagnant = campaign.replay[0].clone();
    stagnant.after = stagnant.before.clone();
    stagnant.after.boundary_index += 1;
    stagnant.after.simulation_tick += 1;
    stagnant.after.tape_frame += 1;
    stagnant.value_sample.terminal = false;
    assert!(has_no_progress_loop(&[stagnant], &[99]).unwrap());
    let mut collapsed = TacticQCampaign::resume(campaign.checkpoint().unwrap()).unwrap();
    collapsed.replay.push(campaign.replay[0].clone());
    collapsed
        .replay_routes
        .push(campaign.replay_routes[0].clone());
    collapsed.episode_groups.push(77);
    let collapsed_diagnostics = collapsed.diagnostics().unwrap();
    assert!(collapsed_diagnostics.zero_diversity_selection);
    assert!(collapsed_diagnostics.repeated_identical_compositions);
    let diagnostics = restored.diagnostics().unwrap();
    assert_eq!(diagnostics.logical_frontier_records, 2);
    assert_eq!(diagnostics.directly_restorable_native_frontiers, 0);
    assert_eq!(diagnostics.replay_only_frontiers, 1);
    let [root_branch, frontier_branch] = restored
        .sample_root_and_frontier(5, 0, &[], usize::MAX)
        .unwrap();
    assert_eq!(root_branch.kind, TacticBranchKind::Root);
    assert_eq!(frontier_branch.kind, TacticBranchKind::RetainedFrontier);
    assert_eq!(
        frontier_branch.logical_frontier.state_sha256,
        campaign.current.snapshot_sha256
    );
    assert!(root_branch.restorable_native_checkpoint.is_none());
    assert!(frontier_branch.restorable_native_checkpoint.is_none());
    assert_eq!(root_branch.logical_frontier.replayed_prefix_ticks, 0);
    assert!(frontier_branch.logical_frontier.replayed_prefix_ticks > 0);
    let frontier_restoration = restored.current_restoration_contract().unwrap();
    assert_eq!(
        frontier_restoration.plan.node.state_sha256,
        frontier_branch.logical_frontier.state_sha256
    );
    assert_eq!(
        frontier_restoration.receipt.observed_state_sha256,
        restored.current.snapshot_sha256
    );
    assert_eq!(
        frontier_restoration.plan.route.tape_frames,
        restored.route_tape.frames.len() as u64
    );
    let [scheduled_root, scheduled_frontier] = restored
        .graph_scheduled_root_and_frontier(5, 0, usize::MAX)
        .unwrap();
    assert_eq!(scheduled_root, root_branch);
    assert_eq!(
        scheduled_frontier.logical_frontier.state_sha256,
        campaign.current.snapshot_sha256
    );
    assert_eq!(
        scheduled_frontier
            .acquisition
            .as_ref()
            .unwrap()
            .expansion_count,
        0
    );
    let restarted_node_schedule = TacticQCampaign::resume(restored.checkpoint().unwrap()).unwrap();
    assert_eq!(
        restarted_node_schedule
            .graph_scheduled_root_and_frontier(5, 0, usize::MAX)
            .unwrap(),
        [scheduled_root, scheduled_frontier]
    );
    let [ranked_root, ranked_frontier] = restored
        .sample_root_and_ranked_frontier(
            5,
            0,
            &[],
            usize::MAX,
            false,
            0,
            &encode,
            &|_: &FactSnapshot| {
                Ok::<_, &'static str>(catalog.option_descriptors().cloned().collect())
            },
        )
        .unwrap();
    assert!(ranked_root.acquisition.is_none());
    let acquisition = ranked_frontier.acquisition.as_ref().unwrap();
    assert_eq!(acquisition.expansion_count, 0);
    assert!(!acquisition.terminal_value_supported);
    assert_eq!(
        acquisition.replayed_prefix_ticks,
        ranked_frontier.logical_frontier.replayed_prefix_ticks
    );
    assert!(acquisition.best_mean_q.is_none());
    assert!(acquisition.maximum_ensemble_variance.is_none());
    assert!(acquisition.generalized_nearest_distance.is_none());
    let mut model_only = TacticQCampaign::resume(restored.checkpoint().unwrap()).unwrap();
    model_only
        .training_episode_groups
        .fill(TACTIC_Q_MODEL_ONLY_EPISODE_GROUP);
    model_only.frontier_archive = build_frontier_archive(
        model_only.root_checkpoint_sha256,
        &model_only.training_replay,
        &model_only.training_replay_routes,
        &model_only.training_episode_groups,
    )
    .unwrap();
    assert_eq!(model_only.frontier_archive().unwrap().tactic_len(), 0);
    assert_eq!(model_only.frontier_cell_count(), 1);
    assert_eq!(model_only.demonstration_frontier_count(), 0);
    let mut demonstration = TacticQCampaign::resume(restored.checkpoint().unwrap()).unwrap();
    demonstration
        .training_episode_groups
        .fill(TACTIC_Q_DEMONSTRATION_EPISODE_GROUP);
    demonstration.frontier_archive = build_frontier_archive(
        demonstration.root_checkpoint_sha256,
        &demonstration.training_replay,
        &demonstration.training_replay_routes,
        &demonstration.training_episode_groups,
    )
    .unwrap();
    assert_eq!(demonstration.frontier_archive().unwrap().tactic_len(), 1);
    assert_eq!(demonstration.frontier_cell_count(), 1);
    assert_eq!(demonstration.demonstration_frontier_count(), 1);
    let mut terminal_leaf = TacticQCampaign::resume(restored.checkpoint().unwrap()).unwrap();
    terminal_leaf
        .training_episode_groups
        .fill(TACTIC_Q_DEMONSTRATION_EPISODE_GROUP);
    terminal_leaf.training_replay[0].value_sample.terminal = true;
    terminal_leaf.frontier_archive = build_frontier_archive(
        terminal_leaf.root_checkpoint_sha256,
        &terminal_leaf.training_replay,
        &terminal_leaf.training_replay_routes,
        &terminal_leaf.training_episode_groups,
    )
    .unwrap();
    assert_eq!(terminal_leaf.frontier_archive().unwrap().tactic_len(), 0);
    assert_eq!(terminal_leaf.frontier_cell_count(), 1);
    assert_eq!(terminal_leaf.demonstration_frontier_count(), 0);
    let mut forged_native_frontier = frontier_branch.clone();
    forged_native_frontier.restorable_native_checkpoint = Some(RestorableNativeTacticCheckpoint {
        worker_slot: 0,
        native_source_sha256: campaign.root_checkpoint_sha256,
        logical_frontier_sha256: frontier_branch.logical_frontier.identity_sha256,
        state_sha256: frontier_branch.logical_frontier.state_sha256,
        restore_identity: "unadmitted-process-local-handle".into(),
        checkpoint_bytes: 4096,
    });
    let mut rejects_forged_native =
        TacticQCampaign::resume(campaign.checkpoint().unwrap()).unwrap();
    assert!(
        rejects_forged_native
            .restore_branch(
                &forged_native_frontier,
                23,
                &registry,
                &catalog,
                &[],
                |_| true,
            )
            .is_err()
    );
    assert!(
        restored
            .sample_root_and_frontier(5, 0, &[], frontier_branch.route_tape.frames.len() - 1,)
            .is_err()
    );
    let mut branched = TacticQCampaign::resume(checkpoint.clone()).unwrap();
    branched
        .restore_branch(&root_branch, 22, &registry, &catalog, &[], |_| true)
        .unwrap();
    assert_eq!(branched.episode_group, 22);
    assert_eq!(
        branched.current.snapshot_sha256,
        root_branch.logical_frontier.state_sha256
    );
    let root_restoration = branched.current_restoration_contract().unwrap();
    assert_eq!(
        root_restoration.plan.node.route_checkpoint_sha256,
        root_branch.logical_frontier.identity_sha256
    );
    branched.current.snapshot.player.position_f32_bits[0] ^= 1;
    assert!(branched.current_restoration_contract().is_err());
    branched.current.snapshot.player.position_f32_bits[0] ^= 1;
    assert!(branched.model().is_none());
    branched.checkpoint().unwrap();
    let mut detached_projection = checkpoint.clone();
    detached_projection
        .training_replay
        .push(checkpoint.training_replay[0].clone());
    detached_projection
        .training_replay_routes
        .push(checkpoint.training_replay_routes[0].clone());
    detached_projection
        .training_episode_groups
        .push(checkpoint.training_episode_groups[0]);
    detached_projection.content_sha256 = checkpoint_digest(&detached_projection).unwrap();
    assert!(TacticQCampaign::resume(detached_projection).is_err());
    let mut tampered = checkpoint;
    tampered.decision_index += 1;
    assert!(TacticQCampaign::resume(tampered).is_err());

    let directory = std::env::temp_dir().join(format!(
        "dusklight-tactic-q-checkpoint-{}-{}",
        std::process::id(),
        campaign.current.snapshot_sha256
    ));
    let path = campaign.write_checkpoint(&directory).unwrap();
    assert_eq!(
        path.extension().and_then(|value| value.to_str()),
        Some(TACTIC_Q_CHECKPOINT_EXTENSION)
    );
    let stored = fs::read(&path).unwrap();
    assert_eq!(&stored[..8], b"DSKTQZ01");
    assert_ne!(stored.first(), Some(&b'{'));
    assert!(
        stored.len()
            < serde_cbor::to_vec(&campaign.checkpoint().unwrap())
                .unwrap()
                .len()
    );
    let payload = TacticQCampaign::read_checkpoint_payload(&path).unwrap();
    assert_eq!(
        payload.content_sha256,
        campaign.checkpoint().unwrap().content_sha256
    );
    let from_file = TacticQCampaign::read_checkpoint(&path).unwrap();
    assert_eq!(from_file.replay, campaign.replay);
    let objects = directory.join("objects");
    let hidden_objects = directory.join("objects-unavailable");
    fs::rename(&objects, &hidden_objects).unwrap();
    assert!(TacticQCampaign::read_checkpoint(&path).is_err());
    fs::rename(&hidden_objects, &objects).unwrap();
    let mut tampered_envelope = stored;
    let last = tampered_envelope.len() - 1;
    tampered_envelope[last] ^= 1;
    let tampered_path = path.with_file_name("tampered.dtqz");
    fs::write(&tampered_path, tampered_envelope).unwrap();
    assert!(TacticQCampaign::read_checkpoint_payload(&tampered_path).is_err());
    fs::remove_file(tampered_path).unwrap();
    fs::remove_file(&path).unwrap();
    fs::remove_dir_all(&directory).unwrap();

    if terminal {
        let final_result = campaign.final_result().unwrap();
        validate_final_result(&final_result).unwrap();
        let final_directory = std::env::temp_dir().join(format!(
            "dusklight-tactic-final-result-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&final_directory);
        let final_path = final_directory.join("result.dtqz");
        final_result.write(&final_path).unwrap();
        assert_eq!(TacticQFinalResult::read(&final_path).unwrap(), final_result);
        fs::remove_dir_all(final_directory).unwrap();
        let mut tampered = final_result;
        tampered.route_tape.frames[0].owned_ports ^= 1;
        assert!(validate_final_result(&tampered).is_err());
    } else {
        assert!(campaign.final_result().is_err());
    }

    let next = restored.decide(&catalog, &[], &encode).unwrap();
    assert_eq!(
        next.selected.reason,
        TacticSelectionReason::UnsupportedBootstrap
    );
    assert!(next.ranking.values.ranked.is_empty());
    assert_eq!(next.ranking.values.unsupported.len(), 1);

    // Continue the original in-memory campaign and the campaign loaded
    // from the sealed checkpoint through the same terminal outcome. This
    // makes interruption equivalence cover selection, refit, frontier,
    // tape, and final proof identities rather than only decoding.
    let mut uninterrupted = campaign;
    let mut resumed = from_file;
    let uninterrupted_decision = uninterrupted.decide(&catalog, &[], &encode).unwrap();
    let resumed_decision = resumed.decide(&catalog, &[], &encode).unwrap();
    assert_eq!(uninterrupted_decision, resumed_decision);
    assert_eq!(uninterrupted_decision, next);

    let mut terminal_route = uninterrupted.route_tape.clone();
    terminal_route.frames.push(InputFrame {
        owned_ports: 1,
        ..InputFrame::default()
    });
    let start_frame = uninterrupted.current.snapshot.tape_frame;
    let terminal_execution = OptionExecution::capture(
        uninterrupted_decision.selected.descriptor.option_id.clone(),
        uninterrupted_decision
            .selected
            .descriptor
            .option_type
            .clone(),
        uninterrupted_decision
            .selected
            .descriptor
            .parameters
            .clone(),
        1,
        1,
        OptionCondition::DurationElapsed,
        Vec::new(),
        OptionEndReason::Completed,
        &terminal_route,
        TapeRange {
            start_frame,
            end_frame_exclusive: start_frame + 1,
        },
    )
    .unwrap();
    let mut terminal_facts = uninterrupted.current.snapshot.clone();
    terminal_facts.boundary_index += 1;
    terminal_facts.simulation_tick += 1;
    terminal_facts.tape_frame += 1;
    terminal_facts.state_identity = [0x5a; 16];
    terminal_facts.player.position_f32_bits[0] =
        (f32::from_bits(terminal_facts.player.position_f32_bits[0]) + 512.0).to_bits();
    terminal_facts.terminal.configured = Some(true);
    terminal_facts.terminal.reached = Some(true);
    terminal_facts.terminal.reason =
        dusklight_learning::fact_snapshot::FactTerminalReason::GoalReached;
    terminal_facts.terminal.first_hit_tick = Some(terminal_facts.simulation_tick);
    let terminal_outcome = NativeTacticWorkerOutcome {
        schema: crate::native_tactic_worker::NATIVE_TACTIC_WORKER_OUTCOME_SCHEMA_V2.into(),
        source_checkpoint_sha256: uninterrupted.root_checkpoint_sha256,
        checkpoint_identity: "resume-equivalence-terminal".into(),
        episode_shard_sha256: shard.content_sha256,
        selected: uninterrupted_decision.selected.clone(),
        execution: terminal_execution,
        native_queries: Vec::new(),
        route_tape: terminal_route,
        next_facts: terminal_facts,
        state_extraction_micros: 1,
        intermediate_boundaries: Vec::new(),
        terminal: true,
        retained_native_checkpoint: None,
        retained_native_boundary_fingerprint: None,
    };
    let evaluated_terminal = uninterrupted
        .evaluate_rewarded_outcome(terminal_outcome.clone(), &encode, &reward_spec)
        .unwrap();
    let evaluated_terminal_result = uninterrupted
        .final_result_from_evaluated_terminal(&evaluated_terminal)
        .unwrap();
    validate_final_result(&evaluated_terminal_result).unwrap();
    assert!(
        !uninterrupted
            .final_result_matches_graph_terminal(&evaluated_terminal_result)
            .unwrap()
    );
    assert!(uninterrupted.final_result().is_err());
    let uninterrupted_step = uninterrupted
        .retain_and_refit_rewarded(
            uninterrupted_decision,
            terminal_outcome.clone(),
            &catalog,
            &[],
            &registry,
            &encode,
            |_| true,
            &reward_spec,
            true,
        )
        .unwrap();
    let resumed_step = resumed
        .retain_and_refit_rewarded(
            resumed_decision,
            terminal_outcome,
            &catalog,
            &[],
            &registry,
            &encode,
            |_| true,
            &reward_spec,
            true,
        )
        .unwrap();
    let cached_model = uninterrupted.generalized_model(0).unwrap().unwrap();
    let reused_model = uninterrupted.generalized_model(0).unwrap().unwrap();
    assert!(Arc::ptr_eq(&cached_model, &reused_model));
    uninterrupted.model_revision = uninterrupted.model_revision.saturating_add(1);
    resumed.model_revision = resumed.model_revision.saturating_add(1);
    let refitted_model = uninterrupted.generalized_model(0).unwrap().unwrap();
    assert!(!Arc::ptr_eq(&cached_model, &refitted_model));
    assert_eq!(uninterrupted_step, resumed_step);
    assert_eq!(
        serde_cbor::to_vec(&uninterrupted.model()).unwrap(),
        serde_cbor::to_vec(&resumed.model()).unwrap()
    );
    assert_eq!(
        uninterrupted.graph_projection().unwrap(),
        resumed.graph_projection().unwrap()
    );
    assert_eq!(
        uninterrupted
            .sample_root_and_frontier(8, 0, &[], usize::MAX)
            .unwrap(),
        resumed
            .sample_root_and_frontier(8, 0, &[], usize::MAX)
            .unwrap()
    );
    assert_eq!(uninterrupted.route_tape, resumed.route_tape);
    assert_eq!(
        uninterrupted.checkpoint().unwrap(),
        resumed.checkpoint().unwrap()
    );
    assert_eq!(
        uninterrupted.final_result().unwrap(),
        resumed.final_result().unwrap()
    );
    assert_eq!(
        uninterrupted.final_result().unwrap(),
        evaluated_terminal_result
    );
    assert!(
        uninterrupted
            .final_result_matches_graph_terminal(&evaluated_terminal_result)
            .unwrap()
    );
    assert_eq!(
        uninterrupted
            .best_graph_terminal_path()
            .unwrap()
            .unwrap()
            .route_checkpoint_sha256,
        route_checkpoint(
            uninterrupted.root_checkpoint_sha256,
            &evaluated_terminal_result.route_tape
        )
        .unwrap()
    );
}
