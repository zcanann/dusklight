use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

fn proven(sim_tick: u64, tape_frame: u64, digest: &str) -> ProvenBootCandidate {
    let candidate = Candidate::baseline(SegmentProfile::BootToFsp103);
    ProvenBootCandidate {
        tape: candidate.compile().unwrap(),
        candidate,
        sim_tick,
        tape_frame,
        boundary_fingerprint: BoundaryFingerprint {
            schema: "dusklight.milestone-boundary/v1".into(),
            algorithm: "xxh3-128".into(),
            canonical_encoding: "little-endian-fixed-v1".into(),
            digest: digest.into(),
        },
    }
}

fn test_run_identity() -> BootGolfRunIdentity {
    BootGolfRunIdentity {
        schema: "dusklight-boot-timing-golf-run/v1".into(),
        strategy: "a-start-coordinate-descent/v3".into(),
        source_candidate_id: "source".into(),
        source_goal_sim_tick: 439,
        source_goal_tape_frame: 439,
        source_boundary_fingerprint: BoundaryFingerprint {
            schema: "dusklight.milestone-boundary/v1".into(),
            algorithm: "xxh3-128".into(),
            canonical_encoding: "little-endian-fixed-v1".into(),
            digest: "a".repeat(32),
        },
        game_sha256: ArtifactDigest([1; 32]),
        dvd_sha256: ArtifactDigest([2; 32]),
        working_directory: PathBuf::from("C:/repo"),
        game_args_prefix: vec!["--automation-headless".into()],
        repetitions: 2,
        timeout_millis: 120_000,
        harness_request_sha256: Some(ArtifactDigest([3; 32])),
    }
}

fn unique_test_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "dusklight-finalist-reduction-{label}-{}-{nonce}",
        std::process::id()
    ))
}

#[test]
fn target_rejects_later_or_different_proof() {
    let source = proven(439, 439, &"a".repeat(32));
    let target = BootReductionTarget {
        sim_tick: source.sim_tick,
        tape_frame: source.tape_frame,
        boundary_fingerprint: source.boundary_fingerprint.clone(),
    };
    assert!(target.accepts(&source));
    assert!(!target.accepts(&proven(440, 439, &"a".repeat(32))));
    assert!(!target.accepts(&proven(439, 440, &"a".repeat(32))));
    assert!(!target.accepts(&proven(439, 439, &"b".repeat(32))));
}

#[test]
fn candidate_hash_is_only_a_tie_breaker_not_progress() {
    let with_button = |button| {
        let mut candidate = Candidate::baseline(SegmentProfile::BootToFsp103);
        candidate.actions = vec![
            MacroAction::Neutral { frames: 3 },
            MacroAction::Press {
                buttons: vec![button],
                hold_frames: 1,
                neutral_frames: 1,
            },
        ];
        ProvenBootCandidate {
            tape: candidate.compile().unwrap(),
            candidate,
            sim_tick: 439,
            tape_frame: 439,
            boundary_fingerprint: BoundaryFingerprint {
                schema: "dusklight.milestone-boundary/v1".into(),
                algorithm: "xxh3-128".into(),
                canonical_encoding: "little-endian-fixed-v1".into(),
                digest: "a".repeat(32),
            },
        }
    };
    let left = with_button(dusklight_search::search::ControllerButton::A);
    let right = with_button(dusklight_search::search::ControllerButton::Start);
    assert_ne!(left.candidate.id().unwrap(), right.candidate.id().unwrap());
    assert_eq!(
        boot_golf_quality_cmp(&left, &right),
        std::cmp::Ordering::Equal
    );
}

