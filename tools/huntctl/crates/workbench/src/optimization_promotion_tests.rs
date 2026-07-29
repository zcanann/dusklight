use super::*;
use dusklight_automation_contracts::tape::InputFrame;
use dusklight_orchestration::native_residual_campaign::{
    NATIVE_RESIDUAL_EXECUTION_SCHEMA_V1, NativeResidualAttempt,
};
use dusklight_search::residual_action::{
    AnalogChannel, AnalogResidual, ResidualCandidate, TemporalBasis,
    compile_residual_candidate_to_horizon,
};
use dusklight_search::residual_optimizer::ResidualGenome;

fn test_root(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "dusklight-promotion-{name}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root.canonicalize().unwrap()
}

fn reference(path: &str, byte: u8) -> ArtifactReference {
    ArtifactReference {
        path: path.into(),
        sha256: ArtifactDigest([byte; 32]),
    }
}

fn prepared_fixture(name: &str) -> PreparedOptimizationPromotion {
    let root = test_root(name);
    let timeline_path = root.join("route.timeline");
    let parent_tape = InputTape {
        frames: vec![
            InputFrame {
                owned_ports: 1,
                ..InputFrame::default()
            };
            2
        ],
        ..InputTape::default()
    };
    let parent_bytes = parent_tape.encode().unwrap();
    fs::write(root.join("parent.tape"), &parent_bytes).unwrap();
    fs::write(root.join("incumbent.tape"), &parent_bytes).unwrap();
    fs::write(
            &timeline_path,
            format!(
                "timeline promotion\nsegment parent root profile boot_to_fsp103 uses tape parent.tape starts {} produces {}\nsegment incumbent after parent profile fsp103_to_fsp104 uses tape incumbent.tape starts {} produces {}\ncontinuation main starts root@{}\ncontinue main with parent after root@{}\ncontinue main with incumbent after parent@{}\n",
                "1".repeat(32),
                "a".repeat(32),
                "a".repeat(32),
                "c".repeat(32),
                "1".repeat(32),
                "1".repeat(32),
                "a".repeat(32),
            ),
        )
        .unwrap();
    fs::write(
        root.join("goal.milestones"),
        "milestones 1.3\nmilestone terminal { phase post_sim when player.exists }\n",
    )
    .unwrap();
    fs::create_dir(root.join("route")).unwrap();

    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../..")
        .canonicalize()
        .unwrap();
    let mut request: OptimizationRequest = serde_json::from_slice(
        &fs::read(repository.join(
            "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json",
        ))
        .unwrap(),
    )
    .unwrap();
    request.route.timeline.path = "route.timeline".into();
    request.route.timeline.sha256 =
        ArtifactDigest(Sha256::digest(fs::read(&timeline_path).unwrap()).into());
    request.route.lineage = "main".into();
    request.route.segment = "incumbent".into();
    request.route.source_boundary_index = 2;
    request.route.source_boundary_fingerprint = "a".repeat(32);
    request.route.native_source_boundary_fingerprint = "d".repeat(32);
    request.terminal_predicate.goal = "terminal".into();
    request.terminal_predicate.source = ArtifactReference {
        path: "goal.milestones".into(),
        sha256: ArtifactDigest(
            Sha256::digest(fs::read(root.join("goal.milestones")).unwrap()).into(),
        ),
    };
    let compiled_goal =
        milestone_dsl::compile_source(&fs::read_to_string(root.join("goal.milestones")).unwrap())
            .unwrap();
    request.terminal_predicate.program_sha256 = ArtifactDigest(compiled_goal.program_sha256);
    request.terminal_predicate.definition_sha256 =
        ArtifactDigest(compiled_goal.definitions[0].sha256);
    request.budgets.exploration_horizon_ticks = 10;
    request.budgets.promotion_before_tick = 5;
    request.execution.workers = 1;
    request.execution.deterministic_seeds = vec![17];
    request.execution.repetitions = 1;
    request.refresh_content_sha256().unwrap();

    let residual = ResidualCandidate::seal(
        &parent_bytes,
        vec![AnalogResidual {
            port: 0,
            channel: AnalogChannel::MainX,
            basis: TemporalBasis::ExactFrame { frame: 0, delta: 8 },
        }],
        Vec::new(),
    )
    .unwrap();
    let compiled = compile_residual_candidate_to_horizon(
        &parent_tape,
        &parent_bytes,
        &residual,
        request.budgets.exploration_horizon_ticks,
    )
    .unwrap();
    let candidate = ResidualCampaignCandidate::seal(
        "g000001-s00000-promote".into(),
        1,
        0,
        17,
        ResidualGenome { genes: Vec::new() },
        residual,
        &compiled,
    )
    .unwrap();
    let execution = NativeResidualExecutionBinding {
        schema: NATIVE_RESIDUAL_EXECUTION_SCHEMA_V1.into(),
        content_sha256: ArtifactDigest([2; 32]),
        optimization_request_sha256: request.content_sha256,
        executable: reference("build/Dusklight", 3),
        runtime_dependencies: vec![],
        game_data: reference("build/game.iso", 4),
        process_boot_tape: reference("build/process.tape", 5),
        milestone_program: reference("build/goal.dmsp", 6),
        world_context: reference("build/world.json", 7),
        card_fixture_manifest: reference("build/card.json", 8),
        checkpoint_validation_ticks: 2,
        verify_state_hashes: false,
    };
    let evaluation = NativeResidualCampaignEvaluation::seal(
        &request,
        &execution,
        &candidate,
        vec![NativeResidualAttempt {
            repetition: 1,
            worker_seed: 17,
            wire_candidate_id: "candidate-1".into(),
            batch_request: reference("build/request.json", 9),
            batch_result: reference("build/result.json", 10),
            episode_shard: reference("build/episode.bin", 11),
            restore_identity: "e".repeat(32),
            checkpoint_bytes: 64,
            simulated_ticks: 3,
            first_hit_tick: Some(2),
            terminal_boundary_fingerprint: "b".repeat(32),
            behavior_sha256: ArtifactDigest([12; 32]),
        }],
    )
    .unwrap();
    let mut local_tape = InputTape::decode(&compiled.bytes).unwrap().tape;
    local_tape.frames.truncate(2);
    let full_tape = concatenate(vec![
        ChainSegment::all(parent_tape),
        ChainSegment::all(local_tape.clone()),
    ])
    .unwrap()
    .tape;
    let promotion_root = root.join("route/segments/optimized");
    let timeline_source = fs::read(&timeline_path).unwrap();
    PreparedOptimizationPromotion {
        root: root.clone(),
        timeline_path,
        timeline_source_sha256: source_revision(&timeline_source),
        request,
        execution,
        candidate,
        candidate_artifact_sha256: ArtifactDigest([13; 32]),
        evaluation,
        evaluation_artifact_sha256: ArtifactDigest([14; 32]),
        graph_candidate_id: "optimization-candidate".into(),
        promoted_segment_id: "optimized_terminal_fast".into(),
        promoted_label: "Optimized terminal 2f".into(),
        promoted_goal_id: "optimized_terminal_fast_goal".into(),
        promoted_lineage_id: "promoted_optimized_terminal_fast".into(),
        parent_segment: "parent".into(),
        profile: "fsp103_to_fsp104".into(),
        tape_path: promotion_root.join("optimized_terminal_fast.tape"),
        tape_relative: "route/segments/optimized/optimized_terminal_fast.tape".into(),
        proof_path: promotion_root.join("optimized_terminal_fast.promotion.json"),
        proof_relative: "route/segments/optimized/optimized_terminal_fast.promotion.json".into(),
        predicate_source_relative: "goal.milestones".into(),
        lineage_dsl: format!(
            "continuation promoted_optimized_terminal_fast starts root@{}\ncontinue promoted_optimized_terminal_fast with parent after root@{}\ncontinue promoted_optimized_terminal_fast with optimized_terminal_fast after parent@{}\n",
            "1".repeat(32),
            "1".repeat(32),
            "a".repeat(32),
        ),
        local_tape,
        full_tape,
        first_hit_tick: 2,
    }
}

