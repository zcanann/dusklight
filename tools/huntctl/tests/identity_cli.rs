use huntctl::{ARTIFACT_SCHEMA_VERSION, ArtifactIdentity, BuildIdentity, Digest};
use serde_json::Value;
use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn digest(byte: u8) -> Digest {
    Digest([byte; 32])
}

fn identity() -> ArtifactIdentity {
    ArtifactIdentity {
        schema_version: ARTIFACT_SCHEMA_VERSION,
        content_digest: digest(1),
        build: BuildIdentity {
            dusklight_commit: "dusk-commit".into(),
            aurora_commit: "aurora-commit".into(),
            compiler: "Apple clang 18".into(),
            target: "arm64-apple-darwin".into(),
            profile: "debug".into(),
            feature_digest: digest(2),
            game_digest: digest(3),
            dirty_digest: None,
            fidelity_profile: "native".into(),
        },
        protocol_name: "dusklight-automation".into(),
        protocol_version: 2,
        protocol_capabilities_digest: digest(4),
        scenario_id: "fixture-a".into(),
        region_digest: digest(5),
        language_assets_digest: digest(6),
        scenario_digest: digest(7),
        predicate_program_digest: digest(8),
        action_schema_digest: digest(9),
        observation_schema_digest: digest(10),
        settings_digest: digest(11),
    }
}

fn unique_directory() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "huntctl-identity-cli-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[test]
fn reports_every_replay_mismatch_and_respects_mode_specific_compatibility() {
    let executable = env!("CARGO_BIN_EXE_huntctl");
    let directory = unique_directory();
    fs::create_dir_all(&directory).unwrap();
    let expected_path = directory.join("expected.json");
    let actual_path = directory.join("actual.json");

    let expected = identity();
    let mut actual = expected.clone();
    actual.build.compiler = "Apple clang 19".into();
    actual.scenario_id = "fixture-b".into();
    fs::write(
        &expected_path,
        serde_json::to_vec_pretty(&expected).unwrap(),
    )
    .unwrap();
    fs::write(&actual_path, serde_json::to_vec_pretty(&actual).unwrap()).unwrap();

    let replay = Command::new(executable)
        .args(["identity", "compare", "--mode", "replay", "--expected"])
        .arg(&expected_path)
        .arg("--actual")
        .arg(&actual_path)
        .output()
        .unwrap();
    assert!(!replay.status.success());
    let stderr = String::from_utf8_lossy(&replay.stderr);
    assert!(stderr.contains("2 differences"), "{stderr}");
    assert!(stderr.contains("build.compiler"), "{stderr}");
    assert!(stderr.contains("scenario_id"), "{stderr}");

    let training = Command::new(executable)
        .args([
            "identity",
            "compare",
            "--mode",
            "model-training",
            "--expected",
        ])
        .arg(&expected_path)
        .arg("--actual")
        .arg(&actual_path)
        .output()
        .unwrap();
    assert!(
        training.status.success(),
        "{}",
        String::from_utf8_lossy(&training.stderr)
    );
    let report: Value = serde_json::from_slice(&training.stdout).unwrap();
    assert_eq!(report["compatible"], true);
    assert_eq!(report["mode"], "model-training");

    actual.settings_digest = Digest::ZERO;
    fs::write(&actual_path, serde_json::to_vec_pretty(&actual).unwrap()).unwrap();
    let invalid = Command::new(executable)
        .args(["identity", "compare", "--mode", "replay", "--expected"])
        .arg(&expected_path)
        .arg("--actual")
        .arg(&actual_path)
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    let stderr = String::from_utf8_lossy(&invalid.stderr);
    assert!(stderr.contains("invalid actual identity"), "{stderr}");
    assert!(stderr.contains("settings_digest"), "{stderr}");

    fs::remove_dir_all(directory).unwrap();
}
