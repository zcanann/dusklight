use huntctl::Digest;
use huntctl::benchmark::skybook::SkybookManifest;
use huntctl::benchmark::skybook_selection::{
    SKYBOOK_SELECTION_SCHEMA, SkybookSelection, SkybookSelectionDisposition, SkybookSelectionEntry,
};
use serde_json::Value;
use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn validates_human_selection_against_exact_checked_manifest() {
    let executable = env!("CARGO_BIN_EXE_huntctl");
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest_path = repository.join("benchmarks/skybook/manifest.json");
    let manifest: SkybookManifest =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest.validate().unwrap();

    let directory = std::env::temp_dir().join(format!(
        "huntctl-benchmark-cli-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&directory).unwrap();
    let selection_path = directory.join("selection.json");
    let mut selection = SkybookSelection {
        schema: SKYBOOK_SELECTION_SCHEMA.into(),
        content_sha256: Digest::ZERO,
        source_manifest_content_sha256: manifest.content_sha256,
        source_repository_url: manifest.source.repository_url.clone(),
        source_git_revision: manifest.source.git_revision.clone(),
        approved_by: "test-suite".into(),
        approval_reference: "test:exact-selection".into(),
        entries: manifest
            .pages
            .iter()
            .take(3)
            .map(|page| SkybookSelectionEntry {
                slug: page.slug.clone(),
                source_path: page.source_path.clone(),
                source_sha256: page.source_sha256,
                disposition: SkybookSelectionDisposition::Selected,
                rationale: "Selected to exercise the CLI validation boundary.".into(),
            })
            .collect(),
    };
    selection.refresh_content_sha256().unwrap();
    fs::write(&selection_path, selection.to_pretty_json().unwrap()).unwrap();

    let output = Command::new(executable)
        .args(["benchmark", "validate-skybook-selection", "--manifest"])
        .arg(&manifest_path)
        .arg("--selection")
        .arg(&selection_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["selected_page_count"], 3);
    assert_eq!(report["source_revision"], manifest.source.git_revision);

    selection.entries[0].source_sha256 = Digest([9; 32]);
    selection.refresh_content_sha256().unwrap();
    fs::write(&selection_path, selection.to_pretty_json().unwrap()).unwrap();
    let rejected = Command::new(executable)
        .args(["benchmark", "validate-skybook-selection", "--manifest"])
        .arg(&manifest_path)
        .arg("--selection")
        .arg(&selection_path)
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("identity is stale"));
    fs::remove_dir_all(directory).unwrap();
}
