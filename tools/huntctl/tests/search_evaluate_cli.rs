use huntctl::search::{Candidate, SearchResults};
use huntctl::tape::InputTape;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_huntctl"))
        .args(args)
        .output()
        .unwrap()
}

fn seed(root: &Path, segment: &str, size: &str) {
    let output = run(&[
        "search",
        "seed",
        "--segment",
        segment,
        "--output",
        root.to_str().unwrap(),
        "--size",
        size,
        "--rng-seed",
        "7",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn native_evaluator_handles_hits_goal_misses_timeouts_and_tape_import() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("huntctl-native-eval-{unique}"));
    fs::create_dir_all(&root).unwrap();
    let dvd = root.join("disc.iso");
    fs::write(&dvd, b"mock disc").unwrap();
    let game = env!("CARGO_BIN_EXE_huntctl");

    let route = root.join("route-population");
    seed(&route, "fsp103_to_fsp104", "4");
    let route_artifacts = root.join("route-evaluations");
    let route_results = root.join("route-results.json");
    let output = run(&[
        "search",
        "evaluate",
        "--population",
        route.join("manifest.json").to_str().unwrap(),
        "--game",
        game,
        "--game-arg",
        "mock-search-worker",
        "--game-arg",
        "--mock-mode",
        "--game-arg",
        "hit",
        "--dvd",
        dvd.to_str().unwrap(),
        "--output",
        route_artifacts.to_str().unwrap(),
        "--results",
        route_results.to_str().unwrap(),
        "--workers",
        "2",
        "--repetitions",
        "2",
        "--timeout-ms",
        "2000",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let results: SearchResults =
        serde_json::from_slice(&fs::read(&route_results).unwrap()).unwrap();
    assert!(results.candidates.values().all(|result| {
        result.milestone_depth == 4
            && result.attempts == 2
            && result.successes == 2
            && result.first_hit_ticks == [572, 572]
    }));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["planned_attempts"], 8);
    assert_eq!(report["infrastructure_faults"], 0);
    assert!(
        report["attempts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|attempt| {
                attempt["boundary_fingerprints"]["entered-f-sp104"].is_object()
                    && Path::new(attempt["state_root"].as_str().unwrap()).is_dir()
                    && Path::new(attempt["milestone_result"].as_str().unwrap()).is_file()
            })
    );

    let boot = root.join("boot-population");
    seed(&boot, "boot_to_fsp103", "2");
    let boot_results = root.join("boot-results.json");
    let output = run(&[
        "search",
        "evaluate",
        "--population",
        boot.join("manifest.json").to_str().unwrap(),
        "--game",
        game,
        "--game-arg",
        "mock-search-worker",
        "--game-arg",
        "--mock-mode",
        "--game-arg",
        "miss",
        "--dvd",
        dvd.to_str().unwrap(),
        "--output",
        root.join("boot-evaluations").to_str().unwrap(),
        "--results",
        boot_results.to_str().unwrap(),
        "--workers",
        "2",
        "--repetitions",
        "1",
    ]);
    assert!(
        output.status.success(),
        "valid goal miss was treated as infrastructure failure: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let results: SearchResults = serde_json::from_slice(&fs::read(&boot_results).unwrap()).unwrap();
    assert!(results.candidates.values().all(|result| {
        result.milestone_depth == 0 && result.attempts == 1 && result.successes == 0
    }));

    let timeout_root = root.join("timeout-evaluations");
    let output = run(&[
        "search",
        "evaluate",
        "--population",
        boot.join("manifest.json").to_str().unwrap(),
        "--game",
        game,
        "--game-arg",
        "mock-search-worker",
        "--game-arg",
        "--mock-mode",
        "--game-arg",
        "timeout",
        "--dvd",
        dvd.to_str().unwrap(),
        "--output",
        timeout_root.to_str().unwrap(),
        "--workers",
        "1",
        "--repetitions",
        "1",
        "--timeout-ms",
        "40",
    ]);
    assert!(!output.status.success());
    let evidence: serde_json::Value =
        serde_json::from_slice(&fs::read(timeout_root.join("evaluation.json")).unwrap()).unwrap();
    assert!(evidence["infrastructure_faults"].as_u64().unwrap() > 0);
    assert!(!timeout_root.join("results.json").exists());

    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(boot.join("manifest.json")).unwrap()).unwrap();
    let tape_path = boot.join(manifest["members"][0]["tape_file"].as_str().unwrap());
    let candidate_path = root.join("imported.candidate.json");
    let output = run(&[
        "search",
        "import-tape",
        "--segment",
        "boot_to_fsp103",
        "--tape",
        tape_path.to_str().unwrap(),
        "--output",
        candidate_path.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let candidate: Candidate = serde_json::from_slice(&fs::read(candidate_path).unwrap()).unwrap();
    let tape = InputTape::decode(&fs::read(tape_path).unwrap())
        .unwrap()
        .tape;
    assert_eq!(candidate.compile().unwrap(), tape);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_evaluator_rejects_repeat_disagreement_as_invalid_evidence() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("huntctl-unstable-eval-{unique}"));
    fs::create_dir_all(&root).unwrap();
    let dvd = root.join("disc.iso");
    fs::write(&dvd, b"mock disc").unwrap();
    let population = root.join("population");
    seed(&population, "fsp103_to_fsp104", "1");

    for mode in [
        "unstable-tick",
        "unstable-frame",
        "unstable-fingerprint",
        "unstable-goal",
    ] {
        let output_root = root.join(mode);
        let results = root.join(format!("{mode}.results.json"));
        let output = run(&[
            "search",
            "evaluate",
            "--population",
            population.join("manifest.json").to_str().unwrap(),
            "--game",
            env!("CARGO_BIN_EXE_huntctl"),
            "--game-arg",
            "mock-search-worker",
            "--game-arg",
            "--mock-mode",
            "--game-arg",
            mode,
            "--dvd",
            dvd.to_str().unwrap(),
            "--output",
            output_root.to_str().unwrap(),
            "--results",
            results.to_str().unwrap(),
            "--workers",
            "2",
            "--repetitions",
            "2",
            "--timeout-ms",
            "2000",
        ]);
        assert!(!output.status.success(), "{mode} disagreement was accepted");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("nondeterministic milestone evidence"),
            "unexpected {mode} error: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!results.exists());
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn search_run_owns_the_complete_generation_loop() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("huntctl-native-run-{unique}"));
    fs::create_dir_all(&root).unwrap();
    let dvd = root.join("disc.iso");
    fs::write(&dvd, b"mock disc").unwrap();
    let mut seed_candidate = Candidate::baseline("fsp103_to_fsp104".parse().unwrap());
    if let huntctl::search::MacroAction::Neutral { frames } = &mut seed_candidate.actions[0] {
        *frames += 7;
    }
    let seed_id = seed_candidate.id().unwrap();
    let seed_path = root.join("seed.candidate.json");
    fs::write(
        &seed_path,
        serde_json::to_vec_pretty(&seed_candidate).unwrap(),
    )
    .unwrap();
    let output_root = root.join("run");
    let output = run(&[
        "search",
        "run",
        "--segment",
        "fsp103_to_fsp104",
        "--candidate",
        seed_path.to_str().unwrap(),
        "--game",
        env!("CARGO_BIN_EXE_huntctl"),
        "--game-arg",
        "mock-search-worker",
        "--dvd",
        dvd.to_str().unwrap(),
        "--output",
        output_root.to_str().unwrap(),
        "--generations",
        "2",
        "--size",
        "4",
        "--elites",
        "2",
        "--workers",
        "2",
        "--repetitions",
        "1",
        "--timeout-ms",
        "2000",
        "--rng-seed",
        "11",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(summary["schema"], "dusklight-search-run/v2");
    assert!(output_root.join("champion.tape").is_file());
    assert!(output_root.join("champion.candidate.json").is_file());
    let champion: Candidate =
        serde_json::from_slice(&fs::read(output_root.join("champion.candidate.json")).unwrap())
            .unwrap();
    let champion_tape = InputTape::decode(&fs::read(output_root.join("champion.tape")).unwrap())
        .unwrap()
        .tape;
    assert_eq!(champion.compile().unwrap(), champion_tape);
    let initial_manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(output_root.join("g000/manifest.json")).unwrap()).unwrap();
    assert!(
        initial_manifest["members"]
            .as_array()
            .unwrap()
            .iter()
            .any(|member| member["candidate_id"] == seed_id)
    );
    assert_eq!(
        summary["champion_candidate"],
        output_root
            .join("champion.candidate.json")
            .to_string_lossy()
            .as_ref()
    );
    assert!(output_root.join("g000/results.json").is_file());
    assert!(output_root.join("g001/results.json").is_file());
    fs::remove_dir_all(root).unwrap();
}
