use huntctl::artifact::Digest;
use huntctl::transition_corpus::{
    MacroAction, StateReference, StateReferenceKind, Transition, TransitionCorpus,
};
use serde_json::json;
use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn state(value: f32) -> Vec<f32> {
    vec![value]
}

fn reference(byte: u8) -> StateReference {
    StateReference {
        kind: StateReferenceKind::Boundary,
        digest: Digest([byte; 32]),
    }
}

fn transition(from: f32, action_id: u32, reward: f32, to: f32, terminal: bool) -> Transition {
    Transition {
        source: reference(from as u8 + 1),
        state: state(from),
        action: MacroAction {
            action_id,
            macro_kind: 1,
            parameters: Vec::new(),
        },
        duration_ticks: 1,
        reward,
        next: reference(to as u8 + 1),
        next_state: state(to),
        terminal,
    }
}

#[test]
fn native_learning_cli_inspects_and_ranks_a_compact_batch() {
    const ADVANCE: u32 = 3;
    const WAIT: u32 = 9;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("huntctl-learn-{nonce}"));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("shortest-path.dtcz");
    let corpus = TransitionCorpus::new(
        Digest([0x11; 32]),
        Digest([0x22; 32]),
        1,
        vec![
            transition(0.0, ADVANCE, 0.0, 1.0, false),
            transition(0.0, WAIT, -1.0, 0.0, false),
            transition(1.0, ADVANCE, 10.0, 2.0, true),
            transition(1.0, WAIT, -1.0, 1.0, false),
        ],
    )
    .unwrap();
    corpus.write_zstd_file(&path, 1).unwrap();

    let executable = env!("CARGO_BIN_EXE_huntctl");
    let inspect = Command::new(executable)
        .args(["learn", "inspect", "--input"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        inspect.status.success(),
        "{}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let inspect: serde_json::Value = serde_json::from_slice(&inspect.stdout).unwrap();
    assert_eq!(inspect["transitions"], 4);
    assert_eq!(inspect["action_counts"][ADVANCE.to_string()], 2);

    let undeclared_feature_kinds = Command::new(executable)
        .args(["learn", "fit", "--input"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(!undeclared_feature_kinds.status.success());
    assert!(
        String::from_utf8_lossy(&undeclared_feature_kinds.stderr)
            .contains("declare --all-continuous or repeat --categorical-feature")
    );

    let fit = Command::new(executable)
        .args(["learn", "fit", "--input"])
        .arg(&path)
        .args([
            "--query-transition",
            "0",
            "--all-continuous",
            "--iterations",
            "16",
            "--trees",
            "31",
            "--seed",
            "7",
        ])
        .output()
        .unwrap();
    assert!(
        fit.status.success(),
        "{}",
        String::from_utf8_lossy(&fit.stderr)
    );
    let fit: serde_json::Value = serde_json::from_slice(&fit.stdout).unwrap();
    assert_eq!(fit["transition_count"], 4);
    assert_eq!(fit["categorical_features"], json!([]));
    assert_eq!(fit["ranking"][0]["action"], ADVANCE);
    assert_eq!(fit["ranking"][0]["support"], 2);

    let oversized_config = Command::new(executable)
        .args(["learn", "fit", "--input"])
        .arg(&path)
        .args(["--iterations", "129"])
        .output()
        .unwrap();
    assert!(!oversized_config.status.success());
    assert!(
        String::from_utf8_lossy(&oversized_config.stderr)
            .contains("iterations must not exceed 128")
    );

    let mut too_many_inputs = Command::new(executable);
    too_many_inputs.args(["learn", "fit"]);
    for _ in 0..65 {
        too_many_inputs.arg("--input").arg(&path);
    }
    let too_many_inputs = too_many_inputs.output().unwrap();
    assert!(!too_many_inputs.status.success());
    assert!(
        String::from_utf8_lossy(&too_many_inputs.stderr)
            .contains("at most 64 input corpora; received 65")
    );

    let many_actions_path = root.join("too-many-actions.dtcz");
    let many_actions = TransitionCorpus::new(
        Digest([0x11; 32]),
        Digest([0x22; 32]),
        1,
        (0..129)
            .map(|action| transition(0.0, action, 0.0, 0.0, true))
            .collect(),
    )
    .unwrap();
    many_actions.write_zstd_file(&many_actions_path, 1).unwrap();
    let too_many_actions = Command::new(executable)
        .args(["learn", "fit", "--input"])
        .arg(&many_actions_path)
        .output()
        .unwrap();
    assert!(!too_many_actions.status.success());
    assert!(
        String::from_utf8_lossy(&too_many_actions.stderr).contains("at most 128 distinct actions")
    );

    let benchmark = Command::new(executable)
        .args(["learn", "benchmark"])
        .output()
        .unwrap();
    assert!(benchmark.status.success());
    let benchmark: serde_json::Value = serde_json::from_slice(&benchmark.stdout).unwrap();
    assert_eq!(benchmark["passed"], true);

    fs::remove_dir_all(root).unwrap();
}
