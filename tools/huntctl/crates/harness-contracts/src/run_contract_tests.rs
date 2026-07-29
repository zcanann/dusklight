
use super::*;
use crate::artifact::ARTIFACT_SCHEMA_VERSION;
use crate::milestone_dsl;
use crate::observation_view::movement_state_v2_spec;
use crate::scenario_fixture::{SCENARIO_FIXTURE_SCHEMA, ScenarioFixture};
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

fn root() -> PathBuf {
    std::env::temp_dir().join(format!(
        "huntctl-run-contract-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT_ROOT.fetch_add(1, Ordering::Relaxed),
    ))
}

fn build(game_digest: Digest) -> BuildIdentity {
    BuildIdentity {
        dusklight_commit: "1".repeat(40),
        aurora_commit: "2".repeat(40),
        compiler: "apple-clang-20".into(),
        target: "arm64-apple-darwin".into(),
        profile: "debug-observers".into(),
        feature_digest: Digest([3; 32]),
        game_digest,
        dirty_digest: None,
        fidelity_profile: "native-read-only".into(),
    }
}

fn protocol(capabilities: &[&str]) -> HarnessProtocolIdentity {
    let mut value = HarnessProtocolIdentity {
        name: "dusklight-automation".into(),
        version: 2,
        capabilities_sha256: Digest::ZERO,
        capabilities: capabilities.iter().map(|value| (*value).into()).collect(),
    };
    value.refresh_capabilities_sha256().unwrap();
    value
}

fn request(root: &Path) -> HarnessRunRequest {
    fs::create_dir_all(root.join("inputs")).unwrap();
    let executable = b"test executable";
    let game = b"test game data";
    fs::write(root.join("inputs/dusklight"), executable).unwrap();
    fs::write(root.join("inputs/game.iso"), game).unwrap();

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
    fs::write(root.join("inputs/scenario.json"), &scenario_bytes).unwrap();

    let objective_bytes = b"milestones 1.0\n\nmilestone stage_ready {\n  phase post_sim\n  when stage.name == \"F_SP103\" && player.exists\n}\n";
    fs::write(root.join("inputs/objective.milestones"), objective_bytes).unwrap();
    let objective = milestone_dsl::parse(std::str::from_utf8(objective_bytes).unwrap()).unwrap();
    let compiled = milestone_dsl::compile(&objective).unwrap();

    let mut observation = movement_state_v2_spec();
    observation.objective.id = "stage_ready".into();
    let observation_bytes = serde_json::to_vec_pretty(&observation).unwrap();
    fs::write(root.join("inputs/observation.json"), &observation_bytes).unwrap();

    let build = build(digest(game));
    let protocol = protocol(&[
        "gameplay-trace-v5",
        "input-tape-v3",
        "milestone-program-v1.5",
        "stage-boot",
    ]);
    let scenario = artifact("inputs/scenario.json", &scenario_bytes);
    let objective = ObjectiveProgramReference {
        source: artifact("inputs/objective.milestones", objective_bytes),
        program_sha256: Digest(compiled.program_sha256),
        goal: "stage_ready".into(),
    };
    let observation_view = ObservationViewReference {
        source: artifact("inputs/observation.json", &observation_bytes),
        schema_sha256: observation.digest().unwrap(),
    };
    let action_schema = SchemaIdentity {
        id: "movement-pad-frame/v2".into(),
        sha256: Digest([4; 32]),
    };
    let identity = ArtifactIdentity {
        schema_version: ARTIFACT_SCHEMA_VERSION,
        content_digest: Digest([5; 32]),
        build: build.clone(),
        protocol_name: protocol.name.clone(),
        protocol_version: protocol.version,
        protocol_capabilities_digest: protocol.capabilities_sha256,
        scenario_id: "stage-ready-scenario".into(),
        region_digest: Digest([6; 32]),
        language_assets_digest: Digest([7; 32]),
        scenario_digest: scenario.sha256,
        predicate_program_digest: objective.program_sha256,
        action_schema_digest: action_schema.sha256,
        observation_schema_digest: observation_view.schema_sha256,
        settings_digest: Digest([8; 32]),
    };
    let mut request = HarnessRunRequest {
        schema: RUN_REQUEST_SCHEMA_V2.into(),
        content_sha256: Digest::ZERO,
        id: "stage-ready-attempt".into(),
        executable: artifact("inputs/dusklight", executable),
        game_data: artifact("inputs/game.iso", game),
        build,
        identity,
        protocol,
        boot: ObjectiveBoot::Stage {
            stage: "F_SP103".into(),
            room: 1,
            point: 1,
            layer: 3,
            save_slot: None,
        },
        scenario,
        objective,
        observation_view,
        action_schema,
        observation_requirements: ObjectiveObservationRequirements {
            schema: crate::observation_contract::OBJECTIVE_OBSERVATION_REQUIREMENTS_SCHEMA_V1
                .into(),
            families: vec![
                crate::observation_contract::ObservationFamilyRequirement {
                    id: "player_motion".into(),
                    minimum_version: 1,
                },
                crate::observation_contract::ObservationFamilyRequirement {
                    id: "stage".into(),
                    minimum_version: 1,
                },
            ],
            facts: vec!["player.exists".into(), "stage.name".into()],
        },
        input: ObjectiveSeed::Neutral,
        native_evidence: None,
        rng_seed: 42,
        logical_tick_budget: 300,
        host_timeout_seconds: 30,
        fidelity: HarnessFidelityMode::Headless,
        artifact_destination: "build/harness/stage-ready-attempt".into(),
    };
    request.refresh_content_sha256().unwrap();
    request
}

