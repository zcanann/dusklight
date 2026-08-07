use super::*;
use dusklight_automation_contracts::tape::{InputFrame, InputTape};
use dusklight_control::option_execution::{
    OptionCondition, OptionEndReason, OptionExecution, OptionType, TapeRange,
};
use dusklight_evidence::native_episode_shard::NativeEpisodeShard;
use dusklight_learning::fact_snapshot::{FactPhase, FactSnapshot};
use dusklight_learning::option_transition::{
    AuthenticatedOptionTransition, OptionIntermediateBoundary, OptionTransitionSample,
};
use sha2::Sha256;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

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
    transition.intermediate_boundaries =
        vec![OptionIntermediateBoundary::capture(&transition.execution, 4, interior).unwrap()];
    transition.validate().unwrap();
    (graph, transition, route)
}

#[test]
fn process_validation_token_survives_checked_mutation_and_rejects_identity_drift() {
    let (mut graph, transition, route) = graph_and_transition();
    let token = graph.validation_token().unwrap();
    graph
        .admit_completed_expansion(
            transition,
            route,
            17,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap();

    graph.validated_with_token(&token).unwrap();
    graph.validate().unwrap();

    let mut detached = graph;
    detached.identity.objective_sha256 = Digest([9; 32]);
    assert!(detached.validated_with_token(&token).is_err());
}

#[test]
fn combined_route_identity_preserves_both_persisted_digests() {
    let (graph, _, route) = graph_and_transition();
    let (route_sha256, tape_sha256) =
        super::route_and_tape_sha256(graph.identity.root_checkpoint_sha256, &route).unwrap();
    assert_eq!(
        route_sha256,
        super::route_checkpoint_sha256(graph.identity.root_checkpoint_sha256, &route).unwrap()
    );
    assert_eq!(tape_sha256, super::tape_sha256(&route).unwrap());
}

#[test]
fn shared_node_state_preserves_legacy_serialization_bytes() {
    #[derive(serde::Serialize)]
    struct LegacyStateGraphNode<'a> {
        id: ExactStateId,
        state: &'a FactSnapshot,
        terminal: bool,
        root_ticks: u64,
        restoration: &'a RestorationLocator,
        incoming_segments: &'a BTreeSet<Digest>,
        outgoing_segments: &'a BTreeSet<Digest>,
        outgoing_expansions: &'a BTreeSet<Digest>,
    }

    let state = fixture_state();
    let shared = Arc::new(state.clone());
    assert_eq!(
        serde_cbor::to_vec(&state).unwrap(),
        serde_cbor::to_vec(&shared).unwrap()
    );
    assert_eq!(
        serde_json::to_vec(&state).unwrap(),
        serde_json::to_vec(&shared).unwrap()
    );
    let (graph, _, _) = graph_and_transition();
    let node = graph.node(graph.root()).unwrap();
    let legacy = LegacyStateGraphNode {
        id: node.id,
        state: node.state.as_ref(),
        terminal: node.terminal,
        root_ticks: node.root_ticks,
        restoration: &node.restoration,
        incoming_segments: &node.incoming_segments,
        outgoing_segments: &node.outgoing_segments,
        outgoing_expansions: &node.outgoing_expansions,
    };
    assert_eq!(
        serde_cbor::to_vec(node).unwrap(),
        serde_cbor::to_vec(&legacy).unwrap()
    );
}

