use super::*;
use dusklight_automation_contracts::tape::InputFrame;
use dusklight_learning::factorized_policy_suffix_batch::NativeFactorizedPolicyBatchConfig;
use dusklight_learning::native_frozen_policy_suffix_batch::{
    NativeFrozenPolicySuffixBatch, native_frozen_policy_probe_model,
};
use dusklight_search::search::MacroAction;
use dusklight_search::suffix_batch::{NativeCheckpointValidation, NativeSuffixCandidate};
use std::sync::atomic::{AtomicU64, Ordering};

#[test]
fn process_local_checkpoint_rejection_is_structured() {
    let missing = worker_client_error(ClientError::Worker {
        code: "batch_rejected".into(),
        message: "requested process-local checkpoint is absent or invalid".into(),
    });
    assert!(missing.is_missing_process_local_checkpoint());

    let unrelated = worker_client_error(ClientError::Worker {
        code: "batch_rejected".into(),
        message: "batch horizon exceeds the authenticated source tape".into(),
    });
    assert!(!unrelated.is_missing_process_local_checkpoint());
}

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

struct TestRoot(PathBuf);

impl TestRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "dusklight-native-suffix-worker-{}-{}",
            std::process::id(),
            NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
        ));
        if path.exists() {
            fs::remove_dir_all(&path).unwrap();
        }
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixture() -> (TestRoot, NativeSuffixWorkerLaunch) {
    let root = TestRoot::new();
    let executable = root.0.join("Dusklight");
    let game_data = root.0.join("game.iso");
    let input_tape = root.0.join("full.tape");
    let milestone_program = root.0.join("goal.dmsp");
    let working_directory = root.0.join("cwd");
    let card_fixture = root.0.join("card-fixture");
    let initial_batch = root.0.join("batch.json");
    fs::write(&executable, b"executable").unwrap();
    fs::write(&game_data, b"game-data").unwrap();
    fs::create_dir(&working_directory).unwrap();
    fs::create_dir(&card_fixture).unwrap();

    let tape = InputTape {
        frames: vec![InputFrame::default(); 3],
        ..InputTape::default()
    };
    fs::write(&input_tape, tape.encode().unwrap()).unwrap();

    let source = "milestones 1.7\nmilestone goal {\n  phase post_sim\n  when stage.room == 1\n}\n";
    let program = dusklight_objectives::milestone_dsl::parse(source).unwrap();
    let compiled = dusklight_objectives::milestone_dsl::compile(&program).unwrap();
    fs::write(&milestone_program, &compiled.bytes).unwrap();

    let batch = NativeSuffixBatch {
        schema: NATIVE_SUFFIX_BATCH_SCHEMA.into(),
        source_frame: 1,
        source_boundary_fingerprint: "1".repeat(32),
        checkpoint_validation: NativeCheckpointValidation {
            kind: "recorded_replay_window".into(),
            ticks: 2,
        },
        maximum_ticks: 2,
        verify_state_hashes: false,
        checkpoint_cache: None,
        candidates: vec![NativeSuffixCandidate {
            id: "candidate-0".into(),
            actions: vec![MacroAction::Neutral { frames: 2 }],
            controller_program_hex: None,
        }],
    };
    fs::write(&initial_batch, serde_json::to_vec_pretty(&batch).unwrap()).unwrap();

    let launch = NativeSuffixWorkerLaunch {
        executable,
        game_data,
        input_tape,
        milestone_program,
        card_fixture,
        card_fixture_sha256: Digest([5; 32]),
        working_directory,
        state_root: root.0.join("state"),
        world_context_sha256: Digest([4; 32]),
        terminal: NativeTerminalBinding {
            goal: "goal".into(),
            program_sha256: Digest(compiled.program_sha256),
            definition_sha256: Digest(compiled.definitions[0].sha256),
        },
        initial_batch,
        initial_result: root.0.join("result.json"),
        initial_winner_tape: Some(root.0.join("winner.tape")),
    };
    (root, launch)
}

