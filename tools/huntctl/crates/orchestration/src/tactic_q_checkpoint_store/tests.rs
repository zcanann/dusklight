use super::*;
use dusklight_control::option_execution::OptionParameter;
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
    let tactic_ref = store.store_tactic(&tactic).unwrap();
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
        maximum_delta_len = maximum_delta_len.max(store.read_bytes(reference).unwrap().len());
        graph.install_persistence_head(head);
    }
    let final_head = graph.persistence_plan().unwrap();
    assert!(matches!(final_head, StateGraphPersistencePlan::Reuse(_)));
    assert!(
        maximum_delta_len <= first_delta_len + 32,
        "journal records grew from {first_delta_len} to {maximum_delta_len} bytes"
    );
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
