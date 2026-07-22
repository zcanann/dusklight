use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn unique_output() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!(
        "build/harness/campaign-dry-run-{}-{nanos}",
        std::process::id()
    )
}

fn sha256(path: impl AsRef<std::path::Path>) -> String {
    let bytes = fs::read(path).unwrap();
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn dry_run_resolves_a_content_bound_case_without_writing() {
    let repository = repository();
    let relative_output = unique_output();
    let absolute_output = repository.join(&relative_output);
    assert!(!absolute_output.exists());
    let result = Command::new(env!("CARGO_BIN_EXE_huntctl"))
        .current_dir(&repository)
        .args([
            "campaign",
            "--suite",
            "tests/fixtures/automation/objective_conformance_suite.json",
            "--case",
            "reach-point-ordon-ranch",
            "--output",
            &relative_output,
            "--dry-run",
            "--proposer",
            "structured",
            "--proposer",
            "scripted",
        ])
        .arg("--repository-root")
        .arg(&repository)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!absolute_output.exists());

    let plan: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(plan["schema"], "dusklight-campaign-plan/v1");
    assert_eq!(plan["dry_run"], true);
    assert_eq!(plan["suite_id"], "core-conformance/v1");
    assert_eq!(plan["case_id"], "reach-point-ordon-ranch");
    assert_eq!(plan["case_role"], "positive");
    assert_eq!(plan["expected_terminal"], "reached");
    assert_eq!(
        plan["proposers"],
        serde_json::json!(["scripted", "structured"])
    );
    assert_eq!(
        plan["required_facts"],
        serde_json::json!([
            "player.exists",
            "player.in_aabb",
            "player.is_link",
            "stage.name",
            "stage.room"
        ])
    );
    let capabilities = plan["required_capabilities"].as_array().unwrap();
    for required in [
        "gameplay-trace-v5",
        "input-tape-v3",
        "milestone-program-v1.5",
        "observation-family:player_motion/v1",
        "observation-family:stage/v1",
        "scenario-fixture-v1",
        "stage-boot",
        "typed-fact-response-v1",
    ] {
        assert!(capabilities.iter().any(|value| value == required));
    }
    assert_eq!(plan["budgets"]["logical_ticks_per_episode"], 799);
    assert_eq!(plan["budgets"]["repetitions"], 2);
    assert_eq!(plan["budgets"]["selected_proposers"], 2);
    assert_eq!(plan["budgets"]["planned_episodes"], 4);
    assert_eq!(plan["outputs"]["root"], absolute_output.to_str().unwrap());
    assert_eq!(plan["outputs"]["available"], true);
    assert_eq!(
        plan["outputs"]["report"],
        absolute_output.join("report.json").to_str().unwrap()
    );
    assert!(
        PathBuf::from(plan["resolved_paths"]["objective"].as_str().unwrap()).ends_with(
            PathBuf::from("routes/samples/maps/reach_point_ordon.milestones")
        )
    );
    assert_eq!(
        plan["identities"]["objective_program_sha256"],
        "a8a3fe13c4958ae73d6a635120176ea1702e964bc94610ca677fae77c7bf97b0"
    );
}

#[test]
fn dry_run_rejects_unknown_cases_and_non_build_outputs() {
    let repository = repository();
    for (case, output, expected) in [
        ("missing-case", "build/harness/missing", "suite has no case"),
        (
            "reach-point-ordon-ranch",
            "artifacts/outside-build",
            "beneath build/",
        ),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_huntctl"))
            .current_dir(&repository)
            .args([
                "campaign",
                "--suite",
                "tests/fixtures/automation/objective_conformance_suite.json",
                "--case",
                case,
                "--output",
                output,
                "--dry-run",
            ])
            .arg("--repository-root")
            .arg(&repository)
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains(expected));
    }
}