fn frozen_fixture() -> (TestRoot, NativeFrozenPolicyWorkerLaunch) {
    let (root, launch) = fixture();
    let model_path = root.0.join("policy.dsfrozen");
    let model = native_frozen_policy_probe_model(launch.terminal.definition_sha256).unwrap();
    let model_bytes = model.to_bytes().unwrap();
    fs::write(&model_path, &model_bytes).unwrap();
    let model_path = model_path.canonicalize().unwrap();
    let batch = NativeFrozenPolicySuffixBatch::build(
        &model_bytes,
        model_path.to_string_lossy().into_owned(),
        launch.terminal.definition_sha256,
        "policy-generation-0".into(),
        NativeFactorizedPolicyBatchConfig {
            source_frame: 1,
            source_boundary_fingerprint: "1".repeat(32),
            checkpoint_validation_ticks: 2,
            maximum_ticks: 2,
            verify_state_hashes: false,
        },
    )
    .unwrap();
    fs::write(
        &launch.initial_batch,
        serde_json::to_vec_pretty(&batch).unwrap(),
    )
    .unwrap();
    let frozen = NativeFrozenPolicyWorkerLaunch {
        executable: launch.executable,
        game_data: launch.game_data,
        input_tape: launch.input_tape,
        milestone_program: launch.milestone_program,
        card_fixture: launch.card_fixture,
        card_fixture_sha256: launch.card_fixture_sha256,
        working_directory: launch.working_directory,
        state_root: launch.state_root,
        world_context_sha256: launch.world_context_sha256,
        terminal: launch.terminal,
        initial_batch: launch.initial_batch,
        initial_result: launch.initial_result,
    };
    (root, frozen)
}

#[test]
fn launch_preflight_binds_every_persistent_source_identity() {
    let (_root, launch) = fixture();
    let prepared =
        prepare_launch(&launch, None, NativeHeadlessAuditComparators::default()).unwrap();

    assert_eq!(prepared.identity.source_frame, 1);
    assert_eq!(prepared.identity.maximum_ticks, 2);
    assert_eq!(prepared.identity.terminal, launch.terminal);
    assert_ne!(prepared.identity.executable_sha256, Digest::ZERO);
    assert_ne!(prepared.identity.game_data_sha256, Digest::ZERO);
    assert_ne!(prepared.identity.input_tape_sha256, Digest::ZERO);
    assert_ne!(prepared.identity.milestone_program_sha256, Digest::ZERO);
    for required in [
        "--automation-engine-worker",
        "--headless",
        "--suffix-batch",
        "--automation-game-data-sha256",
        "--automation-card-fixture",
        "--automation-world-context-sha256",
        "--milestone-program",
        "--milestone-goal",
    ] {
        assert!(prepared.args.iter().any(|argument| argument == required));
    }
    for cvar in FIXED_AUTOMATION_CVARS {
        assert!(prepared.args.iter().any(|argument| argument == cvar));
    }
}

#[test]
fn launch_preflight_materializes_every_headless_audit_comparator() {
    let (_root, launch) = fixture();
    let prepared = prepare_launch(
        &launch,
        None,
        NativeHeadlessAuditComparators {
            gpu_frame_submission: true,
            cpu_renderer_submission: true,
            presentation_lifecycle: true,
            imgui_frame_lifecycle: true,
            host_pacing: true,
            host_audio_device: true,
            suppress_cpu_draw_traversal: true,
            suppress_deterministic_audio_emulation: true,
            suppress_game_audio_update: true,
        },
    )
    .unwrap();

    for argument in [
        "--headless-submit-gpu-frames",
        "--headless-retain-cpu-renderer-submission",
        "--headless-retain-presentation-lifecycle",
        "--headless-retain-imgui-frame-lifecycle",
        "--headless-retain-host-pacing",
        "--headless-retain-host-audio-device",
        "--headless-suppress-cpu-draw-traversal",
        "--headless-suppress-deterministic-audio-emulation",
        "--headless-suppress-game-audio-update",
    ] {
        assert!(prepared.args.iter().any(|actual| actual == argument));
    }
}

#[test]
fn production_headless_suppresses_only_parity_proven_audio_work() {
    let (_root, launch) = fixture();
    let prepared =
        prepare_launch(&launch, None, NativeHeadlessAuditComparators::production()).unwrap();

    assert!(
        prepared
            .args
            .iter()
            .any(|argument| argument == "--headless-suppress-deterministic-audio-emulation")
    );
    assert!(
        prepared
            .args
            .iter()
            .any(|argument| argument == "--headless-suppress-game-audio-update")
    );
    assert!(
        !prepared
            .args
            .iter()
            .any(|argument| argument == "--headless-suppress-cpu-draw-traversal")
    );
}

#[test]
fn launch_preflight_rejects_broken_headless_comparator_dependencies() {
    let (_root, launch) = fixture();
    for comparators in [
        NativeHeadlessAuditComparators {
            presentation_lifecycle: true,
            ..NativeHeadlessAuditComparators::default()
        },
        NativeHeadlessAuditComparators {
            imgui_frame_lifecycle: true,
            ..NativeHeadlessAuditComparators::default()
        },
        NativeHeadlessAuditComparators {
            cpu_renderer_submission: true,
            ..NativeHeadlessAuditComparators::default()
        },
    ] {
        assert!(prepare_launch(&launch, None, comparators).is_err());
    }
}