fn reached_result(request: &HarnessRunRequest, root: &Path) -> HarnessRunResult {
    fs::create_dir_all(root).unwrap();
    let tape = b"realized input";
    let trace = b"gameplay trace";
    let evidence = b"objective evidence";
    fs::write(root.join("realized.tape"), tape).unwrap();
    fs::write(root.join("gameplay.trace"), trace).unwrap();
    fs::write(root.join("objective.json"), evidence).unwrap();
    let objective = artifact("objective.json", evidence);
    let mut result = HarnessRunResult {
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
            native_phase_timing: None,
            native_evidence: None,
            complete: true,
        },
        timing: HarnessRunTiming {
            logical_ticks: 6,
            consumed_input_ticks: 6,
            host_elapsed_millis: 50,
            native_phases: None,
        },
    };
    result.refresh_content_sha256().unwrap();
    result
}

#[test]
fn request_binds_all_inputs_and_validates_their_bytes() {
    let root = root();
    let mut request = request(&root);
    let report = request.validate_files(&root).unwrap();
    assert_eq!(report.request_sha256, request.content_sha256);
    assert_eq!(report.objective_id, "stage_ready");
    let decoded: HarnessRunRequest =
        serde_json::from_slice(&request.to_pretty_json().unwrap()).unwrap();
    assert_eq!(decoded, request);

    fs::write(root.join("inputs/game.iso"), b"changed").unwrap();
    assert!(request.validate_files(&root).is_err());
    fs::write(root.join("inputs/game.iso"), b"test game data").unwrap();
    let mut detached = request.clone();
    detached.identity.action_schema_digest = Digest([99; 32]);
    detached.refresh_content_sha256().unwrap();
    assert!(detached.validate().is_err());

    request.observation_requirements.facts.reverse();
    request.refresh_content_sha256().unwrap();
    assert!(request.validate().is_err());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn request_authenticates_a_repo_relative_game_data_symlink() {
    let repository_root = root();
    let external_root = root();
    let request = request(&repository_root);
    fs::create_dir_all(&external_root).unwrap();
    let external_game = external_root.join("game.iso");
    fs::write(&external_game, b"test game data").unwrap();
    fs::remove_file(repository_root.join("inputs/game.iso")).unwrap();
    std::os::unix::fs::symlink(&external_game, repository_root.join("inputs/game.iso")).unwrap();

    request.validate_files(&repository_root).unwrap();
    fs::write(&external_game, b"changed").unwrap();
    assert!(request.validate_files(&repository_root).is_err());

    fs::remove_dir_all(repository_root).unwrap();
    fs::remove_dir_all(external_root).unwrap();
}

#[test]
fn reached_result_requires_and_authenticates_replay_proof() {
    let repository_root = root();
    let request = request(&repository_root);
    let artifact_root = repository_root.join("result");
    let mut result = reached_result(&request, &artifact_root);
    let report = result.validate_files(&request, &artifact_root).unwrap();
    assert_eq!(report.terminal, HarnessTerminalReason::Reached);
    assert!(report.artifacts_complete);

    result.artifacts.realized_input = None;
    result.refresh_content_sha256().unwrap();
    assert!(result.validate_against(&request).is_err());
    fs::remove_dir_all(repository_root).unwrap();
}

#[test]
fn mismatch_and_unsupported_terminals_are_not_ambiguous_successes() {
    let root = root();
    let request = request(&root);
    let mut result = reached_result(&request, &root.join("result"));
    result.terminal = HarnessTerminalReason::Unsupported;
    result.detail.message = "required observation is unavailable".into();
    result.detail.missing_query_facts = vec!["player.exists".into()];
    result.objective = HarnessObjectiveResult {
        reached: false,
        first_hit_tick: None,
        evidence: None,
        boundary_fingerprint: None,
    };
    result.artifacts.complete = false;
    result.refresh_content_sha256().unwrap();
    assert!(result.validate_against(&request).is_ok());

    result.terminal = HarnessTerminalReason::CapabilityMismatch;
    result.detail.missing_query_facts.clear();
    result.detail.missing_capabilities = vec!["stage-boot".into()];
    result.worker.protocol = protocol(&["gameplay-trace-v5"]);
    result.refresh_content_sha256().unwrap();
    assert!(result.validate_against(&request).is_ok());

    result.worker.protocol = request.protocol.clone();
    result.refresh_content_sha256().unwrap();
    assert!(result.validate_against(&request).is_err());

    result.terminal = HarnessTerminalReason::IdentityMismatch;
    result.detail.missing_capabilities.clear();
    result.worker.protocol.name = "different-automation-protocol".into();
    result.refresh_content_sha256().unwrap();
    assert!(result.validate_against(&request).is_ok());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unavailable_or_truncated_objective_families_admit_only_unsupported() {
    use crate::observation_contract::{
        OBSERVATION_INVENTORY_SCHEMA_V1, ObservationFamilyAvailability, ObservationFamilyStatus,
    };

    let root = root();
    let request = request(&root);
    let inventory = ObservationInventory {
        schema: OBSERVATION_INVENTORY_SCHEMA_V1.into(),
        families: vec![
            ObservationFamilyAvailability {
                id: "player_motion".into(),
                version: Some(1),
                status: ObservationFamilyStatus::Truncated,
            },
            ObservationFamilyAvailability {
                id: "stage".into(),
                version: Some(1),
                status: ObservationFamilyStatus::Present,
            },
        ],
    };
    let detail = request
        .unsupported_observation_detail(&inventory)
        .unwrap()
        .unwrap();
    assert_eq!(detail.missing_query_facts, ["player.exists"]);
    assert_eq!(detail.observation_issues.len(), 1);
    assert_eq!(detail.observation_issues[0].family, "player_motion");

    let mut result = reached_result(&request, &root.join("result"));
    result.terminal = HarnessTerminalReason::Unsupported;
    result.detail = detail;
    result.objective = HarnessObjectiveResult {
        reached: false,
        first_hit_tick: None,
        evidence: None,
        boundary_fingerprint: None,
    };
    result.artifacts.complete = false;
    result.refresh_content_sha256().unwrap();
    assert!(result.validate_against(&request).is_ok());

    result.terminal = HarnessTerminalReason::Exhausted;
    result.refresh_content_sha256().unwrap();
    assert!(result.validate_against(&request).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn crash_can_retain_authenticated_partial_artifacts_without_success() {
    let root = root();
    let request = request(&root);
    let mut result = reached_result(&request, &root.join("result"));
    result.terminal = HarnessTerminalReason::WorkerCrashed;
    result.detail.message = "worker exited after signal 11".into();
    result.objective = HarnessObjectiveResult {
        reached: false,
        first_hit_tick: None,
        evidence: None,
        boundary_fingerprint: None,
    };
    result.artifacts.objective_result = None;
    result.artifacts.complete = false;
    result.refresh_content_sha256().unwrap();
    assert!(
        result
            .validate_files(&request, &root.join("result"))
            .is_ok()
    );

    result.identity.settings_digest = Digest([98; 32]);
    result.refresh_content_sha256().unwrap();
    let error = result.validate_against(&request).unwrap_err();
    assert!(error.to_string().contains("settings_digest"));
    fs::remove_dir_all(root).unwrap();
}

fn native_phase_timing_v2() -> HarnessNativePhaseTiming {
    HarnessNativePhaseTiming {
        schema: NATIVE_LIFECYCLE_TIMING_SCHEMA_V2.into(),
        clock: "steady_clock".into(),
        process_cpu_micros: None,
        process_entry_micros: 0,
        cli_configured_micros: 1,
        aurora_initialized_micros: 2,
        engine_ready_micros: 3,
        stage_ready_micros: 4,
        first_simulation_tick_micros: 5,
        last_simulation_tick_micros: 6,
        proof_artifacts_written_micros: 7,
        engine_shutdown_micros: 8,
        exit_ready_micros: 9,
        session_reuse_audit: Some(SessionReuseAudit {
            schema: ENGINE_SESSION_REUSE_AUDIT_SCHEMA_V1.into(),
            reusable: false,
            evaluated_boundary: POST_AUTHENTICATED_RUN_BOUNDARY.into(),
            target_boundary: POST_AUTHENTICATED_RUN_BOUNDARY.into(),
            blockers: vec![SessionReuseBlocker {
                code: "game_global_reconstruction".into(),
                subsystem: "game_state".into(),
                required_guarantee: "game state reconstructs from a clean origin".into(),
            }],
        }),
    }
}

#[test]
fn native_phase_v2_authenticates_post_run_reuse_refusal() {
    native_phase_timing_v2().validate(1).unwrap();
}

#[test]
fn native_phase_v3_authenticates_process_cpu_time() {
    let mut timing = native_phase_timing_v2();
    timing.schema = NATIVE_LIFECYCLE_TIMING_SCHEMA_V3.into();
    timing.process_cpu_micros = Some(12_345);
    timing.validate(1).unwrap();

    timing.process_cpu_micros = None;
    assert!(timing.validate(1).is_err());
}

#[test]
fn native_phase_v2_rejects_a_preboot_only_audit() {
    let mut timing = native_phase_timing_v2();
    timing
        .session_reuse_audit
        .as_mut()
        .unwrap()
        .evaluated_boundary = "pre_engine_boot".into();
    let error = timing.validate(1).unwrap_err().to_string();
    assert!(error.contains("was not evaluated after the authenticated run"));
}

#[test]
fn terminal_reason_names_are_stable_and_round_trip() {
    for terminal in HarnessTerminalReason::ALL {
        let encoded = serde_json::to_string(&terminal).unwrap();
        assert_eq!(encoded, format!("\"{}\"", terminal.name()));
        assert_eq!(
            serde_json::from_str::<HarnessTerminalReason>(&encoded).unwrap(),
            terminal
        );
    }
    assert_eq!(HarnessTerminalReason::ALL.len(), 15);
}
