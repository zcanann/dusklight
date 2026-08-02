use super::*;
use crate::native_residual_campaign::NATIVE_RESIDUAL_EXECUTION_SCHEMA_V1;
use crate::residual_campaign_runner::prepare_batch;
use dusklight_harness_contracts::objective_suite::ArtifactReference;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEST_ROOT: AtomicU64 = AtomicU64::new(0);

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../..")
        .canonicalize()
        .unwrap()
}

fn optimization(root: &Path) -> OptimizationRequest {
    serde_json::from_slice(
        &fs::read(root.join(
            "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json",
        ))
        .unwrap(),
    )
    .unwrap()
}

fn placeholder(path: &str, byte: u8) -> ArtifactReference {
    ArtifactReference {
        path: path.into(),
        sha256: Digest([byte; 32]),
    }
}

fn execution(optimization: &OptimizationRequest) -> NativeResidualExecutionBinding {
    NativeResidualExecutionBinding {
        schema: NATIVE_RESIDUAL_EXECUTION_SCHEMA_V1.into(),
        content_sha256: Digest([9; 32]),
        optimization_request_sha256: optimization.content_sha256,
        executable: placeholder("build/test/Dusklight", 1),
        runtime_dependencies: vec![],
        game_data: placeholder("build/test/game.iso", 2),
        process_boot_tape: placeholder("build/test/process.tape", 3),
        milestone_program: placeholder("build/test/terminal.dmsp", 4),
        world_context: placeholder("build/test/world.context.json", 5),
        card_fixture_manifest: placeholder("build/test/card-fixture.json", 6),
        checkpoint_validation_ticks: 8,
        verify_state_hashes: false,
    }
}

fn prepared_generation(
    root: &Path,
    optimization: &OptimizationRequest,
) -> (InputTape, Vec<u8>, Vec<PreparedCandidate>) {
    let incumbent = optimization.incumbent.as_ref().unwrap();
    let parent_bytes = fs::read(root.join(&incumbent.tape.path)).unwrap();
    let parent = InputTape::decode(&parent_bytes).unwrap().tape;
    let mut optimizer = new_optimizer(optimization, &parent_bytes).unwrap();
    let ResidualCampaignOptimizer::Cem(cem) = &mut optimizer else {
        panic!("checked Ordon request must use CEM")
    };
    let proposal = cem.ask(&parent, &parent_bytes).unwrap();
    let prepared = prepare_batch(optimization, &parent, &parent_bytes, 0, proposal).unwrap();
    (parent, parent_bytes, prepared)
}

#[test]
fn incumbent_demonstration_uses_the_full_exploration_horizon() {
    let root = repository();
    let optimization = optimization(&root);
    let execution = execution(&optimization);
    let incumbent = optimization.incumbent.as_ref().unwrap();
    let parent = InputTape::decode(&fs::read(root.join(&incumbent.tape.path)).unwrap())
        .unwrap()
        .tape;
    assert_eq!(parent.frames.len(), 126);

    let profile = segment_profile(&root, &optimization).unwrap();
    let batch = incumbent_demonstration_batch(&optimization, &execution, profile, &parent).unwrap();
    assert_eq!(batch.maximum_ticks, 160);
    assert_eq!(batch.candidates.len(), 1);

    let mut expected = parent.clone();
    extend_tape_with_released_input(
        &mut expected,
        optimization.budgets.exploration_horizon_ticks,
    )
    .unwrap();
    assert_eq!(expected.frames.len(), 160);
    assert_eq!(expected.frames[..parent.frames.len()], parent.frames);
    let imported = Candidate::from_absolute_tape(profile, &expected).unwrap();
    assert_eq!(batch.candidates[0].actions, imported.actions);
    assert_eq!(imported.frame_count(), batch.maximum_ticks as u64);
}

