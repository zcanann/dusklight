use huntctl::Digest;
use huntctl::stage_boot_catalog::{
    BootLayerSource, BootLayerSourceKind, BootPointSource, BootPointSourceKind,
    STAGE_BOOT_CATALOG_SCHEMA, StageBootCandidate, StageBootCatalog, StageCatalogStatus,
    StageInventoryStatus,
};
use huntctl::stage_observation_coverage::StageObservationCoverageReport;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn temporary_root() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("huntctl-survey-cli-{nonce}"))
}

fn catalog() -> StageBootCatalog {
    StageBootCatalog {
        schema: STAGE_BOOT_CATALOG_SCHEMA.into(),
        known_loader_sha256: None,
        stages: vec![StageCatalogStatus {
            stage: "F_SP103".into(),
            resources_present: true,
            inventory_status: StageInventoryStatus::Complete,
            inventory_sha256: Some(Digest([7; 32])),
            diagnostic: None,
            room_count: 1,
            player_spawn_count: 1,
            candidate_count: 1,
        }],
        candidates: vec![StageBootCandidate {
            id: "F_SP103/room/0/point/0/layer/-1".into(),
            stage: "F_SP103".into(),
            room: 0,
            point: 0,
            layer: -1,
            point_sources: vec![BootPointSource {
                kind: BootPointSourceKind::RetailPlayerSpawn,
                stable_id: Some("spawn-0".into()),
            }],
            layer_sources: vec![BootLayerSource {
                kind: BootLayerSourceKind::ResolvedDefault,
                chunk_tag: None,
            }],
        }],
    }
}

