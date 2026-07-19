use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_huntctl"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn composes_and_evaluates_checked_oracle_evidence() {
    let repository = repository();
    let fixtures = repository.join("tests/fixtures/automation");
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let output_root = std::env::temp_dir().join(format!("huntctl-oracle-cli-{unique}"));
    std::fs::create_dir_all(&output_root).unwrap();

    let composed_path = output_root.join("comparison-evidence.json");
    let output = run(&[
        "oracle",
        "compose",
        "--manifest",
        fixtures.join("oracle_composition.json").to_str().unwrap(),
        "--output",
        composed_path.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let composed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&composed_path).unwrap()).unwrap();
    assert_eq!(composed["schema"], "dusklight-comparison-evidence/v1");

    let report_path = output_root.join("comparison-report.json");
    let output = run(&[
        "oracle",
        "compare",
        "--program",
        fixtures.join("comparison_oracles.json").to_str().unwrap(),
        "--evidence",
        fixtures.join("comparison_evidence.json").to_str().unwrap(),
        "--output",
        report_path.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(report["schema"], "dusklight-comparison-oracle-results/v1");
    assert!(!report["results"].as_array().unwrap().is_empty());

    std::fs::remove_dir_all(output_root).unwrap();
}
