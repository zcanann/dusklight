use huntctl::content_store::{ContentKind, ContentStore};
use huntctl::corpus::{Corpus, CorpusError};
use huntctl::episode_store::EpisodeStore;
use huntctl::tape::{InputFrame, InputTape};
use huntctl::transition_corpus::{
    MacroAction, StateReference, StateReferenceKind, Transition, TransitionCorpus,
};
use huntctl::{BuildIdentity, Digest};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("huntctl-{name}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn build() -> BuildIdentity {
    BuildIdentity {
        dusklight_commit: "0123456789abcdef".into(),
        aurora_commit: "fedcba9876543210".into(),
        compiler: "clang-21".into(),
        target: "x86_64-pc-windows-msvc".into(),
        profile: "release".into(),
        feature_digest: Digest([1; 32]),
        game_digest: Digest([2; 32]),
        dirty_digest: None,
        fidelity_profile: "safe".into(),
    }
}

fn tape_bytes() -> Vec<u8> {
    InputTape {
        frames: vec![InputFrame::default(); 3],
        ..InputTape::default()
    }
    .encode()
    .unwrap()
}

#[test]
fn duplicate_ingest_is_content_addressed_and_idempotent() {
    let root = temp_dir("corpus-dedup");
    let corpus = Corpus::initialize(&root).unwrap();
    let first = corpus
        .ingest(
            &tape_bytes(),
            build(),
            "boot-title".into(),
            json!({"language":"en"}),
        )
        .unwrap();
    let second = corpus
        .ingest(
            &tape_bytes(),
            build(),
            "boot-title".into(),
            json!({"language":"en"}),
        )
        .unwrap();
    assert!(first.created);
    assert!(!second.created);
    assert_eq!(first.artifact_id, second.artifact_id);
    assert_eq!(first.tape_digest, second.tape_digest);
    assert_eq!(corpus.list().unwrap().len(), 1);
    assert_eq!(corpus.show(first.artifact_id).unwrap().frame_count, 3);
    assert_eq!(corpus.verify().unwrap().blobs, 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn verify_detects_same_size_blob_tampering() {
    let root = temp_dir("corpus-tamper");
    let corpus = Corpus::initialize(&root).unwrap();
    let result = corpus
        .ingest(&tape_bytes(), build(), "boot-title".into(), json!({}))
        .unwrap();
    let path = corpus.blob_path(result.tape_digest);
    let mut bytes = fs::read(&path).unwrap();
    bytes[0] ^= 0xff;
    fs::write(path, bytes).unwrap();
    assert!(matches!(
        corpus.verify(),
        Err(CorpusError::HashMismatch { .. })
    ));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn open_rejects_unknown_corpus_schema() {
    let root = temp_dir("corpus-schema");
    Corpus::initialize(&root).unwrap();
    fs::write(
        root.join("corpus.json"),
        r#"{"schema":"dusklight-corpus/v999"}"#,
    )
    .unwrap();
    assert!(matches!(
        Corpus::open(&root),
        Err(CorpusError::SchemaMismatch { .. })
    ));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn corpus_cli_initializes_ingests_lists_shows_and_verifies() {
    let root = temp_dir("corpus-cli");
    let source = root.join("source.tape");
    let build_path = root.join("build.json");
    fs::write(&source, tape_bytes()).unwrap();
    fs::write(&build_path, serde_json::to_vec_pretty(&build()).unwrap()).unwrap();
    let corpus_root = root.join("store");
    let executable = env!("CARGO_BIN_EXE_huntctl");
    assert_success(
        executable,
        &["corpus", "init", corpus_root.to_str().unwrap()],
    );
    let ingest = assert_success(
        executable,
        &[
            "corpus",
            "ingest",
            corpus_root.to_str().unwrap(),
            "--tape",
            source.to_str().unwrap(),
            "--scenario",
            "boot-title",
            "--build",
            build_path.to_str().unwrap(),
        ],
    );
    let artifact = serde_json::from_slice::<serde_json::Value>(&ingest).unwrap()["artifact_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let list = assert_success(
        executable,
        &["corpus", "list", corpus_root.to_str().unwrap()],
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&list)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let show = assert_success(
        executable,
        &["corpus", "show", corpus_root.to_str().unwrap(), &artifact],
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&show).unwrap()["scenario"]["id"],
        "boot-title"
    );
    let verify = assert_success(
        executable,
        &["corpus", "verify", corpus_root.to_str().unwrap()],
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&verify).unwrap()["artifacts"],
        1
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn corpus_lifecycle_cli_is_schema_aware_and_gc_is_recoverable() {
    let root = temp_dir("corpus-lifecycle");
    let first = root.join("first.dtcz");
    let second = root.join("second.dtcz");
    let transition = |action_id, reward, terminal| Transition {
        source: StateReference {
            kind: StateReferenceKind::Boundary,
            digest: Digest([3; 32]),
        },
        state: vec![1.0],
        action: MacroAction {
            action_id,
            macro_kind: 1,
            parameters: Vec::new(),
        },
        duration_ticks: 1,
        reward,
        next: StateReference {
            kind: StateReferenceKind::Boundary,
            digest: Digest([4; 32]),
        },
        next_state: vec![2.0],
        terminal,
    };
    TransitionCorpus::new(
        Digest([1; 32]),
        Digest([2; 32]),
        1,
        vec![transition(7, 1.0, true)],
    )
    .unwrap()
    .write_zstd_file(&first, 1)
    .unwrap();
    fs::copy(&first, &second).unwrap();
    let executable = env!("CARGO_BIN_EXE_huntctl");

    let query = Command::new(executable)
        .args(["corpus", "query", "--input"])
        .arg(&first)
        .args(["--action", "7"])
        .output()
        .unwrap();
    assert!(query.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&query.stdout)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let merged = root.join("merged.dtcz");
    let merge = Command::new(executable)
        .args(["corpus", "merge", "--input"])
        .arg(&first)
        .args(["--input"])
        .arg(&second)
        .args(["--output"])
        .arg(&merged)
        .output()
        .unwrap();
    assert!(
        merge.status.success(),
        "{}",
        String::from_utf8_lossy(&merge.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&merge.stdout).unwrap();
    assert_eq!(report["duplicates_removed"], 1);
    assert_success(
        executable,
        &[
            "corpus",
            "validate-transitions",
            "--input",
            merged.to_str().unwrap(),
        ],
    );
    let shard_root = root.join("shards");
    let shard = Command::new(executable)
        .args(["corpus", "shard", "--input"])
        .arg(&merged)
        .args(["--output-directory"])
        .arg(&shard_root)
        .args(["--maximum-transitions", "1"])
        .output()
        .unwrap();
    assert!(
        shard.status.success(),
        "{}",
        String::from_utf8_lossy(&shard.stderr)
    );
    assert!(shard_root.join("shard-00000.dtcz").exists());
    let arrow = root.join("analysis.arrow");
    let export = Command::new(executable)
        .args(["corpus", "export-arrow", "--input"])
        .arg(&merged)
        .args(["--output"])
        .arg(&arrow)
        .output()
        .unwrap();
    assert!(
        export.status.success(),
        "{}",
        String::from_utf8_lossy(&export.stderr)
    );
    let export: serde_json::Value = serde_json::from_slice(&export.stdout).unwrap();
    assert_eq!(export["format"], "apache-arrow-ipc-file");
    assert_eq!(export["replay_authority"], false);
    assert_eq!(export["rows"], 1);
    assert!(arrow.exists());
    assert!(root.join("analysis.arrow.manifest.json").exists());

    let content_root = root.join("content");
    let trash_root = root.join("recoverable-trash");
    let store = ContentStore::initialize(&content_root).unwrap();
    let kept = store.put_bytes(b"kept", ContentKind::Model).unwrap();
    let orphan = store
        .put_bytes(b"orphan thumbnail", ContentKind::Screenshot)
        .unwrap();
    let manifest_path = root.join("roots.json");
    let manifest_bytes = serde_json::to_vec_pretty(&json!({"model_sha256": kept.sha256})).unwrap();
    fs::write(&manifest_path, &manifest_bytes).unwrap();
    store
        .put_bytes(&manifest_bytes, ContentKind::DatasetManifest)
        .unwrap();
    let preview = Command::new(executable)
        .args(["corpus", "gc-content", "--store"])
        .arg(&content_root)
        .args(["--trash-root"])
        .arg(&trash_root)
        .args(["--manifest"])
        .arg(&manifest_path)
        .output()
        .unwrap();
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    assert!(store.blob_path(orphan.sha256).exists());
    let apply = Command::new(executable)
        .args(["corpus", "gc-content", "--store"])
        .arg(&content_root)
        .args(["--trash-root"])
        .arg(&trash_root)
        .args(["--manifest"])
        .arg(&manifest_path)
        .arg("--apply")
        .output()
        .unwrap();
    assert!(
        apply.status.success(),
        "{}",
        String::from_utf8_lossy(&apply.stderr)
    );
    assert!(!store.blob_path(orphan.sha256).exists());
    let applied: serde_json::Value = serde_json::from_slice(&apply.stdout).unwrap();
    assert_eq!(applied["moved"], 1);
    assert!(
        PathBuf::from(
            applied["unreachable"][0]["trash_destination"]
                .as_str()
                .unwrap()
        )
        .exists()
    );
    assert!(store.blob_path(kept.sha256).exists());

    let episode_root = root.join("episode-store");
    EpisodeStore::initialize(&episode_root).unwrap();
    let verify_episodes = Command::new(executable)
        .args(["corpus", "verify-episodes", "--store"])
        .arg(&episode_root)
        .output()
        .unwrap();
    assert!(
        verify_episodes.status.success(),
        "{}",
        String::from_utf8_lossy(&verify_episodes.stderr)
    );
    let verified: serde_json::Value = serde_json::from_slice(&verify_episodes.stdout).unwrap();
    assert_eq!(verified["entries"], 0);
    let unsafe_empty_gc = Command::new(executable)
        .args(["corpus", "gc-episodes", "--store"])
        .arg(&episode_root)
        .args(["--trash-root"])
        .arg(root.join("episode-trash"))
        .output()
        .unwrap();
    assert!(!unsafe_empty_gc.status.success());
    assert!(String::from_utf8_lossy(&unsafe_empty_gc.stderr).contains("nonempty retention set"));

    fs::remove_dir_all(root).unwrap();
}

fn assert_success(executable: &str, args: &[&str]) -> Vec<u8> {
    let output = Command::new(executable).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
