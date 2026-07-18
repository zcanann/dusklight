use huntctl::observation_view::movement_state_v2_spec;
use serde_json::Value;
use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn observation_cli_emits_and_inspects_the_authenticated_v2_spec() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("huntctl-observation-{nonce}"));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("movement-state-v2.json");
    let executable = env!("CARGO_BIN_EXE_huntctl");

    let emit = Command::new(executable)
        .args(["observe", "spec", "movement-state/v2", "--output"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        emit.status.success(),
        "{}",
        String::from_utf8_lossy(&emit.stderr)
    );
    let emitted: Value = serde_json::from_slice(&emit.stdout).unwrap();
    let expected = movement_state_v2_spec();
    assert_eq!(emitted["digest"], expected.digest().unwrap().to_string());
    assert_eq!(emitted["feature_count"], expected.feature_count());
    assert_eq!(
        fs::read(&path).unwrap(),
        expected.canonical_bytes().unwrap()
    );

    let inspect = Command::new(executable)
        .args(["observe", "inspect"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        inspect.status.success(),
        "{}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let inspected: Value = serde_json::from_slice(&inspect.stdout).unwrap();
    assert_eq!(inspected["id"], "movement-state/v2");
    assert_eq!(inspected["digest"], expected.digest().unwrap().to_string());
    assert_eq!(inspected["feature_count"], expected.feature_count());
    assert_eq!(
        inspected["objective"]["target"]["stage"],
        expected.objective.target.stage
    );

    fs::remove_dir_all(root).unwrap();
}