#[test]
fn shared_completed_evidence_preserves_legacy_serialization_bytes() {
    #[derive(serde::Serialize)]
    struct LegacyCompletedExpansionEvidence<'a> {
        episode_group: u64,
        authority: ExpansionEvidenceAuthority,
        transition: &'a OptionTransitionSample,
    }

    let (mut graph, transition, route) = graph_and_transition();
    let admission = graph
        .admit_completed_expansion(
            transition,
            route,
            17,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap();
    let expansion = graph.expansion(admission.expansion_sha256).unwrap();
    let ActionExpansionStatus::Completed { evidence, .. } = &expansion.status else {
        panic!("admitted expansion is not complete");
    };
    let shared = evidence.get(&admission.evidence_sha256).unwrap();
    let legacy = LegacyCompletedExpansionEvidence {
        episode_group: shared.episode_group,
        authority: shared.authority,
        transition: shared.transition.as_ref(),
    };

    assert_eq!(
        serde_cbor::to_vec(shared).unwrap(),
        serde_cbor::to_vec(&legacy).unwrap()
    );
    assert_eq!(
        serde_json::to_vec(shared).unwrap(),
        serde_json::to_vec(&legacy).unwrap()
    );
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
    assert_eq!(graph.completed_executable_expansion_count(), 1);
    assert_ne!(
        graph.completed_executable_expansion_set_sha256(),
        Digest::ZERO
    );
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
fn graph_transaction_clone_shares_immutable_storage_and_isolates_mutation() {
    let (graph, transition, route) = graph_and_transition();
    let evidence_sha256 = transition.replay_identity_sha256().unwrap();
    let mut transaction = graph.clone();

    assert!(graph.nodes.ptr_eq(&transaction.nodes));
    assert!(graph.expansions.ptr_eq(&transaction.expansions));
    assert!(graph.segments.ptr_eq(&transaction.segments));
    assert!(graph.routes.ptr_eq(&transaction.routes));
    assert!(
        graph
            .future_equivalence_proofs
            .ptr_eq(&transaction.future_equivalence_proofs)
    );

    let admission = transaction
        .admit_completed_expansion(
            transition,
            route,
            17,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap();
    assert_eq!(admission.evidence_sha256, evidence_sha256);

    assert_eq!(graph.node_count(), 1);
    assert_eq!(graph.expansion_count(), 0);
    assert_eq!(graph.segment_count(), 0);
    assert!(!graph.nodes.ptr_eq(&transaction.nodes));
    assert!(!graph.expansions.ptr_eq(&transaction.expansions));
    assert!(!graph.segments.ptr_eq(&transaction.segments));
    assert!(!graph.routes.ptr_eq(&transaction.routes));
    assert!(
        Arc::ptr_eq(
            &graph.nodes.get(&graph.root()).unwrap().state,
            &transaction.nodes.get(&transaction.root()).unwrap().state,
        ),
        "topology mutation must not clone its large immutable typed state"
    );
    assert!(
        graph
            .future_equivalence_proofs
            .ptr_eq(&transaction.future_equivalence_proofs),
        "an untouched graph collection must remain structurally shared"
    );
    graph.validate().unwrap();
    transaction.validate().unwrap();
}

#[test]
fn authenticated_admission_is_content_identical_to_raw_admission() {
    let (graph, transition, route) = graph_and_transition();
    let mut raw_graph = graph.clone();
    let mut authenticated_graph = graph;
    let authenticated = AuthenticatedOptionTransition::new(transition.clone()).unwrap();

    let raw_admission = raw_graph
        .admit_completed_expansion(
            transition,
            route.clone(),
            17,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap();
    let authenticated_admission = authenticated_graph
        .admit_authenticated_completed_expansion(
            authenticated,
            route,
            17,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap();

    assert_eq!(authenticated_admission, raw_admission);
    assert_eq!(authenticated_graph, raw_graph);
    assert_eq!(
        authenticated_graph.encode().unwrap(),
        raw_graph.encode().unwrap()
    );
    assert_eq!(
        authenticated_graph.content_sha256().unwrap(),
        raw_graph.content_sha256().unwrap()
    );
}

#[test]
fn grown_graph_transaction_clone_remains_structurally_constant() {
    let (mut graph, template, route) = graph_and_transition();
    for index in 0..33 {
        let mut transition = template.clone();
        let option_id = format!("move-{index:03}");
        transition.execution.option_id.clone_from(&option_id);
        transition.value_sample.action.option_id = option_id;
        for boundary in &mut transition.intermediate_boundaries {
            *boundary = OptionIntermediateBoundary::capture(
                &transition.execution,
                boundary.offset_ticks,
                boundary.state.clone(),
            )
            .unwrap();
        }
        transition.validate().unwrap();
        graph
            .admit_completed_expansion(
                transition,
                route.clone(),
                index + 1,
                ExpansionEvidenceAuthority::Executable,
            )
            .unwrap();
    }
    assert_eq!(graph.expansion_count(), 33);
    assert_eq!(graph.segment_count(), 66);

    let transaction = graph.clone();
    assert!(graph.nodes.ptr_eq(&transaction.nodes));
    assert!(graph.expansions.ptr_eq(&transaction.expansions));
    assert!(graph.segments.ptr_eq(&transaction.segments));
    assert!(graph.routes.ptr_eq(&transaction.routes));
    assert!(
        graph
            .future_equivalence_proofs
            .ptr_eq(&transaction.future_equivalence_proofs)
    );
    assert_eq!(graph, transaction);
}

#[test]
fn structurally_shared_graph_preserves_btree_serialization_bytes() {
    #[derive(serde::Serialize)]
    struct BTreeBackedStateGraph<'a> {
        schema: &'a str,
        identity: &'a StateGraphIdentity,
        root: ExactStateId,
        root_route_frames: u64,
        nodes: BTreeMap<ExactStateId, StateGraphNode>,
        expansions: BTreeMap<Digest, ActionExpansion>,
        segments: BTreeMap<Digest, ObservedSegment>,
        routes: BTreeMap<Digest, InputTape>,
        future_equivalence_proofs: BTreeMap<Digest, FutureEquivalenceProof>,
        best_terminal: &'a Option<TerminalPath>,
    }

    let (mut graph, transition, route) = graph_and_transition();
    graph
        .admit_completed_expansion(
            transition,
            route,
            17,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap();
    let legacy = BTreeBackedStateGraph {
        schema: &graph.schema,
        identity: &graph.identity,
        root: graph.root,
        root_route_frames: graph.root_route_frames,
        nodes: graph
            .nodes
            .iter()
            .map(|(key, value)| (*key, value.as_ref().clone()))
            .collect(),
        expansions: graph
            .expansions
            .iter()
            .map(|(key, value)| (*key, value.as_ref().clone()))
            .collect(),
        segments: graph
            .segments
            .iter()
            .map(|(key, value)| (*key, value.as_ref().clone()))
            .collect(),
        routes: graph
            .routes
            .iter()
            .map(|(key, value)| (*key, value.as_ref().clone()))
            .collect(),
        future_equivalence_proofs: graph
            .future_equivalence_proofs
            .iter()
            .map(|(key, value)| (*key, value.as_ref().clone()))
            .collect(),
        best_terminal: &graph.best_terminal,
    };

    assert_eq!(
        graph.encode().unwrap(),
        serde_cbor::to_vec(&legacy).unwrap()
    );
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
    let intermediate_boundaries = (4..40)
        .step_by(4)
        .map(|offset| {
            let state = advanced_state(&before, offset);
            OptionIntermediateBoundary::capture(&long_transition.execution, offset as u32, state)
                .unwrap()
        })
        .collect();
    long_transition.intermediate_boundaries = intermediate_boundaries;
    terminalize(&mut long_transition);
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

    let mut counterfactual_targets = Vec::with_capacity(interior.len());
    for (index, source) in interior.iter().copied().enumerate() {
        let before = graph.node(source).unwrap().state.as_ref().clone();
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
        counterfactual_targets.push(counterfactual.target);
    }

    for source in &interior {
        let node = graph.node(*source).unwrap();
        assert_eq!(node.outgoing_expansions.len(), 1);
        let expansion = graph
            .expansion(*node.outgoing_expansions.first().unwrap())
            .unwrap();
        assert!(matches!(
            expansion.status,
            ActionExpansionStatus::Completed { .. }
        ));
    }
    let scheduled = crate::scheduler::rank_schedulable_nodes(
        &graph,
        crate::scheduler::SearchRegime::Optimization,
        u64::MAX,
        7,
        1,
    )
    .unwrap();
    let scheduled_with_exact_return = scheduled
        .iter()
        .filter(|entry| entry.exact_terminal_ticks_to_go.is_some())
        .count();
    assert_eq!(
        scheduled_with_exact_return,
        interior.len() + counterfactual_targets.len()
    );
    assert!(interior.iter().all(|source| {
        scheduled.iter().any(|entry| {
            entry.node == *source && entry.exact_terminal_ticks_to_go == Some(40 - entry.root_ticks)
        })
    }));
    assert!(counterfactual_targets.iter().all(|target| {
        scheduled.iter().any(|entry| {
            entry.node == *target && entry.exact_terminal_ticks_to_go == Some(40 - entry.root_ticks)
        })
    }));
    graph.validate().unwrap();
}

#[test]
fn exact_terminal_returns_cover_route_specific_interior_nodes() {
    let (mut graph, mut transition, route) = graph_and_transition();
    let terminal_route = route.clone();
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

    let interior = graph.nodes().find(|node| node.root_ticks == 4).unwrap().id;
    let continuation = graph
        .exact_terminal_continuation(interior)
        .unwrap()
        .unwrap();
    assert_eq!(continuation.source, interior);
    assert_eq!(continuation.source_prefix_ticks, 4);
    assert_eq!(continuation.ticks_to_terminal, 4);
    let source_frames = graph
        .route(interior.route_checkpoint_sha256)
        .unwrap()
        .frames
        .len();
    assert_eq!(
        continuation.tape.frames,
        terminal_route.frames[source_frames..]
    );
    assert_eq!(
        continuation.terminal_route_checkpoint_sha256,
        graph.best_terminal_path().unwrap().route_checkpoint_sha256
    );
    assert!(
        graph
            .exact_terminal_continuation(graph.best_terminal_path().unwrap().terminal)
            .is_err()
    );
}

#[test]
fn optimization_schedules_interiors_from_every_authenticated_terminal_route() {
    let (mut graph, mut best_transition, best_route) = graph_and_transition();
    terminalize(&mut best_transition);
    graph
        .admit_completed_expansion(
            best_transition,
            best_route,
            17,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap();

    let before = graph.node(graph.root()).unwrap().state.as_ref().clone();
    let mut alternate_route = graph
        .route(graph.root().route_checkpoint_sha256)
        .unwrap()
        .clone();
    let mut alternate_frames = vec![InputFrame::default(); 12];
    alternate_frames[0].owned_ports = 1;
    alternate_route.frames.extend(alternate_frames);
    let execution = OptionExecution::capture(
        "alternate-terminal".into(),
        OptionType::Move,
        BTreeMap::new(),
        12,
        12,
        OptionCondition::DurationElapsed,
        Vec::new(),
        OptionEndReason::Completed,
        &alternate_route,
        TapeRange {
            start_frame: before.tape_frame,
            end_frame_exclusive: before.tape_frame + 12,
        },
    )
    .unwrap();
    let alternate_interior_states = [advanced_state(&before, 4), advanced_state(&before, 8)];
    let mut alternate_transition = OptionTransitionSample::capture(
        graph.identity.feature_schema_sha256,
        graph.root().route_checkpoint_sha256,
        route_checkpoint_sha256(graph.identity.root_checkpoint_sha256, &alternate_route).unwrap(),
        before.clone(),
        advanced_state(&before, 12),
        execution,
        &alternate_route,
        -12.0,
        false,
        |state| Ok::<_, &'static str>(vec![state.tape_frame as f32]),
    )
    .unwrap();
    alternate_transition.execution_authority_sha256 = graph.identity.execution_authority_sha256;
    let intermediate_boundaries = alternate_interior_states
        .into_iter()
        .enumerate()
        .map(|(index, state)| {
            OptionIntermediateBoundary::capture(
                &alternate_transition.execution,
                4 + index as u32 * 4,
                state,
            )
            .unwrap()
        })
        .collect();
    alternate_transition.intermediate_boundaries = intermediate_boundaries;
    terminalize(&mut alternate_transition);
    alternate_transition.validate().unwrap();
    graph
        .admit_completed_expansion(
            alternate_transition,
            alternate_route,
            18,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap();

    assert_eq!(
        graph.best_terminal_path().unwrap().root_to_terminal_ticks,
        8
    );
    let alternate_interior = graph
        .nodes()
        .find(|node| node.root_ticks == 8 && !node.terminal)
        .unwrap()
        .id;
    let alternate_continuation = graph
        .exact_terminal_continuation(alternate_interior)
        .unwrap()
        .unwrap();
    assert_eq!(alternate_continuation.source_prefix_ticks, 8);
    assert_eq!(alternate_continuation.ticks_to_terminal, 4);
    assert_eq!(alternate_continuation.tape.frames.len(), 4);
    assert_ne!(
        alternate_continuation.terminal_route_checkpoint_sha256,
        graph.best_terminal_path().unwrap().route_checkpoint_sha256
    );
    let scheduled = crate::scheduler::rank_schedulable_nodes(
        &graph,
        crate::scheduler::SearchRegime::Optimization,
        u64::MAX,
        31,
        0,
    )
    .unwrap();
    assert!(scheduled.iter().any(|entry| {
        entry.node == alternate_interior && entry.exact_terminal_ticks_to_go == Some(4)
    }));
}

#[test]
fn optimization_keeps_terminal_interior_when_transposition_prefers_another_route() {
    let (mut graph, mut terminal_transition, terminal_route) = graph_and_transition();
    terminalize(&mut terminal_transition);
    graph
        .admit_completed_expansion(
            terminal_transition,
            terminal_route,
            17,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap();
    let successful_interior = graph
        .nodes()
        .find(|node| node.root_ticks == 4 && !node.terminal)
        .unwrap()
        .id;

    let before = graph.node(graph.root()).unwrap().state.as_ref().clone();
    let mut alternate_route = graph
        .route(graph.root().route_checkpoint_sha256)
        .unwrap()
        .clone();
    alternate_route.frames.extend([
        InputFrame {
            owned_ports: 1,
            ..InputFrame::default()
        },
        InputFrame::default(),
    ]);
    let execution = OptionExecution::capture(
        "alternate".into(),
        OptionType::Move,
        BTreeMap::new(),
        2,
        2,
        OptionCondition::DurationElapsed,
        Vec::new(),
        OptionEndReason::Completed,
        &alternate_route,
        TapeRange {
            start_frame: before.tape_frame,
            end_frame_exclusive: before.tape_frame + 2,
        },
    )
    .unwrap();
    let mut alternate_transition = OptionTransitionSample::capture(
        graph.identity.feature_schema_sha256,
        graph.root().route_checkpoint_sha256,
        route_checkpoint_sha256(graph.identity.root_checkpoint_sha256, &alternate_route).unwrap(),
        before.clone(),
        advanced_state(&before, 2),
        execution,
        &alternate_route,
        -2.0,
        false,
        |state| Ok::<_, &'static str>(vec![state.tape_frame as f32]),
    )
    .unwrap();
    alternate_transition.execution_authority_sha256 = graph.identity.execution_authority_sha256;
    alternate_transition.validate().unwrap();
    let alternate = graph
        .admit_completed_expansion(
            alternate_transition,
            alternate_route,
            18,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap()
        .target;

    graph
        .admit_future_equivalence_proof(
            FutureEquivalenceProof::new(
                alternate,
                successful_interior,
                graph.identity.future_equivalence_validator_sha256,
                Digest([10; 32]),
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(
        graph
            .canonical_restoration_node(successful_interior)
            .unwrap(),
        alternate
    );
    let exact_returns = graph.exact_terminal_returns().unwrap();
    assert_eq!(exact_returns.get(&successful_interior), Some(&4));
    assert_eq!(exact_returns.get(&alternate), None);

    let scheduled = crate::scheduler::rank_schedulable_nodes(
        &graph,
        crate::scheduler::SearchRegime::Optimization,
        u64::MAX,
        31,
        0,
    )
    .unwrap();
    let supported = scheduled
        .iter()
        .find(|entry| entry.node == successful_interior)
        .unwrap();
    assert_eq!(supported.exact_terminal_ticks_to_go, Some(4));
    assert_eq!(scheduled.first(), Some(supported));
    assert!(scheduled.iter().any(|entry| entry.node == alternate));
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
    let prepared_replay =
        crate::learner::GraphReplayPlan::prepare(&contract, &batch, &policy_actions, 0).unwrap();
    let prepared_snapshot = crate::learner::ExactGraphTableLearner
        .fit_prepared(prepared_replay)
        .unwrap();
    assert_eq!(prepared_snapshot, prioritized_snapshot);
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
        graph.validated().unwrap().legacy_content_sha256().unwrap(),
        Digest(Sha256::digest(&encoded).into())
    );
    assert_ne!(
        graph.content_sha256().unwrap(),
        graph.validated().unwrap().legacy_content_sha256().unwrap()
    );
    assert_eq!(
        restored.content_sha256().unwrap(),
        graph.content_sha256().unwrap()
    );
    assert_eq!(restored.best_terminal_path(), graph.best_terminal_path());
    assert_eq!(restored.completed_transitions().count(), 1);
}

#[test]
fn compact_content_identity_tracks_expansion_lifecycle() {
    let (mut graph, transition, _) = graph_and_transition();
    let initial = graph.content_sha256().unwrap();
    let expansion = graph
        .register_action_expansion(graph.root(), transition.value_sample.action)
        .unwrap();
    let registered = graph.content_sha256().unwrap();
    graph
        .lease_action_expansion(expansion, Digest([21; 32]), 3, 4)
        .unwrap();
    let leased = graph.content_sha256().unwrap();
    graph
        .mark_expansion_retryable(expansion, Digest([21; 32]), 1)
        .unwrap();
    let retryable = graph.content_sha256().unwrap();

    assert_ne!(initial, registered);
    assert_ne!(registered, leased);
    assert_ne!(leased, retryable);
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
fn independently_captured_boundary_provenance_does_not_split_graph_truth() {
    let (mut graph, transition, route) = graph_and_transition();
    let first = graph
        .admit_completed_expansion(
            transition.clone(),
            route.clone(),
            17,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap();
    let mut independent_capture = transition;
    let original_boundary = independent_capture.intermediate_boundaries[0].clone();
    independent_capture.intermediate_boundaries[0] = OptionIntermediateBoundary::capture(
        &independent_capture.execution,
        original_boundary.offset_ticks,
        original_boundary.state.clone(),
    )
    .unwrap();
    assert_eq!(
        independent_capture.intermediate_boundaries[0].evidence_sha256,
        original_boundary.evidence_sha256
    );
    independent_capture.validate().unwrap();
    let duplicate = graph
        .admit_completed_expansion(
            independent_capture,
            route,
            18,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap();

    assert_eq!(duplicate.expansion_sha256, first.expansion_sha256);
    assert!(duplicate.duplicate);
    assert_eq!(graph.node_count(), 3);
    assert_eq!(graph.expansion_count(), 1);
    assert_eq!(graph.segment_count(), 2);
    graph.validate().unwrap();
}

#[test]
fn conflicting_intermediate_game_state_still_fails_closed() {
    let (mut graph, transition, route) = graph_and_transition();
    graph
        .admit_completed_expansion(
            transition.clone(),
            route.clone(),
            17,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap();
    let mut conflicting = transition;
    conflicting.intermediate_boundaries[0]
        .state
        .player
        .position_f32_bits[0] = 999.0_f32.to_bits();
    conflicting.intermediate_boundaries[0].state_sha256 = conflicting.intermediate_boundaries[0]
        .state
        .content_sha256()
        .unwrap();
    let conflicting_boundary = conflicting.intermediate_boundaries[0].clone();
    conflicting.intermediate_boundaries[0] = OptionIntermediateBoundary::capture(
        &conflicting.execution,
        conflicting_boundary.offset_ticks,
        conflicting_boundary.state,
    )
    .unwrap();
    conflicting.validate().unwrap();
    let error = graph
        .admit_completed_expansion(
            conflicting,
            route,
            18,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap_err();

    assert!(matches!(
        error,
        StateGraphError::ConflictingNativeEvidence {
            differing_fields,
            ..
        } if differing_fields == "intermediate_boundaries"
    ));
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
    assert_eq!(graph.completed_executable_expansion_count(), 0);
    let learner_only_set_sha256 = graph.completed_executable_expansion_set_sha256();

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
    assert_eq!(graph.completed_executable_expansion_count(), 1);
    assert_ne!(
        graph.completed_executable_expansion_set_sha256(),
        learner_only_set_sha256
    );
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
    let before = graph
        .node(slower_equivalent)
        .unwrap()
        .state
        .as_ref()
        .clone();
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
    let legacy_sha256 = graph.validated().unwrap().legacy_content_sha256().unwrap();
    let legacy_path = directory.join("legacy.dsg");
    std::fs::write(&legacy_path, graph.encode().unwrap()).unwrap();
    let legacy_restored =
        crate::persistence::read_state_graph(&legacy_path, legacy_sha256).unwrap();
    let report = crate::reporting::GraphSearchReport::from_graph(&restored).unwrap();

    assert_eq!(stored, duplicate);
    assert_eq!(restored, graph);
    assert_eq!(legacy_restored, graph);
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
    let (mut graph, transition, route) = graph_and_transition();
    let plan = graph.restoration_plan(graph.root()).unwrap();
    let expected = graph.node(graph.root()).unwrap().state.as_ref().clone();
    let receipt = graph.validate_restored_state(&plan, &expected).unwrap();

    assert_eq!(plan.schema, GRAPH_RESTORATION_PLAN_SCHEMA_V2);
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
    assert!(
        graph
            .validate_prehashed_restored_state(
                &plan,
                &wrong_state,
                wrong_state.content_sha256().unwrap(),
            )
            .is_err()
    );

    let mut tampered_plan = plan.clone();
    tampered_plan.route.tape_frames += 1;
    assert!(graph.restoration_route(&tampered_plan).is_err());

    graph
        .admit_completed_expansion(
            transition,
            route,
            17,
            ExpansionEvidenceAuthority::Executable,
        )
        .unwrap();
    assert_eq!(
        graph.restoration_route(&plan).unwrap(),
        graph.route(graph.root().route_checkpoint_sha256).unwrap(),
        "unrelated outgoing graph growth must not invalidate an immutable node plan"
    );
    assert!(
        graph
            .validate_prehashed_restored_state(
                &plan,
                graph.node(graph.root()).unwrap().state.as_ref(),
                graph.root().state_sha256,
            )
            .is_ok()
    );

    let mut detached_authority = plan;
    detached_authority.dispatch_graph_sha256.0[0] ^= 1;
    assert!(graph.restoration_route(&detached_authority).is_err());
}

#[test]
fn legacy_whole_graph_restoration_plan_remains_valid() {
    let (graph, _, _) = graph_and_transition();
    let mut legacy = graph.restoration_plan(graph.root()).unwrap();
    legacy.schema = GRAPH_RESTORATION_PLAN_SCHEMA_V1.into();
    legacy.dispatch_graph_sha256 = graph.content_sha256().unwrap();
    legacy.plan_sha256 = super::restoration::restoration_plan_sha256(
        &legacy.schema,
        legacy.dispatch_graph_sha256,
        legacy.node,
        legacy.expected_state_sha256,
        legacy.route.route_checkpoint_sha256,
        legacy.route.tape_sha256,
        legacy.route.tape_frames,
        legacy
            .native_boundary
            .as_ref()
            .map(|boundary| (boundary.evidence_sha256, boundary.option_offset_ticks)),
    );

    assert_eq!(
        graph.restoration_route(&legacy).unwrap(),
        graph.route(graph.root().route_checkpoint_sha256).unwrap()
    );
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
