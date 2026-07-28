//! Group-isolated generalized tactic-value calibration command.

use crate::{option, required_path, usize_option};
use huntctl::learning::generalized_tactic_calibration::{
    GeneralizedTacticCalibrationConfig, calibrate_generalized_tactic_value,
};
use huntctl::learning::tactic_features::GoalConditionedTacticFeatureEncoder;
use huntctl::search_evaluator::tactic_q_campaign::{TacticQCampaign, TacticQTrainingCorpus};
use serde_json::json;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

pub(super) fn command(args: &[String]) -> Result<(), Box<dyn Error>> {
    let learn_args = &args[1..];
    let checkpoint_path = option(learn_args, "--checkpoint").map(PathBuf::from);
    let corpus_path = option(learn_args, "--input").map(PathBuf::from);
    if checkpoint_path.is_some() == corpus_path.is_some() {
        return Err(
            "generalized tactic calibration requires exactly one of --checkpoint or --input".into(),
        );
    }
    let output = required_path(learn_args, "--output")?;
    if output.exists() {
        return Err(format!(
            "generalized tactic calibration output already exists: {}",
            output.display()
        )
        .into());
    }

    let defaults = GeneralizedTacticCalibrationConfig::default();
    let config = GeneralizedTacticCalibrationConfig {
        state_region_width: option(learn_args, "--state-region-width")
            .map(|value| value.parse::<f32>())
            .transpose()?
            .unwrap_or(defaults.state_region_width),
        group_folds: u8::try_from(usize_option(
            learn_args,
            "--group-folds",
            usize::from(defaults.group_folds),
        )?)
        .map_err(|_| "--group-folds exceeds u8")?,
        validation_fold: u8::try_from(usize_option(
            learn_args,
            "--validation-fold",
            usize::from(defaults.validation_fold),
        )?)
        .map_err(|_| "--validation-fold exceeds u8")?,
        test_fold: u8::try_from(usize_option(
            learn_args,
            "--test-fold",
            usize::from(defaults.test_fold),
        )?)
        .map_err(|_| "--test-fold exceeds u8")?,
        interval_coverage_millionths: u32::try_from(usize_option(
            learn_args,
            "--interval-coverage-millionths",
            defaults.interval_coverage_millionths as usize,
        )?)
        .map_err(|_| "--interval-coverage-millionths exceeds u32")?,
        fitted_q_iterations: u16::try_from(usize_option(
            learn_args,
            "--fitted-q-iterations",
            usize::from(defaults.fitted_q_iterations),
        )?)
        .map_err(|_| "--fitted-q-iterations exceeds u16")?,
        per_tick_discount: option(learn_args, "--per-tick-discount")
            .map(|value| value.parse::<f32>())
            .transpose()?
            .unwrap_or(defaults.per_tick_discount),
    };
    config.validate()?;

    let (transitions, replay_source, source_sha256) =
        if let Some(checkpoint_path) = &checkpoint_path {
            let checkpoint = TacticQCampaign::read_checkpoint_payload(checkpoint_path)?;
            let (transitions, replay_source) = if checkpoint.training_replay.is_empty() {
                (checkpoint.replay, "episode_replay")
            } else {
                (checkpoint.training_replay, "training_replay")
            };
            (transitions, replay_source, checkpoint.content_sha256)
        } else if let Some(corpus_path) = &corpus_path {
            let corpus = TacticQTrainingCorpus::read(corpus_path)?;
            (
                corpus.transitions,
                "training_corpus",
                corpus.root_checkpoint_sha256,
            )
        } else {
            unreachable!("exactly one tactic calibration source was checked")
        };
    let default_goal_distance_feature =
        GoalConditionedTacticFeatureEncoder::new([0.0; 3])?.goal_distance_feature();
    let report = calibrate_generalized_tactic_value(
        &transitions,
        usize_option(
            learn_args,
            "--goal-distance-feature",
            default_goal_distance_feature,
        )?,
        config,
    )?;

    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let mut bytes = serde_json::to_vec_pretty(&report)?;
    bytes.push(b'\n');
    fs::write(&output, bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": report.schema,
            "output": output,
            "checkpoint": checkpoint_path,
            "input": corpus_path,
            "source_sha256": source_sha256,
            "replay_source": replay_source,
            "source_transitions": report.source_transitions,
            "state_region_test": report.state_region.test,
            "action_realization_test": report.action_realization.test,
            "state_region_test_coverage_at_least_nominal":
                report.state_region.test_coverage_at_least_nominal,
            "action_realization_test_coverage_at_least_nominal":
                report.action_realization.test_coverage_at_least_nominal,
            "report_sha256": report.report_sha256,
        }))?
    );
    Ok(())
}