#[test]
fn validates_and_seals_the_ordon_optimization_request() {
    let repository = repository();
    let request_path =
        "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json";
    let validated = Command::new(env!("CARGO_BIN_EXE_huntctl"))
        .current_dir(&repository)
        .args([
            "campaign",
            "validate-optimization-request",
            "--input",
            request_path,
        ])
        .arg("--repository-root")
        .arg(&repository)
        .output()
        .unwrap();
    assert!(
        validated.status.success(),
        "{}",
        String::from_utf8_lossy(&validated.stderr)
    );
    let report: Value = serde_json::from_slice(&validated.stdout).unwrap();
    assert_eq!(report["segment"], "to_ordon_spring_q125");
    assert_eq!(report["incumbent_first_hit_tick"], 125);
    assert_eq!(report["exploration_horizon_ticks"], 160);
    assert_eq!(report["promotion_before_tick"], 125);
    assert_eq!(
        report["search_space_sha256"],
        "a19105c390e4a32232e50da81d290994ee035ca614c1dc0181d5784bd7dd1879"
    );

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let temporary = std::env::temp_dir();
    let draft_path = temporary.join(format!("dusklight-optimization-draft-{nonce}.json"));
    let sealed_path = temporary.join(format!("dusklight-optimization-sealed-{nonce}.json"));
    let mut draft: Value =
        serde_json::from_slice(&fs::read(repository.join(request_path)).unwrap()).unwrap();
    draft["content_sha256"] = Value::String("0".repeat(64));
    fs::write(&draft_path, serde_json::to_vec_pretty(&draft).unwrap()).unwrap();
    let sealed = Command::new(env!("CARGO_BIN_EXE_huntctl"))
        .current_dir(&repository)
        .args(["campaign", "seal-optimization-request", "--input"])
        .arg(&draft_path)
        .arg("--output")
        .arg(&sealed_path)
        .arg("--repository-root")
        .arg(&repository)
        .output()
        .unwrap();
    assert!(
        sealed.status.success(),
        "{}",
        String::from_utf8_lossy(&sealed.stderr)
    );
    assert_eq!(
        fs::read(&sealed_path).unwrap(),
        fs::read(repository.join(request_path)).unwrap()
    );
    fs::remove_file(draft_path).unwrap();
    fs::remove_file(sealed_path).unwrap();
}

#[test]
fn optimization_request_rejects_coupled_horizons_and_timeline_tampering() {
    let repository = repository();
    let request_path = repository.join(
        "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json",
    );
    let request: Value = serde_json::from_slice(&fs::read(request_path).unwrap()).unwrap();
    for (suffix, mutation, expected) in [
        (
            "horizon",
            ("/budgets/exploration_horizon_ticks", Value::from(125)),
            "strictly larger exploration horizon",
        ),
        (
            "timeline",
            ("/route/timeline/sha256", Value::String("1".repeat(64))),
            "timeline content digest differs",
        ),
        (
            "proposal-schema",
            (
                "/proposal/proposal_schema/sha256",
                Value::String("2".repeat(64)),
            ),
            "detached from the implemented raw-PAD compiler",
        ),
        (
            "residual-space",
            (
                "/proposal/search_space/end_frame_exclusive",
                Value::from(127),
            ),
            "exceeds the incumbent tape",
        ),
        (
            "failure-retention",
            ("/retention/failed_episodes", Value::String("none".into())),
            "failures must be retained",
        ),
    ] {
        let mut changed = request.clone();
        *changed.pointer_mut(mutation.0).unwrap() = mutation.1;
        changed["content_sha256"] = Value::String("0".repeat(64));
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "dusklight-optimization-{suffix}-{}-{nonce}.json",
            std::process::id()
        ));
        let output_path = path.with_extension("sealed.json");
        fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_huntctl"))
            .current_dir(&repository)
            .args(["campaign", "seal-optimization-request", "--input"])
            .arg(&path)
            .arg("--output")
            .arg(&output_path)
            .arg("--repository-root")
            .arg(&repository)
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains(expected));
        assert!(!output_path.exists());
        fs::remove_file(path).unwrap();
    }
}

