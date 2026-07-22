use serde_json::Value;
use std::fs;
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
