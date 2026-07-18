use huntctl::Digest;
use huntctl::artifact::{ARTIFACT_SCHEMA_VERSION, ArtifactIdentity, BuildIdentity};
use huntctl::controller_program::ControllerProgram;
use huntctl::harness::objective_suite::{
    ArtifactReference, ExpectedTerminalClass, OBJECTIVE_SUITE_SCHEMA_V2, ObjectiveBoot,
    ObjectiveCaseRole, ObjectiveProgramReference, ObjectiveSeed, ObjectiveSuite,
    ObjectiveSuiteCase, ObservationViewReference, SchemaIdentity,
};
use huntctl::harness::observation_contract::{
    OBJECTIVE_OBSERVATION_REQUIREMENTS_SCHEMA_V1, ObjectiveObservationRequirements,
    ObservationFamilyRequirement,
};
use huntctl::harness::run_contract::{
    HarnessBoundaryFingerprint, HarnessFidelityMode, HarnessObjectiveResult,
    HarnessProtocolIdentity, HarnessRunArtifacts, HarnessRunRequest, HarnessRunResult,
    HarnessRunTiming, HarnessTerminalDetail, HarnessTerminalReason, HarnessWorkerIdentity,
    RUN_REQUEST_SCHEMA_V2, RUN_RESULT_SCHEMA_V2,
};
use huntctl::milestone_dsl;
use huntctl::observation_view::movement_state_v2_spec;
use huntctl::scenario_fixture::{SCENARIO_FIXTURE_SCHEMA, ScenarioFixture};
use huntctl::search::{
    Ancestry, CANDIDATE_SCHEMA, Candidate, MacroAction, SegmentProfile, write_explicit_population,
};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

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
        "huntctl-harness-cli-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT_ROOT.fetch_add(1, Ordering::Relaxed),
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
        schema: OBJECTIVE_SUITE_SCHEMA_V2.into(),
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
            observation_requirements: ObjectiveObservationRequirements {
                schema: OBJECTIVE_OBSERVATION_REQUIREMENTS_SCHEMA_V1.into(),
                families: vec![
                    ObservationFamilyRequirement {
                        id: "player_motion".into(),
                        minimum_version: 1,
                    },
                    ObservationFamilyRequirement {
                        id: "stage".into(),
                        minimum_version: 1,
                    },
                ],
                facts: vec!["player.exists".into(), "stage.name".into()],
            },
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
        schema: RUN_REQUEST_SCHEMA_V2.into(),
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
        observation_requirements: case.observation_requirements,
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
        schema: RUN_RESULT_SCHEMA_V2.into(),
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
            observation_issues: Vec::new(),
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

#[cfg(unix)]
fn write_mock_native_wrapper(root: &Path, huntctl_executable: &str) -> String {
    write_mock_native_wrapper_mode(root, huntctl_executable, None)
}

#[cfg(unix)]
fn write_mock_native_wrapper_mode(
    root: &Path,
    huntctl_executable: &str,
    mode: Option<&str>,
) -> String {
    use std::os::unix::fs::PermissionsExt;

    let mode = mode
        .map(|mode| format!(" --mock-mode {mode}"))
        .unwrap_or_default();
    let wrapper = format!(
        "#!/bin/sh\nexec \"{}\" mock-search-worker{} \"$@\"\n",
        huntctl_executable, mode
    );
    let wrapper_path = root.join("inputs/dusklight");
    fs::write(&wrapper_path, wrapper.as_bytes()).unwrap();
    let mut permissions = fs::metadata(&wrapper_path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&wrapper_path, permissions).unwrap();
    wrapper
}