#[test]
fn initializes_and_reopens_a_content_bound_survey_ledger() {
    let root = temporary_root();
    fs::create_dir_all(&root).unwrap();
    let catalog_path = root.join("catalog.json");
    let ledger_path = root.join("ledger.json");
    let game_path = root.join("game.exe");
    let dvd_path = root.join("disc.iso");
    fs::write(&catalog_path, catalog().canonical_bytes().unwrap()).unwrap();
    fs::write(&game_path, b"test executable identity").unwrap();
    fs::write(&dvd_path, b"test disc identity").unwrap();

    let initialized = Command::new(env!("CARGO_BIN_EXE_huntctl"))
        .args(["survey", "init", "--catalog"])
        .arg(&catalog_path)
        .args(["--ledger"])
        .arg(&ledger_path)
        .args(["--game"])
        .arg(&game_path)
        .args(["--dvd"])
        .arg(&dvd_path)
        .args([
            "--probe",
            "movement",
            "--probe-ticks",
            "12",
            "--timeout-ms",
            "500",
            "--attempts",
            "2",
        ])
        .output()
        .unwrap();
    assert!(
        initialized.status.success(),
        "{}",
        String::from_utf8_lossy(&initialized.stderr)
    );

    let status = Command::new(env!("CARGO_BIN_EXE_huntctl"))
        .args(["survey", "status", "--catalog"])
        .arg(&catalog_path)
        .args(["--ledger"])
        .arg(&ledger_path)
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let document: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(document["progress"]["total"], 1);
    assert_eq!(document["progress"]["pending"], 1);
    assert_eq!(document["progress"]["attempted"], 0);
    assert_eq!(document["policy"]["probe_ticks"], 12);
    assert_eq!(document["policy"]["probe"], "movement");

    let invalid_probe_ledger = root.join("invalid-probe-ledger.json");
    let invalid_probe = Command::new(env!("CARGO_BIN_EXE_huntctl"))
        .args(["survey", "init", "--catalog"])
        .arg(&catalog_path)
        .args(["--ledger"])
        .arg(&invalid_probe_ledger)
        .args(["--game"])
        .arg(&game_path)
        .args(["--dvd"])
        .arg(&dvd_path)
        .args(["--probe", "benchmark-route"])
        .output()
        .unwrap();
    assert!(!invalid_probe.status.success());
    assert!(String::from_utf8_lossy(&invalid_probe.stderr).contains("unknown survey probe"));

    let unknown = Command::new(env!("CARGO_BIN_EXE_huntctl"))
        .args(["survey", "run", "--catalog"])
        .arg(&catalog_path)
        .args(["--ledger"])
        .arg(&ledger_path)
        .args(["--game"])
        .arg(&game_path)
        .args(["--dvd"])
        .arg(&dvd_path)
        .args(["--state-root"])
        .arg(root.join("state"))
        .args(["--candidate", "missing/room/0/point/0/layer/-1"])
        .output()
        .unwrap();
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("unknown survey candidate"));

    let duplicate_candidate = Command::new(env!("CARGO_BIN_EXE_huntctl"))
        .args(["survey", "run", "--catalog"])
        .arg(&catalog_path)
        .args(["--ledger"])
        .arg(&ledger_path)
        .args(["--game"])
        .arg(&game_path)
        .args(["--dvd"])
        .arg(&dvd_path)
        .args(["--state-root"])
        .arg(root.join("state"))
        .args(["--candidate", "F_SP103/room/0/point/0/layer/-1"])
        .args(["--candidate", "F_SP103/room/0/point/0/layer/-1"])
        .output()
        .unwrap();
    assert!(!duplicate_candidate.status.success());
    assert!(
        String::from_utf8_lossy(&duplicate_candidate.stderr).contains("duplicate survey candidate")
    );

    let mixed_selection = Command::new(env!("CARGO_BIN_EXE_huntctl"))
        .args(["survey", "run", "--catalog"])
        .arg(&catalog_path)
        .args(["--ledger"])
        .arg(&ledger_path)
        .args(["--game"])
        .arg(&game_path)
        .args(["--dvd"])
        .arg(&dvd_path)
        .args(["--state-root"])
        .arg(root.join("state"))
        .args(["--candidate", "F_SP103/room/0/point/0/layer/-1"])
        .args(["--limit", "1"])
        .output()
        .unwrap();
    assert!(!mixed_selection.status.success());
    assert!(
        String::from_utf8_lossy(&mixed_selection.stderr)
            .contains("repeated --candidate values or --limit")
    );

    let invalid_workers = Command::new(env!("CARGO_BIN_EXE_huntctl"))
        .args(["survey", "run", "--catalog"])
        .arg(&catalog_path)
        .args(["--ledger"])
        .arg(&ledger_path)
        .args(["--game"])
        .arg(&game_path)
        .args(["--dvd"])
        .arg(&dvd_path)
        .args(["--state-root"])
        .arg(root.join("state"))
        .args(["--workers", "0"])
        .output()
        .unwrap();
    assert!(!invalid_workers.status.success());
    assert!(String::from_utf8_lossy(&invalid_workers.stderr).contains("--workers"));

    let duplicate = Command::new(env!("CARGO_BIN_EXE_huntctl"))
        .args(["survey", "init", "--catalog"])
        .arg(&catalog_path)
        .args(["--ledger"])
        .arg(&ledger_path)
        .args(["--game"])
        .arg(&game_path)
        .args(["--dvd"])
        .arg(&dvd_path)
        .output()
        .unwrap();
    assert!(!duplicate.status.success());

    let coverage_path = root.join("observation-coverage.json");
    let coverage = Command::new(env!("CARGO_BIN_EXE_huntctl"))
        .args(["survey", "observation-coverage", "--catalog"])
        .arg(&catalog_path)
        .args(["--ledger"])
        .arg(&ledger_path)
        .args(["--state-root"])
        .arg(root.join("state"))
        .args(["--output"])
        .arg(&coverage_path)
        .output()
        .unwrap();
    assert!(
        coverage.status.success(),
        "{}",
        String::from_utf8_lossy(&coverage.stderr)
    );
    let coverage_summary: Value = serde_json::from_slice(&coverage.stdout).unwrap();
    assert_eq!(
        coverage_summary["schema"],
        "dusklight-stage-observation-coverage/v1"
    );
    assert_eq!(coverage_summary["sources"], 1);
    assert_eq!(coverage_summary["cases"], 0);
    assert_eq!(coverage_summary["cells"], 0);
    StageObservationCoverageReport::decode_canonical(&fs::read(coverage_path).unwrap()).unwrap();

    let mismatched_sources = Command::new(env!("CARGO_BIN_EXE_huntctl"))
        .args(["survey", "observation-coverage", "--catalog"])
        .arg(&catalog_path)
        .args(["--ledger"])
        .arg(&ledger_path)
        .args(["--ledger"])
        .arg(&ledger_path)
        .args(["--state-root"])
        .arg(root.join("state"))
        .args(["--output"])
        .arg(root.join("invalid-coverage.json"))
        .output()
        .unwrap();
    assert!(!mismatched_sources.status.success());
    assert!(
        String::from_utf8_lossy(&mismatched_sources.stderr)
            .contains("one --state-root for every --ledger")
    );

    fs::remove_dir_all(root).unwrap();
}
