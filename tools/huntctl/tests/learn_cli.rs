use huntctl::artifact::Digest;
use huntctl::reward_shaping::{
    POTENTIAL_SHAPING_SCHEMA_V1, PotentialShapingSpec, PotentialTerm, REWARD_REPORT_SCHEMA_V1,
};
use huntctl::transition_corpus::{
    MacroAction, StateReference, StateReferenceKind, Transition, TransitionCorpus,
};
use serde_json::json;
use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn trace_extraction_requires_complete_episode_context() {
    let output = Command::new(env!("CARGO_BIN_EXE_huntctl"))
        .args([
            "learn",
            "extract-trace",
            "--trace",
            "missing.trace",
            "--tape",
            "missing.tape",
            "--start-frame",
            "0",
            "--end-frame",
            "1",
            "--output",
            "unused.dtcz",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("missing required --episode-context <path>")
    );
}

fn state(value: f32) -> Vec<f32> {
    vec![value]
}

fn reference(byte: u8) -> StateReference {
    StateReference {
        kind: StateReferenceKind::Boundary,
        digest: Digest([byte; 32]),
    }
}

fn transition(from: f32, action_id: u32, reward: f32, to: f32, terminal: bool) -> Transition {
    Transition {
        source: reference(from as u8 + 1),
        state: state(from),
        action: MacroAction {
            action_id,
            macro_kind: 1,
            parameters: Vec::new(),
        },
        duration_ticks: 1,
        reward,
        next: reference(to as u8 + 1),
        next_state: state(to),
        terminal,
    }
}

#[test]
fn native_learning_cli_inspects_and_ranks_a_compact_batch() {
    const ADVANCE: u32 = 3;
    const WAIT: u32 = 9;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("huntctl-learn-{nonce}"));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("shortest-path.dtcz");
    let corpus = TransitionCorpus::new(
        Digest([0x11; 32]),
        Digest([0x22; 32]),
        1,
        vec![
            transition(0.0, ADVANCE, 0.0, 1.0, false),
            transition(0.0, WAIT, -1.0, 0.0, false),
            transition(1.0, ADVANCE, 10.0, 2.0, true),
            transition(1.0, WAIT, -1.0, 1.0, false),
        ],
    )
    .unwrap();
    corpus.write_zstd_file(&path, 1).unwrap();

    let executable = env!("CARGO_BIN_EXE_huntctl");
    let inspect = Command::new(executable)
        .args(["learn", "inspect", "--input"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        inspect.status.success(),
        "{}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let inspect: serde_json::Value = serde_json::from_slice(&inspect.stdout).unwrap();
    assert_eq!(inspect["transitions"], 4);
    assert_eq!(inspect["action_counts"][ADVANCE.to_string()], 2);

    let undeclared_feature_kinds = Command::new(executable)
        .args(["learn", "fit", "--input"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(!undeclared_feature_kinds.status.success());
    assert!(
        String::from_utf8_lossy(&undeclared_feature_kinds.stderr)
            .contains("declare --all-continuous or repeat --categorical-feature")
    );

    let fit = Command::new(executable)
        .args(["learn", "fit", "--input"])
        .arg(&path)
        .arg("--model-output")
        .arg(root.join("model.json"))
        .args([
            "--query-transition",
            "0",
            "--all-continuous",
            "--iterations",
            "16",
            "--trees",
            "31",
            "--seed",
            "7",
        ])
        .output()
        .unwrap();
    assert!(
        fit.status.success(),
        "{}",
        String::from_utf8_lossy(&fit.stderr)
    );
    let fit: serde_json::Value = serde_json::from_slice(&fit.stdout).unwrap();
    assert_eq!(fit["transition_count"], 4);
    assert_eq!(fit["backup_steps"], 1);
    assert_eq!(fit["categorical_features"], json!([]));
    assert_eq!(fit["ranking"][0]["action"], ADVANCE);
    assert_eq!(fit["ranking"][0]["support"], 2);
    assert_eq!(fit["bootstrap_unit"], "episode");
    let model_blob = &fit["model_content_blob"];
    assert_eq!(model_blob["kind"], "model");
    let stored_model = root
        .join("content")
        .join(model_blob["relative_path"].as_str().unwrap());
    assert_eq!(
        fs::read(root.join("model.json")).unwrap(),
        fs::read(stored_model).unwrap()
    );
    let model: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("model.json")).unwrap()).unwrap();
    assert_eq!(model["schema"], "dusklight-fitted-q-model/v2");
    assert_eq!(model["model"]["bootstrap_unit"], "episode");

    let held_out_path = root.join("held-out.dtcz");
    TransitionCorpus::new(
        Digest([0x11; 32]),
        Digest([0x22; 32]),
        1,
        vec![
            transition(0.0, ADVANCE, 4.0, 1.0, true),
            transition(0.0, WAIT, 0.0, 1.0, true),
            // Action 77 is intentionally outside training support.
            transition(2.0, 77, 9.0, 3.0, true),
        ],
    )
    .unwrap()
    .write_zstd_file(&held_out_path, 1)
    .unwrap();
    let calibration_path = root.join("calibration.json");
    let calibration = Command::new(executable)
        .args(["learn", "calibrate", "--training"])
        .arg(&path)
        .args(["--held-out"])
        .arg(&held_out_path)
        .args(["--output"])
        .arg(&calibration_path)
        .args([
            "--all-continuous",
            "--iterations",
            "4",
            "--trees",
            "3",
            "--seed",
            "7",
        ])
        .output()
        .unwrap();
    assert!(
        calibration.status.success(),
        "{}",
        String::from_utf8_lossy(&calibration.stderr)
    );
    let calibration: serde_json::Value = serde_json::from_slice(&calibration.stdout).unwrap();
    assert_eq!(
        calibration["schema"],
        "dusklight-held-out-fqi-calibration/v1"
    );
    assert_eq!(calibration["calibration"]["held_out_samples"], 3);
    assert_eq!(
        calibration["calibration"]["unsupported_observed_action_samples"],
        1
    );
    assert_eq!(calibration["calibration"]["proposal_comparable_states"], 1);
    assert_eq!(calibration["calibration"]["proposal_wins"], 1);
    assert_eq!(
        fs::read(&calibration_path).unwrap(),
        serde_json::to_vec_pretty(&calibration).unwrap()
    );

    let overlapping_calibration = Command::new(executable)
        .args(["learn", "calibrate", "--training"])
        .arg(&path)
        .args(["--held-out"])
        .arg(&path)
        .args(["--output"])
        .arg(root.join("overlap.json"))
        .arg("--all-continuous")
        .output()
        .unwrap();
    assert!(!overlapping_calibration.status.success());
    assert!(String::from_utf8_lossy(&overlapping_calibration.stderr).contains("files overlap"));

    let n_step_fit = Command::new(executable)
        .args(["learn", "fit", "--input"])
        .arg(&path)
        .args([
            "--all-continuous",
            "--iterations",
            "1",
            "--n-step",
            "2",
            "--trees",
            "1",
        ])
        .output()
        .unwrap();
    assert!(
        n_step_fit.status.success(),
        "{}",
        String::from_utf8_lossy(&n_step_fit.stderr)
    );
    let n_step_fit: serde_json::Value = serde_json::from_slice(&n_step_fit.stdout).unwrap();
    assert_eq!(n_step_fit["backup_steps"], 2);

    for arguments in [
        vec![
            "--method",
            "nearest-neighbor",
            "--neighbors",
            "2",
            "--feature",
            "0:1:continuous",
        ],
        vec!["--method", "tabular", "--axis", "0:0:1"],
    ] {
        let baseline = Command::new(executable)
            .args(["learn", "baseline", "--input"])
            .arg(&path)
            .args(arguments)
            .output()
            .unwrap();
        assert!(
            baseline.status.success(),
            "{}",
            String::from_utf8_lossy(&baseline.stderr)
        );
        let baseline: serde_json::Value = serde_json::from_slice(&baseline.stdout).unwrap();
        assert_eq!(baseline["schema"], "dusklight-low-data-baseline/v1");
        assert!(!baseline["ranking"].as_array().unwrap().is_empty());
        assert_eq!(baseline["episode_groups"], 2);
    }

    let shaping_path = root.join("shaping.json");
    let shaping_report_path = root.join("reward-components.json");
    let shaping = PotentialShapingSpec {
        schema: POTENTIAL_SHAPING_SCHEMA_V1.into(),
        feature_schema: Digest([0x11; 32]),
        terms: vec![PotentialTerm::CorridorProgress {
            name: "shortest-path".into(),
            feature: 0,
            start: 0.0,
            end: 2.0,
            weight: 1.0,
            unavailable_value: None,
        }],
    };
    fs::write(&shaping_path, serde_json::to_vec_pretty(&shaping).unwrap()).unwrap();

    let missing_report = Command::new(executable)
        .args(["learn", "fit", "--input"])
        .arg(&path)
        .arg("--shaping")
        .arg(&shaping_path)
        .arg("--all-continuous")
        .output()
        .unwrap();
    assert!(!missing_report.status.success());
    assert!(
        String::from_utf8_lossy(&missing_report.stderr)
            .contains("--shaping SPEC.json and --shaping-report REPORT.json")
    );

    let shaped_fit = Command::new(executable)
        .args(["learn", "fit", "--input"])
        .arg(&path)
        .arg("--shaping")
        .arg(&shaping_path)
        .arg("--shaping-report")
        .arg(&shaping_report_path)
        .args([
            "--discount",
            "0.9",
            "--all-continuous",
            "--iterations",
            "4",
            "--trees",
            "3",
        ])
        .output()
        .unwrap();
    assert!(
        shaped_fit.status.success(),
        "{}",
        String::from_utf8_lossy(&shaped_fit.stderr)
    );
    let shaped_fit: serde_json::Value = serde_json::from_slice(&shaped_fit.stdout).unwrap();
    assert!((shaped_fit["per_tick_discount"].as_f64().unwrap() - 0.9).abs() < 1.0e-6);
    assert!(shaped_fit["potential_shaping"].is_string());
    assert_eq!(
        shaped_fit["reward_report"],
        shaping_report_path.to_string_lossy().as_ref()
    );
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(&shaping_report_path).unwrap()).unwrap();
    assert_eq!(report["schema"], REWARD_REPORT_SCHEMA_V1);
    assert_eq!(report["proposal_signal_only"], true);
    assert_eq!(report["terminal_objective"], "unchanged_external_predicate");
    assert_eq!(report["transitions"].as_array().unwrap().len(), 4);
    assert_eq!(
        report["transitions"][0]["reward"]["components"][0]["name"],
        "shortest-path"
    );
    assert_eq!(
        report["transitions"][0]["reward"]["components"][0]["source_fact"],
        0.0
    );
    assert_eq!(
        report["transitions"][0]["reward"]["components"][0]["next_fact"],
        1.0
    );
    assert_eq!(
        report["transitions"][2]["reward"]["effective_next_potential"],
        0.0
    );

    let oversized_config = Command::new(executable)
        .args(["learn", "fit", "--input"])
        .arg(&path)
        .args(["--iterations", "129"])
        .output()
        .unwrap();
    assert!(!oversized_config.status.success());
    assert!(
        String::from_utf8_lossy(&oversized_config.stderr)
            .contains("iterations must not exceed 128")
    );

    let oversized_backup = Command::new(executable)
        .args(["learn", "fit", "--input"])
        .arg(&path)
        .args(["--n-step", "65"])
        .output()
        .unwrap();
    assert!(!oversized_backup.status.success());
    assert!(String::from_utf8_lossy(&oversized_backup.stderr).contains("within 1..=64"));

    let mut too_many_inputs = Command::new(executable);
    too_many_inputs.args(["learn", "fit"]);
    for _ in 0..65 {
        too_many_inputs.arg("--input").arg(&path);
    }
    let too_many_inputs = too_many_inputs.output().unwrap();
    assert!(!too_many_inputs.status.success());
    assert!(
        String::from_utf8_lossy(&too_many_inputs.stderr)
            .contains("at most 64 input corpora; received 65")
    );

    let many_actions_path = root.join("too-many-actions.dtcz");
    let many_actions = TransitionCorpus::new(
        Digest([0x11; 32]),
        Digest([0x22; 32]),
        1,
        (0..129)
            .map(|action| transition(0.0, action, 0.0, 0.0, true))
            .collect(),
    )
    .unwrap();
    many_actions.write_zstd_file(&many_actions_path, 1).unwrap();
    let too_many_actions = Command::new(executable)
        .args(["learn", "fit", "--input"])
        .arg(&many_actions_path)
        .output()
        .unwrap();
    assert!(!too_many_actions.status.success());
    assert!(
        String::from_utf8_lossy(&too_many_actions.stderr).contains("at most 128 distinct actions")
    );

    let benchmark = Command::new(executable)
        .args(["learn", "benchmark"])
        .output()
        .unwrap();
    assert!(benchmark.status.success());
    let benchmark: serde_json::Value = serde_json::from_slice(&benchmark.stdout).unwrap();
    assert_eq!(benchmark["passed"], true);

    fs::remove_dir_all(root).unwrap();
}
