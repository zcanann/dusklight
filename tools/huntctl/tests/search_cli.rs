use huntctl::search::{PopulationManifest, SearchResults};
use huntctl::tape::{InputTape, TapeBoot};
use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_huntctl"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn file_oriented_search_round_trip_works_without_the_game() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("huntctl-search-cli-{unique}"));
    let g0 = root.join("g0");
    let g1 = root.join("g1");
    let results = root.join("results.json");

    let output = run(&[
        "search",
        "seed",
        "--segment",
        "fsp103_to_fsp104",
        "--output",
        g0.to_str().unwrap(),
        "--size",
        "6",
        "--rng-seed",
        "123",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest: PopulationManifest =
        serde_json::from_slice(&fs::read(g0.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest.members.len(), 6);
    assert_eq!(
        manifest.boot,
        TapeBoot::Stage {
            stage: "F_SP103".into(),
            room: 1,
            point: 1,
            layer: 3,
            save_slot: None,
            fixture: None,
        }
    );
    for member in &manifest.members {
        let decoded = InputTape::decode(&fs::read(g0.join(&member.tape_file)).unwrap()).unwrap();
        assert_eq!(decoded.tape.frames.len() as u64, member.frame_count);
    }

    let output = run(&[
        "search",
        "mock-evaluate",
        "--population",
        g0.join("manifest.json").to_str().unwrap(),
        "--output",
        results.to_str().unwrap(),
        "--attempts",
        "4",
    ]);
    assert!(output.status.success());
    let scores: SearchResults = serde_json::from_slice(&fs::read(&results).unwrap()).unwrap();
    assert_eq!(scores.candidates.len(), 6);
    assert_eq!(scores.boot, manifest.boot);

    let output = run(&[
        "search",
        "rank",
        "--population",
        g0.join("manifest.json").to_str().unwrap(),
        "--results",
        results.to_str().unwrap(),
    ]);
    assert!(output.status.success());
    let leaderboard: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(leaderboard[0]["rank"], 1);
    assert_eq!(leaderboard[0]["boot"]["kind"], "stage");

    let output = run(&[
        "search",
        "evolve",
        "--population",
        g0.join("manifest.json").to_str().unwrap(),
        "--results",
        results.to_str().unwrap(),
        "--output",
        g1.to_str().unwrap(),
        "--size",
        "6",
        "--elites",
        "2",
        "--rng-seed",
        "456",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let next: PopulationManifest =
        serde_json::from_slice(&fs::read(g1.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(next.generation, 1);
    assert_eq!(next.members.len(), 6);
    assert!(
        next.members
            .iter()
            .skip(2)
            .all(|member| member.ancestry.parent_id.is_some())
    );
    fs::remove_dir_all(root).unwrap();
}
