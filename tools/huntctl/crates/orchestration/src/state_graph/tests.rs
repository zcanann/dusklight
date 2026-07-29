use super::*;
use dusklight_automation_contracts::tape::{InputFrame, InputTape};
use dusklight_control::option_execution::{
    OptionCondition, OptionEndReason, OptionExecution, OptionType, TapeRange,
};
use dusklight_evidence::native_episode_shard::NativeEpisodeShard;
use dusklight_learning::fact_snapshot::{FactPhase, FactSnapshot};
use dusklight_learning::option_transition::{OptionIntermediateBoundary, OptionTransitionSample};
use std::collections::{BTreeMap, BTreeSet};

fn fixture_state() -> FactSnapshot {
    let shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v28.dseps"
    ))
    .unwrap();
    FactSnapshot::from_native_learning(&shard.episodes[0].steps[0].pre_input, &[], None, Vec::new())
        .unwrap()
}

fn advanced_state(root: &FactSnapshot, ticks: u64) -> FactSnapshot {
    let mut state = root.clone();
    state.phase = FactPhase::PreInput;
    state.boundary_index += ticks;
    state.simulation_tick += ticks;
    state.tape_frame += ticks;
    state.recent_history.clear();
    state.recent_option = None;
    state.terminal.reached = Some(false);
    state.validate().unwrap();
    state
}

fn graph_and_transition() -> (StateGraph, OptionTransitionSample, InputTape) {
    let before = fixture_state();
    let root_route = InputTape {
        frames: vec![InputFrame::default(); before.tape_frame as usize],
        ..InputTape::default()
    };
    let identity = StateGraphIdentity {
        execution_authority_sha256: Digest([1; 32]),
        future_equivalence_validator_sha256: Digest([1; 32]),
        feature_schema_sha256: Digest([2; 32]),
        objective_sha256: Digest([3; 32]),
        root_checkpoint_sha256: Digest([4; 32]),
    };
    let graph = StateGraph::new(identity.clone(), before.clone(), root_route.clone()).unwrap();
    let mut route = root_route;
    route.frames.extend(vec![InputFrame::default(); 8]);
    let execution = OptionExecution::capture(
        "move".into(),
        OptionType::Move,
        BTreeMap::new(),
        8,
        8,
        OptionCondition::DurationElapsed,
        Vec::new(),
        OptionEndReason::Completed,
        &route,
        TapeRange {
            start_frame: before.tape_frame,
            end_frame_exclusive: before.tape_frame + 8,
        },
    )
    .unwrap();
    let source_checkpoint_sha256 = route_checkpoint_sha256(
        identity.root_checkpoint_sha256,
        &tape_prefix(&route, before.tape_frame as usize).unwrap(),
    )
    .unwrap();
    let next_checkpoint_sha256 =
        route_checkpoint_sha256(identity.root_checkpoint_sha256, &route).unwrap();
    let after = advanced_state(&before, 8);
    let mut transition = OptionTransitionSample::capture(
        identity.feature_schema_sha256,
        source_checkpoint_sha256,
        next_checkpoint_sha256,
        before.clone(),
        after,
        execution,
        &route,
        -8.0,
        false,
        |state| Ok::<_, &'static str>(vec![state.tape_frame as f32]),
    )
    .unwrap();
    transition.execution_authority_sha256 = identity.execution_authority_sha256;
    let interior = advanced_state(&before, 4);
    transition.intermediate_boundaries = vec![OptionIntermediateBoundary {
        episode_shard_sha256: Digest([5; 32]),
        offset_ticks: 4,
        state_sha256: interior.content_sha256().unwrap(),
        state: interior,
    }];
    transition.validate().unwrap();
    (graph, transition, route)
}

fn terminalize(transition: &mut OptionTransitionSample) {
    transition.after.terminal.reached = Some(true);
    transition.after.terminal.reason =
        dusklight_learning::fact_snapshot::FactTerminalReason::GoalReached;
    transition.after.terminal.first_hit_tick = Some(transition.after.simulation_tick);
    let terminal_sha256 = transition.after.content_sha256().unwrap();
    transition.after_state_sha256 = terminal_sha256;
    transition.value_sample.after_state_sha256 = terminal_sha256;
    transition.value_sample.terminal = true;
    transition.validate().unwrap();
}

