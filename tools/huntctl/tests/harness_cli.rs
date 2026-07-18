use huntctl::Digest;
use huntctl::harness::objective_suite::{
    ArtifactReference, ExpectedTerminalClass, OBJECTIVE_SUITE_SCHEMA_V1, ObjectiveBoot,
    ObjectiveCaseRole, ObjectiveProgramReference, ObjectiveSeed, ObjectiveSuite,
    ObjectiveSuiteCase, ObservationViewReference, SchemaIdentity,
};
use huntctl::milestone_dsl;
use huntctl::observation_view::movement_state_v2_spec;
use huntctl::scenario_fixture::{SCENARIO_FIXTURE_SCHEMA, ScenarioFixture};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn digest(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

fn artifact(path: &str, bytes: &[u8]) -> ArtifactReference {
    ArtifactReference {
        path: path.into(),
        sha256: digest(bytes),
    }
}

fn unique_root() -> PathBuf {
    std::env::temp_dir().join(format!(
        "huntctl-harness-cli-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn write_suite(root: &Path) -> PathBuf {
    fs::create_dir_all(root.join("cases/stage-ready")).unwrap();
    let scenario = ScenarioFixture {
        schema: SCENARIO_FIXTURE_SCHEMA.into(),
        name: "stage-ready".into(),
        form: None,
        health: None,
        rng: Vec::new(),
        video_mode: None,
        inventory: Vec::new(),
        equipment: Vec::new(),
        flags: Vec::new(),
        settings: Vec::new(),
    };
    let scenario_bytes = serde_json::to_vec_pretty(&scenario).unwrap();
    fs::write(
        root.join("cases/stage-ready/scenario.json"),
        &scenario_bytes,
    )
    .unwrap();

    let objective_bytes = b"milestones 1.0\n\nmilestone stage_ready {\n  phase post_sim\n  when stage.name == \"F_SP103\" && player.exists\n}\n";
    fs::write(
        root.join("cases/stage-ready/objective.milestones"),
        objective_bytes,
    )
    .unwrap();
    let objective = milestone_dsl::parse(std::str::from_utf8(objective_bytes).unwrap()).unwrap();
    let compiled = milestone_dsl::compile(&objective).unwrap();

    let mut observation = movement_state_v2_spec();
    observation.objective.id = "stage_ready".into();
    let observation_bytes = serde_json::to_vec_pretty(&observation).unwrap();
    fs::write(
        root.join("cases/stage-ready/observation.json"),
        &observation_bytes,
    )
    .unwrap();

    let mut suite = ObjectiveSuite {
        schema: OBJECTIVE_SUITE_SCHEMA_V1.into(),
        content_sha256: Digest::ZERO,
        id: "core-conformance/v1".into(),
        description: "Cheap objective cases proving the public harness boundary.".into(),
        cases: vec![ObjectiveSuiteCase {
            id: "stage-ready".into(),
            description: "A direct stage boot reaches its declared ready state.".into(),
            role: ObjectiveCaseRole::Positive,
            control_for: None,
            boot: ObjectiveBoot::Stage {
                stage: "F_SP103".into(),
                room: 0,
                point: 0,
                layer: 0,
                save_slot: None,
            },
            scenario: artifact("cases/stage-ready/scenario.json", &scenario_bytes),
            objective: ObjectiveProgramReference {
                source: artifact("cases/stage-ready/objective.milestones", objective_bytes),
                program_sha256: Digest(compiled.program_sha256),
                goal: "stage_ready".into(),
            },
            observation_view: ObservationViewReference {
                source: artifact("cases/stage-ready/observation.json", &observation_bytes),
                schema_sha256: observation.digest().unwrap(),
            },
            action_schema: SchemaIdentity {
                id: "neutral/v1".into(),
                sha256: Digest([7; 32]),
            },
            required_query_facts: vec!["player.exists".into(), "stage.name".into()],
            seed: ObjectiveSeed::Neutral,
            logical_tick_budget: 300,
            host_timeout_seconds: 30,
            repetitions: 2,
            expected_terminal: ExpectedTerminalClass::Reached,
        }],
    };
    suite.refresh_content_sha256().unwrap();
    let suite_path = root.join("suite.json");
    fs::write(&suite_path, suite.to_pretty_json().unwrap()).unwrap();
    suite_path
}

#[test]
fn validates_a_content_bound_suite_and_rejects_a_stale_artifact() {
    let executable = env!("CARGO_BIN_EXE_huntctl");
    let root = unique_root();
    let suite_path = write_suite(&root);
    let valid = Command::new(executable)
        .args(["harness", "validate-suite", "--suite"])
        .arg(&suite_path)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        valid.status.success(),
        "{}",
        String::from_utf8_lossy(&valid.stderr)
    );
    let report: Value = serde_json::from_slice(&valid.stdout).unwrap();
    assert_eq!(report["suite_id"], "core-conformance/v1");
    assert_eq!(report["case_count"], 1);
    assert_eq!(report["positive_count"], 1);

    let mut draft: ObjectiveSuite =
        serde_json::from_slice(&fs::read(&suite_path).unwrap()).unwrap();
    draft.content_sha256 = Digest::ZERO;
    let draft_path = root.join("suite.draft.json");
    let sealed_path = root.join("suite.sealed.json");
    fs::write(&draft_path, serde_json::to_vec_pretty(&draft).unwrap()).unwrap();
    let sealed = Command::new(executable)
        .args(["harness", "seal-suite", "--input"])
        .arg(&draft_path)
        .arg("--output")
        .arg(&sealed_path)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        sealed.status.success(),
        "{}",
        String::from_utf8_lossy(&sealed.stderr)
    );
    let sealed_suite: ObjectiveSuite =
        serde_json::from_slice(&fs::read(&sealed_path).unwrap()).unwrap();
    sealed_suite.validate_files(&root).unwrap();
    assert_ne!(sealed_suite.content_sha256, Digest::ZERO);

    fs::write(
        root.join("cases/stage-ready/objective.milestones"),
        b"tampered",
    )
    .unwrap();
    let stale = Command::new(executable)
        .args(["harness", "validate-suite", "--suite"])
        .arg(&suite_path)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(!stale.status.success());
    assert!(String::from_utf8_lossy(&stale.stderr).contains("digest is stale"));
    fs::remove_dir_all(root).unwrap();
}
