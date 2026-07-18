use huntctl::Digest;
use huntctl::artifact::{ARTIFACT_SCHEMA_VERSION, ArtifactIdentity, BuildIdentity};
use huntctl::harness::objective_suite::{
    ArtifactReference, ExpectedTerminalClass, OBJECTIVE_SUITE_SCHEMA_V1, ObjectiveBoot,
    ObjectiveCaseRole, ObjectiveProgramReference, ObjectiveSeed, ObjectiveSuite,
    ObjectiveSuiteCase, ObservationViewReference, SchemaIdentity,
};
use huntctl::harness::run_contract::{
    HarnessBoundaryFingerprint, HarnessFidelityMode, HarnessObjectiveResult,
    HarnessProtocolIdentity, HarnessRunArtifacts, HarnessRunRequest, HarnessRunResult,
    HarnessRunTiming, HarnessTerminalDetail, HarnessTerminalReason, HarnessWorkerIdentity,
    RUN_REQUEST_SCHEMA_V1, RUN_RESULT_SCHEMA_V1,
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

fn write_run_request_draft(root: &Path, suite_path: &Path) -> PathBuf {
    fs::create_dir_all(root.join("inputs")).unwrap();
    let executable = b"test executable";
    let game_data = b"test game data";
    fs::write(root.join("inputs/dusklight"), executable).unwrap();
    fs::write(root.join("inputs/game.iso"), game_data).unwrap();

    let suite: ObjectiveSuite = serde_json::from_slice(&fs::read(suite_path).unwrap()).unwrap();
    let case = suite.cases.into_iter().next().unwrap();
    let build = BuildIdentity {
        dusklight_commit: "1".repeat(40),
        aurora_commit: "2".repeat(40),
        compiler: "apple-clang-20".into(),
        target: "arm64-apple-darwin".into(),
        profile: "debug-observers".into(),
        feature_digest: Digest([3; 32]),
        game_digest: digest(game_data),
        dirty_digest: None,
        fidelity_profile: "native-read-only".into(),
    };
    let mut protocol = HarnessProtocolIdentity {
        name: "dusklight-automation".into(),
        version: 2,
        capabilities_sha256: Digest::ZERO,
        capabilities: vec![
            "gameplay-trace-v5".into(),
            "input-tape-v3".into(),
            "milestone-program-v1.5".into(),
            "stage-boot".into(),
        ],
    };
    protocol.refresh_capabilities_sha256().unwrap();
    let identity = ArtifactIdentity {
        schema_version: ARTIFACT_SCHEMA_VERSION,
        content_digest: Digest([4; 32]),
        build: build.clone(),
        protocol_name: protocol.name.clone(),
        protocol_version: protocol.version,
        protocol_capabilities_digest: protocol.capabilities_sha256,
        scenario_id: "stage-ready-scenario".into(),
        region_digest: Digest([5; 32]),
        language_assets_digest: Digest([6; 32]),
        scenario_digest: case.scenario.sha256,
        predicate_program_digest: case.objective.program_sha256,
        action_schema_digest: case.action_schema.sha256,
        observation_schema_digest: case.observation_view.schema_sha256,
        settings_digest: Digest([8; 32]),
    };
    let request = HarnessRunRequest {
        schema: RUN_REQUEST_SCHEMA_V1.into(),
        content_sha256: Digest::ZERO,
        id: "stage-ready-cli".into(),
        executable: artifact("inputs/dusklight", executable),
        game_data: artifact("inputs/game.iso", game_data),
        build,
        identity,
        protocol,
        boot: case.boot,
        scenario: case.scenario,
        objective: case.objective,
        observation_view: case.observation_view,
        action_schema: case.action_schema,
        required_query_facts: case.required_query_facts,
        input: case.seed,
        rng_seed: 42,
        logical_tick_budget: case.logical_tick_budget,
        host_timeout_seconds: case.host_timeout_seconds,
        fidelity: HarnessFidelityMode::Headless,
        artifact_destination: "artifacts/stage-ready-cli".into(),
    };
    let path = root.join("run-request.draft.json");
    fs::write(path.as_path(), serde_json::to_vec_pretty(&request).unwrap()).unwrap();
    path
}

fn write_run_result_draft(root: &Path, request: &HarnessRunRequest) -> (PathBuf, PathBuf) {
    let artifact_root = root.join(&request.artifact_destination);
    fs::create_dir_all(&artifact_root).unwrap();
    let tape = b"realized input";
    let trace = b"gameplay trace";
    let evidence = b"objective evidence";
    fs::write(artifact_root.join("realized.tape"), tape).unwrap();
    fs::write(artifact_root.join("gameplay.trace"), trace).unwrap();
    fs::write(artifact_root.join("objective.json"), evidence).unwrap();
    let objective = artifact("objective.json", evidence);
    let result = HarnessRunResult {
        schema: RUN_RESULT_SCHEMA_V1.into(),
        content_sha256: Digest::ZERO,
        request_id: request.id.clone(),
        request_sha256: request.content_sha256,
        identity: request.identity.clone(),
        attempt: 1,
        worker: HarnessWorkerIdentity {
            id: "local-worker-0".into(),
            build: request.build.clone(),
            protocol: request.protocol.clone(),
        },
        terminal: HarnessTerminalReason::Reached,
        detail: HarnessTerminalDetail {
            message: "objective reached".into(),
            missing_query_facts: Vec::new(),
            missing_capabilities: Vec::new(),
        },
        objective: HarnessObjectiveResult {
            reached: true,
            first_hit_tick: Some(5),
            evidence: Some(objective.clone()),
            boundary_fingerprint: Some(HarnessBoundaryFingerprint {
                schema: "dusklight.milestone-boundary/v4".into(),
                algorithm: "xxh3-128".into(),
                canonical_encoding: "little-endian-fixed-v4".into(),
                digest: "12".repeat(16),
            }),
        },
        artifacts: HarnessRunArtifacts {
            realized_input: Some(artifact("realized.tape", tape)),
            gameplay_trace: Some(artifact("gameplay.trace", trace)),
            objective_result: Some(objective),
            stdout: None,
            stderr: None,
            complete: true,
        },
        timing: HarnessRunTiming {
            logical_ticks: 6,
            consumed_input_ticks: 6,
            host_elapsed_millis: 50,
        },
    };
    let path = root.join("run-result.draft.json");
    fs::write(path.as_path(), serde_json::to_vec_pretty(&result).unwrap()).unwrap();
    (path, artifact_root)
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

#[test]
fn seals_and_validates_a_complete_run_boundary() {
    let executable = env!("CARGO_BIN_EXE_huntctl");
    let root = unique_root();
    let suite_path = write_suite(&root);
    let request_draft = write_run_request_draft(&root, &suite_path);
    let request_path = root.join("run-request.json");

    let sealed_request = Command::new(executable)
        .args(["harness", "seal-run-request", "--input"])
        .arg(&request_draft)
        .arg("--output")
        .arg(&request_path)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        sealed_request.status.success(),
        "{}",
        String::from_utf8_lossy(&sealed_request.stderr)
    );
    let request: HarnessRunRequest =
        serde_json::from_slice(&fs::read(&request_path).unwrap()).unwrap();
    assert_ne!(request.content_sha256, Digest::ZERO);

    let validated_request = Command::new(executable)
        .args(["harness", "validate-run-request", "--request"])
        .arg(&request_path)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(validated_request.status.success());

    let (result_draft, artifact_root) = write_run_result_draft(&root, &request);
    let result_path = root.join("run-result.json");
    let sealed_result = Command::new(executable)
        .args(["harness", "seal-run-result", "--input"])
        .arg(&result_draft)
        .arg("--output")
        .arg(&result_path)
        .arg("--request")
        .arg(&request_path)
        .arg("--artifact-root")
        .arg(&artifact_root)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        sealed_result.status.success(),
        "{}",
        String::from_utf8_lossy(&sealed_result.stderr)
    );

    let validated_result = Command::new(executable)
        .args(["harness", "validate-run-result", "--result"])
        .arg(&result_path)
        .arg("--request")
        .arg(&request_path)
        .arg("--artifact-root")
        .arg(&artifact_root)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        validated_result.status.success(),
        "{}",
        String::from_utf8_lossy(&validated_result.stderr)
    );
    let report: Value = serde_json::from_slice(&validated_result.stdout).unwrap();
    assert_eq!(report["terminal"], "reached");
    assert_eq!(report["artifacts_complete"], true);

    let overwrite = Command::new(executable)
        .args(["harness", "seal-run-result", "--input"])
        .arg(&result_draft)
        .arg("--output")
        .arg(&result_path)
        .arg("--request")
        .arg(&request_path)
        .arg("--artifact-root")
        .arg(&artifact_root)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(!overwrite.status.success());
    assert!(String::from_utf8_lossy(&overwrite.stderr).contains("already exists"));
    fs::remove_dir_all(root).unwrap();
}