#[test]
fn shifted_boot_pulse_can_swap_between_a_and_start() {
    let mut candidate = Candidate::baseline(SegmentProfile::BootToFsp103);
    candidate.actions = vec![
        MacroAction::Neutral { frames: 3 },
        MacroAction::Press {
            buttons: vec![dusklight_search::search::ControllerButton::Start],
            hold_frames: 1,
            neutral_frames: 1,
        },
    ];
    let tape = candidate.compile().unwrap();
    let parent = ProvenBootCandidate {
        candidate,
        tape,
        sim_tick: 4,
        tape_frame: 4,
        boundary_fingerprint: BoundaryFingerprint {
            schema: "dusklight.milestone-boundary/v1".into(),
            algorithm: "xxh3-128".into(),
            canonical_encoding: "little-endian-fixed-v1".into(),
            digest: "a".repeat(32),
        },
    };

    let swapped = candidate_with_shifted_pulse(&parent, 0, 1, BUTTON_A, 1)
        .unwrap()
        .compile()
        .unwrap();
    assert_eq!(swapped.frames[1].pads[0].buttons, BUTTON_A);
    assert_eq!(swapped.frames[3].pads[0].buttons, 0);

    let in_place = candidate_with_shifted_pulse(&parent, 0, 3, BUTTON_A, 1)
        .unwrap()
        .compile()
        .unwrap();
    assert_eq!(in_place.frames[3].pads[0].buttons, BUTTON_A);
}

