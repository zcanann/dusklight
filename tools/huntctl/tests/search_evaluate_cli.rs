use huntctl::continuous_search::{
    CONTINUOUS_AXES_SCHEMA_V1, ContinuousAxes, ContinuousAxis, ContinuousParameter,
};
use huntctl::search::{
    Ancestry, CANDIDATE_SCHEMA, Candidate, ControllerButton, MacroAction, SearchResults,
    SegmentProfile,
};
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
                let learning_trace_is_correct = if attempt["attempt"] == 1 {
                    attempt["gameplay_trace_error"].is_null()
                        && Path::new(attempt["gameplay_trace"].as_str().unwrap()).is_file()
                        && route_artifacts
                            .join("content")
                            .join(
                                attempt["gameplay_trace_blob"]["relative_path"]
                                    .as_str()
                                    .unwrap(),
                            )
                            .is_file()
                } else {
                    attempt["gameplay_trace"].is_null()
                        && attempt["gameplay_trace_blob"].is_null()
                        && attempt["gameplay_trace_error"].is_null()
                };
                learning_trace_is_correct
                    && attempt["boundary_fingerprints"]["entered-f-sp104"].is_object()
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

#[test]
fn boot_minimizer_ddmins_a_dense_contiguous_mash_and_proves_trim() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("huntctl-boot-minimize-{unique}"));
    fs::create_dir_all(&root).unwrap();
    let dvd = root.join("disc.iso");
    fs::write(&dvd, b"mock disc").unwrap();
    let dense = Candidate {
        schema: CANDIDATE_SCHEMA.into(),
        segment: SegmentProfile::BootToFsp103,
        boot: huntctl::tape::TapeBoot::Process,
        actions: (0..120)
            .map(|index| MacroAction::Press {
                buttons: vec![if index % 2 == 0 {
                    ControllerButton::A
                } else {
                    ControllerButton::Start
                }],
                hold_frames: 1,
                neutral_frames: 0,
            })
            .collect(),
        ancestry: Ancestry::default(),
    };
    let candidate_path = root.join("dense.candidate.json");
    fs::write(&candidate_path, serde_json::to_vec_pretty(&dense).unwrap()).unwrap();
    let rejected = run(&[
        "search",
        "minimize-boot",
        "--candidate",
        candidate_path.to_str().unwrap(),
        "--game",
        env!("CARGO_BIN_EXE_huntctl"),
        "--dvd",
        dvd.to_str().unwrap(),
        "--output",
        root.join("single-sample").to_str().unwrap(),
        "--repetitions",
        "1",
    ]);
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("at least two repetitions"));
    let output_root = root.join("minimized");
    let output = run(&[
        "search",
        "minimize-boot",
        "--candidate",
        candidate_path.to_str().unwrap(),
        "--game",
        env!("CARGO_BIN_EXE_huntctl"),
        "--game-arg",
        "mock-search-worker",
        "--dvd",
        dvd.to_str().unwrap(),
        "--output",
        output_root.to_str().unwrap(),
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
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(summary["schema"], "dusklight-boot-minimization/v1");
    assert_eq!(summary["source_pulse_frames"], 120);
    assert_eq!(summary["minimized_pulse_frames"], 0);
    assert_eq!(summary["goal_tape_frame"], 77);
    assert_eq!(summary["minimized_frames"], 78);
    assert!(output_root.join("minimized.candidate.json").is_file());
    assert!(output_root.join("minimized.tape").is_file());
    assert!(output_root.join("proof.json").is_file());
    assert!(output_root.join("minimize.summary.json").is_file());
    let minimized: Candidate =
        serde_json::from_slice(&fs::read(output_root.join("minimized.candidate.json")).unwrap())
            .unwrap();
    let tape = InputTape::decode(&fs::read(output_root.join("minimized.tape")).unwrap())
        .unwrap()
        .tape;
    assert_eq!(minimized.compile().unwrap(), tape);
    assert_eq!(tape.frames.len(), 78);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn boot_timing_golfer_keeps_same_tick_move_that_unlocks_faster_pair() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("huntctl-boot-golf-{unique}"));
    fs::create_dir_all(&root).unwrap();
    let dvd = root.join("disc.iso");
    fs::write(&dvd, b"mock disc").unwrap();
    let source = Candidate {
        schema: CANDIDATE_SCHEMA.into(),
        segment: SegmentProfile::BootToFsp103,
        boot: huntctl::tape::TapeBoot::Process,
        actions: vec![
            MacroAction::Neutral { frames: 10 },
            MacroAction::Press {
                buttons: vec![ControllerButton::A],
                hold_frames: 1,
                neutral_frames: 9,
            },
            MacroAction::Press {
                buttons: vec![ControllerButton::Start],
                hold_frames: 1,
                neutral_frames: 99,
            },
        ],
        ancestry: Ancestry::default(),
    };
    let candidate_path = root.join("source.candidate.json");
    fs::write(&candidate_path, serde_json::to_vec_pretty(&source).unwrap()).unwrap();
    let rejected = run(&[
        "search",
        "golf-boot",
        "--candidate",
        candidate_path.to_str().unwrap(),
        "--game",
        env!("CARGO_BIN_EXE_huntctl"),
        "--dvd",
        dvd.to_str().unwrap(),
        "--output",
        root.join("single-sample").to_str().unwrap(),
        "--repetitions",
        "1",
    ]);
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("at least two repetitions"));
    let output_root = root.join("golfed");
    let output = run(&[
        "search",
        "golf-boot",
        "--candidate",
        candidate_path.to_str().unwrap(),
        "--game",
        env!("CARGO_BIN_EXE_huntctl"),
        "--game-arg",
        "mock-search-worker",
        "--game-arg",
        "--mock-mode",
        "--game-arg",
        "coordinate-golf",
        "--dvd",
        dvd.to_str().unwrap(),
        "--output",
        output_root.to_str().unwrap(),
        "--workers",
        "4",
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
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(summary["schema"], "dusklight-boot-timing-golf/v1");
    assert_eq!(summary["source_goal_sim_tick"], 100);
    assert_eq!(summary["goal_sim_tick"], 90);
    assert_eq!(
        summary["source_pulse_timestamps"],
        serde_json::json!([10, 20])
    );
    assert_eq!(
        summary["golfed_pulse_timestamps"],
        serde_json::json!([9, 19])
    );
    assert_eq!(summary["accepted_moves"], 2);
    assert_eq!(summary["goal_boundary_fingerprint"], "1".repeat(32));
    assert!(output_root.join("golfed.candidate.json").is_file());
    assert!(output_root.join("golfed.tape").is_file());
    assert!(output_root.join("proof.json").is_file());
    assert!(output_root.join("golf.summary.json").is_file());
    let golfed: Candidate =
        serde_json::from_slice(&fs::read(output_root.join("golfed.candidate.json")).unwrap())
            .unwrap();
    let tape = InputTape::decode(&fs::read(output_root.join("golfed.tape")).unwrap())
        .unwrap()
        .tape;
    assert_eq!(golfed.compile().unwrap(), tape);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn beam_search_spends_a_bounded_budget_on_native_rollouts() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("huntctl-beam-{unique}"));
    fs::create_dir_all(&root).unwrap();
    let dvd = root.join("disc.iso");
    fs::write(&dvd, b"mock disc").unwrap();
    let seed = Candidate {
        schema: CANDIDATE_SCHEMA.into(),
        segment: SegmentProfile::BootToFsp103,
        boot: huntctl::tape::TapeBoot::Process,
        actions: vec![MacroAction::Neutral { frames: 1 }],
        ancestry: Ancestry::default(),
    };
    let candidate_path = root.join("seed.json");
    fs::write(&candidate_path, serde_json::to_vec_pretty(&seed).unwrap()).unwrap();
    let options_path = root.join("options.json");
    fs::write(
        &options_path,
        serde_json::to_vec_pretty(&vec![
            MacroAction::Neutral { frames: 1 },
            MacroAction::Press {
                buttons: vec![ControllerButton::A],
                hold_frames: 1,
                neutral_frames: 0,
            },
        ])
        .unwrap(),
    )
    .unwrap();
    let output_root = root.join("beam");
    let output = run(&[
        "search",
        "beam",
        "--candidate",
        candidate_path.to_str().unwrap(),
        "--options",
        options_path.to_str().unwrap(),
        "--game",
        env!("CARGO_BIN_EXE_huntctl"),
        "--game-arg",
        "mock-search-worker",
        "--game-arg",
        "--mock-mode",
        "--game-arg",
        "miss",
        "--dvd",
        dvd.to_str().unwrap(),
        "--output",
        output_root.to_str().unwrap(),
        "--beam-width",
        "2",
        "--maximum-depth",
        "2",
        "--candidate-budget",
        "5",
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
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(summary["schema"], "dusklight-beam-search/v2");
    assert_eq!(summary["evaluated_candidates"], 5);
    assert_eq!(summary["simulator_episodes"], 10);
    assert_eq!(summary["depths_evaluated"], 3);
    assert_eq!(summary["q_prior_table_sha256"], serde_json::Value::Null);
    assert_eq!(summary["q_prior_ranked_children"], 0);
    assert_eq!(summary["native_rollout_ranking_authority"], true);
    assert_eq!(summary["policy_owns_route"], false);
    assert!(
        output_root
            .join("d000/evaluations/evaluation.json")
            .exists()
    );
    assert!(
        output_root
            .join("d001/evaluations/evaluation.json")
            .exists()
    );
    assert!(
        output_root
            .join("d002/evaluations/evaluation.json")
            .exists()
    );
    assert!(output_root.join("champion.candidate.json").exists());
    assert!(output_root.join("champion.tape").exists());
    assert!(output_root.join("beam.summary.json").exists());

    let bounded_root = root.join("beam-terminal-bound");
    let bounded = run(&[
        "search",
        "beam",
        "--candidate",
        candidate_path.to_str().unwrap(),
        "--options",
        options_path.to_str().unwrap(),
        "--game",
        env!("CARGO_BIN_EXE_huntctl"),
        "--game-arg",
        "mock-search-worker",
        "--dvd",
        dvd.to_str().unwrap(),
        "--output",
        bounded_root.to_str().unwrap(),
        "--beam-width",
        "2",
        "--maximum-depth",
        "2",
        "--candidate-budget",
        "5",
        "--repetitions",
        "2",
        "--timeout-ms",
        "2000",
    ]);
    assert!(
        bounded.status.success(),
        "{}",
        String::from_utf8_lossy(&bounded.stderr)
    );
    let bounded: serde_json::Value = serde_json::from_slice(&bounded.stdout).unwrap();
    assert_eq!(bounded["evaluated_candidates"], 1);
    assert_eq!(bounded["terminal_bound_pruned_children"], 2);
    assert_eq!(bounded["champion_score"]["goal_feasible"], true);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cem_and_cma_es_rank_typed_samples_only_after_native_rollout() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("huntctl-continuous-{unique}"));
    fs::create_dir_all(&root).unwrap();
    let dvd = root.join("disc.iso");
    fs::write(&dvd, b"mock disc").unwrap();
    let seed = Candidate {
        schema: CANDIDATE_SCHEMA.into(),
        segment: SegmentProfile::BootToFsp103,
        boot: huntctl::tape::TapeBoot::Process,
        actions: vec![MacroAction::Move {
            angle_degrees: 0,
            magnitude: 64,
            frames: 1,
        }],
        ancestry: Ancestry::default(),
    };
    let candidate_path = root.join("seed.json");
    fs::write(&candidate_path, serde_json::to_vec_pretty(&seed).unwrap()).unwrap();
    let axes_path = root.join("axes.json");
    fs::write(
        &axes_path,
        serde_json::to_vec_pretty(&ContinuousAxes {
            schema: CONTINUOUS_AXES_SCHEMA_V1.into(),
            axes: vec![
                ContinuousAxis {
                    name: "heading".into(),
                    action_index: 0,
                    parameter: ContinuousParameter::MoveHeadingDegrees,
                    minimum: -90.0,
                    maximum: 90.0,
                },
                ContinuousAxis {
                    name: "magnitude".into(),
                    action_index: 0,
                    parameter: ContinuousParameter::MoveMagnitude,
                    minimum: 1.0,
                    maximum: 127.0,
                },
            ],
        })
        .unwrap(),
    )
    .unwrap();

    for method in ["cem", "cma-es"] {
        let output_root = root.join(method);
        let output = Command::new(env!("CARGO_BIN_EXE_huntctl"))
            .args(["search", "continuous", "--method", method, "--candidate"])
            .arg(&candidate_path)
            .args(["--axes"])
            .arg(&axes_path)
            .args(["--game", env!("CARGO_BIN_EXE_huntctl")])
            .args(["--game-arg", "mock-search-worker"])
            .args(["--game-arg", "--mock-mode"])
            .args(["--game-arg", "miss"])
            .args(["--dvd"])
            .arg(&dvd)
            .args(["--output"])
            .arg(&output_root)
            .args([
                "--generations",
                "1",
                "--population",
                "12",
                "--elites",
                "3",
                "--candidate-budget",
                "12",
                "--rng-seed",
                "7",
                "--workers",
                "2",
                "--repetitions",
                "2",
                "--timeout-ms",
                "2000",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{method}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(summary["schema"], "dusklight-continuous-search/v1");
        assert_eq!(summary["generations_completed"], 1);
        let evaluated = summary["evaluated_candidates"].as_u64().unwrap();
        assert!(evaluated >= 3);
        assert_eq!(
            summary["simulator_episodes"].as_u64().unwrap(),
            evaluated * 2
        );
        assert!(
            output_root
                .join("g000/evaluations/evaluation.json")
                .exists()
        );
        assert!(output_root.join("g000/optimizer.json").exists());
        assert!(output_root.join("champion.candidate.json").exists());
        assert!(output_root.join("continuous.summary.json").exists());
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn bayesian_search_uses_native_rank_observations_and_persists_acquisition_state() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("huntctl-bayesian-{unique}"));
    fs::create_dir_all(&root).unwrap();
    let dvd = root.join("disc.iso");
    fs::write(&dvd, b"mock disc").unwrap();
    let seed = Candidate {
        schema: CANDIDATE_SCHEMA.into(),
        segment: SegmentProfile::BootToFsp103,
        boot: huntctl::tape::TapeBoot::Process,
        actions: vec![MacroAction::Move {
            angle_degrees: 0,
            magnitude: 64,
            frames: 1,
        }],
        ancestry: Ancestry::default(),
    };
    let candidate_path = root.join("seed.json");
    fs::write(&candidate_path, serde_json::to_vec_pretty(&seed).unwrap()).unwrap();
    let axes_path = root.join("axes.json");
    fs::write(
        &axes_path,
        serde_json::to_vec_pretty(&ContinuousAxes {
            schema: CONTINUOUS_AXES_SCHEMA_V1.into(),
            axes: vec![
                ContinuousAxis {
                    name: "heading".into(),
                    action_index: 0,
                    parameter: ContinuousParameter::MoveHeadingDegrees,
                    minimum: -90.0,
                    maximum: 90.0,
                },
                ContinuousAxis {
                    name: "magnitude".into(),
                    action_index: 0,
                    parameter: ContinuousParameter::MoveMagnitude,
                    minimum: 1.0,
                    maximum: 127.0,
                },
            ],
        })
        .unwrap(),
    )
    .unwrap();

    let output_root = root.join("search");
    let output = Command::new(env!("CARGO_BIN_EXE_huntctl"))
        .args(["search", "bayesian", "--candidate"])
        .arg(&candidate_path)
        .args(["--axes"])
        .arg(&axes_path)
        .args(["--game", env!("CARGO_BIN_EXE_huntctl")])
        .args(["--game-arg", "mock-search-worker"])
        .args(["--game-arg", "--mock-mode"])
        .args(["--game-arg", "miss"])
        .args(["--dvd"])
        .arg(&dvd)
        .args(["--output"])
        .arg(&output_root)
        .args([
            "--generations",
            "2",
            "--batch-size",
            "4",
            "--initial-samples",
            "4",
            "--acquisition-pool",
            "64",
            "--candidate-budget",
            "8",
            "--rng-seed",
            "7",
            "--workers",
            "2",
            "--repetitions",
            "2",
            "--timeout-ms",
            "2000",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(summary["schema"], "dusklight-bayesian-search/v1");
    assert_eq!(summary["generations_completed"], 2);
    let evaluated = summary["evaluated_candidates"].as_u64().unwrap();
    assert!(evaluated > 0);
    assert_eq!(
        summary["simulator_episodes"].as_u64().unwrap(),
        evaluated * 2
    );
    assert_eq!(summary["final_optimizer"]["observations"], evaluated);
    for generation in ["g000", "g001"] {
        assert!(
            output_root
                .join(generation)
                .join("evaluations/evaluation.json")
                .exists()
        );
        assert!(output_root.join(generation).join("optimizer.json").exists());
    }
    assert!(output_root.join("champion.candidate.json").exists());
    assert!(output_root.join("champion.tape").exists());
    assert!(output_root.join("bayesian.summary.json").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn proposer_tournament_enforces_equal_budgets_and_deduplicates_before_native_rollout() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("huntctl-tournament-{unique}"));
    fs::create_dir_all(&root).unwrap();
    let dvd = root.join("disc.iso");
    fs::write(&dvd, b"mock disc").unwrap();
    let incumbent = root.join("incumbent");
    let blind = root.join("blind");
    seed(&incumbent, "boot_to_fsp103", "3");
    let blind_output = run(&[
        "search",
        "seed",
        "--segment",
        "boot_to_fsp103",
        "--output",
        blind.to_str().unwrap(),
        "--size",
        "3",
        "--rng-seed",
        "11",
    ]);
    assert!(
        blind_output.status.success(),
        "{}",
        String::from_utf8_lossy(&blind_output.stderr)
    );
    let definition = root.join("tournament.json");
    fs::write(
        &definition,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": "dusklight-proposer-tournament-definition/v1",
            "budget_unit": "episodes",
            "budget_per_proposer": 4,
            "proposers": [
                {
                    "name": "incumbent",
                    "kind": "incumbent_mutation",
                    "population": "incumbent/manifest.json"
                },
                {
                    "name": "blind",
                    "kind": "blind_exploration",
                    "population": "blind/manifest.json"
                }
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    let output_root = root.join("results");
    let output = Command::new(env!("CARGO_BIN_EXE_huntctl"))
        .args(["search", "tournament", "--definition"])
        .arg(&definition)
        .args(["--game", env!("CARGO_BIN_EXE_huntctl")])
        .args(["--game-arg", "mock-search-worker"])
        .args(["--game-arg", "--mock-mode"])
        .args(["--game-arg", "miss"])
        .args(["--dvd"])
        .arg(&dvd)
        .args(["--output"])
        .arg(&output_root)
        .args([
            "--workers",
            "2",
            "--repetitions",
            "2",
            "--timeout-ms",
            "2000",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(summary["schema"], "dusklight-proposer-tournament/v1");
    assert_eq!(summary["rows"].as_array().unwrap().len(), 2);
    for row in summary["rows"].as_array().unwrap() {
        assert_eq!(row["selected_candidates"], 2);
        assert_eq!(row["charged_episodes"], 4);
        assert_eq!(row["predicate_hits"], 0);
        assert_eq!(row["misses"], 2);
    }
    let physical_candidates = summary["physical_candidates"].as_u64().unwrap();
    assert!(physical_candidates <= 4);
    assert_eq!(
        summary["physical_episodes"].as_u64().unwrap(),
        physical_candidates * 2
    );
    assert!(output_root.join("evaluations/evaluation.json").exists());
    assert!(output_root.join("leaderboard.json").exists());
    assert!(output_root.join("tournament.summary.json").exists());
    fs::remove_dir_all(root).unwrap();
}
