use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEST_ROOT: AtomicU64 = AtomicU64::new(0);

struct TestRoot(PathBuf);

impl TestRoot {
    fn new() -> Self {
        let serial = NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "dusklight-optimization-resume-{}-{serial}",
            std::process::id()
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

fn source_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(4)
        .unwrap()
        .to_path_buf()
}

fn copy_repository_file(source: &Path, destination_root: &Path, relative: &str) {
    let destination = destination_root.join(relative);
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    fs::copy(source.join(relative), destination).unwrap();
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn fixture(checkpoint_every_candidates: u64) -> (TestRoot, OptimizationRequest) {
    let root = TestRoot::new();
    let source = source_root();
    let request_path =
        "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json";
    let mut request: OptimizationRequest =
        serde_json::from_slice(&fs::read(source.join(request_path)).unwrap()).unwrap();
    copy_repository_file(&source, &root.0, "routes/Glitch Exhibition/intro.timeline");
    copy_tree(
        &source.join("routes/Glitch Exhibition/intro"),
        &root.0.join("routes/Glitch Exhibition/intro"),
    );
    let suffix = NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed);
    request.resume.state_path = format!("build/test-{suffix}/state.json");
    request.resume.journal_path = format!("build/test-{suffix}/journal.jsonl");
    request.resume.checkpoint_every_candidates = checkpoint_every_candidates;
    request.refresh_content_sha256().unwrap();
    request.validate_files(&root.0).unwrap();
    (root, request)
}

fn artifact(root: &Path, relative: &str, bytes: &[u8]) -> ArtifactReference {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
    ArtifactReference {
        path: relative.into(),
        sha256: sha256(bytes),
    }
}

fn candidate_event(
    request: &OptimizationRequest,
    id: &str,
    candidate: ArtifactReference,
    compiled_tape: ArtifactReference,
) -> OptimizationResumeEvent {
    OptimizationResumeEvent::CandidateSealed {
        candidate_id: id.into(),
        candidate,
        compiled_tape,
        parent_tape_sha256: Some(request.incumbent.as_ref().unwrap().tape.sha256),
        generation: 0,
        proposer_seed: 7,
    }
}

#[test]
fn incumbent_demonstration_is_first_unique_and_charged() {
    let (root, request) = fixture(1);
    initialize_optimization_resume(&request, &root.0).unwrap();
    let demonstration = artifact(
        &root.0,
        "build/artifacts/incumbent-demonstration.json",
        b"demonstration",
    );
    let state = append_optimization_resume_event(
        &request,
        &root.0,
        OptimizationResumeEvent::DemonstrationSeeded {
            demonstration: demonstration.clone(),
            simulated_ticks: request.incumbent.as_ref().unwrap().first_hit_tick,
        },
    )
    .unwrap();

    assert_eq!(state.demonstration, Some(demonstration.clone()));
    assert_eq!(state.demonstration_simulated_ticks, 125);
    assert_eq!(state.charged_simulated_ticks, 125);
    assert_eq!(state.record_count, 1);

    let error = append_optimization_resume_event(
        &request,
        &root.0,
        OptimizationResumeEvent::DemonstrationSeeded {
            demonstration,
            simulated_ticks: 125,
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("duplicate"));
}

#[test]
fn alternate_terminal_run_expands_only_the_sealed_candidate_tick_bound() {
    let (root, request) = fixture(2);
    initialize_optimization_resume(&request, &root.0).unwrap();
    let candidate = artifact(&root.0, "build/artifacts/alternate.json", b"candidate");
    let candidate_sha256 = candidate.sha256;
    let tape = artifact(&root.0, "build/artifacts/alternate.tape", b"tape");
    let result = artifact(&root.0, "build/artifacts/alternate-result.json", b"result");
    let state = append_optimization_resume_events(
        &request,
        &root.0,
        vec![
            candidate_event(&request, "g0-alternate", candidate, tape),
            OptimizationResumeEvent::EvaluationCompleted {
                candidate_id: "g0-alternate".into(),
                candidate_sha256,
                result,
                simulated_ticks: request.budgets.exploration_horizon_ticks * 2,
            },
        ],
    )
    .unwrap();
    assert_eq!(state.charged_simulated_ticks, 320);

    let (root, mut request) = fixture(2);
    request.execution.alternate_terminal_goals.clear();
    request.refresh_content_sha256().unwrap();
    initialize_optimization_resume(&request, &root.0).unwrap();
    let candidate = artifact(&root.0, "build/artifacts/main-only.json", b"candidate");
    let candidate_sha256 = candidate.sha256;
    let tape = artifact(&root.0, "build/artifacts/main-only.tape", b"tape");
    let result = artifact(&root.0, "build/artifacts/main-only-result.json", b"result");
    let error = append_optimization_resume_events(
        &request,
        &root.0,
        vec![
            candidate_event(&request, "g0-main-only", candidate, tape),
            OptimizationResumeEvent::EvaluationCompleted {
                candidate_id: "g0-main-only".into(),
                candidate_sha256,
                result,
                simulated_ticks: request.budgets.exploration_horizon_ticks * 2,
            },
        ],
    )
    .unwrap_err();
    assert!(error.to_string().contains("per-candidate"));
}

#[test]
fn v1_candidate_journal_resumes_without_a_late_demonstration() {
    let (root, request) = fixture(1);
    initialize_optimization_resume(&request, &root.0).unwrap();
    let candidate = artifact(&root.0, "build/artifacts/legacy.json", b"legacy");
    let tape = artifact(&root.0, "build/artifacts/legacy.tape", b"legacy-tape");
    let mut record = OptimizationResumeRecord {
        schema: OPTIMIZATION_RESUME_RECORD_SCHEMA_V1.into(),
        request_sha256: request.content_sha256,
        sequence: 1,
        previous_record_sha256: Digest::ZERO,
        event: candidate_event(&request, "legacy-g0-c0", candidate, tape),
        record_sha256: Digest::ZERO,
    };
    record.record_sha256 = record_identity(&record).unwrap();
    fs::write(
        root.0.join(&request.resume.journal_path),
        record_bytes(&record).unwrap(),
    )
    .unwrap();

    let state = load_optimization_resume(&request, &root.0).unwrap();
    assert_eq!(state.schema, OPTIMIZATION_RESUME_STATE_SCHEMA_V2);
    assert!(state.demonstration.is_none());
    assert_eq!(state.candidates.len(), 1);

    let demonstration = artifact(&root.0, "build/artifacts/late-demonstration.json", b"late");
    let error = append_optimization_resume_event(
        &request,
        &root.0,
        OptimizationResumeEvent::DemonstrationSeeded {
            demonstration,
            simulated_ticks: 125,
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("late"));
}

#[test]
fn batch_previews_candidate_evaluation_and_checkpoint_in_order() {
    let (root, request) = fixture(1);
    let initial = initialize_optimization_resume(&request, &root.0).unwrap();
    let candidate = artifact(&root.0, "build/artifacts/candidate.json", b"candidate");
    let candidate_sha256 = candidate.sha256;
    let tape = artifact(&root.0, "build/artifacts/candidate.tape", b"tape");
    let result = artifact(&root.0, "build/artifacts/result.json", b"result");
    let checkpoint = artifact(&root.0, "build/artifacts/checkpoint.json", b"checkpoint");

    let state = append_optimization_resume_events(
        &request,
        &root.0,
        vec![
            candidate_event(&request, "g0-c0", candidate, tape),
            OptimizationResumeEvent::EvaluationCompleted {
                candidate_id: "g0-c0".into(),
                candidate_sha256,
                result,
                simulated_ticks: request.budgets.exploration_horizon_ticks,
            },
            OptimizationResumeEvent::OptimizerCheckpoint {
                generation: 1,
                completed_candidates: 1,
                state: checkpoint,
            },
        ],
    )
    .unwrap();

    assert_eq!(state.record_count, 3);
    assert_eq!(state.next_sequence, 4);
    assert_eq!(state.completed_candidates, 1);
    assert_eq!(state.charged_simulated_ticks, 160);
    assert!(state.pending_candidate_ids.is_empty());
    assert_eq!(state.uncheckpointed_completions, 0);
    assert_eq!(
        state
            .latest_optimizer_checkpoint
            .as_ref()
            .unwrap()
            .generation,
        1
    );
    assert_ne!(state.last_record_sha256, initial.last_record_sha256);
    assert_eq!(load_optimization_resume(&request, &root.0).unwrap(), state);
}

#[test]
fn resume_requires_only_the_latest_checkpoint_artifact() {
    let (root, request) = fixture(1);
    initialize_optimization_resume(&request, &root.0).unwrap();
    let old_checkpoint = artifact(
        &root.0,
        "build/artifacts/checkpoint-old.json",
        b"checkpoint-old",
    );
    append_optimization_resume_event(
        &request,
        &root.0,
        OptimizationResumeEvent::OptimizerCheckpoint {
            generation: 0,
            completed_candidates: 0,
            state: old_checkpoint.clone(),
        },
    )
    .unwrap();
    let candidate = artifact(
        &root.0,
        "build/artifacts/candidate-prune.json",
        b"candidate",
    );
    let candidate_sha256 = candidate.sha256;
    let tape = artifact(&root.0, "build/artifacts/candidate-prune.tape", b"tape");
    let result = artifact(&root.0, "build/artifacts/result-prune.json", b"result");
    let latest_checkpoint = artifact(
        &root.0,
        "build/artifacts/checkpoint-latest.json",
        b"checkpoint-latest",
    );
    append_optimization_resume_events(
        &request,
        &root.0,
        vec![
            candidate_event(&request, "g0-prune", candidate, tape),
            OptimizationResumeEvent::EvaluationCompleted {
                candidate_id: "g0-prune".into(),
                candidate_sha256,
                result,
                simulated_ticks: request.budgets.exploration_horizon_ticks,
            },
            OptimizationResumeEvent::OptimizerCheckpoint {
                generation: 1,
                completed_candidates: 1,
                state: latest_checkpoint.clone(),
            },
        ],
    )
    .unwrap();

    fs::remove_file(root.0.join(&old_checkpoint.path)).unwrap();
    assert_eq!(
        load_optimization_resume(&request, &root.0)
            .unwrap()
            .completed_candidates,
        1
    );
    fs::remove_file(root.0.join(&latest_checkpoint.path)).unwrap();
    assert!(
        load_optimization_resume(&request, &root.0)
            .unwrap_err()
            .to_string()
            .contains("latest optimizer checkpoint")
    );
}

#[test]
fn invalid_later_event_rejects_the_whole_batch_before_append() {
    let (root, request) = fixture(2);
    let initial = initialize_optimization_resume(&request, &root.0).unwrap();
    let first_candidate = artifact(&root.0, "build/artifacts/first.json", b"first");
    let second_candidate = artifact(&root.0, "build/artifacts/second.json", b"second");
    let shared_tape = artifact(&root.0, "build/artifacts/shared.tape", b"shared");

    let error = append_optimization_resume_events(
        &request,
        &root.0,
        vec![
            candidate_event(&request, "g0-c0", first_candidate, shared_tape.clone()),
            candidate_event(&request, "g0-c1", second_candidate, shared_tape),
        ],
    )
    .unwrap_err();

    assert!(error.to_string().contains("duplicate"));
    assert_eq!(
        load_optimization_resume(&request, &root.0).unwrap(),
        initial
    );
}

#[test]
fn batch_cannot_cross_the_checkpoint_boundary() {
    let (root, request) = fixture(1);
    let initial = initialize_optimization_resume(&request, &root.0).unwrap();
    let first_candidate = artifact(&root.0, "build/artifacts/first.json", b"first");
    let first_candidate_sha256 = first_candidate.sha256;
    let first_tape = artifact(&root.0, "build/artifacts/first.tape", b"first-tape");
    let result = artifact(&root.0, "build/artifacts/result.json", b"result");
    let second_candidate = artifact(&root.0, "build/artifacts/second.json", b"second");
    let second_tape = artifact(&root.0, "build/artifacts/second.tape", b"second-tape");

    let error = append_optimization_resume_events(
        &request,
        &root.0,
        vec![
            candidate_event(&request, "g0-c0", first_candidate, first_tape),
            OptimizationResumeEvent::EvaluationCompleted {
                candidate_id: "g0-c0".into(),
                candidate_sha256: first_candidate_sha256,
                result,
                simulated_ticks: 160,
            },
            candidate_event(&request, "g0-c1", second_candidate, second_tape),
        ],
    )
    .unwrap_err();

    assert!(error.to_string().contains("checkpoint is required"));
    assert_eq!(
        load_optimization_resume(&request, &root.0).unwrap(),
        initial
    );
}

#[test]
fn empty_batch_is_rejected_without_changing_the_journal() {
    let (root, request) = fixture(1);
    let initial = initialize_optimization_resume(&request, &root.0).unwrap();
    let error = append_optimization_resume_events(&request, &root.0, Vec::new()).unwrap_err();

    assert!(error.to_string().contains("at least one event"));
    assert_eq!(
        load_optimization_resume(&request, &root.0).unwrap(),
        initial
    );
}

#[test]
fn validated_state_append_rejects_stale_memory() {
    let (root, request) = fixture(2);
    let initial = initialize_optimization_resume(&request, &root.0).unwrap();
    let demonstration = artifact(
        &root.0,
        "build/artifacts/fast-demonstration.json",
        b"demonstration",
    );
    let current = append_optimization_resume_events_from_validated_state(
        &request,
        &root.0,
        &initial,
        vec![OptimizationResumeEvent::DemonstrationSeeded {
            demonstration,
            simulated_ticks: 125,
        }],
    )
    .unwrap();
    let candidate = artifact(&root.0, "build/artifacts/fast.json", b"candidate");
    let tape = artifact(&root.0, "build/artifacts/fast.tape", b"tape");

    let error = append_optimization_resume_events_from_validated_state(
        &request,
        &root.0,
        &initial,
        vec![candidate_event(&request, "g0-fast", candidate, tape)],
    )
    .unwrap_err();

    assert!(error.to_string().contains("differs from durable journal"));
    assert_eq!(
        load_optimization_resume(&request, &root.0).unwrap(),
        current
    );
}

#[test]
fn validated_state_append_authenticates_durable_event_artifacts() {
    let (root, request) = fixture(2);
    let initial = initialize_optimization_resume(&request, &root.0).unwrap();
    let candidate = artifact(&root.0, "build/artifacts/fast-corrupt.json", b"candidate");
    let candidate_path = root.0.join(&candidate.path);
    let tape = artifact(&root.0, "build/artifacts/fast-corrupt.tape", b"tape");
    let current = append_optimization_resume_events_from_validated_state(
        &request,
        &root.0,
        &initial,
        vec![candidate_event(
            &request,
            "g0-fast-corrupt",
            candidate,
            tape,
        )],
    )
    .unwrap();
    fs::write(candidate_path, b"tampered").unwrap();
    let result = artifact(
        &root.0,
        "build/artifacts/fast-corrupt-result.json",
        b"result",
    );

    let error = append_optimization_resume_events_from_validated_state(
        &request,
        &root.0,
        &current,
        vec![OptimizationResumeEvent::EvaluationCompleted {
            candidate_id: "g0-fast-corrupt".into(),
            candidate_sha256: current.candidates[0].candidate_sha256,
            result,
            simulated_ticks: 160,
        }],
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("candidate content digest differs")
    );
}
