use huntctl::artifact::Digest;
use huntctl::learning::option_values::{
    OptionActionDescriptor, OptionValueBatch, OptionValueSample,
};
use huntctl::option_execution::OptionType;
use huntctl::reward_shaping::{
    POTENTIAL_SHAPING_SCHEMA_V1, PotentialShapingSpec, PotentialTerm, REWARD_REPORT_SCHEMA_V1,
};
use huntctl::transition_corpus::{
    MacroAction, StateReference, StateReferenceKind, Transition, TransitionCorpus,
};
use serde_json::json;
use std::collections::BTreeMap;
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

#[test]
fn episode_inspection_rejects_an_unsealed_artifact() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("huntctl-episode-inspect-{nonce}"));
    fs::create_dir_all(&root).unwrap();
    let input = root.join("episode.json");
    let zero = "0".repeat(64);
    fs::write(
        &input,
        serde_json::to_vec_pretty(&json!({
            "schema": "dusklight-immutable-episode/v1",
            "content_sha256": zero.clone(),
            "episode_sha256": zero.clone(),
            "objective": { "id": "objective", "digest": zero.clone() },
            "lineage": {
                "generation": 0
            },
            "terminal": "exhausted",
            "terminal_detail": "unsealed fixture",
            "realized_tape_sha256": zero.clone(),
            "gameplay_trace_sha256": zero.clone(),
            "transition_corpus_sha256": zero.clone(),
            "transition_evidence_sha256": zero,
            "steps": []
        }))
        .unwrap(),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_huntctl"))
        .args(["learn", "inspect-episode", "--input"])
        .arg(&input)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("immutable episode identity"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::remove_dir_all(root).unwrap();
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

fn option_sample(
    option_id: &str,
    option_type: OptionType,
    state: f32,
    duration_ticks: u32,
    reward: f32,
    tape_digest: u8,
) -> OptionValueSample {
    OptionValueSample {
        action: OptionActionDescriptor {
            option_id: option_id.into(),
            option_type,
            parameters: BTreeMap::new(),
        },
        state: vec![state],
        duration_ticks,
        reward,
        next_state: vec![state + 1.0],
        terminal: true,
        realized_tape_sha256: Digest([tape_digest; 32]),
    }
}

#[test]
fn option_value_cli_ranks_authenticated_realized_options() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("huntctl-option-values-{nonce}"));
    fs::create_dir_all(&root).unwrap();
    let input = root.join("options.json");
    let model_path = root.join("option-model.json");
    let batch = OptionValueBatch::new(
        Digest([0x31; 32]),
        Digest([0x32; 32]),
        1,
        vec![
            option_sample("wait", OptionType::Neutral, 0.0, 4, -1.0, 1),
            option_sample("roll_forward", OptionType::Roll, 0.0, 12, 6.0, 2),
            option_sample("wait", OptionType::Neutral, 1.0, 4, -1.0, 3),
            option_sample("roll_forward", OptionType::Roll, 1.0, 12, 6.0, 4),
        ],
        vec![1, 2, 3, 4],
    )
    .unwrap();
    fs::write(&input, serde_json::to_vec(&batch).unwrap()).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_huntctl"))
        .args(["learn", "option-values", "--input"])
        .arg(&input)
        .arg("--model-output")
        .arg(&model_path)
        .args(["--iterations", "12", "--trees", "7", "--seed", "7"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schema"], "dusklight-option-value-ranking/v1");
    assert_eq!(
        report["ranking"][0]["descriptor"]["option_id"],
        "roll_forward"
    );
    assert_eq!(
        report["control_hierarchy"],
        "option_value_then_deterministic_realization"
    );
    assert_eq!(report["raw_frame_policy"], "last_mile_tape_golf_only");
    assert_eq!(report["promotion_authority"], false);

    let model: serde_json::Value = serde_json::from_slice(&fs::read(&model_path).unwrap()).unwrap();
    assert_eq!(model["schema"], "dusklight-option-value-model/v1");
    assert_eq!(
        model["model"]["raw_frame_policy"],
        "last_mile_tape_golf_only"
    );
    assert!(model["model"].get("emitted_raw_actions").is_none());
    assert_eq!(
        model["model"]["realized_tape_sha256"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    let model_blob = &report["model_content_blob"];
    let stored_model = root
        .join("content")
        .join(model_blob["relative_path"].as_str().unwrap());
    assert_eq!(
        fs::read(&model_path).unwrap(),
        fs::read(stored_model).unwrap()
    );

    fs::remove_dir_all(root).unwrap();
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

    let double_q_path = root.join("double-q-model.json");
    let double_q = Command::new(executable)
        .args(["learn", "double-q", "--input"])
        .arg(&path)
        .arg("--model-output")
        .arg(&double_q_path)
        .args([
            "--query-transition",
            "0",
            "--epochs",
            "256",
            "--hidden-width",
            "8",
            "--learning-rate",
            "0.01",
            "--target-sync-steps",
            "3",
            "--seed",
            "7",
        ])
        .output()
        .unwrap();
    assert!(
        double_q.status.success(),
        "{}",
        String::from_utf8_lossy(&double_q.stderr)
    );
    let double_q: serde_json::Value = serde_json::from_slice(&double_q.stdout).unwrap();
    assert_eq!(double_q["schema"], "dusklight-double-q-ranking/v1");
    assert_eq!(double_q["ranking"][0]["action"], ADVANCE);
    assert_eq!(double_q["ranking"][0]["support"], 2);
    assert_eq!(double_q["gradient_updates"], 1024);
    assert_eq!(double_q["target_synchronizations"], 341);
    assert_eq!(double_q["promotion_authority"], false);
    let double_q_model: serde_json::Value =
        serde_json::from_slice(&fs::read(&double_q_path).unwrap()).unwrap();
    assert_eq!(double_q_model["schema"], "dusklight-double-q-model/v1");
    assert_eq!(double_q_model["config"]["target_sync_steps"], 3);
    let double_q_blob = &double_q["model_content_blob"];
    assert_eq!(double_q_blob["kind"], "model");
    assert_eq!(
        fs::read(&double_q_path).unwrap(),
        fs::read(
            root.join("content")
                .join(double_q_blob["relative_path"].as_str().unwrap())
        )
        .unwrap()
    );

    let cql_path = root.join("cql-model.json");
    let cql = Command::new(executable)
        .args(["learn", "cql", "--input"])
        .arg(&path)
        .arg("--model-output")
        .arg(&cql_path)
        .args([
            "--query-transition",
            "0",
            "--epochs",
            "128",
            "--hidden-width",
            "8",
            "--learning-rate",
            "0.01",
            "--target-sync-steps",
            "3",
            "--conservative-weight",
            "0.5",
            "--temperature",
            "1",
            "--seed",
            "7",
        ])
        .output()
        .unwrap();
    assert!(
        cql.status.success(),
        "{}",
        String::from_utf8_lossy(&cql.stderr)
    );
    let cql: serde_json::Value = serde_json::from_slice(&cql.stdout).unwrap();
    assert_eq!(cql["schema"], "dusklight-conservative-q-ranking/v1");
    assert_eq!(cql["ranking"][0]["action"], ADVANCE);
    assert_eq!(cql["conservative_updates"], 512);
    assert_eq!(cql["gradient_updates"], 512);
    assert_eq!(cql["promotion_authority"], false);
    assert!(cql["mean_conservative_gap"].as_f64().unwrap().is_finite());
    let cql_model: serde_json::Value =
        serde_json::from_slice(&fs::read(&cql_path).unwrap()).unwrap();
    assert_eq!(cql_model["schema"], "dusklight-conservative-q-model/v1");
    assert_eq!(cql_model["config"]["conservative_weight"], 0.5);
    let cql_blob = &cql["model_content_blob"];
    assert_eq!(cql_blob["kind"], "model");
    assert_eq!(
        fs::read(&cql_path).unwrap(),
        fs::read(
            root.join("content")
                .join(cql_blob["relative_path"].as_str().unwrap())
        )
        .unwrap()
    );

    let iql_path = root.join("iql-model.json");
    let iql = Command::new(executable)
        .args(["learn", "iql", "--input"])
        .arg(&path)
        .arg("--model-output")
        .arg(&iql_path)
        .args([
            "--query-transition",
            "0",
            "--epochs",
            "128",
            "--hidden-width",
            "8",
            "--learning-rate",
            "0.01",
            "--target-sync-steps",
            "16",
            "--expectile",
            "0.7",
            "--advantage-beta",
            "3",
            "--max-advantage-weight",
            "20",
            "--seed",
            "7",
        ])
        .output()
        .unwrap();
    assert!(
        iql.status.success(),
        "{}",
        String::from_utf8_lossy(&iql.stderr)
    );
    let iql: serde_json::Value = serde_json::from_slice(&iql.stdout).unwrap();
    assert_eq!(iql["schema"], "dusklight-discrete-iql-ranking/v1");
    assert_eq!(iql["ranking"][0]["action"], ADVANCE);
    assert_eq!(iql["gradient_updates"], 512);
    assert_eq!(iql["target_synchronizations"], 32);
    assert_eq!(iql["promotion_authority"], false);
    assert!(iql["mean_advantage_weight"].as_f64().unwrap().is_finite());
    let probability_sum = iql["ranking"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["policy_probability"].as_f64().unwrap())
        .sum::<f64>();
    assert!((probability_sum - 1.0).abs() < 1e-9);
    let iql_model: serde_json::Value =
        serde_json::from_slice(&fs::read(&iql_path).unwrap()).unwrap();
    assert_eq!(iql_model["schema"], "dusklight-discrete-iql-model/v1");
    assert_eq!(iql_model["config"]["expectile"], 0.7);
    let iql_blob = &iql["model_content_blob"];
    assert_eq!(iql_blob["kind"], "model");
    assert_eq!(
        fs::read(&iql_path).unwrap(),
        fs::read(
            root.join("content")
                .join(iql_blob["relative_path"].as_str().unwrap())
        )
        .unwrap()
    );

    let ensemble_path = root.join("ensemble-q-model.json");
    let ensemble = Command::new(executable)
        .args(["learn", "ensemble-q", "--input"])
        .arg(&path)
        .arg("--model-output")
        .arg(&ensemble_path)
        .args([
            "--query-transition",
            "0",
            "--members",
            "3",
            "--epochs",
            "16",
            "--hidden-width",
            "4",
            "--learning-rate",
            "0.01",
            "--target-sync-steps",
            "8",
            "--seed",
            "7",
        ])
        .output()
        .unwrap();
    assert!(
        ensemble.status.success(),
        "{}",
        String::from_utf8_lossy(&ensemble.stderr)
    );
    let ensemble: serde_json::Value = serde_json::from_slice(&ensemble.stdout).unwrap();
    assert_eq!(ensemble["schema"], "dusklight-bootstrapped-q-ranking/v1");
    assert_eq!(ensemble["members"], 3);
    assert_eq!(
        ensemble["member_bootstrap_episode_groups"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(ensemble["promotion_authority"], false);
    assert!(
        ensemble["ranking"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["member_variance"].as_f64().unwrap() >= 0.0)
    );
    let ensemble_model: serde_json::Value =
        serde_json::from_slice(&fs::read(&ensemble_path).unwrap()).unwrap();
    assert_eq!(
        ensemble_model["schema"],
        "dusklight-bootstrapped-q-ensemble/v1"
    );
    let ensemble_blob = &ensemble["model_content_blob"];
    assert_eq!(ensemble_blob["kind"], "model");
    assert_eq!(
        fs::read(&ensemble_path).unwrap(),
        fs::read(
            root.join("content")
                .join(ensemble_blob["relative_path"].as_str().unwrap())
        )
        .unwrap()
    );

    let prioritized_path = root.join("prioritized-q-model.json");
    let prioritized = Command::new(executable)
        .args(["learn", "prioritized-q", "--input"])
        .arg(&path)
        .arg("--model-output")
        .arg(&prioritized_path)
        .args([
            "--query-transition",
            "0",
            "--epochs",
            "32",
            "--hidden-width",
            "4",
            "--learning-rate",
            "0.01",
            "--target-sync-steps",
            "8",
            "--priority-alpha",
            "0.6",
            "--importance-beta-start",
            "0.4",
            "--importance-beta-end",
            "1.0",
            "--importance-weight-cap",
            "0.75",
            "--seed",
            "7",
            "--replay-seed",
            "11",
        ])
        .output()
        .unwrap();
    assert!(
        prioritized.status.success(),
        "{}",
        String::from_utf8_lossy(&prioritized.stderr)
    );
    let prioritized: serde_json::Value = serde_json::from_slice(&prioritized.stdout).unwrap();
    assert_eq!(
        prioritized["schema"],
        "dusklight-prioritized-double-q-ranking/v1"
    );
    assert_eq!(prioritized["replay_diagnostics"]["total_samples"], 128);
    assert!(
        prioritized["replay_diagnostics"]["unique_rows_sampled"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(
        prioritized["replay_diagnostics"]["maximum_importance_weight"]
            .as_f64()
            .unwrap()
            <= 0.75
    );
    assert_eq!(
        prioritized["row_sample_counts"].as_array().unwrap().len(),
        4
    );
    assert_eq!(prioritized["promotion_authority"], false);
    let prioritized_model: serde_json::Value =
        serde_json::from_slice(&fs::read(&prioritized_path).unwrap()).unwrap();
    assert_eq!(
        prioritized_model["schema"],
        "dusklight-prioritized-double-q-model/v1"
    );
    let prioritized_blob = &prioritized["model_content_blob"];
    assert_eq!(prioritized_blob["kind"], "model");
    assert_eq!(
        fs::read(&prioritized_path).unwrap(),
        fs::read(
            root.join("content")
                .join(prioritized_blob["relative_path"].as_str().unwrap())
        )
        .unwrap()
    );

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

#[test]
fn q_component_ablation_cli_runs_only_one_equal_budget_treatment() {
    const WAIT: u32 = 0;
    const ADVANCE: u32 = 1;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("huntctl-ablate-q-{nonce}"));
    fs::create_dir_all(&root).unwrap();
    let training_path = root.join("training.dtcz");
    let held_out_path = root.join("held-out.dtcz");
    TransitionCorpus::new(
        Digest([0x31; 32]),
        Digest([0x32; 32]),
        1,
        vec![
            transition(0.0, WAIT, -1.0, 1.0, true),
            transition(0.0, ADVANCE, 3.0, 1.0, true),
            transition(1.0, WAIT, -1.0, 2.0, true),
            transition(1.0, ADVANCE, 3.0, 2.0, true),
        ],
    )
    .unwrap()
    .write_zstd_file(&training_path, 1)
    .unwrap();
    TransitionCorpus::new(
        Digest([0x31; 32]),
        Digest([0x32; 32]),
        1,
        vec![
            transition(0.0, WAIT, -0.5, 1.0, true),
            transition(0.0, ADVANCE, 2.5, 1.0, true),
            transition(1.0, WAIT, -0.5, 2.0, true),
            transition(1.0, ADVANCE, 2.5, 2.0, true),
            transition(2.0, 77, 9.0, 3.0, true),
        ],
    )
    .unwrap()
    .write_zstd_file(&held_out_path, 1)
    .unwrap();

    for (component, serialized_component) in [
        ("dueling-heads", "dueling_heads"),
        ("n-step", "n_step_returns"),
        ("distributional-values", "distributional_values"),
        ("noisy-exploration", "noisy_exploration"),
    ] {
        let output_path = root.join(format!("{component}.json"));
        let output = Command::new(env!("CARGO_BIN_EXE_huntctl"))
            .args(["learn", "ablate-q", "--component", component, "--training"])
            .arg(&training_path)
            .arg("--held-out")
            .arg(&held_out_path)
            .arg("--output")
            .arg(&output_path)
            .args([
                "--epochs",
                "8",
                "--hidden-width",
                "4",
                "--learning-rate",
                "0.01",
                "--target-sync-steps",
                "4",
                "--seed",
                "7",
                "--n-step",
                "2",
                "--distribution-atoms",
                "11",
                "--distribution-min",
                "-5",
                "--distribution-max",
                "5",
                "--noisy-stddev",
                "0.25",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{component}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["schema"], "dusklight-q-component-ablation-run/v1");
        assert_eq!(
            report["ablation"]["evaluation"]["component"],
            serialized_component
        );
        assert_eq!(
            report["ablation"]["evaluation"]["baseline_gradient_updates"],
            32
        );
        assert_eq!(
            report["ablation"]["evaluation"]["treatment_gradient_updates"],
            32
        );
        assert_eq!(
            report["ablation"]["evaluation"]["equal_gradient_update_budget"],
            true
        );
        assert_eq!(
            report["ablation"]["evaluation"]["baseline"]["supported_logged_actions"],
            4
        );
        assert_eq!(
            report["ablation"]["evaluation"]["baseline"]["unsupported_logged_actions"],
            1
        );
        assert_eq!(
            report["ablation"]["evaluation"]["baseline"]["observed_return_calibration"]["held_out_samples"],
            5
        );
        assert_eq!(
            report["ablation"]["evaluation"]["baseline"]["observed_return_calibration"]["unsupported_observed_action_samples"],
            1
        );
        assert_eq!(report["ablation"]["combined_rainbow_configuration"], false);
        assert_eq!(report["ablation"]["promotion_authority"], false);
        if component == "noisy-exploration" {
            assert_eq!(
                report["ablation"]["evaluation"]["treatment_noise_resamples"],
                128
            );
        } else {
            assert!(report["ablation"]["evaluation"]["treatment_noise_resamples"].is_null());
        }
        let stored: serde_json::Value =
            serde_json::from_slice(&fs::read(&output_path).unwrap()).unwrap();
        assert_eq!(stored, report);
    }

    let overlap = Command::new(env!("CARGO_BIN_EXE_huntctl"))
        .args([
            "learn",
            "ablate-q",
            "--component",
            "dueling-heads",
            "--training",
        ])
        .arg(&training_path)
        .arg("--held-out")
        .arg(&training_path)
        .arg("--output")
        .arg(root.join("overlap.json"))
        .output()
        .unwrap();
    assert!(!overlap.status.success());
    assert!(String::from_utf8_lossy(&overlap.stderr).contains("files overlap"));

    fs::remove_dir_all(root).unwrap();
}