#[test]
fn one_selected_action_owns_all_observed_interior_segments() {
    let (mut graph, transition, route) = graph_and_transition();
    let admission = graph
        .admit_completed_expansion(
            transition,
            route,
            17,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap();

    assert_eq!(admission.source, graph.root());
    assert_eq!(admission.inserted_nodes, 2);
    assert_eq!(admission.inserted_segments, 2);
    assert_eq!(graph.node_count(), 3);
    assert_eq!(graph.expansion_count(), 1);
    assert_eq!(graph.segment_count(), 2);
    assert_eq!(graph.completed_transitions().count(), 1);
    graph.validate().unwrap();

    let expansion = graph.expansion(admission.expansion_sha256).unwrap();
    assert_eq!(expansion.observed_segments.len(), 2);
    for segment_sha256 in &expansion.observed_segments {
        let segment = graph.segment(*segment_sha256).unwrap();
        assert_eq!(segment.parent_expansion_sha256, admission.expansion_sha256);
    }
}

#[test]
fn forty_tick_option_exposes_and_executes_every_four_tick_counterfactual() {
    let before = fixture_state();
    let root_route = InputTape {
        frames: vec![InputFrame::default(); before.tape_frame as usize],
        ..InputTape::default()
    };
    let identity = StateGraphIdentity {
        execution_authority_sha256: Digest([1; 32]),
        future_equivalence_validator_sha256: Digest([1; 32]),
        feature_schema_sha256: Digest([2; 32]),
        objective_sha256: Digest([3; 32]),
        root_checkpoint_sha256: Digest([4; 32]),
    };
    let mut graph = StateGraph::new(identity.clone(), before.clone(), root_route.clone()).unwrap();
    let mut long_route = root_route;
    long_route.frames.extend(vec![InputFrame::default(); 40]);
    let execution = OptionExecution::capture(
        "long-option".into(),
        OptionType::Move,
        BTreeMap::new(),
        40,
        40,
        OptionCondition::DurationElapsed,
        Vec::new(),
        OptionEndReason::Completed,
        &long_route,
        TapeRange {
            start_frame: before.tape_frame,
            end_frame_exclusive: before.tape_frame + 40,
        },
    )
    .unwrap();
    let mut long_transition = OptionTransitionSample::capture(
        identity.feature_schema_sha256,
        graph.root().route_checkpoint_sha256,
        route_checkpoint_sha256(identity.root_checkpoint_sha256, &long_route).unwrap(),
        before.clone(),
        advanced_state(&before, 40),
        execution,
        &long_route,
        -40.0,
        false,
        |state| Ok::<_, &'static str>(vec![state.tape_frame as f32]),
    )
    .unwrap();
    long_transition.execution_authority_sha256 = identity.execution_authority_sha256;
    long_transition.intermediate_boundaries = (4..40)
        .step_by(4)
        .map(|offset| {
            let state = advanced_state(&before, offset);
            OptionIntermediateBoundary {
                episode_shard_sha256: Digest([offset as u8; 32]),
                offset_ticks: offset as u32,
                state_sha256: state.content_sha256().unwrap(),
                state,
            }
        })
        .collect();
    long_transition.validate().unwrap();
    let long = graph
        .admit_completed_expansion(
            long_transition,
            long_route,
            1,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap();

    assert_eq!(long.inserted_nodes, 10);
    assert_eq!(long.inserted_segments, 10);
    let interior = graph
        .nodes()
        .filter(|node| node.root_ticks > 0 && node.root_ticks < 40)
        .map(|node| node.id)
        .collect::<Vec<_>>();
    assert_eq!(interior.len(), 9);

    for (index, source) in interior.iter().copied().enumerate() {
        let before = graph.node(source).unwrap().state.clone();
        let mut route = graph.route(source.route_checkpoint_sha256).unwrap().clone();
        route.frames.push(InputFrame::default());
        let execution = OptionExecution::capture(
            format!("counterfactual-{index}"),
            OptionType::Turn,
            BTreeMap::new(),
            1,
            1,
            OptionCondition::DurationElapsed,
            Vec::new(),
            OptionEndReason::Completed,
            &route,
            TapeRange {
                start_frame: before.tape_frame,
                end_frame_exclusive: before.tape_frame + 1,
            },
        )
        .unwrap();
        let mut transition = OptionTransitionSample::capture(
            identity.feature_schema_sha256,
            source.route_checkpoint_sha256,
            route_checkpoint_sha256(identity.root_checkpoint_sha256, &route).unwrap(),
            before.clone(),
            advanced_state(&before, 1),
            execution,
            &route,
            -1.0,
            false,
            |state| Ok::<_, &'static str>(vec![state.tape_frame as f32]),
        )
        .unwrap();
        transition.execution_authority_sha256 = identity.execution_authority_sha256;
        transition.validate().unwrap();
        let counterfactual = graph
            .admit_completed_expansion(
                transition,
                route,
                2 + index as u64,
                ExpansionEvidenceAuthority::Executable,
            )
            .unwrap();
        assert_eq!(counterfactual.source, source);
    }

    for source in interior {
        let node = graph.node(source).unwrap();
        assert_eq!(node.outgoing_expansions.len(), 1);
        let expansion = graph
            .expansion(*node.outgoing_expansions.first().unwrap())
            .unwrap();
        assert!(matches!(
            expansion.status,
            ActionExpansionStatus::Completed { .. }
        ));
    }
    graph.validate().unwrap();
}

#[test]
fn exact_terminal_returns_cover_route_specific_interior_nodes() {
    let (mut graph, mut transition, route) = graph_and_transition();
    terminalize(&mut transition);
    graph
        .admit_completed_expansion(
            transition,
            route,
            17,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap();

    let returns = graph.exact_terminal_returns().unwrap();
    assert_eq!(returns.len(), 3);
    for node in graph.nodes() {
        assert_eq!(returns.get(&node.id), Some(&(8 - node.root_ticks)));
    }
}

#[test]
fn learner_targets_keep_exact_success_separate_from_censored_continuation() {
    let (mut censored_graph, censored, censored_route) = graph_and_transition();
    censored_graph
        .admit_completed_expansion(
            censored,
            censored_route,
            17,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap();
    let censored_batch = crate::learner::GraphLearningBatch::from_graph(&censored_graph).unwrap();
    assert_eq!(censored_batch.rows.len(), 1);
    assert_eq!(
        censored_batch.rows[0].support,
        crate::learner::GraphTargetSupport::OpenContinuationCensored
    );
    assert_eq!(
        censored_batch.rows[0].exact_conditional_ticks_to_terminal,
        None
    );
    assert_eq!(censored_batch.rows[0].realized_duration_ticks, 8);
    assert!(censored_batch.rows[0].action_accepted);
    assert!(!censored_batch.rows[0].immediate_terminal);
    assert_eq!(
        censored_batch.rows[0].prompted_action_status,
        censored_batch.rows[0]
            .target_state
            .player
            .action_state
            .map(|action| action.do_status)
    );
    let exact_learner = crate::learner::ExactGraphTableLearner;
    let censored_snapshot = crate::learner::ActionConditionedGraphLearner::fit(
        &exact_learner,
        &crate::learner::GraphLearnerContract::default(),
        &censored_batch,
    )
    .unwrap();
    let censored_action_sha256 = censored_batch.rows[0].action.content_sha256().unwrap();
    assert!(
        censored_snapshot
            .generalized_objective_prediction(
                &censored_batch.rows[0].source_features,
                censored_action_sha256,
            )
            .is_none()
    );
    assert_eq!(
        censored_snapshot
            .estimate(censored_batch.rows[0].source, censored_action_sha256)
            .unwrap()
            .terminal_support_per_million,
        None
    );
    let exact_auxiliary = censored_snapshot
        .auxiliary_prediction(censored_batch.rows[0].source, censored_action_sha256)
        .unwrap();
    assert!(!exact_auxiliary.generalized);
    assert_eq!(exact_auxiliary.realized_duration_ticks, 8);
    assert_eq!(exact_auxiliary.action_acceptance_per_million, 1_000_000);
    assert_eq!(
        exact_auxiliary.next_state_feature_f32_bits,
        censored_batch.rows[0]
            .target_features
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>()
    );
    let generalized_auxiliary = censored_snapshot
        .auxiliary_prediction(censored_batch.rows[0].target, censored_action_sha256)
        .unwrap();
    assert!(generalized_auxiliary.generalized);
    assert_eq!(generalized_auxiliary.support_rows, 1);
    assert_eq!(
        generalized_auxiliary.next_state_feature_f32_bits,
        vec![(censored_batch.rows[0].target_features[0] + 8.0).to_bits()]
    );
    let mut mixed_batch = censored_batch.clone();
    let mut second = mixed_batch.rows[0].clone();
    second.expansion_sha256 = Digest([42; 32]);
    second.source = mixed_batch.rows[0].target;
    second.source_state = mixed_batch.rows[0].target_state.clone();
    second.source_features = mixed_batch.rows[0].target_features.clone();
    second.target_state = advanced_state(&second.source_state, 4);
    second.target = ExactStateId {
        route_checkpoint_sha256: Digest([43; 32]),
        state_sha256: second.target_state.content_sha256().unwrap(),
    };
    second.target_features = vec![second.source_features[0] + 4.0];
    second.realized_duration_ticks = 4;
    second.end_reason = OptionEndReason::Completed;
    second.action_accepted = true;
    second.prompted_action_status = second
        .target_state
        .player
        .action_state
        .map(|action| action.do_status);
    second.immediate_terminal = false;
    second.support = crate::learner::GraphTargetSupport::OpenContinuationCensored;
    second.exact_conditional_ticks_to_terminal = None;
    let prediction_source = second.target;
    mixed_batch.rows.push(second);
    mixed_batch.validate().unwrap();
    let mixed_snapshot = crate::learner::ActionConditionedGraphLearner::fit(
        &exact_learner,
        &crate::learner::GraphLearnerContract::default(),
        &mixed_batch,
    )
    .unwrap();
    let mixed_prediction = mixed_snapshot
        .auxiliary_prediction(prediction_source, censored_action_sha256)
        .unwrap();
    assert!(mixed_prediction.generalized);
    assert_eq!(mixed_prediction.realized_duration_ticks, 6);
    assert_eq!(
        mixed_prediction.next_state_feature_f32_bits,
        vec![(mixed_batch.rows[1].target_features[0] + 6.0).to_bits()]
    );
    assert!(mixed_prediction.prediction_error_millionths > 0);
    let calibration = crate::learner::HeldOutGraphCalibrationReport::build(
        &crate::learner::GraphLearnerContract::default(),
        &mixed_batch,
    )
    .unwrap();
    assert_eq!(calibration.training_rows, 1);
    assert_eq!(calibration.held_out_rows, 1);
    assert_eq!(calibration.held_out_state_groups, 1);
    assert_eq!(calibration.independently_realized_action_rows, 1);
    assert_eq!(calibration.auxiliary_predictions, 1);
    assert!(calibration.auxiliary_mean_error_millionths > 0);
    assert_eq!(calibration.objective_predictions, 0);
    calibration.validate().unwrap();
    let mut detached_calibration = calibration;
    detached_calibration.auxiliary_predictions = 0;
    assert!(detached_calibration.validate().is_err());

    let (mut terminal_graph, mut terminal, terminal_route) = graph_and_transition();
    terminalize(&mut terminal);
    terminal_graph
        .admit_completed_expansion(
            terminal,
            terminal_route,
            17,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap();
    let terminal_batch = crate::learner::GraphLearningBatch::from_graph(&terminal_graph).unwrap();
    assert_eq!(terminal_batch.rows.len(), 1);
    assert_eq!(
        terminal_batch.rows[0].support,
        crate::learner::GraphTargetSupport::ExactTerminalPath
    );
    assert_eq!(
        terminal_batch.rows[0].exact_conditional_ticks_to_terminal,
        Some(8)
    );
    let terminal_snapshot = crate::learner::ActionConditionedGraphLearner::fit(
        &exact_learner,
        &crate::learner::GraphLearnerContract::default(),
        &terminal_batch,
    )
    .unwrap();
    let estimate = terminal_snapshot
        .estimate(
            terminal_batch.rows[0].source,
            terminal_batch.rows[0].action.content_sha256().unwrap(),
        )
        .unwrap();
    assert_eq!(estimate.terminal_support_per_million, Some(1_000_000));
    assert_eq!(estimate.conditional_ticks_to_terminal, Some(8));
    let generalized = crate::learner::ActionConditionedGraphLearner::rank(
        &exact_learner,
        &terminal_snapshot,
        &crate::learner::GraphNodeInput {
            id: terminal_batch.rows[0].target,
            state: terminal_batch.rows[0].target_state.clone(),
            graph_visits: 1,
        },
        &[crate::learner::GraphActionInput {
            expansion_sha256: Digest([44; 32]),
            action: terminal_batch.rows[0].action.clone(),
            graph_visits: 0,
        }],
    )
    .unwrap()
    .pop()
    .unwrap();
    assert_eq!(generalized.terminal_support_per_million, None);
    assert_eq!(generalized.conditional_ticks_to_terminal, Some(8));
    assert_eq!(
        terminal_snapshot
            .auxiliary_prediction(
                terminal_batch.rows[0].source,
                terminal_batch.rows[0].action.content_sha256().unwrap(),
            )
            .unwrap()
            .immediate_terminal_per_million,
        1_000_000
    );
}

#[test]
fn held_out_objective_gate_requires_state_conditioned_ranking_gain() {
    let (mut graph, mut transition, route) = graph_and_transition();
    terminalize(&mut transition);
    graph
        .admit_completed_expansion(
            transition,
            route,
            17,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap();
    let mut batch = crate::learner::GraphLearningBatch::from_graph(&graph).unwrap();
    let template = batch.rows[0].clone();
    batch.rows.clear();
    let source_features = [0.0_f32, 10.0, 20.0, 100.0, 30.0];
    let action_a_ticks = [30_u64, 35, 38, 1_000, 40];
    for (source_index, source_feature) in source_features.into_iter().enumerate() {
        let source_number = source_index + 1;
        let source_state = advanced_state(&fixture_state(), source_number as u64);
        let mut target_state = advanced_state(&source_state, 8);
        target_state.terminal = template.target_state.terminal.clone();
        target_state.terminal.first_hit_tick = Some(target_state.simulation_tick);
        target_state.validate().unwrap();
        for action_index in 0..2 {
            let mut row = template.clone();
            row.expansion_sha256 =
                Digest([u8::try_from(source_number * 2 + action_index).unwrap(); 32]);
            row.source = ExactStateId {
                route_checkpoint_sha256: Digest([u8::try_from(source_number).unwrap(); 32]),
                state_sha256: source_state.content_sha256().unwrap(),
            };
            row.target = ExactStateId {
                route_checkpoint_sha256: Digest(
                    [u8::try_from(100 + source_number * 2 + action_index).unwrap(); 32],
                ),
                state_sha256: target_state.content_sha256().unwrap(),
            };
            row.source_state = source_state.clone();
            row.target_state = target_state.clone();
            row.source_features = vec![source_feature];
            row.target_features = vec![source_feature + 1.0];
            row.action.option_id = format!("held-out-action-{action_index}");
            row.prompted_action_status = row
                .target_state
                .player
                .action_state
                .map(|action| action.do_status);
            row.immediate_terminal = true;
            row.graph_visits = 1;
            row.support = crate::learner::GraphTargetSupport::ExactTerminalPath;
            row.exact_conditional_ticks_to_terminal = Some(if action_index == 0 {
                action_a_ticks[source_index]
            } else {
                60
            });
            batch.rows.push(row);
        }
    }
    batch.validate().unwrap();

    let report = crate::learner::HeldOutGraphCalibrationReport::build(
        &crate::learner::GraphLearnerContract::default(),
        &batch,
    )
    .unwrap();
    assert_eq!(report.training_rows, 8);
    assert_eq!(report.held_out_rows, 2);
    assert_eq!(report.independently_realized_objective_targets, 2);
    assert_eq!(report.objective_predictions, 2);
    assert!(report.objective_mean_error_millionths < 250_000);
    assert!(report.objective_error_improvement_millionths > 0);
    assert_eq!(report.ranked_state_action_pairs, 1);
    assert_eq!(report.correctly_ranked_state_action_pairs, 1);
    assert_eq!(report.mean_baseline_correctly_ranked_state_action_pairs, 0);
    assert!(report.ranking_accuracy_improvement_millionths > 0);
    assert!(report.objective_calibration_gate_passed);
    report.validate().unwrap();

    let policy_action = batch
        .rows
        .iter()
        .find(|row| row.action.option_id == "held-out-action-0")
        .unwrap()
        .action
        .content_sha256()
        .unwrap();
    let policy_actions = BTreeSet::from([policy_action]);
    let contract = crate::learner::GraphLearnerContract::default();
    let replay =
        crate::learner::GraphReplayPlan::build(&contract, &batch, &policy_actions, 0).unwrap();
    assert_eq!(
        replay,
        crate::learner::GraphReplayPlan::build(&contract, &batch, &policy_actions, 0).unwrap()
    );
    assert_eq!(replay.ordinary_draws, 16);
    assert_eq!(replay.prioritized_draws, 48);
    assert_eq!(
        replay
            .rows
            .iter()
            .map(crate::learner::GraphReplayRowPriority::total_draws)
            .sum::<u64>(),
        64
    );
    let policy_draws = replay
        .rows
        .iter()
        .filter(|row| row.policy_relevant)
        .map(|row| row.prioritized_draws)
        .sum::<u64>();
    let ordinary_action_draws = replay
        .rows
        .iter()
        .filter(|row| !row.policy_relevant)
        .map(|row| row.prioritized_draws)
        .sum::<u64>();
    assert!(policy_draws > ordinary_action_draws);
    let mut ordinarily_covered = BTreeSet::new();
    for round in 0..replay.maximum_ordinary_starvation_rounds {
        let rotated =
            crate::learner::GraphReplayPlan::build(&contract, &batch, &policy_actions, round)
                .unwrap();
        ordinarily_covered.extend(
            rotated
                .rows
                .iter()
                .filter(|row| row.ordinary_draws > 0)
                .map(|row| row.expansion_sha256),
        );
    }
    assert_eq!(ordinarily_covered.len(), batch.rows.len());
    let prioritized_snapshot = crate::learner::ExactGraphTableLearner
        .fit_prioritized(&contract, &batch, &replay)
        .unwrap();
    assert!(
        prioritized_snapshot
            .generalized_objective_prediction(&[50.0], policy_action)
            .is_some()
    );
    let mut detached_replay = replay;
    detached_replay.rows[0].prioritized_draws += 1;
    assert!(detached_replay.validate(&contract, &batch).is_err());

    let comparison =
        crate::learner::GraphTreatmentComparisonReport::build(&contract, &batch).unwrap();
    assert_eq!(comparison.metrics.len(), 3);
    assert!(
        !comparison
            .metrics
            .iter()
            .find(|metrics| {
                metrics.treatment == crate::learner::GraphObjectiveTreatment::DiscreteActionMean
            })
            .unwrap()
            .passed_gate
    );
    assert!(
        comparison
            .metrics
            .iter()
            .find(|metrics| {
                metrics.treatment == crate::learner::GraphObjectiveTreatment::StateKnn
            })
            .unwrap()
            .passed_gate
    );
    assert_eq!(
        comparison.selected_treatment,
        Some(crate::learner::GraphObjectiveTreatment::StateKnn)
    );
    comparison.validate().unwrap();
}

#[test]
fn scheduled_action_completion_requires_and_consumes_the_exact_lease() {
    let (mut graph, transition, route) = graph_and_transition();
    let expansion_sha256 = graph
        .register_action_expansion(graph.root(), transition.value_sample.action.clone())
        .unwrap();
    assert!(graph.expansion_is_schedulable(expansion_sha256, 0));
    graph
        .lease_action_expansion(expansion_sha256, Digest([6; 32]), 0, 3)
        .unwrap();
    assert!(!graph.expansion_is_schedulable(expansion_sha256, 2));
    assert!(
        graph
            .admit_leased_completed_expansion(
                transition.clone(),
                route.clone(),
                17,
                ExpansionEvidenceAuthority::Executable,
                Digest([7; 32]),
            )
            .is_err()
    );

    let admission = graph
        .admit_leased_completed_expansion(
            transition,
            route,
            17,
            ExpansionEvidenceAuthority::Executable,
            Digest([6; 32]),
        )
        .unwrap();
    assert_eq!(admission.expansion_sha256, expansion_sha256);
    assert!(!graph.expansion_is_schedulable(expansion_sha256, 4));
    graph.validate().unwrap();

    let restored = StateGraph::decode(&graph.encode().unwrap()).unwrap();
    assert!(!restored.expansion_is_schedulable(expansion_sha256, 4));
}

#[test]
fn expired_and_retryable_work_has_an_explicit_lifecycle() {
    let (mut graph, transition, _) = graph_and_transition();
    let expansion_sha256 = graph
        .register_action_expansion(graph.root(), transition.value_sample.action)
        .unwrap();
    graph
        .lease_action_expansion(expansion_sha256, Digest([6; 32]), 0, 3)
        .unwrap();
    assert!(graph.expansion_is_schedulable(expansion_sha256, 3));
    graph
        .lease_action_expansion(expansion_sha256, Digest([7; 32]), 3, 6)
        .unwrap();
    assert!(
        graph
            .mark_expansion_retryable(expansion_sha256, Digest([6; 32]), 1)
            .is_err()
    );
    graph
        .mark_expansion_retryable(expansion_sha256, Digest([7; 32]), 1)
        .unwrap();
    assert!(graph.expansion_is_schedulable(expansion_sha256, 4));
    graph.validate().unwrap();
}

#[test]
fn binary_restart_preserves_graph_identity_and_pending_truth() {
    let (mut graph, transition, route) = graph_and_transition();
    graph
        .admit_completed_expansion(
            transition,
            route,
            17,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap();

    let encoded = graph.encode().unwrap();
    let restored = StateGraph::decode(&encoded).unwrap();

    assert_eq!(restored, graph);
    assert_eq!(
        restored.content_sha256().unwrap(),
        graph.content_sha256().unwrap()
    );
    assert_eq!(restored.best_terminal_path(), graph.best_terminal_path());
    assert_eq!(restored.completed_transitions().count(), 1);
}

#[test]
fn duplicate_completed_evidence_does_not_duplicate_graph_truth() {
    let (mut graph, transition, route) = graph_and_transition();
    graph
        .admit_completed_expansion(
            transition.clone(),
            route.clone(),
            17,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap();
    let duplicate = graph
        .admit_completed_expansion(
            transition,
            route,
            17,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap();

    assert!(duplicate.duplicate);
    assert_eq!(graph.node_count(), 3);
    assert_eq!(graph.expansion_count(), 1);
    assert_eq!(graph.segment_count(), 2);
}

#[test]
fn one_deterministic_action_retains_distinct_learner_labels_as_evidence() {
    let (mut graph, transition, route) = graph_and_transition();
    let first = graph
        .admit_completed_expansion(
            transition.clone(),
            route.clone(),
            17,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap();
    let mut relabeled = transition;
    relabeled.value_sample.reward = -4.0;
    relabeled.validate().unwrap();
    let second = graph
        .admit_completed_expansion(relabeled, route, 18, ExpansionEvidenceAuthority::Executable)
        .unwrap();

    assert_eq!(second.expansion_sha256, first.expansion_sha256);
    assert!(!second.duplicate);
    assert_eq!(graph.expansion_count(), 1);
    assert_eq!(graph.completed_transitions().count(), 2);
    graph.validate().unwrap();
}

#[test]
fn learner_only_evidence_requires_explicit_executable_promotion() {
    let (mut graph, transition, route) = graph_and_transition();
    let admission = graph
        .admit_completed_expansion(
            transition.clone(),
            route.clone(),
            17,
            ExpansionEvidenceAuthority::LearnerEvidenceOnly,
        )
        .unwrap();
    assert!(!graph.node(admission.target).unwrap().restoration.executable);

    let promoted = graph
        .admit_completed_expansion(
            transition,
            route,
            17,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap();
    assert!(promoted.duplicate);
    assert!(promoted.authority_promoted);
    assert!(graph.node(admission.target).unwrap().restoration.executable);
    assert!(matches!(
        graph.expansion(admission.expansion_sha256).unwrap().status,
        ActionExpansionStatus::Completed {
            authority: ExpansionEvidenceAuthority::Executable,
            ..
        }
    ));
    graph.validate().unwrap();
}

#[test]
fn validated_transposition_relaxes_descendants_without_deleting_evidence() {
    let (mut graph, transition, mut route) = graph_and_transition();
    let first = graph
        .admit_completed_expansion(
            transition,
            route.clone(),
            17,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap();
    let interior = graph.nodes().find(|node| node.root_ticks == 4).unwrap().id;
    let slower_equivalent = first.target;
    let before = graph.node(slower_equivalent).unwrap().state.clone();
    route.frames.extend(vec![InputFrame::default(); 4]);
    let execution = OptionExecution::capture(
        "continue".into(),
        OptionType::Move,
        BTreeMap::new(),
        4,
        4,
        OptionCondition::DurationElapsed,
        Vec::new(),
        OptionEndReason::Completed,
        &route,
        TapeRange {
            start_frame: before.tape_frame,
            end_frame_exclusive: before.tape_frame + 4,
        },
    )
    .unwrap();
    let after = advanced_state(&before, 4);
    let mut descendant = OptionTransitionSample::capture(
        graph.identity.feature_schema_sha256,
        slower_equivalent.route_checkpoint_sha256,
        route_checkpoint_sha256(graph.identity.root_checkpoint_sha256, &route).unwrap(),
        before,
        after,
        execution,
        &route,
        -4.0,
        false,
        |state| Ok::<_, &'static str>(vec![state.tape_frame as f32]),
    )
    .unwrap();
    descendant.execution_authority_sha256 = graph.identity.execution_authority_sha256;
    descendant.validate().unwrap();
    let descendant = graph
        .admit_completed_expansion(
            descendant,
            route,
            18,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap()
        .target;
    let nodes_before = graph.node_count();
    let segments_before = graph.segment_count();
    let slower_incoming = graph
        .node(slower_equivalent)
        .unwrap()
        .incoming_segments
        .clone();

    let proof = FutureEquivalenceProof::new(
        interior,
        slower_equivalent,
        graph.identity.future_equivalence_validator_sha256,
        Digest([9; 32]),
    )
    .unwrap();
    assert!(graph.admit_future_equivalence_proof(proof.clone()).unwrap());
    assert!(!graph.admit_future_equivalence_proof(proof).unwrap());

    assert_eq!(
        graph.canonical_restoration_node(slower_equivalent).unwrap(),
        interior
    );
    assert_eq!(graph.relaxed_root_ticks_to(slower_equivalent).unwrap(), 4);
    assert_eq!(graph.relaxed_root_ticks_to(descendant).unwrap(), 8);
    assert_eq!(graph.node(descendant).unwrap().root_ticks, 12);
    assert_eq!(graph.node_count(), nodes_before);
    assert_eq!(graph.segment_count(), segments_before);
    assert_eq!(
        graph.node(slower_equivalent).unwrap().incoming_segments,
        slower_incoming
    );
    let scheduled = crate::scheduler::rank_schedulable_nodes(
        &graph,
        crate::scheduler::SearchRegime::Discovery,
        u64::MAX,
        31,
        0,
    )
    .unwrap();
    assert!(
        scheduled
            .iter()
            .any(|entry| entry.node == interior && entry.root_ticks == 4)
    );
    assert!(
        scheduled
            .iter()
            .any(|entry| entry.node == descendant && entry.root_ticks == 8)
    );
    assert!(
        scheduled
            .iter()
            .all(|entry| entry.node != slower_equivalent)
    );

    let restored = StateGraph::decode(&graph.encode().unwrap()).unwrap();
    assert_eq!(restored, graph);
    assert_eq!(restored.future_equivalence_proof_count(), 1);
    assert_eq!(restored.relaxed_root_ticks_to(descendant).unwrap(), 8);
}

#[test]
fn transposition_proof_rejects_tampering_and_absent_nodes() {
    let (mut graph, transition, route) = graph_and_transition();
    let admitted = graph
        .admit_completed_expansion(
            transition,
            route,
            17,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap();
    let mut tampered = FutureEquivalenceProof::new(
        graph.root(),
        admitted.target,
        graph.identity.future_equivalence_validator_sha256,
        Digest([9; 32]),
    )
    .unwrap();
    let unauthorized = FutureEquivalenceProof::new(
        graph.root(),
        admitted.target,
        Digest([8; 32]),
        Digest([9; 32]),
    )
    .unwrap();
    assert!(graph.admit_future_equivalence_proof(unauthorized).is_err());
    tampered.native_evidence_sha256 = Digest([7; 32]);
    assert!(graph.admit_future_equivalence_proof(tampered).is_err());
    assert_eq!(graph.future_equivalence_proof_count(), 0);

    let absent = FutureEquivalenceProof::new(
        admitted.target,
        ExactStateId {
            route_checkpoint_sha256: Digest([10; 32]),
            state_sha256: Digest([11; 32]),
        },
        graph.identity.future_equivalence_validator_sha256,
        Digest([9; 32]),
    )
    .unwrap();
    assert!(graph.admit_future_equivalence_proof(absent).is_err());
    assert_eq!(graph.future_equivalence_proof_count(), 0);
}

#[test]
fn durable_graph_and_exported_report_share_one_content_identity() {
    let (mut graph, mut transition, route) = graph_and_transition();
    terminalize(&mut transition);
    graph
        .admit_completed_expansion(
            transition,
            route,
            17,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap();
    let directory = std::env::temp_dir().join(format!(
        "dusklight-state-graph-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));

    let stored = crate::persistence::persist_state_graph(&directory, &graph).unwrap();
    let duplicate = crate::persistence::persist_state_graph(&directory, &graph).unwrap();
    let restored = crate::persistence::read_state_graph(&stored.path, stored.graph_sha256).unwrap();
    let report = crate::reporting::GraphSearchReport::from_graph(&restored).unwrap();

    assert_eq!(stored, duplicate);
    assert_eq!(restored, graph);
    assert_eq!(report.graph_sha256, stored.graph_sha256);
    assert_eq!(report.graph_identity, graph.identity);
    assert_eq!(report.completed_expansions, 1);
    assert_eq!(report.best_terminal, graph.best_terminal_path().cloned());
    report.validate_against(&graph).unwrap();
    let exported: serde_json::Value =
        serde_json::from_slice(&report.to_pretty_json().unwrap()).unwrap();
    assert_eq!(
        exported["graph_sha256"],
        serde_json::to_value(stored.graph_sha256).unwrap()
    );

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn restoration_plan_requires_the_complete_typed_state_before_execution() {
    let (graph, _, _) = graph_and_transition();
    let plan = graph.restoration_plan(graph.root()).unwrap();
    let expected = graph.node(graph.root()).unwrap().state.clone();
    let receipt = graph.validate_restored_state(&plan, &expected).unwrap();

    assert_eq!(receipt.restoration_plan_sha256, plan.plan_sha256);
    assert_eq!(receipt.node, graph.root());
    assert_eq!(
        graph.restoration_route(&plan).unwrap(),
        graph.route(graph.root().route_checkpoint_sha256).unwrap()
    );

    let mut wrong_state = expected;
    wrong_state.state_identity[0] ^= 1;
    wrong_state.validate().unwrap();
    assert!(graph.validate_restored_state(&plan, &wrong_state).is_err());

    let mut tampered_plan = plan;
    tampered_plan.route.tape_frames += 1;
    assert!(graph.restoration_route(&tampered_plan).is_err());
}

#[test]
fn one_expansion_identity_flows_from_lease_through_worker_learner_and_report() {
    let (mut graph, mut transition, route) = graph_and_transition();
    terminalize(&mut transition);
    let primitive = transition.value_sample.action.clone();
    let primitive_sha256 = primitive.content_sha256().unwrap();
    let mut tactics = crate::tactics::GraphTacticCatalog::new([primitive.clone()]).unwrap();
    tactics
        .promote(crate::tactics::PromotedGraphTactic {
            descriptor: OptionActionDescriptor {
                option_id: "learned/move-twice".into(),
                option_type: OptionType::Custom("composition".into()),
                parameters: BTreeMap::new(),
            },
            primitive_components: vec![primitive_sha256],
            held_out_evidence_sha256: Digest([12; 32]),
        })
        .unwrap();
    assert_eq!(tactics.primitive_count(), 1);
    assert_eq!(tactics.promoted_count(), 1);
    assert_eq!(tactics.actions().count(), 2);
    let root = graph.root();
    let registered = tactics
        .register_applicable(&mut graph, root, |action| {
            action.option_id == primitive.option_id
        })
        .unwrap();
    assert_eq!(registered.len(), 1);

    let priorities = crate::scheduler::GraphPrioritySnapshot::cold_start(&graph).unwrap();
    let config = crate::scheduler::ExpansionSchedulerConfig {
        schema: crate::scheduler::EXPANSION_SCHEDULER_CONFIG_SCHEMA_V1.into(),
        regime: crate::scheduler::SearchRegime::Discovery,
        seed: 41,
        generation: 3,
        lease_generations: 2,
    };
    let lease_sha256 = Digest([13; 32]);
    let queue =
        crate::scheduler::lease_replayed_expansion(&mut graph, &config, &priorities, lease_sha256)
            .unwrap();
    let job =
        crate::worker_pool::GraphExpansionJob::from_leased_graph(&graph, &queue, lease_sha256)
            .unwrap();
    let job_bytes = serde_cbor::to_vec(&job).unwrap();
    assert!(job_bytes.len() < 2_048, "job bytes: {}", job_bytes.len());
    let admission = crate::worker_pool::admit_graph_expansion_completion(
        &mut graph,
        &job,
        crate::worker_pool::GraphExpansionCompletion {
            job_sha256: job.job_sha256,
            transition,
            route,
            episode_group: 17,
            authority: ExpansionEvidenceAuthority::Executable,
        },
    )
    .unwrap();
    let learner = crate::learner::GraphLearningBatch::from_graph(&graph).unwrap();
    let report = crate::reporting::GraphSearchReport::from_graph(&graph).unwrap();
    let directory = std::env::temp_dir().join(format!(
        "dusklight-expansion-identity-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let stored = crate::persistence::persist_state_graph(&directory, &graph).unwrap();

    assert_eq!(admission.expansion_sha256, job.expansion_sha256);
    assert_eq!(learner.rows.len(), 1);
    assert_eq!(learner.rows[0].expansion_sha256, job.expansion_sha256);
    assert_eq!(learner.rows[0].source, job.source);
    assert_eq!(
        learner.rows[0].source_state.content_sha256().unwrap(),
        job.source.state_sha256
    );
    assert_eq!(learner.graph_sha256, report.graph_sha256);
    assert_eq!(report.graph_sha256, stored.graph_sha256);
    assert_eq!(report.completed_expansions, 1);
    assert_eq!(report.untried_expansions, 0);
    assert_eq!(
        report.best_terminal.as_ref().map(|path| path.terminal),
        Some(admission.target)
    );

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn detached_evidence_is_rejected_before_mutating_the_graph() {
    let (mut graph, mut transition, route) = graph_and_transition();
    transition.source_checkpoint_sha256 = Digest([9; 32]);
    transition.value_sample.source_checkpoint_sha256 = Digest([9; 32]);

    assert!(
        graph
            .admit_completed_expansion(
                transition,
                route,
                17,
                ExpansionEvidenceAuthority::Executable,
            )
            .is_err()
    );
    assert_eq!(graph.node_count(), 1);
    assert_eq!(graph.expansion_count(), 0);
    assert_eq!(graph.segment_count(), 0);
}
