use super::*;
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