#[test]
fn optimization_resume_recovers_a_partial_tail_without_repeating_candidates() {
    let repository = repository();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let relative_root = format!(
        "build/harness/optimization-resume-test-{}-{nonce}",
        std::process::id()
    );
    let absolute_root = repository.join(&relative_root);
    let temporary = std::env::temp_dir();
    let draft_path = temporary.join(format!("dusklight-resume-draft-{nonce}.json"));
    let request_path = temporary.join(format!("dusklight-resume-request-{nonce}.json"));
    let event_path = temporary.join(format!("dusklight-resume-event-{nonce}.json"));
    let checked_request = repository.join(
        "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json",
    );
    let mut draft: Value = serde_json::from_slice(&fs::read(&checked_request).unwrap()).unwrap();
    draft["content_sha256"] = Value::String("0".repeat(64));
    draft["resume"]["state_path"] = Value::String(format!("{relative_root}/state.json"));
    draft["resume"]["journal_path"] = Value::String(format!("{relative_root}/journal.jsonl"));
    draft["resume"]["checkpoint_every_candidates"] = Value::from(1);
    fs::write(&draft_path, serde_json::to_vec_pretty(&draft).unwrap()).unwrap();

    let seal = Command::new(env!("CARGO_BIN_EXE_huntctl"))
        .current_dir(&repository)
        .args(["campaign", "seal-optimization-request", "--input"])
        .arg(&draft_path)
        .arg("--output")
        .arg(&request_path)
        .arg("--repository-root")
        .arg(&repository)
        .output()
        .unwrap();
    assert!(
        seal.status.success(),
        "{}",
        String::from_utf8_lossy(&seal.stderr)
    );
    let init = Command::new(env!("CARGO_BIN_EXE_huntctl"))
        .current_dir(&repository)
        .args(["campaign", "init-optimization-resume", "--request"])
        .arg(&request_path)
        .arg("--repository-root")
        .arg(&repository)
        .output()
        .unwrap();
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&init.stdout).unwrap()["record_count"],
        0
    );

    let request_artifact =
        "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json";
    let tape_artifact = "routes/Glitch Exhibition/intro/segments/to_ordon_spring_q125.tape";
    let result_artifact =
        "routes/Glitch Exhibition/intro/benchmarks/ordon_spring_load_committed.observation.json";
    let request_digest = sha256(repository.join(request_artifact));
    let tape_digest = sha256(repository.join(tape_artifact));
    let result_digest = sha256(repository.join(result_artifact));
    let append = |event: Value| {
        fs::write(&event_path, serde_json::to_vec(&event).unwrap()).unwrap();
        Command::new(env!("CARGO_BIN_EXE_huntctl"))
            .current_dir(&repository)
            .args(["campaign", "append-optimization-resume", "--request"])
            .arg(&request_path)
            .arg("--event")
            .arg(&event_path)
            .arg("--repository-root")
            .arg(&repository)
            .output()
            .unwrap()
    };
    let candidate = serde_json::json!({
        "kind": "candidate_sealed",
        "candidate_id": "candidate-0001",
        "candidate": {"path": request_artifact, "sha256": request_digest.clone()},
        "compiled_tape": {"path": tape_artifact, "sha256": tape_digest.clone()},
        "parent_tape_sha256": tape_digest.clone(),
        "generation": 0,
        "proposer_seed": 104729
    });
    let candidate_result = append(candidate.clone());
    assert!(
        candidate_result.status.success(),
        "{}",
        String::from_utf8_lossy(&candidate_result.stderr)
    );
    let candidate_state: Value = serde_json::from_slice(&candidate_result.stdout).unwrap();
    assert_eq!(
        candidate_state["pending_candidate_ids"],
        serde_json::json!(["candidate-0001"])
    );

    let oversized_evaluation = append(serde_json::json!({
        "kind": "evaluation_completed",
        "candidate_id": "candidate-0001",
        "candidate_sha256": request_digest.clone(),
        "result": {"path": result_artifact, "sha256": result_digest.clone()},
        "simulated_ticks": 161
    }));
    assert!(!oversized_evaluation.status.success());
    assert!(
        String::from_utf8_lossy(&oversized_evaluation.stderr)
            .contains("per-candidate exploration tick bound")
    );

    let evaluation_result = append(serde_json::json!({
        "kind": "evaluation_completed",
        "candidate_id": "candidate-0001",
        "candidate_sha256": request_digest.clone(),
        "result": {"path": result_artifact, "sha256": result_digest},
        "simulated_ticks": 125
    }));
    assert!(evaluation_result.status.success());
    let evaluation_state: Value = serde_json::from_slice(&evaluation_result.stdout).unwrap();
    assert_eq!(evaluation_state["completed_candidates"], 1);
    assert_eq!(evaluation_state["uncheckpointed_completions"], 1);

    let before_checkpoint = append(serde_json::json!({
        "kind": "candidate_sealed",
        "candidate_id": "candidate-0002",
        "candidate": {"path": request_artifact, "sha256": request_digest.clone()},
        "compiled_tape": {"path": tape_artifact, "sha256": tape_digest.clone()},
        "parent_tape_sha256": tape_digest.clone(),
        "generation": 1,
        "proposer_seed": 130363
    }));
    assert!(!before_checkpoint.status.success());
    assert!(
        String::from_utf8_lossy(&before_checkpoint.stderr)
            .contains("optimizer checkpoint is required")
    );

    let checkpoint_result = append(serde_json::json!({
        "kind": "optimizer_checkpoint",
        "generation": 0,
        "completed_candidates": 1,
        "state": {"path": request_artifact, "sha256": request_digest.clone()}
    }));
    assert!(checkpoint_result.status.success());
    let checkpoint_state: Value = serde_json::from_slice(&checkpoint_result.stdout).unwrap();
    assert_eq!(checkpoint_state["record_count"], 3);
    assert_eq!(checkpoint_state["uncheckpointed_completions"], 0);

    let journal_path = absolute_root.join("journal.jsonl");
    let valid_length = fs::metadata(&journal_path).unwrap().len();
    let mut journal = OpenOptions::new().append(true).open(&journal_path).unwrap();
    journal.write_all(br#"{"partial":"#).unwrap();
    journal.sync_all().unwrap();
    drop(journal);
    let status = Command::new(env!("CARGO_BIN_EXE_huntctl"))
        .current_dir(&repository)
        .args(["campaign", "status-optimization-resume", "--request"])
        .arg(&request_path)
        .arg("--repository-root")
        .arg(&repository)
        .output()
        .unwrap();
    assert!(status.status.success());
    assert_eq!(fs::metadata(&journal_path).unwrap().len(), valid_length);
    assert_eq!(
        serde_json::from_slice::<Value>(&status.stdout).unwrap()["record_count"],
        3
    );

    let duplicate = append(candidate);
    assert!(!duplicate.status.success());
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("duplicate"));

    let mut corrupt = fs::read(&journal_path).unwrap();
    let marker = b"candidate-0001";
    let offset = corrupt
        .windows(marker.len())
        .position(|window| window == marker)
        .unwrap();
    corrupt[offset + marker.len() - 1] = b'2';
    fs::write(&journal_path, corrupt).unwrap();
    let rejected = Command::new(env!("CARGO_BIN_EXE_huntctl"))
        .current_dir(&repository)
        .args(["campaign", "status-optimization-resume", "--request"])
        .arg(&request_path)
        .arg("--repository-root")
        .arg(&repository)
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("corrupt"));

    fs::remove_dir_all(absolute_root).unwrap();
    fs::remove_file(draft_path).unwrap();
    fs::remove_file(request_path).unwrap();
    fs::remove_file(event_path).unwrap();
}