#[test]
fn native_batch_losslessly_bridges_residual_tapes_at_the_route_checkpoint() {
    let root = repository();
    let optimization = optimization(&root);
    let execution = execution(&optimization);
    let (_parent, _parent_bytes, prepared) = prepared_generation(&root, &optimization);
    let selected = prepared.iter().take(4).collect::<Vec<_>>();
    let batch = native_batch(
        &optimization,
        &execution,
        segment_profile(&root, &optimization).unwrap(),
        &selected,
        1,
    )
    .unwrap();

    assert_eq!(batch.source_frame, 506);
    assert_eq!(
        batch.source_boundary_fingerprint,
        optimization.route.native_source_boundary_fingerprint
    );
    assert_eq!(batch.maximum_ticks, 160);
    assert_eq!(batch.checkpoint_validation.ticks, 8);
    assert_eq!(batch.candidates.len(), selected.len());
    for (actual, expected) in batch.candidates.iter().zip(selected.iter().copied()) {
        let imported = Candidate::from_absolute_tape(
            segment_profile(&root, &optimization).unwrap(),
            &expected.compiled.tape,
        )
        .unwrap();
        assert_eq!(actual.actions, imported.actions);
        assert_eq!(actual.id, wire_candidate_id(&expected.envelope.id, 1));
    }

    let exact = selected
        .iter()
        .map(|candidate| NativeResidualExactReplayCandidate {
            id: candidate.envelope.id.clone(),
            tape: candidate.compiled.tape.clone(),
        })
        .collect::<Vec<_>>();
    let exact_refs = exact.iter().collect::<Vec<_>>();
    let exact_batch = exact_replay_batch(
        &optimization,
        &execution,
        segment_profile(&root, &optimization).unwrap(),
        &exact_refs,
        1,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(exact_batch).unwrap(),
        serde_json::to_value(batch).unwrap()
    );
}

#[test]
fn native_wire_projection_discards_only_neutral_secondary_port_envelopes() {
    let pad = dusklight_search::search::SearchPadState::from(RawPadState {
        stick_x: -126,
        stick_y: -14,
        ..RawPadState::default()
    });
    let projected = project_native_port_one_actions(vec![MacroAction::PadRun {
        pad,
        frames: 4,
        imported_owned_ports: Some(15),
        port_one_secondary_pads: Some([RawPadState::default(); 3]),
    }])
    .unwrap();

    assert_eq!(
        projected,
        vec![MacroAction::PadRun {
            pad,
            frames: 4,
            imported_owned_ports: None,
            port_one_secondary_pads: None,
        }]
    );

    let mut active_secondary = RawPadState::default();
    active_secondary.buttons = 1;
    assert!(
        project_native_port_one_actions(vec![MacroAction::PadRun {
            pad,
            frames: 4,
            imported_owned_ports: Some(15),
            port_one_secondary_pads: Some([
                active_secondary,
                RawPadState::default(),
                RawPadState::default(),
            ]),
        }])
        .is_err()
    );
}

#[test]
fn exact_replay_batch_and_validator_share_projected_native_actions() {
    let root = repository();
    let optimization = optimization(&root);
    let (_parent, _parent_bytes, prepared) = prepared_generation(&root, &optimization);
    let mut tape = prepared[0].compiled.tape.clone();
    for frame in &mut tape.frames {
        frame.owned_ports = 15;
        frame.pads[1..].fill(RawPadState::default());
    }
    let segment = segment_profile(&root, &optimization).unwrap();
    let imported = Candidate::from_absolute_tape(segment, &tape).unwrap();
    let expected = project_native_port_one_actions(imported.actions).unwrap();
    assert_eq!(
        exact_replay_native_actions(segment, &tape).unwrap(),
        expected
    );
}