fn attempts(prepared: &PreparedOptimizationPromotion) -> Vec<OptimizationColdReplayAttempt> {
    (1..=OPTIMIZATION_PROMOTION_REPETITIONS)
        .map(|repetition| OptimizationColdReplayAttempt {
            repetition,
            milestone_result_sha256: ArtifactDigest([repetition as u8; 32]),
            sim_tick: 3,
            tape_frame: 3,
            boundary_index: 4,
            boundary_fingerprint: BoundaryFingerprint {
                schema: "dusklight.milestone-boundary/v6".into(),
                algorithm: "xxh3-128".into(),
                canonical_encoding: "little-endian-fixed-v6".into(),
                digest: prepared.evaluation.terminal_boundary_fingerprint.clone(),
            },
        })
        .collect()
}

fn cold_result(prepared: &PreparedOptimizationPromotion) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema": {"name": "dusklight.automation.milestones", "version": 5},
        "boot": prepared.full_tape.boot,
        "boot_origin_established": true,
        "goal": prepared.request.terminal_predicate.goal,
        "goal_reached": true,
        "program_digest": prepared.request.terminal_predicate.program_sha256,
        "milestones": [{
            "id": prepared.request.terminal_predicate.goal,
            "hit": true,
            "sim_tick": 3,
            "tape_frame": 3,
            "phase": "post_sim",
            "definition_digest": prepared.request.terminal_predicate.definition_sha256,
            "program_digest": prepared.request.terminal_predicate.program_sha256,
            "boundary_index": 4,
            "evidence": {
                "boundary_fingerprint": {
                    "schema": "dusklight.milestone-boundary/v6",
                    "algorithm": "xxh3-128",
                    "canonical_encoding": "little-endian-fixed-v6",
                    "digest": prepared.evaluation.terminal_boundary_fingerprint,
                }
            }
        }]
    }))
    .unwrap()
}

