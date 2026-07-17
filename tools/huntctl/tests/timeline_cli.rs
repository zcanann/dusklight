use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_huntctl"))
        .args(args)
        .output()
        .unwrap()
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn authored_timeline_and_content_addressed_store_round_trip() {
    let route = repo_root().join("routes/intro.timeline");
    let parsed = run(&["timeline", "parse", route.to_str().unwrap()]);
    assert!(
        parsed.status.success(),
        "{}",
        String::from_utf8_lossy(&parsed.stderr)
    );
    let summary: serde_json::Value = serde_json::from_slice(&parsed.stdout).unwrap();
    assert_eq!(summary["valid"], true);
    assert_eq!(summary["segments"], 2);

    let status = run(&[
        "timeline",
        "status",
        "--timeline",
        route.to_str().unwrap(),
        "--continuation",
        "main",
    ]);
    assert!(status.status.success());
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["immutable_lineages"][0]["stale"], false);
    assert_eq!(
        status["workspace"]["steps"][0]["workspace_segment"],
        "golf439"
    );
    assert_eq!(status["workspace"]["steps"][0]["state"], "unchanged");
    assert_eq!(
        status["workspace"]["steps"][1]["workspace_segment"],
        "to_ordon_spring_human150"
    );
    assert_eq!(status["workspace"]["steps"][1]["state"], "unchanged");

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let store = std::env::temp_dir().join(format!("huntctl-timeline-cli-{unique}"));
    assert!(
        run(&["timeline", "store", "init", store.to_str().unwrap()])
            .status
            .success()
    );
    let imported = run(&[
        "timeline",
        "store",
        "import",
        "--store",
        store.to_str().unwrap(),
        "--timeline",
        route.to_str().unwrap(),
        "--ref",
        "routes/intro",
    ]);
    assert!(
        imported.status.success(),
        "{}",
        String::from_utf8_lossy(&imported.stderr)
    );
    let imported: serde_json::Value = serde_json::from_slice(&imported.stdout).unwrap();
    assert_eq!(
        imported["segments"]["to_ordon_spring_human150"]["parent"],
        "golf439"
    );
    assert!(
        imported["segments"]["to_ordon_spring_human150"]["goals"]["ordon_spring_load_committed"]
            .is_string()
    );
    assert!(imported["segments"]["to_ordon_spring_human150"]["goal_proofs"]
        ["ordon_spring_load_committed"]
        .is_string());
    assert!(imported["segments"]["to_ordon_spring_human150"]["tape"].is_string());
    assert!(
        run(&[
            "timeline",
            "store",
            "fork",
            "--store",
            store.to_str().unwrap(),
            "--from",
            "routes/intro",
            "--to",
            "experiments/main",
            "--lineage",
            "main",
        ])
        .status
        .success()
    );
    assert!(
        run(&[
            "timeline",
            "store",
            "append",
            "--store",
            store.to_str().unwrap(),
            "--ref",
            "experiments/main",
            "--timeline",
            route.to_str().unwrap(),
            "--continuation",
            "main",
        ])
        .status
        .success()
    );
    let verified = run(&[
        "timeline",
        "store",
        "verify",
        "--store",
        store.to_str().unwrap(),
    ]);
    assert!(verified.status.success());
    let verified: serde_json::Value = serde_json::from_slice(&verified.stdout).unwrap();
    assert_eq!(verified["valid"], true);
    assert!(verified["objects"].as_u64().unwrap() > 0);
    let gc = run(&[
        "timeline",
        "store",
        "gc",
        "--store",
        store.to_str().unwrap(),
    ]);
    assert!(gc.status.success());
    let gc: serde_json::Value = serde_json::from_slice(&gc.stdout).unwrap();
    assert_eq!(gc["removed"], 0);
    fs::remove_dir_all(store).unwrap();
}
