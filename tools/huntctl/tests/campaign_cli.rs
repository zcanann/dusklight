use serde_json::Value;
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
        plan["resolved_paths"]["objective"]
            .as_str()
            .unwrap()
            .ends_with("tests/fixtures/automation/reach_point_ordon.milestones")
    );
    assert_eq!(
        plan["identities"]["objective_program_sha256"],
        "05c586902adeff1bcd06151002e496ff7a1024795bef18fd347f7b40ca4e5fe2"
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
