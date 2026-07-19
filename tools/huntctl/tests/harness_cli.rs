use huntctl::Digest;
use huntctl::artifact::{ARTIFACT_SCHEMA_VERSION, ArtifactIdentity, BuildIdentity};
use huntctl::candidate_envelope::{
    CandidateEnvelope, CandidateEnvelopeSet, NamedDigest, ProposerIdentity, ProposerKind,
};
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
    Ancestry, CANDIDATE_SCHEMA, Candidate, ControllerButton, MacroAction, PopulationManifest,
    SegmentProfile, write_explicit_population,
};
use huntctl::throughput_benchmark::ColdProcessBenchmarkReport;
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

fn digest(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

fn write_proposal_envelopes(
    population_root: &Path,
    file_name: &str,
    kind: ProposerKind,
    proposer_id: &str,
    request: &HarnessRunRequest,
    objective: Option<NamedDigest>,
) {
    let manifest: PopulationManifest =
        serde_json::from_slice(&fs::read(population_root.join("manifest.json")).unwrap()).unwrap();
    let configuration_byte = match kind {
        ProposerKind::Scripted => 1,
        ProposerKind::Random => 2,
        ProposerKind::StructuredSearch => 3,
        ProposerKind::Learned => 4,
    };
    let proposer = ProposerIdentity {
        kind,
        id: proposer_id.into(),
        version: "test-v1".into(),
        configuration_sha256: Digest([configuration_byte; 32]),
    };
    let envelopes = manifest
        .members
        .iter()
        .map(|member| {
            CandidateEnvelope::build(
                member.candidate_id.parse().unwrap(),
                member
                    .ancestry
                    .parent_id
                    .as_deref()
                    .map(str::parse)
                    .transpose()
                    .unwrap(),
                member.ancestry.generation,
                objective.clone().unwrap_or_else(|| {
                    NamedDigest::new(
                        request.objective.goal.clone(),
                        request.objective.program_sha256,
                    )
                }),
                NamedDigest::new(
                    request.action_schema.id.clone(),
                    request.action_schema.sha256,
                ),
                manifest.rng_seed,
                proposer.clone(),
            )
            .unwrap()
        })
        .collect();
    let set = CandidateEnvelopeSet::build(envelopes).unwrap();
    fs::write(
        population_root.join(file_name),
        serde_json::to_vec_pretty(&set).unwrap(),
    )
    .unwrap();
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
fn cold_process_benchmark_retains_comparable_authenticated_attempts() {
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

    let report_path = root.join("artifacts/cold-process/report.json");
    let benchmark = Command::new(executable)
        .args(["benchmark", "cold-process", "--request"])
        .arg(&request_path)
        .args(["--artifact-root", "artifacts/cold-process"])
        .arg("--output")
        .arg(&report_path)
        .args(["--repository-root"])
        .arg(&root)
        .args(["--repetitions", "2"])
        .output()
        .unwrap();
    assert!(
        benchmark.status.success(),
        "{}",
        String::from_utf8_lossy(&benchmark.stderr)
    );
    let report: ColdProcessBenchmarkReport =
        serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
    report.validate().unwrap();
    assert!(report.comparable);
    assert_eq!(report.attempts.len(), 2);
    assert_ne!(
        report.attempts[0].request_sha256,
        report.attempts[1].request_sha256
    );
    assert_eq!(
        report.attempts[0].gameplay_trace_sha256,
        report.attempts[1].gameplay_trace_sha256
    );
    assert_eq!(report.summary.total_logical_ticks, 2);
    assert!(report.summary.candidates_per_second_millionths > 0);
    let validated = Command::new(executable)
        .args(["benchmark", "validate-cold-process", "--report"])
        .arg(&report_path)
        .output()
        .unwrap();
    assert!(validated.status.success());
    for attempt in 1..=2 {
        assert!(
            root.join(format!(
                "artifacts/cold-process/requests/attempt-{attempt:03}.json"
            ))
            .is_file()
        );
        assert!(
            root.join(format!(
                "artifacts/cold-process/attempt-{attempt:03}/result.json"
            ))
            .is_file()
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn cold_process_benchmark_rejects_different_native_boundaries() {
    let executable = env!("CARGO_BIN_EXE_huntctl");
    let root = unique_root();
    let suite_path = write_suite(&root);
    let request_draft = write_run_request_draft(&root, &suite_path);
    rewrite_request_for_mock_controller(
        &root,
        &request_draft,
        executable,
        Some("unstable-fingerprint"),
        "artifacts/unused-template-destination",
    );
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

    let report_path = root.join("artifacts/unstable/report.json");
    let benchmark = Command::new(executable)
        .args(["benchmark", "cold-process", "--request"])
        .arg(&request_path)
        .args(["--artifact-root", "artifacts/unstable"])
        .arg("--output")
        .arg(&report_path)
        .args(["--repository-root"])
        .arg(&root)
        .args(["--repetitions", "2"])
        .output()
        .unwrap();
    assert!(!benchmark.status.success());
    assert!(
        String::from_utf8_lossy(&benchmark.stderr)
            .contains("cold-process attempts are not comparable")
    );
    let report: ColdProcessBenchmarkReport =
        serde_json::from_slice(&fs::read(report_path).unwrap()).unwrap();
    report.validate().unwrap();
    assert!(!report.comparable);
    assert!(
        report
            .comparison_issue
            .as_deref()
            .unwrap()
            .contains("not semantically and artifact-identical")
    );
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
        actions: vec![MacroAction::Move {
            angle_degrees: 0,
            magnitude: 64,
            frames: 1,
        }],
        ancestry: Ancestry::default(),
    };
    let optimizer_seed = root.join("optimizer-seed.json");
    fs::write(&optimizer_seed, serde_json::to_vec_pretty(&seed).unwrap()).unwrap();
    let beam_options = root.join("beam-options.json");
    fs::write(
        &beam_options,
        serde_json::to_vec_pretty(&vec![MacroAction::Neutral { frames: 1 }]).unwrap(),
    )
    .unwrap();
    let continuous_axes = root.join("continuous-axes.json");
    fs::write(
        &continuous_axes,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": "dusklight-continuous-axes/v1",
            "axes": [{
                "name": "heading",
                "action_index": 0,
                "parameter": { "kind": "move_heading_degrees" },
                "minimum": -90.0,
                "maximum": 90.0
            }]
        }))
        .unwrap(),
    )
    .unwrap();
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
    write_proposal_envelopes(
        &population,
        "scripted-envelopes.json",
        ProposerKind::Scripted,
        "scripted.fixture",
        &request,
        None,
    );
    write_proposal_envelopes(
        &population,
        "random-envelopes.json",
        ProposerKind::Random,
        "random.uniform",
        &request,
        None,
    );

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

    let beam_root = root.join("authenticated-beam");
    let beam = Command::new(executable)
        .args(["search", "beam", "--candidate"])
        .arg(&optimizer_seed)
        .arg("--options")
        .arg(&beam_options)
        .arg("--output")
        .arg(&beam_root)
        .args([
            "--beam-width",
            "1",
            "--maximum-depth",
            "1",
            "--candidate-budget",
            "1",
            "--workers",
            "1",
            "--repetitions",
            "2",
            "--run-request",
        ])
        .arg(&request_path)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        beam.status.success(),
        "{}",
        String::from_utf8_lossy(&beam.stderr)
    );

    let continuous_root = root.join("authenticated-continuous");
    let continuous = Command::new(executable)
        .args(["search", "continuous", "--method", "cem", "--candidate"])
        .arg(&optimizer_seed)
        .arg("--axes")
        .arg(&continuous_axes)
        .arg("--output")
        .arg(&continuous_root)
        .args([
            "--generations",
            "1",
            "--population",
            "4",
            "--elites",
            "1",
            "--candidate-budget",
            "4",
            "--rng-seed",
            "7",
            "--workers",
            "1",
            "--repetitions",
            "2",
            "--run-request",
        ])
        .arg(&request_path)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        continuous.status.success(),
        "{}",
        String::from_utf8_lossy(&continuous.stderr)
    );

    let bayesian_root = root.join("authenticated-bayesian");
    let bayesian = Command::new(executable)
        .args(["search", "bayesian", "--candidate"])
        .arg(&optimizer_seed)
        .arg("--axes")
        .arg(&continuous_axes)
        .arg("--output")
        .arg(&bayesian_root)
        .args([
            "--generations",
            "1",
            "--batch-size",
            "2",
            "--initial-samples",
            "2",
            "--acquisition-pool",
            "16",
            "--candidate-budget",
            "2",
            "--rng-seed",
            "7",
            "--workers",
            "1",
            "--repetitions",
            "2",
            "--run-request",
        ])
        .arg(&request_path)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        bayesian.status.success(),
        "{}",
        String::from_utf8_lossy(&bayesian.stderr)
    );

    for evaluation_path in [
        beam_root.join("d000/evaluations/evaluation.json"),
        continuous_root.join("g000/evaluations/evaluation.json"),
        bayesian_root.join("g000/evaluations/evaluation.json"),
    ] {
        let evaluation: Value =
            serde_json::from_slice(&fs::read(evaluation_path).unwrap()).unwrap();
        for attempt in evaluation["attempts"].as_array().unwrap() {
            assert_eq!(attempt["harness_terminal"], "reached");
            let request_path = PathBuf::from(attempt["harness_request"].as_str().unwrap());
            let result_path = PathBuf::from(attempt["harness_result"].as_str().unwrap());
            let request: HarnessRunRequest =
                serde_json::from_slice(&fs::read(request_path).unwrap()).unwrap();
            let result: HarnessRunResult =
                serde_json::from_slice(&fs::read(&result_path).unwrap()).unwrap();
            result
                .validate_files(&request, result_path.parent().unwrap())
                .unwrap();
        }
    }

    let conflicting_beam = Command::new(executable)
        .args(["search", "beam", "--candidate"])
        .arg(&optimizer_seed)
        .arg("--options")
        .arg(&beam_options)
        .arg("--output")
        .arg(root.join("conflicting-beam"))
        .arg("--run-request")
        .arg(&request_path)
        .arg("--repository-root")
        .arg(&root)
        .arg("--game")
        .arg(root.join("inputs/dusklight"))
        .output()
        .unwrap();
    assert!(!conflicting_beam.status.success());
    assert!(String::from_utf8_lossy(&conflicting_beam.stderr).contains("sole execution authority"));

    let tournament_definition = root.join("tournament.json");
    fs::write(
        &tournament_definition,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": "dusklight-proposer-tournament-definition/v2",
            "budget_unit": "episodes",
            "budget_per_proposer": 2,
            "proposers": [
                {
                    "name": "scripted",
                    "kind": "incumbent_mutation",
                    "population": "population/manifest.json",
                    "proposal_envelopes": "population/scripted-envelopes.json"
                },
                {
                    "name": "random",
                    "kind": "blind_exploration",
                    "population": "population/manifest.json",
                    "proposal_envelopes": "population/random-envelopes.json"
                }
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    let tournament_root = root.join("authenticated-tournament");
    let tournament = Command::new(executable)
        .args(["search", "tournament", "--definition"])
        .arg(&tournament_definition)
        .arg("--output")
        .arg(&tournament_root)
        .args(["--workers", "1", "--repetitions", "2", "--run-request"])
        .arg(&request_path)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        tournament.status.success(),
        "{}",
        String::from_utf8_lossy(&tournament.stderr)
    );
    let tournament_summary: Value = serde_json::from_slice(&tournament.stdout).unwrap();
    assert_eq!(
        tournament_summary["schema"],
        "dusklight-proposer-tournament/v3"
    );
    assert_eq!(tournament_summary["rows"].as_array().unwrap().len(), 2);
    let tournament_evaluation: Value = serde_json::from_slice(
        &fs::read(tournament_root.join("evaluations/evaluation.json")).unwrap(),
    )
    .unwrap();
    assert!(
        tournament_evaluation["attempts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|attempt| attempt["harness_terminal"] == "reached"
                && attempt["harness_request_sha256"].is_string()
                && attempt["harness_result_sha256"].is_string())
    );

    let conflicting_tournament = Command::new(executable)
        .args(["search", "tournament", "--definition"])
        .arg(&tournament_definition)
        .arg("--output")
        .arg(root.join("conflicting-tournament"))
        .arg("--run-request")
        .arg(&request_path)
        .arg("--repository-root")
        .arg(&root)
        .arg("--dvd")
        .arg(root.join("inputs/disc.iso"))
        .output()
        .unwrap();
    assert!(!conflicting_tournament.status.success());
    assert!(
        String::from_utf8_lossy(&conflicting_tournament.stderr)
            .contains("sole execution authority")
    );

    let coordinate_wrapper =
        write_mock_native_wrapper_mode(&root, executable, Some("coordinate-golf"));
    let boot_objective_bytes = b"milestones 1.0\n\nmilestone gameplay-ready-f-sp103 {\n  phase post_sim\n  when boundary.reached\n}\n";
    fs::write(
        root.join("inputs/boot-objective.milestones"),
        boot_objective_bytes,
    )
    .unwrap();
    let boot_program =
        milestone_dsl::compile_source(std::str::from_utf8(boot_objective_bytes).unwrap()).unwrap();
    let mut boot_request = request.clone();
    boot_request.executable = artifact("inputs/dusklight", coordinate_wrapper.as_bytes());
    boot_request.objective = ObjectiveProgramReference {
        source: artifact("inputs/boot-objective.milestones", boot_objective_bytes),
        program_sha256: Digest(boot_program.program_sha256),
        goal: "gameplay-ready-f-sp103".into(),
    };
    boot_request.identity.predicate_program_digest = boot_request.objective.program_sha256;
    let observation_path = root.join(&boot_request.observation_view.source.path);
    let mut observation: huntctl::observation_view::ObservationSpec =
        serde_json::from_slice(&fs::read(observation_path).unwrap()).unwrap();
    observation.objective.id = boot_request.objective.goal.clone();
    let observation_bytes = serde_json::to_vec_pretty(&observation).unwrap();
    fs::write(
        root.join("inputs/boot-observation.json"),
        &observation_bytes,
    )
    .unwrap();
    boot_request.observation_view = huntctl::harness::objective_suite::ObservationViewReference {
        source: artifact("inputs/boot-observation.json", &observation_bytes),
        schema_sha256: observation.digest().unwrap(),
    };
    boot_request.identity.observation_schema_digest = boot_request.observation_view.schema_sha256;
    boot_request.logical_tick_budget = 1_000;
    boot_request.content_sha256 = Digest::ZERO;
    let boot_request_draft = root.join("boot-request.draft.json");
    let boot_request_path = root.join("boot-request.json");
    fs::write(
        &boot_request_draft,
        serde_json::to_vec_pretty(&boot_request).unwrap(),
    )
    .unwrap();
    let sealed_boot = Command::new(executable)
        .args(["harness", "seal-run-request", "--input"])
        .arg(&boot_request_draft)
        .arg("--output")
        .arg(&boot_request_path)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        sealed_boot.status.success(),
        "{}",
        String::from_utf8_lossy(&sealed_boot.stderr)
    );

    let boot_candidate = Candidate {
        schema: CANDIDATE_SCHEMA.into(),
        segment: SegmentProfile::BootToFsp103,
        boot: huntctl::tape::TapeBoot::Process,
        actions: vec![
            MacroAction::Neutral { frames: 10 },
            MacroAction::Press {
                buttons: vec![ControllerButton::Start],
                hold_frames: 1,
                neutral_frames: 9,
            },
            MacroAction::Press {
                buttons: vec![ControllerButton::A],
                hold_frames: 1,
                neutral_frames: 99,
            },
        ],
        ancestry: Ancestry::default(),
    };
    let boot_candidate_path = root.join("boot-candidate.json");
    fs::write(
        &boot_candidate_path,
        serde_json::to_vec_pretty(&boot_candidate).unwrap(),
    )
    .unwrap();

    let minimize_root = root.join("authenticated-boot-minimize");
    let minimized = Command::new(executable)
        .args(["search", "minimize-boot", "--candidate"])
        .arg(&boot_candidate_path)
        .arg("--output")
        .arg(&minimize_root)
        .args(["--workers", "2", "--repetitions", "2", "--run-request"])
        .arg(&boot_request_path)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        minimized.status.success(),
        "{}",
        String::from_utf8_lossy(&minimized.stderr)
    );

    let golf_root = root.join("authenticated-boot-golf");
    let golfed = Command::new(executable)
        .args(["search", "golf-boot", "--candidate"])
        .arg(&boot_candidate_path)
        .arg("--output")
        .arg(&golf_root)
        .args(["--workers", "2", "--repetitions", "2", "--run-request"])
        .arg(&boot_request_path)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        golfed.status.success(),
        "{}",
        String::from_utf8_lossy(&golfed.stderr)
    );
    for proof_path in [
        minimize_root.join("proof.json"),
        golf_root.join("proof.json"),
    ] {
        let proof: Value = serde_json::from_slice(&fs::read(proof_path).unwrap()).unwrap();
        assert!(proof["attempts"].as_array().unwrap().iter().all(|attempt| {
            attempt["harness_terminal"] == "reached"
                && attempt["harness_request_sha256"].is_string()
                && attempt["harness_result_sha256"].is_string()
        }));
    }

    let conflicting_golf = Command::new(executable)
        .args(["search", "golf-boot", "--candidate"])
        .arg(&boot_candidate_path)
        .arg("--output")
        .arg(root.join("conflicting-golf"))
        .arg("--run-request")
        .arg(&boot_request_path)
        .arg("--repository-root")
        .arg(&root)
        .arg("--dvd")
        .arg(root.join("inputs/disc.iso"))
        .output()
        .unwrap();
    assert!(!conflicting_golf.status.success());
    assert!(String::from_utf8_lossy(&conflicting_golf.stderr).contains("sole execution authority"));

    let anchored_wrapper = write_mock_native_wrapper(&root, executable);
    let anchored_source = b"milestones 1.0\n\nmilestone source-ready {\n  phase post_sim\n  when boundary.reached\n}\n\nmilestone entered-f-sp104 {\n  phase post_sim\n  when boundary.reached\n}\n";
    let anchored_program =
        milestone_dsl::compile_source(std::str::from_utf8(anchored_source).unwrap()).unwrap();
    fs::write(
        root.join("inputs/anchored-objective.milestones"),
        anchored_source,
    )
    .unwrap();
    let anchored_program_path = root.join("anchored-objective.dmsp");
    fs::write(&anchored_program_path, &anchored_program.bytes).unwrap();
    let disconnected_pad = huntctl::tape::RawPadState {
        connected: false,
        error: -1,
        ..huntctl::tape::RawPadState::default()
    };
    let mut prefix_frame = huntctl::tape::InputFrame {
        owned_ports: 0x01,
        pads: [disconnected_pad; 4],
        ..huntctl::tape::InputFrame::default()
    };
    prefix_frame.pads[0] = huntctl::tape::RawPadState::default();
    let prefix = huntctl::tape::InputTape {
        boot: huntctl::tape::TapeBoot::Stage {
            stage: "F_SP103".into(),
            room: 1,
            point: 1,
            layer: 3,
            save_slot: None,
            fixture: None,
        },
        frames: vec![prefix_frame],
        ..huntctl::tape::InputTape::default()
    };
    let prefix_path = root.join("anchored-prefix.tape");
    fs::write(&prefix_path, prefix.encode().unwrap()).unwrap();

    let mut anchored_request = request.clone();
    anchored_request.executable = artifact("inputs/dusklight", anchored_wrapper.as_bytes());
    anchored_request.boot = ObjectiveBoot::Stage {
        stage: "F_SP103".into(),
        room: 1,
        point: 1,
        layer: 3,
        save_slot: None,
    };
    anchored_request.objective = ObjectiveProgramReference {
        source: artifact("inputs/anchored-objective.milestones", anchored_source),
        program_sha256: Digest(anchored_program.program_sha256),
        goal: "entered-f-sp104".into(),
    };
    anchored_request.identity.predicate_program_digest = anchored_request.objective.program_sha256;
    anchored_request.action_schema = SchemaIdentity {
        id: "movement-action/v2".into(),
        sha256: huntctl::learning::offline_rl::movement_action_schema_digest_v2(),
    };
    anchored_request.identity.action_schema_digest = anchored_request.action_schema.sha256;
    let mut anchored_observation = observation;
    anchored_observation.objective.id = anchored_request.objective.goal.clone();
    let anchored_observation_bytes = serde_json::to_vec_pretty(&anchored_observation).unwrap();
    fs::write(
        root.join("inputs/anchored-observation.json"),
        &anchored_observation_bytes,
    )
    .unwrap();
    anchored_request.observation_view =
        huntctl::harness::objective_suite::ObservationViewReference {
            source: artifact(
                "inputs/anchored-observation.json",
                &anchored_observation_bytes,
            ),
            schema_sha256: anchored_observation.digest().unwrap(),
        };
    anchored_request.identity.observation_schema_digest =
        anchored_request.observation_view.schema_sha256;
    anchored_request.logical_tick_budget = 1_000;
    anchored_request.content_sha256 = Digest::ZERO;
    let anchored_request_draft = root.join("anchored-request.draft.json");
    let anchored_request_path = root.join("anchored-request.json");
    fs::write(
        &anchored_request_draft,
        serde_json::to_vec_pretty(&anchored_request).unwrap(),
    )
    .unwrap();
    let sealed_anchored = Command::new(executable)
        .args(["harness", "seal-run-request", "--input"])
        .arg(&anchored_request_draft)
        .arg("--output")
        .arg(&anchored_request_path)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        sealed_anchored.status.success(),
        "{}",
        String::from_utf8_lossy(&sealed_anchored.stderr)
    );
    let sealed_anchored_request: HarnessRunRequest =
        serde_json::from_slice(&fs::read(&anchored_request_path).unwrap()).unwrap();

    let anchored_candidate = Candidate {
        schema: CANDIDATE_SCHEMA.into(),
        segment: SegmentProfile::Fsp103ToFsp104,
        boot: huntctl::tape::TapeBoot::Stage {
            stage: "F_SP103".into(),
            room: 1,
            point: 1,
            layer: 3,
            save_slot: None,
            fixture: None,
        },
        actions: vec![MacroAction::PadRun {
            pad: huntctl::tape::RawPadState::default().into(),
            frames: 700,
        }],
        ancestry: Ancestry::default(),
    };
    let anchored_objective_config = huntctl::search_evaluator::AnchoredObjectiveConfig {
        segment: SegmentProfile::Fsp103ToFsp104,
        prefix_tape: prefix_path.clone(),
        milestone_program: anchored_program_path.clone(),
        game: root.join(&sealed_anchored_request.executable.path),
        dvd: root.join(&sealed_anchored_request.game_data.path),
        source_milestone: "source-ready".into(),
        source_boundary_fingerprint: "11111111111111111111111111111111".into(),
        goal_milestone: "entered-f-sp104".into(),
    };
    let anchored_identity =
        huntctl::search_evaluator::prepare_anchored_evaluator(&anchored_objective_config)
            .unwrap()
            .identity()
            .clone();
    let anchored_search_root = root.join("authenticated-anchored-search");
    let anchored_search = huntctl::search_evaluator::run_anchored_search(
        &huntctl::search_evaluator::AnchoredSearchRunConfig {
            search: huntctl::search_evaluator::SearchRunConfig {
                segment: SegmentProfile::Fsp103ToFsp104,
                seed_candidate: Some(anchored_candidate.clone()),
                game: root.join(&sealed_anchored_request.executable.path),
                dvd: root.join(&sealed_anchored_request.game_data.path),
                output_root: anchored_search_root.clone(),
                working_directory: root.clone(),
                game_args_prefix: Vec::new(),
                generations: 1,
                population_size: 1,
                elite_count: 1,
                workers: 1,
                repetitions: 2,
                timeout: Duration::from_secs(30),
                rng_seed: 1,
                harness: Some(huntctl::search_evaluator::HarnessEvaluateConfig {
                    repository_root: root.clone(),
                    request_template: sealed_anchored_request.clone(),
                }),
            },
            objective: anchored_objective_config.clone(),
        },
    )
    .unwrap();
    assert_eq!(anchored_search.objective, anchored_identity);
    let anchored_search_evaluation: Value = serde_json::from_slice(
        &fs::read(anchored_search_root.join("g000/evaluations/evaluation.json")).unwrap(),
    )
    .unwrap();
    assert!(
        anchored_search_evaluation["attempts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|attempt| attempt["harness_terminal"] == "reached"
                && attempt["harness_request_sha256"].is_string()
                && attempt["harness_result_sha256"].is_string())
    );
    let anchored_envelope_objective = NamedDigest::new(
        anchored_identity.goal_milestone.clone(),
        anchored_identity.digest.parse().unwrap(),
    );
    let anchored_tournament_population = root.join("anchored-tournament-population");
    write_explicit_population(
        &anchored_tournament_population,
        SegmentProfile::Fsp103ToFsp104,
        0,
        vec![anchored_candidate.clone()],
    )
    .unwrap();
    write_proposal_envelopes(
        &anchored_tournament_population,
        "scripted-envelopes.json",
        ProposerKind::Scripted,
        "scripted.anchored-fixture",
        &sealed_anchored_request,
        Some(anchored_envelope_objective.clone()),
    );
    write_proposal_envelopes(
        &anchored_tournament_population,
        "random-envelopes.json",
        ProposerKind::Random,
        "random.anchored-uniform",
        &sealed_anchored_request,
        Some(anchored_envelope_objective),
    );
    let anchored_tournament_definition = root.join("anchored-tournament.json");
    fs::write(
        &anchored_tournament_definition,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": "dusklight-proposer-tournament-definition/v2",
            "budget_unit": "episodes",
            "budget_per_proposer": 2,
            "proposers": [
                {
                    "name": "scripted",
                    "kind": "incumbent_mutation",
                    "population": "anchored-tournament-population/manifest.json",
                    "proposal_envelopes": "anchored-tournament-population/scripted-envelopes.json"
                },
                {
                    "name": "random",
                    "kind": "blind_exploration",
                    "population": "anchored-tournament-population/manifest.json",
                    "proposal_envelopes": "anchored-tournament-population/random-envelopes.json"
                }
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    let anchored_tournament_root = root.join("authenticated-anchored-tournament");
    let anchored_tournament = Command::new(executable)
        .args(["search", "tournament", "--definition"])
        .arg(&anchored_tournament_definition)
        .arg("--output")
        .arg(&anchored_tournament_root)
        .arg("--anchored-prefix")
        .arg(&prefix_path)
        .arg("--milestones")
        .arg(&anchored_program_path)
        .args([
            "--segment",
            "fsp103_to_fsp104",
            "--source-milestone",
            "source-ready",
            "--source-boundary-fingerprint",
            "11111111111111111111111111111111",
            "--goal-milestone",
            "entered-f-sp104",
            "--workers",
            "1",
            "--repetitions",
            "2",
            "--run-request",
        ])
        .arg(&anchored_request_path)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        anchored_tournament.status.success(),
        "{}",
        String::from_utf8_lossy(&anchored_tournament.stderr)
    );
    let anchored_tournament_evaluation: Value = serde_json::from_slice(
        &fs::read(anchored_tournament_root.join("evaluations/evaluation.json")).unwrap(),
    )
    .unwrap();
    assert!(
        anchored_tournament_evaluation["attempts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|attempt| attempt["harness_terminal"] == "reached"
                && attempt["harness_request_sha256"].is_string()
                && attempt["harness_result_sha256"].is_string())
    );
    let anchored_candidate_path = root.join("anchored-candidate.json");
    fs::write(
        &anchored_candidate_path,
        serde_json::to_vec_pretty(&anchored_candidate).unwrap(),
    )
    .unwrap();
    let route_minimize_root = root.join("authenticated-route-minimize");
    let route_minimized = Command::new(executable)
        .args(["search", "minimize-route", "--candidate"])
        .arg(&anchored_candidate_path)
        .arg("--anchored-prefix")
        .arg(&prefix_path)
        .arg("--milestones")
        .arg(&anchored_program_path)
        .args([
            "--segment",
            "fsp103_to_fsp104",
            "--source-milestone",
            "source-ready",
            "--source-boundary-fingerprint",
            "11111111111111111111111111111111",
            "--goal-milestone",
            "entered-f-sp104",
            "--candidate-budget",
            "1",
            "--workers",
            "1",
            "--repetitions",
            "2",
            "--output",
        ])
        .arg(&route_minimize_root)
        .arg("--run-request")
        .arg(&anchored_request_path)
        .arg("--repository-root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        route_minimized.status.success(),
        "{}",
        String::from_utf8_lossy(&route_minimized.stderr)
    );
    let route_summary: Value = serde_json::from_slice(&route_minimized.stdout).unwrap();
    assert_eq!(
        route_summary["schema"],
        "dusklight-anchored-route-minimization/v1"
    );
    let final_evaluation: Value = serde_json::from_slice(
        &fs::read(route_minimize_root.join("final-proof/evidence/evaluation.json")).unwrap(),
    )
    .unwrap();
    assert!(final_evaluation["attempts"].as_array().unwrap().iter().all(
        |attempt| attempt["harness_terminal"] == "reached"
            && attempt["harness_request_sha256"].is_string()
            && attempt["harness_result_sha256"].is_string()
    ));
    let mut checkpoints = fs::read_dir(route_minimize_root.join("checkpoints"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    checkpoints.sort();
    let checkpoint: Value =
        serde_json::from_slice(&fs::read(checkpoints.last().unwrap()).unwrap()).unwrap();
    assert_eq!(
        checkpoint["schema"],
        "dusklight-anchored-route-minimization-checkpoint/v2"
    );
    assert_eq!(
        checkpoint["harness_request_sha256"],
        sealed_anchored_request.content_sha256.to_string()
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn campaign_runs_ranked_proposers_and_cold_replays_their_finalists() {
    let executable = env!("CARGO_BIN_EXE_huntctl");
    let root = unique_root();
    let suite_path = write_suite(&root);
    let mut suite: ObjectiveSuite =
        serde_json::from_slice(&fs::read(&suite_path).unwrap()).unwrap();
    suite.cases[0].boot = ObjectiveBoot::Stage {
        stage: "F_SP103".into(),
        room: 1,
        point: 1,
        layer: 3,
        save_slot: None,
    };
    let unsupported_objective = suite.cases[0].objective.clone();
    let unsupported_requirements = suite.cases[0].observation_requirements.clone();
    suite.refresh_content_sha256().unwrap();
    fs::write(&suite_path, suite.to_pretty_json().unwrap()).unwrap();
    let request_draft = write_run_request_draft(&root, &suite_path);
    rewrite_request_for_mock_native_execution(&root, &request_draft, executable);
    let rewritten_request: HarnessRunRequest =
        serde_json::from_slice(&fs::read(&request_draft).unwrap()).unwrap();
    suite.cases[0].objective = rewritten_request.objective.clone();
    suite.cases[0].observation_requirements = rewritten_request.observation_requirements.clone();
    suite.refresh_content_sha256().unwrap();
    fs::write(&suite_path, suite.to_pretty_json().unwrap()).unwrap();
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
        segment: SegmentProfile::Fsp103ToFsp104,
        boot: huntctl::tape::TapeBoot::Stage {
            stage: "F_SP103".into(),
            room: 1,
            point: 1,
            layer: 3,
            save_slot: None,
            fixture: None,
        },
        actions: vec![MacroAction::Neutral { frames: 1 }],
        ancestry: Ancestry::default(),
    };
    write_explicit_population(&population, SegmentProfile::Fsp103ToFsp104, 0, vec![seed]).unwrap();
    let tournament_request: HarnessRunRequest =
        serde_json::from_slice(&fs::read(&request_path).unwrap()).unwrap();
    write_proposal_envelopes(
        &population,
        "scripted-envelopes.json",
        ProposerKind::Scripted,
        "scripted.fixture",
        &tournament_request,
        None,
    );
    write_proposal_envelopes(
        &population,
        "random-envelopes.json",
        ProposerKind::Random,
        "random.uniform",
        &tournament_request,
        None,
    );
    let definition_path = root.join("tournament.json");
    fs::write(
        &definition_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": "dusklight-proposer-tournament-definition/v2",
            "budget_unit": "episodes",
            "budget_per_proposer": 2,
            "proposers": [
                {
                    "name": "scripted",
                    "kind": "incumbent_mutation",
                    "population": "population/manifest.json",
                    "proposal_envelopes": "population/scripted-envelopes.json"
                },
                {
                    "name": "random",
                    "kind": "blind_exploration",
                    "population": "population/manifest.json",
                    "proposal_envelopes": "population/random-envelopes.json"
                }
            ]
        }))
        .unwrap(),
    )
    .unwrap();

    let campaign = Command::new(executable)
        .current_dir(&root)
        .args(["campaign", "--suite"])
        .arg(&suite_path)
        .args([
            "--case",
            "stage-ready",
            "--output",
            "build/stage-ready-campaign",
            "--run-request",
        ])
        .arg(&request_path)
        .arg("--definition")
        .arg(&definition_path)
        .arg("--repository-root")
        .arg(&root)
        .args(["--workers", "1"])
        .output()
        .unwrap();
    assert!(
        campaign.status.success(),
        "{}",
        String::from_utf8_lossy(&campaign.stderr)
    );
    let report: Value = serde_json::from_slice(&campaign.stdout).unwrap();
    assert_eq!(report["schema"], "dusklight-campaign-report/v1");
    assert_eq!(report["passed"], true);
    assert_eq!(report["plan"]["dry_run"], false);
    assert_eq!(report["plan"]["case_id"], "stage-ready");
    assert!(report.get("first_blocker").is_none());
    assert!(
        PathBuf::from(report["plan"]["outputs"]["episodes"].as_str().unwrap())
            .ends_with("build/stage-ready-campaign/evaluations")
    );
    assert_eq!(
        report["observed_terminal_classes"],
        serde_json::json!(["reached"])
    );
    assert_ne!(
        report["request_template_sha256"],
        report["materialized_request_sha256"]
    );
    assert!(
        PathBuf::from(report["materialized_request"].as_str().unwrap())
            .ends_with("build/stage-ready-campaign/requests/template.json")
    );
    assert_eq!(report["rows"].as_array().unwrap().len(), 2);
    for row in report["rows"].as_array().unwrap() {
        assert_eq!(row["charged_episodes"], 2);
        assert_eq!(row["objective_hits"], 1);
        assert_eq!(row["tournament_replay_verdict"], "proved");
        assert_eq!(row["cold_replay_verdict"], "proved");
        assert_eq!(row["cold_replay_attempts"], 2);
        assert!(row["best_candidate_id"].is_string());
        assert_eq!(row["best_score"]["goal_feasible"], true);
        assert!(PathBuf::from(row["best_proved_tape"].as_str().unwrap()).is_file());
        for result in row["replay_results"].as_array().unwrap() {
            assert!(PathBuf::from(result.as_str().unwrap()).is_file());
        }
    }
    assert!(report["winner_proposer"].is_string());
    assert!(PathBuf::from(report["winner_tape"].as_str().unwrap()).is_file());
    let output_root = root.join("build/stage-ready-campaign");
    assert!(output_root.join("report.json").is_file());
    assert!(output_root.join("tournament.summary.json").is_file());
    assert!(output_root.join("requests/template.json").is_file());

    suite.cases[0].objective = unsupported_objective;
    suite.cases[0].observation_requirements = unsupported_requirements;
    suite.refresh_content_sha256().unwrap();
    fs::write(&suite_path, suite.to_pretty_json().unwrap()).unwrap();
    let mut unsupported_request = tournament_request.clone();
    unsupported_request.objective = suite.cases[0].objective.clone();
    unsupported_request.action_schema = suite.cases[0].action_schema.clone();
    write_proposal_envelopes(
        &population,
        "scripted-envelopes.json",
        ProposerKind::Scripted,
        "scripted.fixture",
        &unsupported_request,
        None,
    );
    write_proposal_envelopes(
        &population,
        "random-envelopes.json",
        ProposerKind::Random,
        "random.uniform",
        &unsupported_request,
        None,
    );
    let unsupported_campaign = Command::new(executable)
        .current_dir(&root)
        .args(["campaign", "--suite"])
        .arg(&suite_path)
        .args([
            "--case",
            "stage-ready",
            "--output",
            "build/unsupported-campaign",
            "--run-request",
        ])
        .arg(&request_path)
        .arg("--definition")
        .arg(&definition_path)
        .arg("--repository-root")
        .arg(&root)
        .args(["--workers", "1"])
        .output()
        .unwrap();
    assert!(!unsupported_campaign.status.success());
    let unsupported_report: Value = serde_json::from_slice(&unsupported_campaign.stdout).unwrap();
    assert_eq!(unsupported_report["passed"], false);
    assert_eq!(
        unsupported_report["first_blocker"]["terminal"],
        "unsupported"
    );
    assert_eq!(unsupported_report["first_blocker"]["kind"], "fact");
    assert_eq!(
        unsupported_report["first_blocker"]["value"],
        "player.exists"
    );
    let blocker_artifact = PathBuf::from(
        unsupported_report["first_blocker"]["artifact"]
            .as_str()
            .unwrap(),
    );
    assert!(blocker_artifact.is_file());
    let stderr = String::from_utf8_lossy(&unsupported_campaign.stderr);
    assert!(stderr.contains("first fact player.exists"));
    assert!(stderr.contains(blocker_artifact.to_str().unwrap()));
    fs::remove_dir_all(root).unwrap();
}
