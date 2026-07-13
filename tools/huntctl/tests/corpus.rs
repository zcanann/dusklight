use huntctl::corpus::{Corpus, CorpusError};
use huntctl::tape::{InputFrame, InputTape};
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

fn assert_success(executable: &str, args: &[&str]) -> Vec<u8> {
    let output = Command::new(executable).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