#[test]
fn launch_reuses_enclosing_execution_file_identities() {
    let (_root, launch) = fixture();
    let identities = NativeSuffixPrevalidatedFileIdentities {
        executable_sha256: Digest([0xaa; 32]),
        game_data_sha256: Digest([0xbb; 32]),
    };

    let prepared = prepare_launch(
        &launch,
        Some(identities),
        NativeHeadlessAuditComparators::default(),
    )
    .unwrap();

    assert_eq!(
        prepared.identity.executable_sha256,
        identities.executable_sha256
    );
    assert_eq!(
        prepared.identity.game_data_sha256,
        identities.game_data_sha256
    );
    assert!(
        prepare_launch(
            &launch,
            Some(NativeSuffixPrevalidatedFileIdentities {
                executable_sha256: Digest::ZERO,
                game_data_sha256: identities.game_data_sha256,
            }),
            NativeHeadlessAuditComparators::default(),
        )
        .is_err()
    );
}

#[test]
fn persistent_source_accepts_a_different_bounded_tactic_horizon() {
    let (_root, launch) = fixture();
    let prepared =
        prepare_launch(&launch, None, NativeHeadlessAuditComparators::default()).unwrap();
    let mut next = prepared.batch;
    next.maximum_ticks = 1;
    validate_batch_identity(&next, &prepared.identity).unwrap();

    next.maximum_ticks = MAXIMUM_PERSISTENT_BATCH_TICKS + 1;
    assert!(validate_batch_identity(&next, &prepared.identity).is_err());
}

#[test]
fn launch_preflight_rejects_terminal_and_horizon_drift() {
    let (_root, mut launch) = fixture();
    launch.terminal.definition_sha256 = Digest([9; 32]);
    assert!(prepare_launch(&launch, None, NativeHeadlessAuditComparators::default()).is_err());

    let (_root, mut launch) = fixture();
    let mut batch: NativeSuffixBatch =
        serde_json::from_slice(&fs::read(&launch.initial_batch).unwrap()).unwrap();
    batch.maximum_ticks = 3;
    fs::write(
        &launch.initial_batch,
        serde_json::to_vec_pretty(&batch).unwrap(),
    )
    .unwrap();
    assert!(prepare_launch(&launch, None, NativeHeadlessAuditComparators::default()).is_err());

    launch.world_context_sha256 = Digest::ZERO;
    assert!(prepare_launch(&launch, None, NativeHeadlessAuditComparators::default()).is_err());
}

#[test]
fn frozen_launch_preflight_binds_model_goal_and_persistent_source() {
    let (_root, launch) = frozen_fixture();
    let prepared = prepare_frozen_launch(&launch).unwrap();
    assert_eq!(prepared.identity.source_frame, 1);
    assert_eq!(prepared.identity.maximum_ticks, 2);
    assert_eq!(prepared.identity.terminal, launch.terminal);
    assert_eq!(
        FrozenInferenceModel::from_bytes(&prepared.model_bytes)
            .unwrap()
            .objective_sha256,
        launch.terminal.definition_sha256
    );
    assert!(
        prepared
            .args
            .iter()
            .any(|argument| argument == "--automation-engine-worker")
    );
    assert!(
        prepared
            .args
            .iter()
            .any(|argument| argument == "--suffix-batch")
    );
}

#[test]
fn frozen_launch_preflight_rejects_model_and_terminal_detachment() {
    let (_root, launch) = frozen_fixture();
    let mut batch: NativeFrozenPolicySuffixBatch =
        serde_json::from_slice(&fs::read(&launch.initial_batch).unwrap()).unwrap();
    let replacement = if batch.frozen_policy.model_xxh3_128.starts_with('0') {
        "1"
    } else {
        "0"
    };
    batch
        .frozen_policy
        .model_xxh3_128
        .replace_range(0..1, replacement);
    fs::write(
        &launch.initial_batch,
        serde_json::to_vec_pretty(&batch).unwrap(),
    )
    .unwrap();
    assert!(prepare_frozen_launch(&launch).is_err());

    let (_root, mut launch) = frozen_fixture();
    launch.terminal.definition_sha256 = Digest([9; 32]);
    assert!(prepare_frozen_launch(&launch).is_err());
}
