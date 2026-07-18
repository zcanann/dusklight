use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn stage_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("orig/GZ2E01/files/res/Stage/F_SP103")
}

fn run(args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_huntctl"))
        .args(args)
        .output()
        .expect("huntctl must launch");
    assert!(
        output.status.success(),
        "huntctl failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("huntctl must emit JSON")
}

#[test]
fn spatial_index_and_filtered_point_query_cli_match_real_fixture_when_present() {
    let stage_dir = stage_dir();
    if !stage_dir.is_dir() {
        eprintln!("skipping world spatial CLI golden: original disc data is absent");
        return;
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let output_path = std::env::temp_dir().join(format!(
        "dusklight-world-spatial-{}-{nonce}.json",
        std::process::id()
    ));
    let stage_dir = stage_dir.to_string_lossy();
    let output = output_path.to_string_lossy();
    let summary = run(&[
        "world",
        "spatial-index",
        "--stage-dir",
        &stage_dir,
        "--stage",
        "F_SP103",
        "--output",
        &output,
    ]);
    assert_eq!(summary["indexed_surfaces"], 10_790);
    assert_eq!(summary["excluded_surfaces"], 4);
    assert_eq!(
        summary["spatial_index_sha256"],
        "2ad975eee45193b4325bb420a7ba5a78d533bed80cbcfeace29dcc5418e73834"
    );
    assert!(output_path.is_file());

    let query = run(&[
        "world",
        "query",
        "point",
        "--stage-dir",
        &stage_dir,
        "--stage",
        "F_SP103",
        "--room",
        "1",
        "--point",
        "-2037.3324,729.72,-4264.551",
        "--max-distance",
        "0.001",
        "--limit",
        "4",
        "--destination-stage",
        "F_SP104",
    ]);
    assert_eq!(query["returned_count"], 1);
    assert_eq!(
        query["results"][0]["surface"]["authored"]["prism_index"],
        2217
    );
    assert_eq!(
        query["results"][0]["surface"]["load_trigger"]["destination_stage"],
        "F_SP104"
    );

    fs::remove_file(output_path).expect("temporary spatial artifact must be removable");
}
