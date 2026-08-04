use super::*;
use dusklight_automation_contracts::tape::InputFrame;
use dusklight_control::option_execution::OptionParameter;
use dusklight_control::option_execution::OptionType;
use dusklight_evidence::native_episode_shard::NativeEpisodeShard;
use dusklight_learning::fact_snapshot::FactSnapshot;
use dusklight_learning::option_values::OptionActionDescriptor;
use std::collections::BTreeMap;
use std::fs;

fn fact() -> FactSnapshot {
    let shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v28.dseps"
    ))
    .unwrap();
    FactSnapshot::from_native_learning(&shard.episodes[0].steps[0].pre_input, &[], None, Vec::new())
        .unwrap()
}

#[test]
fn shared_store_round_trips_whole_facts_and_reads_legacy_split_objects() {
    let root = std::env::temp_dir().join(format!(
        "dusklight-tactic-shared-content-store-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let store = TacticQContentStore::initialize(&root).unwrap();
    let first = fact();
    let mut second = first.clone();
    second.boundary_index += 1;
    second.simulation_tick += 1;
    second.tape_frame += 1;
    let first_ref = store.store_fact(&first).unwrap();
    let second_ref = store.store_fact(&second).unwrap();
    assert_ne!(first_ref, second_ref);
    assert_eq!(
        first_ref.sha256,
        Digest(Sha256::digest(serde_cbor::to_vec(&first).unwrap()).into())
    );
    assert_eq!(store.load_fact(first_ref).unwrap(), first);
    assert_eq!(store.load_fact(second_ref).unwrap(), second);

    let tactic = OptionActionDescriptor {
        option_id: "move.east".into(),
        option_type: dusklight_control::option_execution::OptionType::Move,
        parameters: BTreeMap::new(),
    };
    let tactic_ref = StoredContentRef::from(
        &ContentStore::open(&root)
            .unwrap()
            .put_bytes(
                &serde_cbor::to_vec(&tactic).unwrap(),
                ContentKind::TacticDefinition,
            )
            .unwrap(),
    );
    assert_eq!(store.load_tactic(tactic_ref).unwrap(), tactic);
    let tape = InputTape::default();
    let tape_ref = store.store_tape(&tape).unwrap();
    assert_eq!(store.load_tape(tape_ref).unwrap(), tape);

    let actors = first
        .actors
        .iter()
        .map(|actor| store.store_actor(actor).unwrap())
        .collect::<Vec<_>>();
    let mut snapshot_without_actors = first.clone();
    snapshot_without_actors.actors.clear();
    let legacy_raw = serde_cbor::to_vec(&StoredFactSnapshot {
        schema: FACT_OBJECT_SCHEMA_V1.into(),
        snapshot_sha256: first.content_sha256().unwrap(),
        actors,
        snapshot_without_actors,
    })
    .unwrap();
    let legacy_ref = StoredContentRef::from(
        &ContentStore::open(&root)
            .unwrap()
            .put_bytes(&legacy_raw, ContentKind::FactSnapshot)
            .unwrap(),
    );
    assert_eq!(store.load_fact(legacy_ref).unwrap(), first);
    fs::remove_dir_all(root).unwrap();
}

fn advanced_fact(root: &FactSnapshot, ticks: u64) -> FactSnapshot {
    let mut state = root.clone();
    state.boundary_index += ticks;
    state.simulation_tick += ticks;
    state.tape_frame += ticks;
    state.recent_history.clear();
    state.recent_option = None;
    state.terminal.reached = Some(false);
    state.validate().unwrap();
    state
}

fn option_transition() -> (OptionTransitionSample, InputTape) {
    let before = fact();
    let mut route = InputTape {
        frames: vec![InputFrame::default(); before.tape_frame as usize],
        ..InputTape::default()
    };
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
    let root_checkpoint_sha256 = Digest([4; 32]);
    let source_checkpoint_sha256 = route_checkpoint_sha256(
        root_checkpoint_sha256,
        &crate::state_graph::tape_prefix(&route, before.tape_frame as usize).unwrap(),
    )
    .unwrap();
    let next_checkpoint_sha256 = route_checkpoint_sha256(root_checkpoint_sha256, &route).unwrap();
    let after = advanced_fact(&before, 8);
    let mut transition = OptionTransitionSample::capture(
        Digest([2; 32]),
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
    transition.execution_authority_sha256 = Digest([1; 32]);
    transition.intermediate_boundaries = vec![
        OptionIntermediateBoundary::capture(&transition.execution, 4, advanced_fact(&before, 4))
            .unwrap(),
    ];
    transition.validate().unwrap();
    (transition, route)
}

#[test]
fn option_transition_store_packs_new_rows_and_reads_legacy_split_rows() {
    let root = std::env::temp_dir().join(format!(
        "dusklight-tactic-transition-content-store-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let store = TacticQContentStore::initialize(&root).unwrap();
    let (transition, route) = option_transition();

    let packed_ref = store.store_option_transition(&transition, &route).unwrap();
    assert_eq!(
        store.load_option_transition(packed_ref).unwrap(),
        transition
    );
    assert_eq!(
        decode_cbor::<PackedOptionTransition>(&store.read_bytes(packed_ref).unwrap())
            .unwrap()
            .schema,
        PACKED_TRANSITION_SCHEMA_V1
    );

    let before = store.store_fact(&transition.before).unwrap();
    let after = store.store_fact(&transition.after).unwrap();
    let tactic = StoredContentRef::from(
        &ContentStore::open(&root)
            .unwrap()
            .put_bytes(
                &serde_cbor::to_vec(&transition.value_sample.action).unwrap(),
                ContentKind::TacticDefinition,
            )
            .unwrap(),
    );
    let emitted_tape = store
        .store_tape(&InputTape {
            boot: route.boot.clone(),
            tick_rate_numerator: route.tick_rate_numerator,
            tick_rate_denominator: route.tick_rate_denominator,
            frames: transition.execution.emitted_raw_actions.clone(),
        })
        .unwrap();
    let intermediate_boundaries = transition
        .intermediate_boundaries
        .iter()
        .map(|boundary| StoredOptionIntermediateBoundary {
            evidence_sha256: boundary.evidence_sha256,
            offset_ticks: boundary.offset_ticks,
            state_sha256: boundary.state_sha256,
            state: store.store_fact(&boundary.state).unwrap(),
        })
        .collect();
    let legacy = StoredOptionTransition {
        schema: transition.schema.clone(),
        execution_authority_sha256: transition.execution_authority_sha256,
        feature_schema_sha256: transition.feature_schema_sha256,
        before_state_sha256: transition.before_state_sha256,
        after_state_sha256: transition.after_state_sha256,
        source_checkpoint_sha256: transition.source_checkpoint_sha256,
        next_checkpoint_sha256: transition.next_checkpoint_sha256,
        before,
        after,
        execution: StoredOptionExecution {
            schema: transition.execution.schema.clone(),
            tactic,
            duration: transition.execution.duration,
            termination_condition: transition.execution.termination_condition.clone(),
            cancellation_conditions: transition.execution.cancellation_conditions.clone(),
            end_reason: transition.execution.end_reason,
            emitted_tape,
            realized_tape_range: transition.execution.realized_tape_range,
            tape_sha256: transition.execution.tape_sha256,
        },
        value_sample: StoredOptionValueSample {
            tactic,
            state: transition.value_sample.state.clone(),
            duration_ticks: transition.value_sample.duration_ticks,
            reward: transition.value_sample.reward,
            next_state: transition.value_sample.next_state.clone(),
            terminal: transition.value_sample.terminal,
            before_state_sha256: transition.value_sample.before_state_sha256,
            after_state_sha256: transition.value_sample.after_state_sha256,
            source_checkpoint_sha256: transition.value_sample.source_checkpoint_sha256,
            next_checkpoint_sha256: transition.value_sample.next_checkpoint_sha256,
            realized_tape_range: transition.value_sample.realized_tape_range,
            realized_tape_sha256: transition.value_sample.realized_tape_sha256,
        },
        intermediate_boundaries,
    };
    let legacy_ref = StoredContentRef::from(
        &ContentStore::open(&root)
            .unwrap()
            .put_bytes(
                &serde_cbor::to_vec(&legacy).unwrap(),
                ContentKind::TacticTransition,
            )
            .unwrap(),
    );
    assert_eq!(
        store.load_option_transition(legacy_ref).unwrap(),
        transition
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn inline_training_corpus_reader_accepts_legacy_reference_envelopes() {
    let root = std::env::temp_dir().join(format!(
        "dusklight-tactic-training-corpus-legacy-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let content_root = root.join(CONTENT_DIRECTORY);
    TacticQContentStore::initialize(&content_root).unwrap();
    let expected = TacticQTrainingCorpus {
        execution_authority_sha256: Digest::ZERO,
        feature_schema_sha256: Digest([1; 32]),
        objective_sha256: Digest([2; 32]),
        root_checkpoint_sha256: Digest([3; 32]),
        transitions: Vec::new(),
        routes: Vec::new(),
        episode_groups: Vec::new(),
    };
    let legacy = StoredTrainingCorpusManifest {
        schema: TRAINING_CORPUS_MANIFEST_SCHEMA_V1.into(),
        execution_authority_sha256: Digest::ZERO,
        feature_schema_sha256: expected.feature_schema_sha256,
        objective_sha256: expected.objective_sha256,
        root_checkpoint_sha256: expected.root_checkpoint_sha256,
        transitions: Vec::new(),
        routes: Vec::new(),
        episode_groups: Vec::new(),
    };
    let envelope = encode_binary_envelope(
        &serde_cbor::to_vec(&legacy).unwrap(),
        TRAINING_CORPUS_MAGIC,
        TRAINING_CORPUS_FORMAT_VERSION_V1,
    )
    .unwrap();
    let path = root.join("legacy.dtqc");
    fs::create_dir_all(&root).unwrap();
    fs::write(&path, envelope).unwrap();
    assert_eq!(read_training_corpus(&path).unwrap(), expected);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn state_graph_journal_stores_one_base_then_only_dirty_upserts() {
    let root = std::env::temp_dir().join(format!(
        "dusklight-tactic-state-graph-journal-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let store = TacticQContentStore::initialize(&root).unwrap();
    let mut root_fact = fact();
    root_fact.terminal.configured = Some(true);
    root_fact.terminal.reached = Some(false);
    let mut root_tape = InputTape::default();
    root_tape
        .frames
        .resize_with(root_fact.tape_frame as usize, Default::default);
    let mut graph = StateGraph::new(
        crate::state_graph::StateGraphIdentity {
            execution_authority_sha256: Digest([7; 32]),
            future_equivalence_validator_sha256: Digest::ZERO,
            feature_schema_sha256: Digest([8; 32]),
            objective_sha256: Digest([9; 32]),
            root_checkpoint_sha256: Digest([10; 32]),
        },
        root_fact,
        root_tape,
    )
    .unwrap();

    let (base_reference, base_head) = store.store_state_graph_journal(&graph).unwrap();
    assert_eq!(base_head.depth, 0);
    graph.install_persistence_head(base_head);
    graph
        .register_action_expansion(
            graph.root(),
            OptionActionDescriptor {
                option_id: "move.east".into(),
                option_type: dusklight_control::option_execution::OptionType::Move,
                parameters: BTreeMap::new(),
            },
        )
        .unwrap();
    let (delta_reference, delta_head) = store.store_state_graph_journal(&graph).unwrap();
    assert_eq!(delta_head.depth, 1);
    assert_ne!(delta_reference, base_reference);
    graph.install_persistence_head(delta_head);
    let (reused_reference, reused_head) = store.store_state_graph_journal(&graph).unwrap();
    assert_eq!(reused_reference, delta_reference);
    assert_eq!(reused_head, delta_head);

    let base_bytes = store.read_bytes(base_reference).unwrap();
    let delta_bytes = store.read_bytes(delta_reference).unwrap();
    assert!(delta_bytes.len() < base_bytes.len());
    let first_delta_len = delta_bytes.len();

    let mut maximum_delta_len = first_delta_len;
    let mut compacted_bases = 0;
    for variant in 1..=128 {
        graph
            .register_action_expansion(
                graph.root(),
                OptionActionDescriptor {
                    option_id: "move.east".into(),
                    option_type: dusklight_control::option_execution::OptionType::Move,
                    parameters: BTreeMap::from([(
                        "variant".into(),
                        OptionParameter::Unsigned(variant),
                    )]),
                },
            )
            .unwrap();
        let (reference, head) = store.store_state_graph_journal(&graph).unwrap();
        if head.depth == 0 {
            compacted_bases += 1;
        } else {
            maximum_delta_len = maximum_delta_len.max(store.read_bytes(reference).unwrap().len());
        }
        graph.install_persistence_head(head);
    }
    let final_head = graph.persistence_plan().unwrap();
    assert!(matches!(final_head, StateGraphPersistencePlan::Reuse(_)));
    assert!(
        maximum_delta_len <= first_delta_len + 32,
        "journal records grew from {first_delta_len} to {maximum_delta_len} bytes"
    );
    assert_eq!(compacted_bases, 2);
    assert!(
        serde_cbor::to_vec(&graph).unwrap().len() > maximum_delta_len * 10,
        "test graph did not grow enough to expose whole-graph persistence"
    );
    assert_eq!(
        store
            .load_state_graph_journal_validated(match final_head {
                StateGraphPersistencePlan::Reuse(head) => head,
                StateGraphPersistencePlan::Store { .. } => unreachable!(),
            })
            .unwrap()
            .0,
        graph
    );
    fs::remove_dir_all(root).unwrap();
}
