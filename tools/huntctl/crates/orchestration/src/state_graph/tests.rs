use super::*;
use dusklight_automation_contracts::tape::{InputFrame, InputTape};
use dusklight_control::option_execution::{
    OptionCondition, OptionEndReason, OptionExecution, OptionType, TapeRange,
};
use dusklight_evidence::native_episode_shard::NativeEpisodeShard;
use dusklight_learning::fact_snapshot::{FactPhase, FactSnapshot};
use dusklight_learning::option_transition::{OptionIntermediateBoundary, OptionTransitionSample};
use std::collections::BTreeMap;

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