#[test]
fn residual_evaluation_charges_and_binds_alternate_terminal_attempts() {
    let root = repository();
    let optimization = optimization(&root);
    let execution = execution(&optimization);
    let (_parent, _parent_bytes, prepared) = prepared_generation(&root, &optimization);
    let alternates = optimization.alternate_terminal_predicates(&root).unwrap();
    assert_eq!(
        optimization
            .alternate_terminal_predicates_after_request_validation(&root)
            .unwrap(),
        alternates
    );
    let alternate = alternates.into_iter().next().unwrap();
    let attempt = |first_hit_tick: Option<u64>, byte: u8| NativeResidualAttempt {
        repetition: 1,
        worker_seed: optimization.execution.deterministic_seeds[0],
        wire_candidate_id: wire_candidate_id(&prepared[0].envelope.id, 1),
        batch_request: placeholder("build/test/request.json", byte),
        batch_result: placeholder("build/test/result.json", byte.saturating_add(1)),
        episode_shard: placeholder("build/test/episodes.dseps", byte.saturating_add(2)),
        restore_identity: "7".repeat(32),
        checkpoint_bytes: 1,
        simulated_ticks: first_hit_tick.map_or(160, |tick| tick + 1),
        first_hit_tick,
        terminal_boundary_fingerprint: "8".repeat(32),
        behavior_sha256: Digest([byte.saturating_add(3); 32]),
    };
    let evaluation = NativeResidualCampaignEvaluation::seal_with_alternate_terminals(
        &optimization,
        &execution,
        &prepared[0].envelope,
        vec![attempt(None, 10)],
        vec![NativeAlternateTerminalEvaluation {
            terminal: NativeTerminalBinding {
                goal: alternate.goal,
                program_sha256: alternate.program_sha256,
                definition_sha256: alternate.definition_sha256,
            },
            attempts: vec![attempt(Some(113), 20)],
        }],
    )
    .unwrap();

    assert_eq!(evaluation.simulated_ticks, 274);
    assert_eq!(evaluation.alternate_terminals.len(), 1);
    assert_eq!(
        evaluation.alternate_terminals[0].attempts[0].first_hit_tick,
        Some(113)
    );
    evaluation
        .validate(&optimization, &execution, &prepared[0].envelope)
        .unwrap();
}

#[test]
fn residual_evaluations_distinguish_candidates_in_a_shared_episode_shard() {
    let root = repository();
    let optimization = optimization(&root);
    let execution = execution(&optimization);
    let (_parent, _parent_bytes, prepared) = prepared_generation(&root, &optimization);
    let attempt = |candidate: &PreparedCandidate| NativeResidualAttempt {
        repetition: 1,
        worker_seed: optimization.execution.deterministic_seeds[0],
        wire_candidate_id: wire_candidate_id(&candidate.envelope.id, 1),
        batch_request: placeholder("build/test/shared-request.json", 10),
        batch_result: placeholder("build/test/shared-result.json", 11),
        episode_shard: placeholder("build/test/shared-episodes.dseps", 12),
        restore_identity: "7".repeat(32),
        checkpoint_bytes: 1,
        simulated_ticks: 160,
        first_hit_tick: None,
        terminal_boundary_fingerprint: "8".repeat(32),
        behavior_sha256: Digest([13; 32]),
    };
    let first = NativeResidualCampaignEvaluation::seal(
        &optimization,
        &execution,
        &prepared[0].envelope,
        vec![attempt(&prepared[0])],
    )
    .unwrap();
    let second = NativeResidualCampaignEvaluation::seal(
        &optimization,
        &execution,
        &prepared[1].envelope,
        vec![attempt(&prepared[1])],
    )
    .unwrap();

    assert_eq!(
        first.attempts[0].episode_shard,
        second.attempts[0].episode_shard
    );
    assert_ne!(
        first.evidence.episode_sha256,
        second.evidence.episode_sha256
    );
}