#[cfg(unix)]
fn rewrite_request_for_mock_controller(
    root: &Path,
    draft_path: &Path,
    huntctl_executable: &str,
    mode: Option<&str>,
    artifact_destination: &str,
) {
    rewrite_request_for_mock_native_execution(root, draft_path, huntctl_executable);
    let wrapper = write_mock_native_wrapper_mode(root, huntctl_executable, mode);
    let controller =
        ControllerProgram::parse("duskcontrol 1\nframes 3\nneutral replace from 0 for 3\n")
            .unwrap()
            .encode()
            .unwrap();
    fs::write(root.join("inputs/controller.dctl"), &controller).unwrap();
    let mut request: HarnessRunRequest =
        serde_json::from_slice(&fs::read(draft_path).unwrap()).unwrap();
    request.executable = artifact("inputs/dusklight", wrapper.as_bytes());
    request.input = ObjectiveSeed::Controller {
        artifact: artifact("inputs/controller.dctl", &controller),
    };
    request.logical_tick_budget = 3;
    request.artifact_destination = artifact_destination.into();
    request.content_sha256 = Digest::ZERO;
    fs::write(draft_path, serde_json::to_vec_pretty(&request).unwrap()).unwrap();
}

#[cfg(unix)]
fn execute_mock_controller(
    huntctl_executable: &str,
    mode: Option<&str>,
    artifact_destination: &str,
) -> (PathBuf, HarnessRunRequest, HarnessRunResult) {
    let root = unique_root();
    let suite_path = write_suite(&root);
    let request_draft = write_run_request_draft(&root, &suite_path);
    rewrite_request_for_mock_controller(
        &root,
        &request_draft,
        huntctl_executable,
        mode,
        artifact_destination,
    );
    let request_path = root.join("run-request.json");
    let sealed = Command::new(huntctl_executable)
        .args(["harness", "seal-run-request", "--input"])
        .arg(&request_draft)
        .arg("--output")
        .arg(&request_path)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        sealed.status.success(),
        "{}",
        String::from_utf8_lossy(&sealed.stderr)
    );
    let executed = Command::new(huntctl_executable)
        .args(["harness", "execute", "--request"])
        .arg(&request_path)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        executed.status.success(),
        "{}",
        String::from_utf8_lossy(&executed.stderr)
    );
    (
        root,
        serde_json::from_slice(&fs::read(request_path).unwrap()).unwrap(),
        serde_json::from_slice(&executed.stdout).unwrap(),
    )
}

#[cfg(unix)]
fn rewrite_request_for_mock_native_execution(
    root: &Path,
    draft_path: &Path,
    huntctl_executable: &str,
) {
    let wrapper = write_mock_native_wrapper(root, huntctl_executable);

    let objective_bytes = b"milestones 1.0\n\nmilestone stage_ready {\n  phase post_sim\n  when boundary.reached\n}\n";
    fs::write(
        root.join("inputs/core-objective.milestones"),
        objective_bytes,
    )
    .unwrap();
    let compiled =
        milestone_dsl::compile_source(std::str::from_utf8(objective_bytes).unwrap()).unwrap();

    let mut request: HarnessRunRequest =
        serde_json::from_slice(&fs::read(draft_path).unwrap()).unwrap();
    request.executable = artifact("inputs/dusklight", wrapper.as_bytes());
    request.boot = ObjectiveBoot::Process;
    request.objective = ObjectiveProgramReference {
        source: artifact("inputs/core-objective.milestones", objective_bytes),
        program_sha256: Digest(compiled.program_sha256),
        goal: "stage_ready".into(),
    };
    request.identity.predicate_program_digest = request.objective.program_sha256;
    request.observation_requirements = ObjectiveObservationRequirements {
        schema: OBJECTIVE_OBSERVATION_REQUIREMENTS_SCHEMA_V1.into(),
        families: vec![ObservationFamilyRequirement {
            id: "core".into(),
            minimum_version: 1,
        }],
        facts: vec!["boundary.reached".into()],
    };
    request.input = ObjectiveSeed::Neutral;
    request.logical_tick_budget = 2;
    request.artifact_destination = "artifacts/mock-native-run".into();
    request.content_sha256 = Digest::ZERO;
    fs::write(draft_path, serde_json::to_vec_pretty(&request).unwrap()).unwrap();
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

    let pending_inspection = Command::new(executable)
        .args(["harness", "inspect-objective", "--request"])
        .arg(&request_path)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(pending_inspection.status.success());
    let pending_text = String::from_utf8(pending_inspection.stdout).unwrap();
    assert!(pending_text.contains("Objective: stage_ready"));
    assert!(pending_text.contains("Progress: not run"));
    assert!(pending_text.contains("  - run result"));
    assert!(pending_text.contains("Source objective:\n---\n"));

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

    let completed_inspection = Command::new(executable)
        .args(["harness", "inspect-objective", "--request"])
        .arg(&request_path)
        .arg("--result")
        .arg(&result_path)
        .arg("--artifact-root")
        .arg(&artifact_root)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        completed_inspection.status.success(),
        "{}",
        String::from_utf8_lossy(&completed_inspection.stderr)
    );
    let completed_text = String::from_utf8(completed_inspection.stdout).unwrap();
    assert!(completed_text.contains("Progress: reached at logical tick 5"));
    assert!(completed_text.contains("First-hit boundary:"));
    assert!(completed_text.contains("Missing evidence:\n  - none"));

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