#[test]
fn cold_replay_result_requires_the_exact_trimmed_terminal_boundary() {
    let prepared = prepared_fixture("cold-result");
    let result = cold_result(&prepared);
    let attempt = validate_cold_replay_result(&prepared, 1, &result).unwrap();
    assert_eq!(attempt.tape_frame, 3);
    assert_eq!(attempt.boundary_index, 4);
    assert_eq!(
        attempt.boundary_fingerprint.digest,
        prepared.evaluation.terminal_boundary_fingerprint
    );

    let mut tampered: serde_json::Value = serde_json::from_slice(&result).unwrap();
    tampered["milestones"][0]["evidence"]["boundary_fingerprint"]["digest"] =
        serde_json::Value::String("f".repeat(32));
    assert!(
        validate_cold_replay_result(&prepared, 1, &serde_json::to_vec(&tampered).unwrap()).is_err()
    );
    fs::remove_dir_all(&prepared.root).unwrap();
}

#[test]
fn promotion_installs_compact_tape_sealed_proof_and_explicit_lineage() {
    let prepared = prepared_fixture("install");
    let proof = OptimizationPromotionProof::seal(&prepared, attempts(&prepared)).unwrap();
    install_optimization_promotion(&prepared, &proof).unwrap();

    let source = fs::read_to_string(&prepared.timeline_path).unwrap();
    let timeline = Timeline::parse(&source).unwrap();
    let promoted = &timeline.segments[&prepared.promoted_segment_id];
    assert_eq!(promoted.parent.as_deref(), Some("parent"));
    assert_eq!(promoted.start_fingerprint, "a".repeat(32));
    assert_eq!(promoted.end_fingerprint, "b".repeat(32));
    assert_eq!(
        timeline.goals[&prepared.promoted_goal_id].predicate,
        "terminal"
    );
    assert!(
        timeline
            .continuations
            .contains_key(&prepared.promoted_lineage_id)
    );
    let decoded = InputTape::decode(&fs::read(&prepared.tape_path).unwrap())
        .unwrap()
        .tape;
    assert_eq!(decoded.frames.len(), 2);
    let stored: OptimizationPromotionProof =
        serde_json::from_slice(&fs::read(&prepared.proof_path).unwrap()).unwrap();
    stored.validate().unwrap();
    assert_eq!(stored, proof);
    assert_eq!(stored.repetitions.len(), 5);
    assert!(source.contains(&format!("proof {} satisfies", prepared.promoted_segment_id)));
    assert!(source.contains(&prepared.proof_relative));
    fs::remove_dir_all(&prepared.root).unwrap();
}

#[test]
fn promotion_rejects_tampered_proof_and_stale_timeline_without_partial_files() {
    let prepared = prepared_fixture("stale");
    let proof = OptimizationPromotionProof::seal(&prepared, attempts(&prepared)).unwrap();
    let mut tampered = proof.clone();
    tampered.repetitions[4].boundary_fingerprint.digest = "f".repeat(32);
    assert!(tampered.validate().is_err());

    fs::write(&prepared.timeline_path, "timeline changed\n").unwrap();
    assert!(install_optimization_promotion(&prepared, &proof).is_err());
    assert!(!prepared.tape_path.exists());
    assert!(!prepared.proof_path.exists());
    fs::remove_dir_all(&prepared.root).unwrap();
}

#[cfg(unix)]
#[test]
fn promotion_rejects_symlinked_artifact_directories_before_writing() {
    use std::os::unix::fs::symlink;

    let prepared = prepared_fixture("symlink");
    let outside = test_root("symlink-outside");
    symlink(&outside, prepared.root.join("route/segments")).unwrap();
    let proof = OptimizationPromotionProof::seal(&prepared, attempts(&prepared)).unwrap();

    assert!(install_optimization_promotion(&prepared, &proof).is_err());
    assert!(fs::read_dir(&outside).unwrap().next().is_none());
    assert!(!prepared.tape_path.exists());
    assert!(!prepared.proof_path.exists());

    fs::remove_dir_all(&prepared.root).unwrap();
    fs::remove_dir_all(outside).unwrap();
}