#[test]
fn batch_cache_is_bound_to_run_candidates_and_native_results() {
    let root = unique_test_root("batch-cache");
    fs::create_dir_all(root.join("native")).unwrap();
    fs::write(root.join("native/results.json"), b"sealed native results").unwrap();
    fs::write(root.join("native/evaluation.json"), b"sealed evaluation").unwrap();
    let run = test_run_identity();
    let candidate_ids = vec!["candidate-a".into(), "candidate-b".into()];
    let mut cache = BootGolfBatchCache {
        schema: "dusklight-boot-timing-golf-batch/v1".into(),
        content_sha256: ArtifactDigest::ZERO,
        run: run.clone(),
        round: 1,
        batch_index: 2,
        candidate_ids: candidate_ids.clone(),
        proven: vec![BootGolfCachedProof {
            candidate_id: "candidate-b".into(),
            sim_tick: 439,
            tape_frame: 439,
            boundary_fingerprint: run.source_boundary_fingerprint.clone(),
        }],
        evaluation: PathBuf::from("native/evaluation.json"),
        evaluation_sha256: sha256_file(&root.join("native/evaluation.json")).unwrap(),
        results: PathBuf::from("native/results.json"),
        results_sha256: sha256_file(&root.join("native/results.json")).unwrap(),
    };
    cache.content_sha256 = boot_golf_batch_cache_digest(&cache).unwrap();
    validate_boot_golf_batch_cache(&cache, &run, &root, 1, 2, &candidate_ids).unwrap();

    let mut changed_run = run.clone();
    changed_run.source_goal_sim_tick += 1;
    assert!(
        validate_boot_golf_batch_cache(&cache, &changed_run, &root, 1, 2, &candidate_ids).is_err()
    );
    let mut changed_candidates = candidate_ids.clone();
    changed_candidates.reverse();
    assert!(
        validate_boot_golf_batch_cache(&cache, &run, &root, 1, 2, &changed_candidates).is_err()
    );

    fs::write(root.join("native/results.json"), b"changed").unwrap();
    assert!(validate_boot_golf_batch_cache(&cache, &run, &root, 1, 2, &candidate_ids).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn batch_cache_rejects_foreign_duplicate_and_tampered_proofs() {
    let root = unique_test_root("batch-cache-tamper");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("results.json"), b"sealed").unwrap();
    fs::write(root.join("evaluation.json"), b"sealed evaluation").unwrap();
    let run = test_run_identity();
    let candidate_ids = vec!["candidate-a".into()];
    let proof = BootGolfCachedProof {
        candidate_id: "candidate-a".into(),
        sim_tick: 439,
        tape_frame: 439,
        boundary_fingerprint: run.source_boundary_fingerprint.clone(),
    };
    let mut cache = BootGolfBatchCache {
        schema: "dusklight-boot-timing-golf-batch/v1".into(),
        content_sha256: ArtifactDigest::ZERO,
        run: run.clone(),
        round: 1,
        batch_index: 0,
        candidate_ids: candidate_ids.clone(),
        proven: vec![proof.clone(), proof],
        evaluation: PathBuf::from("evaluation.json"),
        evaluation_sha256: sha256_file(&root.join("evaluation.json")).unwrap(),
        results: PathBuf::from("results.json"),
        results_sha256: sha256_file(&root.join("results.json")).unwrap(),
    };
    cache.content_sha256 = boot_golf_batch_cache_digest(&cache).unwrap();
    assert!(validate_boot_golf_batch_cache(&cache, &run, &root, 1, 0, &candidate_ids).is_err());

    cache.proven = vec![BootGolfCachedProof {
        candidate_id: "foreign".into(),
        sim_tick: 439,
        tape_frame: 439,
        boundary_fingerprint: run.source_boundary_fingerprint.clone(),
    }];
    cache.content_sha256 = boot_golf_batch_cache_digest(&cache).unwrap();
    assert!(validate_boot_golf_batch_cache(&cache, &run, &root, 1, 0, &candidate_ids).is_err());

    cache.proven.clear();
    cache.content_sha256 = boot_golf_batch_cache_digest(&cache).unwrap();
    cache.round = 2;
    assert!(validate_boot_golf_batch_cache(&cache, &run, &root, 1, 0, &candidate_ids).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn partial_evidence_gets_a_fresh_resume_root() {
    let root = unique_test_root("partial-evidence");
    let base = root.join("rounds/0001/batch-0000");
    fs::create_dir_all(&base).unwrap();
    assert!(fresh_boot_evidence_root(&base, false).is_err());
    assert_eq!(
        fresh_boot_evidence_root(&base, true).unwrap(),
        root.join("rounds/0001/batch-0000-resume-0001")
    );
    fs::create_dir_all(root.join("rounds/0001/batch-0000-resume-0001")).unwrap();
    assert_eq!(
        fresh_boot_evidence_root(&base, true).unwrap(),
        root.join("rounds/0001/batch-0000-resume-0002")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sealed_batch_cache_skips_native_evaluation() {
    let root = unique_test_root("cached-evaluation");
    fs::create_dir_all(root.join("sealed/evidence")).unwrap();
    fs::write(root.join("sealed/evidence/evaluation.json"), b"evaluation").unwrap();
    fs::write(root.join("sealed/results.json"), b"results").unwrap();
    let candidate = Candidate::baseline(SegmentProfile::BootToFsp103);
    let candidate_id = candidate.id().unwrap();
    let run = test_run_identity();
    let cache_path = boot_golf_batch_cache_path(&root, 1, 0);
    write_boot_golf_batch_cache(
        &cache_path,
        BootGolfBatchCache {
            schema: "dusklight-boot-timing-golf-batch/v1".into(),
            content_sha256: ArtifactDigest::ZERO,
            run: run.clone(),
            round: 1,
            batch_index: 0,
            candidate_ids: vec![candidate_id.clone()],
            proven: vec![BootGolfCachedProof {
                candidate_id,
                sim_tick: 439,
                tape_frame: 439,
                boundary_fingerprint: run.source_boundary_fingerprint.clone(),
            }],
            evaluation: PathBuf::from("sealed/evidence/evaluation.json"),
            evaluation_sha256: sha256_file(&root.join("sealed/evidence/evaluation.json")).unwrap(),
            results: PathBuf::from("sealed/results.json"),
            results_sha256: sha256_file(&root.join("sealed/results.json")).unwrap(),
        },
    )
    .unwrap();
    let config = BootMinimizeConfig {
        candidate: candidate.clone(),
        game: root.join("intentionally-absent-game.exe"),
        dvd: root.join("intentionally-absent.iso"),
        output_root: root.clone(),
        working_directory: root.clone(),
        game_args_prefix: Vec::new(),
        workers: 1,
        repetitions: 2,
        timeout: Duration::from_secs(1),
        harness: None,
    };
    let loaded =
        evaluate_or_load_boot_golf_batch(&config, &run, vec![candidate], 1, 0, true).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].sim_tick, 439);
    fs::remove_dir_all(root).unwrap();
}