#[cfg(unix)]
#[test]
fn executes_a_tape_request_through_the_authenticated_boundary() {
    let executable = env!("CARGO_BIN_EXE_huntctl");
    let root = unique_root();
    let suite_path = write_suite(&root);
    let request_draft = write_run_request_draft(&root, &suite_path);
    rewrite_request_for_mock_native_execution(&root, &request_draft, executable);
    let request_path = root.join("run-request.json");

    let sealed = Command::new(executable)
        .args(["harness", "seal-run-request", "--input"])
        .arg(&request_draft)
        .arg("--output")
        .arg(&request_path)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        sealed.status.success(),
        "{}",
        String::from_utf8_lossy(&sealed.stderr)
    );

    let executed = Command::new(executable)
        .args(["harness", "execute", "--request"])
        .arg(&request_path)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        executed.status.success(),
        "{}",
        String::from_utf8_lossy(&executed.stderr)
    );
    let result: HarnessRunResult = serde_json::from_slice(&executed.stdout).unwrap();
    assert_eq!(result.terminal, HarnessTerminalReason::Reached);
    assert!(result.artifacts.complete);
    assert_eq!(result.timing.logical_ticks, 1);

    let artifact_root = root.join("artifacts/mock-native-run");
    assert!(artifact_root.join("result.json").is_file());
    let request: HarnessRunRequest =
        serde_json::from_slice(&fs::read(&request_path).unwrap()).unwrap();
    result.validate_files(&request, &artifact_root).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn execution_reports_missing_trace_families_as_unsupported() {
    let executable = env!("CARGO_BIN_EXE_huntctl");
    let root = unique_root();
    let suite_path = write_suite(&root);
    let request_draft = write_run_request_draft(&root, &suite_path);
    let wrapper = write_mock_native_wrapper(&root, executable);
    let mut request: HarnessRunRequest =
        serde_json::from_slice(&fs::read(&request_draft).unwrap()).unwrap();
    request.executable = artifact("inputs/dusklight", wrapper.as_bytes());
    request.artifact_destination = "artifacts/unsupported-stage-run".into();
    request.content_sha256 = Digest::ZERO;
    fs::write(&request_draft, serde_json::to_vec_pretty(&request).unwrap()).unwrap();
    let request_path = root.join("run-request.json");

    let sealed = Command::new(executable)
        .args(["harness", "seal-run-request", "--input"])
        .arg(&request_draft)
        .arg("--output")
        .arg(&request_path)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        sealed.status.success(),
        "{}",
        String::from_utf8_lossy(&sealed.stderr)
    );

    let executed = Command::new(executable)
        .args(["harness", "execute", "--request"])
        .arg(&request_path)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        executed.status.success(),
        "{}",
        String::from_utf8_lossy(&executed.stderr)
    );
    let result: HarnessRunResult = serde_json::from_slice(&executed.stdout).unwrap();
    assert_eq!(result.terminal, HarnessTerminalReason::Unsupported);
    assert!(!result.objective.reached);
    assert!(!result.artifacts.complete);
    assert_eq!(
        result
            .detail
            .observation_issues
            .iter()
            .map(|issue| issue.family.as_str())
            .collect::<Vec<_>>(),
        ["player_motion", "stage"]
    );
    assert_eq!(
        result.detail.missing_query_facts,
        ["player.exists", "stage.name"]
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn host_timeout_retains_authenticated_partial_artifacts() {
    let executable = env!("CARGO_BIN_EXE_huntctl");
    let root = unique_root();
    let suite_path = write_suite(&root);
    let request_draft = write_run_request_draft(&root, &suite_path);
    rewrite_request_for_mock_native_execution(&root, &request_draft, executable);
    let wrapper = write_mock_native_wrapper_mode(&root, executable, Some("timeout"));
    let mut request: HarnessRunRequest =
        serde_json::from_slice(&fs::read(&request_draft).unwrap()).unwrap();
    request.executable = artifact("inputs/dusklight", wrapper.as_bytes());
    request.host_timeout_seconds = 1;
    request.artifact_destination = "artifacts/mock-host-timeout".into();
    request.content_sha256 = Digest::ZERO;
    fs::write(&request_draft, serde_json::to_vec_pretty(&request).unwrap()).unwrap();
    let request_path = root.join("run-request.json");

    let sealed = Command::new(executable)
        .args(["harness", "seal-run-request", "--input"])
        .arg(&request_draft)
        .arg("--output")
        .arg(&request_path)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(sealed.status.success());
    let executed = Command::new(executable)
        .args(["harness", "execute", "--request"])
        .arg(&request_path)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        executed.status.success(),
        "{}",
        String::from_utf8_lossy(&executed.stderr)
    );
    let result: HarnessRunResult = serde_json::from_slice(&executed.stdout).unwrap();
    assert_eq!(result.terminal, HarnessTerminalReason::HostTimeout);
    assert!(!result.objective.reached);
    assert!(!result.artifacts.complete);
    assert!(result.artifacts.stdout.is_some());
    assert!(result.artifacts.stderr.is_some());
    assert!(result.artifacts.realized_input.is_none());
    assert!(result.artifacts.gameplay_trace.is_none());
    assert!(result.artifacts.objective_result.is_none());

    let request: HarnessRunRequest =
        serde_json::from_slice(&fs::read(&request_path).unwrap()).unwrap();
    let artifact_root = root.join(&request.artifact_destination);
    assert!(artifact_root.join("result.json").is_file());
    result.validate_files(&request, &artifact_root).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn executes_a_reactive_controller_through_the_authenticated_boundary() {
    let executable = env!("CARGO_BIN_EXE_huntctl");
    let (root, request, result) =
        execute_mock_controller(executable, None, "artifacts/mock-controller-run");
    assert_eq!(result.terminal, HarnessTerminalReason::Reached);
    assert!(result.artifacts.complete);
    assert_eq!(result.timing.consumed_input_ticks, 1);
    result
        .validate_files(&request, &root.join(&request.artifact_destination))
        .unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn enforces_the_logical_tick_budget_independently_of_host_time() {
    let executable = env!("CARGO_BIN_EXE_huntctl");
    let (root, request, result) = execute_mock_controller(
        executable,
        Some("miss"),
        "artifacts/mock-controller-budget-exhausted",
    );
    assert_eq!(result.terminal, HarnessTerminalReason::Exhausted);
    assert!(!result.objective.reached);
    assert!(result.artifacts.complete);
    assert_eq!(
        result.timing.consumed_input_ticks,
        request.logical_tick_budget
    );
    result
        .validate_files(&request, &root.join(&request.artifact_destination))
        .unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn failure_terminals_retain_authenticated_partial_artifacts() {
    let executable = env!("CARGO_BIN_EXE_huntctl");
    for (mode, terminal, destination) in [
        (
            "protocol-failure",
            HarnessTerminalReason::ProtocolFailure,
            "artifacts/mock-controller-protocol-failure",
        ),
        (
            "game-crash",
            HarnessTerminalReason::GameCrashed,
            "artifacts/mock-controller-game-crash",
        ),
    ] {
        let (root, request, result) = execute_mock_controller(executable, Some(mode), destination);
        assert_eq!(result.terminal, terminal);
        assert!(!result.objective.reached);
        assert!(!result.artifacts.complete);
        assert!(result.artifacts.stdout.is_some());
        assert!(result.artifacts.stderr.is_some());
        result
            .validate_files(&request, &root.join(&request.artifact_destination))
            .unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn reports_an_exact_controller_target_loss_without_calling_it_exhaustion() {
    let executable = env!("CARGO_BIN_EXE_huntctl");
    let (root, request, result) = execute_mock_controller(
        executable,
        Some("target-lost"),
        "artifacts/mock-controller-target-lost",
    );
    assert_eq!(result.terminal, HarnessTerminalReason::TargetLost);
    assert!(!result.objective.reached);
    assert!(!result.artifacts.complete);
    assert_eq!(result.timing.consumed_input_ticks, 2);
    result
        .validate_files(&request, &root.join(&request.artifact_destination))
        .unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn search_and_learned_origin_candidates_share_the_authenticated_executor() {
    let executable = env!("CARGO_BIN_EXE_huntctl");
    let root = unique_root();
    let suite_path = write_suite(&root);
    let request_draft = write_run_request_draft(&root, &suite_path);
    rewrite_request_for_mock_native_execution(&root, &request_draft, executable);
    let mut request: HarnessRunRequest =
        serde_json::from_slice(&fs::read(&request_draft).unwrap()).unwrap();
    request.logical_tick_budget = 1_000;
    request.content_sha256 = Digest::ZERO;
    fs::write(&request_draft, serde_json::to_vec_pretty(&request).unwrap()).unwrap();
    let request_path = root.join("run-request.json");
    let sealed = Command::new(executable)
        .args(["harness", "seal-run-request", "--input"])
        .arg(&request_draft)
        .arg("--output")
        .arg(&request_path)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        sealed.status.success(),
        "{}",
        String::from_utf8_lossy(&sealed.stderr)
    );

    let population = root.join("population");
    let seed = Candidate {
        schema: CANDIDATE_SCHEMA.into(),
        segment: SegmentProfile::BootToFsp103,
        boot: huntctl::tape::TapeBoot::Process,
        actions: vec![MacroAction::Neutral { frames: 1 }],
        ancestry: Ancestry::default(),
    };
    let seed_id = seed.id().unwrap();
    let learned = Candidate {
        actions: vec![MacroAction::Neutral { frames: 2 }],
        ancestry: Ancestry {
            generation: 1,
            parent_id: Some(seed_id.clone()),
            mutation: Some("q_fitted_proposal:test-model".into()),
            intervention: None,
        },
        ..seed.clone()
    };
    let learned_id = learned.id().unwrap();
    write_explicit_population(
        &population,
        SegmentProfile::BootToFsp103,
        1,
        vec![seed, learned],
    )
    .unwrap();

    let output = root.join("search-evaluation");
    let evaluated = Command::new(executable)
        .args(["search", "evaluate", "--population"])
        .arg(population.join("manifest.json"))
        .arg("--output")
        .arg(&output)
        .args(["--workers", "1", "--repetitions", "1", "--run-request"])
        .arg(&request_path)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        evaluated.status.success(),
        "{}",
        String::from_utf8_lossy(&evaluated.stderr)
    );
    let report: Value = serde_json::from_slice(&evaluated.stdout).unwrap();
    assert_eq!(report["schema"], "dusklight-search-evaluation/v5");
    assert_eq!(report["attempts"].as_array().unwrap().len(), 2);
    let mut tape_digests = Vec::new();
    let mut objective_digests = Vec::new();
    let mut identity_digests = Vec::new();
    let mut candidate_ids = Vec::new();
    for attempt in report["attempts"].as_array().unwrap() {
        assert_eq!(attempt["harness_terminal"], "reached");
        assert!(attempt["harness_request_sha256"].is_string());
        assert!(attempt["harness_result_sha256"].is_string());
        let request = PathBuf::from(attempt["harness_request"].as_str().unwrap());
        let result = PathBuf::from(attempt["harness_result"].as_str().unwrap());
        assert!(request.is_file());
        assert!(result.is_file());
        let run_request: HarnessRunRequest =
            serde_json::from_slice(&fs::read(&request).unwrap()).unwrap();
        let run_result: HarnessRunResult =
            serde_json::from_slice(&fs::read(&result).unwrap()).unwrap();
        assert_eq!(
            attempt["harness_request_sha256"],
            run_request.content_sha256.to_string()
        );
        assert_eq!(
            attempt["harness_result_sha256"],
            run_result.content_sha256.to_string()
        );
        assert_eq!(run_result.request_sha256, run_request.content_sha256);
        run_result
            .validate_files(&run_request, result.parent().unwrap())
            .unwrap();
        let ObjectiveSeed::Tape { artifact } = run_request.input else {
            panic!("search candidate did not become an authenticated tape request");
        };
        tape_digests.push(artifact.sha256);
        objective_digests.push(run_request.objective.program_sha256);
        identity_digests.push(run_request.identity.content_digest);
        candidate_ids.push(attempt["candidate_id"].as_str().unwrap().to_owned());
    }
    let mut expected_ids = vec![seed_id.clone(), learned_id.clone()];
    expected_ids.sort();
    assert_eq!(candidate_ids, expected_ids);
    assert_ne!(tape_digests[0], tape_digests[1]);
    assert_eq!(objective_digests[0], objective_digests[1]);
    assert_eq!(identity_digests[0], identity_digests[1]);
    let learned_attempt = report["attempts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|attempt| attempt["candidate_id"] == learned_id)
        .unwrap();
    assert_eq!(learned_attempt["ancestry"]["parent_id"], seed_id);
    assert_eq!(
        learned_attempt["ancestry"]["mutation"],
        "q_fitted_proposal:test-model"
    );

    let conflicting = Command::new(executable)
        .args(["search", "evaluate", "--population"])
        .arg(population.join("manifest.json"))
        .arg("--output")
        .arg(root.join("conflicting-evaluation"))
        .arg("--run-request")
        .arg(&request_path)
        .arg("--repository-root")
        .arg(&root)
        .arg("--game")
        .arg(root.join("inputs/dusklight"))
        .output()
        .unwrap();
    assert!(!conflicting.status.success());
    assert!(String::from_utf8_lossy(&conflicting.stderr).contains("sole execution authority"));

    let search_run = root.join("search-run");
    let searched = Command::new(executable)
        .args(["search", "run", "--segment", "boot_to_fsp103", "--output"])
        .arg(&search_run)
        .args([
            "--size",
            "2",
            "--elites",
            "1",
            "--generations",
            "2",
            "--workers",
            "1",
            "--repetitions",
            "1",
            "--run-request",
        ])
        .arg(&request_path)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        searched.status.success(),
        "{}",
        String::from_utf8_lossy(&searched.stderr)
    );
    for generation in ["g000", "g001"] {
        let evaluation: Value = serde_json::from_slice(
            &fs::read(
                search_run
                    .join(generation)
                    .join("evaluations/evaluation.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(
            evaluation["attempts"]
                .as_array()
                .unwrap()
                .iter()
                .all(|attempt| attempt["harness_terminal"] == "reached")
        );
    }
    fs::remove_dir_all(root).unwrap();
}