#[test]
fn crash_recovery_never_reuses_a_partial_native_result_path() {
    let root = repository();
    let optimization = optimization(&root);
    let execution = execution(&optimization);
    let (_parent, _parent_bytes, prepared) = prepared_generation(&root, &optimization);
    let batch = native_batch(
        &optimization,
        &execution,
        segment_profile(&root, &optimization).unwrap(),
        &[&prepared[0]],
        1,
    )
    .unwrap();
    let nonce = NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed);
    let batch_root = root.join("build").join(format!(
        "native-residual-crash-path-test-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&batch_root).unwrap();
    fs::write(batch_root.join("result-try001.json"), b"{\"partial\":").unwrap();
    fs::write(
        batch_root.join("result-try001.json.episodes.dseps"),
        b"partial episode",
    )
    .unwrap();
    let terminal = NativeTerminalBinding {
        goal: optimization.terminal_predicate.goal.clone(),
        program_sha256: optimization.terminal_predicate.program_sha256,
        definition_sha256: optimization.terminal_predicate.definition_sha256,
    };

    let (path, adopted) = select_result_path(&batch_root, &batch, &terminal).unwrap();
    assert_eq!(path, batch_root.join("result-try002.json"));
    assert!(adopted.is_none());
    assert_eq!(
        fs::read(batch_root.join("result-try001.json")).unwrap(),
        b"{\"partial\":"
    );
    fs::remove_dir_all(batch_root).unwrap();
}

#[test]
fn crash_recovery_replaces_only_an_uncommitted_native_request() {
    let root = repository();
    let nonce = NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed);
    let batch_root = root.join("build").join(format!(
        "native-residual-uncommitted-request-test-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&batch_root).unwrap();
    let request_path = batch_root.join("request.json");
    fs::write(&request_path, b"old request").unwrap();

    write_uncommitted_native_request(&batch_root, &request_path, b"new request").unwrap();
    assert_eq!(fs::read(&request_path).unwrap(), b"new request");

    fs::write(batch_root.join("result-try001.json"), b"attached result").unwrap();
    let error =
        write_uncommitted_native_request(&batch_root, &request_path, b"third request").unwrap_err();
    assert!(error.to_string().contains("acquired artifacts"));
    assert_eq!(fs::read(&request_path).unwrap(), b"new request");
    fs::remove_dir_all(batch_root).unwrap();
}

#[test]
fn pre_cancelled_campaign_returns_a_typed_outcome_without_launching_workers() {
    let root = repository();
    let optimization = optimization(&root);
    let execution = execution(&optimization);
    let cancellation = AtomicBool::new(true);

    let error = run_native_residual_campaign(&NativeResidualCampaignRunConfig {
        repository_root: &root,
        optimization: &optimization,
        execution: &execution,
        cancellation: Some(&cancellation),
    })
    .unwrap_err();

    assert!(error.is_cancelled());
    assert!(error.to_string().contains("durable boundary"));
}

#[test]
fn worker_pool_drop_shuts_down_and_removes_its_ephemeral_session_tree() {
    let root = repository();
    let optimization = optimization(&root);
    let execution = execution(&optimization);
    let nonce = NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed);
    let session_root = root.join("build").join(format!(
        "native-residual-session-cleanup-test-{}-{nonce}/native-sessions/run-test",
        std::process::id()
    ));
    fs::create_dir_all(session_root.join("worker-000/renderer-cache")).unwrap();
    fs::write(session_root.join("worker-000/transient.bin"), b"transient").unwrap();
    {
        let _pool = WorkerPool {
            root: &root,
            optimization: &optimization,
            execution: &execution,
            terminal: NativeTerminalBinding {
                goal: optimization.terminal_predicate.goal.clone(),
                program_sha256: optimization.terminal_predicate.program_sha256,
                definition_sha256: optimization.terminal_predicate.definition_sha256,
            },
            milestone_program: root.join(&execution.milestone_program.path),
            card_fixture_root: root.clone(),
            session_root: session_root.clone(),
            lanes: Vec::new(),
        };
    }
    assert!(!session_root.exists());
    fs::remove_dir_all(session_root.ancestors().nth(2).expect("test campaign root")).unwrap();
}

#[test]
fn generation_budget_uses_exact_retained_ticks_but_never_speculates_on_missing_work() {
    assert!(generation_exceeds_remaining_tick_budget(
        9_570, 131_072, None
    ));
    assert!(!generation_exceeds_remaining_tick_budget(
        9_570,
        131_072,
        Some(9_000)
    ));
    assert!(generation_exceeds_remaining_tick_budget(
        9_570,
        131_072,
        Some(9_571)
    ));
}
