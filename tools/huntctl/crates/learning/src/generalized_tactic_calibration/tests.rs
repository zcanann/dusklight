use super::*;
use crate::fact_snapshot::{FactSnapshot, FactTerminalReason};
use crate::tactic_value_treatment::{ContinuousTacticDoubleQModel, ContinuousTacticValueModel};
use crate::tape::{InputFrame, InputTape};
use dusklight_control::option_execution::{
    OptionCondition, OptionEndReason, OptionExecution, OptionParameter, OptionType, TapeRange,
};
use dusklight_evidence::native_episode_shard::NativeEpisodeShard;
use std::collections::BTreeMap;

fn fact_pair() -> (FactSnapshot, FactSnapshot) {
    let shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v28.dseps"
    ))
    .unwrap();
    let step = &shard.episodes[0].steps[0];
    let mut before = FactSnapshot::from_native_learning(&step.pre_input, &[], None, Vec::new())
        .expect("fixture facts");
    let mut after = FactSnapshot::from_native_learning(
        &step.post_simulation,
        &[step.pre_input.clone()],
        None,
        Vec::new(),
    )
    .expect("fixture facts");
    before.terminal.configured = Some(true);
    before.terminal.reached = Some(false);
    before.terminal.reason = FactTerminalReason::None;
    after.world = before.world.clone();
    after.player.present = before.player.present;
    after.player.is_link = before.player.is_link;
    after.player.procedure = before.player.procedure;
    after.player.mode_flags = before.player.mode_flags;
    after.player.action_lanes = before.player.action_lanes.clone();
    after.player.action_state = before.player.action_state;
    after.event = before.event.clone();
    after.channels.player_action = before.channels.player_action;
    after.terminal.configured = Some(true);
    after.terminal.reached = Some(false);
    after.terminal.reason = FactTerminalReason::None;
    (before, after)
}

fn transition(_index: usize, state_group: usize, action_group: usize) -> OptionTransitionSample {
    let (mut before, mut after) = fact_pair();
    let x = state_group as f32 * 300.0;
    before.player.position_f32_bits[0] = x.to_bits();
    before.player.position_f32_bits[2] = 0.0_f32.to_bits();
    after.player.position_f32_bits[0] = (x + action_group as f32 + 1.0).to_bits();
    after.player.position_f32_bits[2] = 1.0_f32.to_bits();
    let terminal = state_group == 9 && action_group == 4;
    after.terminal.reached = Some(terminal);
    after.terminal.reason = if terminal {
        FactTerminalReason::GoalReached
    } else {
        FactTerminalReason::None
    };
    let mut tape = InputTape {
        frames: vec![InputFrame::default(); after.tape_frame as usize + 1],
        ..InputTape::default()
    };
    tape.frames[before.tape_frame as usize].pads[0].stick_x = action_group as i8 * 20;
    tape.frames[before.tape_frame as usize].pads[0].stick_y = 100;
    let action = format!("direction-{action_group}");
    let execution = OptionExecution::capture(
        action.clone(),
        OptionType::Move,
        BTreeMap::from([
            ("duration_ticks".into(), OptionParameter::Unsigned(1)),
            (
                "direction_degrees".into(),
                OptionParameter::Signed(action_group as i64 * 15),
            ),
        ]),
        1,
        1,
        OptionCondition::DurationElapsed,
        Vec::new(),
        OptionEndReason::Completed,
        &tape,
        TapeRange {
            start_frame: before.tape_frame,
            end_frame_exclusive: after.tape_frame + 1,
        },
    )
    .unwrap();
    OptionTransitionSample::capture(
        Digest([1; 32]),
        Digest([2; 32]),
        Digest([3; 32]),
        before,
        after,
        execution,
        &tape,
        if terminal { 99.0 } else { -1.0 },
        terminal,
        |facts| {
            Ok::<_, &'static str>(vec![
                10_000.0 - f32::from_bits(facts.player.position_f32_bits[0]),
                f32::from_bits(facts.player.position_f32_bits[0]),
            ])
        },
    )
    .unwrap()
}

fn corpus() -> Vec<OptionTransitionSample> {
    (0..10)
        .flat_map(|state| (0..5).map(move |action| (state, action)))
        .enumerate()
        .map(|(index, (state, action))| transition(index, state, action))
        .collect()
}

#[test]
fn continuous_double_q_ranks_supported_parameterized_actions() {
    let corpus = corpus();
    let model = ContinuousTacticDoubleQModel::fit(&corpus, 0, 16, 1.0).unwrap();
    let source = &corpus[0];
    let context = GeneralizedTacticContext::from_facts(&source.before).unwrap();
    let descriptors = corpus
        .iter()
        .take(5)
        .map(|transition| transition.value_sample.action.clone())
        .collect::<Vec<_>>();
    let ranked = model
        .rank(&source.value_sample.state, &context, &descriptors)
        .unwrap();

    assert_eq!(ranked.len(), descriptors.len());
    assert!(ranked.iter().all(|estimate| {
        estimate.mean_q.is_finite()
            && estimate.ensemble_variance.is_finite()
            && estimate.ensemble_variance >= 0.0
    }));
}

#[test]
fn withholds_whole_state_regions_and_action_realizations() {
    let report = calibrate_generalized_tactic_value(
        &corpus(),
        0,
        GeneralizedTacticCalibrationConfig {
            fitted_q_iterations: 8,
            ..GeneralizedTacticCalibrationConfig::default()
        },
    )
    .unwrap();

    report.validate().unwrap();
    assert_eq!(report.source_transitions, 50);
    assert_eq!(report.state_region.group_overlap_count, 0);
    assert_eq!(report.action_realization.group_overlap_count, 0);
    assert_eq!(report.state_region.training_groups.len(), 6);
    assert_eq!(report.state_region.validation_groups.len(), 2);
    assert_eq!(report.state_region.test_groups.len(), 2);
    assert_eq!(report.action_realization.training_groups.len(), 3);
    assert_eq!(report.action_realization.validation_groups.len(), 1);
    assert_eq!(report.action_realization.test_groups.len(), 1);
    assert!(report.state_region.validation.interval_coverage > 0.0);
    assert!(report.action_realization.validation.interval_coverage > 0.0);
    assert_ne!(report.report_sha256, Digest::ZERO);

    let mut overlapping = report.clone();
    overlapping.state_region.test_groups[0] = overlapping.state_region.training_groups[0].clone();
    overlapping.state_region.group_overlap_count = 0;
    overlapping.report_sha256 = overlapping.digest().unwrap();
    assert!(overlapping.validate().is_err());

    let mut false_coverage_claim = report.clone();
    false_coverage_claim
        .action_realization
        .test_coverage_at_least_nominal = !false_coverage_claim
        .action_realization
        .test_coverage_at_least_nominal;
    false_coverage_claim.report_sha256 = false_coverage_claim.digest().unwrap();
    assert!(false_coverage_claim.validate().is_err());
}

#[test]
fn rejects_row_scale_splits_without_enough_semantic_groups() {
    let repeated = (0..5)
        .map(|index| transition(index, 0, 0))
        .collect::<Vec<_>>();
    let error = calibrate_generalized_tactic_value(
        &repeated,
        0,
        GeneralizedTacticCalibrationConfig::default(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("fewer groups"));
}

#[test]
fn compares_all_controls_on_the_same_whole_group_partitions() {
    let report = compare_generalized_tactic_controls(
        &corpus(),
        0,
        GeneralizedTacticCalibrationConfig {
            fitted_q_iterations: 8,
            ..GeneralizedTacticCalibrationConfig::default()
        },
    )
    .unwrap();

    report.validate().unwrap();
    for axis in [&report.state_region, &report.action_realization] {
        assert_eq!(axis.group_overlap_count, 0);
        assert_eq!(axis.models.len(), 5);
        assert_eq!(
            axis.models
                .iter()
                .map(|metrics| metrics.model.as_str())
                .collect::<Vec<_>>(),
            vec![
                "local_generalized_fitted_q_knn",
                "continuous_fitted_q_forest",
                "continuous_double_q",
                "continuous_conservative_offline_q",
                "structured_shortest_valid_action",
            ]
        );
        assert!(
            axis.models
                .iter()
                .all(|metrics| metrics.evaluation_samples > 0)
        );
    }

    let mut mislabeled = report;
    mislabeled.state_region.models[0].model = "structured_shortest_valid_action".into();
    mislabeled.report_sha256 = mislabeled.digest().unwrap();
    assert!(mislabeled.validate().is_err());
}

#[test]
fn extracted_continuous_forest_ranks_continuous_action_features_by_fitted_q() {
    let transitions = corpus();
    let model = ContinuousTacticValueModel::fit(&transitions, 0, 8, 0.99).unwrap();
    let probe = transition(999, 9, 0);
    let context = GeneralizedTacticContext::from_facts(&probe.before).unwrap();
    let descriptors = (0..5)
        .map(|action| transition(1_000 + action, 9, action).value_sample.action)
        .collect::<Vec<_>>();
    let ranked = model
        .rank(&probe.value_sample.state, &context, &descriptors)
        .unwrap();

    assert_eq!(ranked.len(), descriptors.len());
    assert!(
        ranked
            .windows(2)
            .all(|pair| pair[0].mean_q >= pair[1].mean_q)
    );
    assert!(
        ranked
            .iter()
            .map(|estimate| estimate.mean_q.to_bits())
            .collect::<BTreeSet<_>>()
            .len()
            > 1
    );
    assert_ne!(
        ranked
            .iter()
            .map(|estimate| estimate.descriptor.clone())
            .collect::<Vec<_>>(),
        descriptors
    );
}

#[test]
fn cross_calibration_tests_every_complete_group_exactly_once() {
    let report = cross_calibrate_generalized_tactic_value(
        &corpus(),
        0,
        GeneralizedTacticCalibrationConfig {
            fitted_q_iterations: 8,
            ..GeneralizedTacticCalibrationConfig::default()
        },
    )
    .unwrap();

    report.validate().unwrap();
    assert_eq!(report.state_region.pooled_test.samples, 50);
    assert_eq!(report.action_realization.pooled_test.samples, 50);
    for axis in [&report.state_region, &report.action_realization] {
        assert_eq!(axis.folds.len(), 5);
        let tested = axis
            .folds
            .iter()
            .flat_map(|fold| fold.test_groups.iter())
            .collect::<BTreeSet<_>>();
        let validated = axis
            .folds
            .iter()
            .flat_map(|fold| fold.validation_groups.iter())
            .collect::<BTreeSet<_>>();
        assert_eq!(tested, validated);
        assert_eq!(
            axis.folds
                .iter()
                .map(|fold| fold.test_samples)
                .sum::<usize>(),
            50
        );
    }

    let mut detached = report;
    detached.action_realization.folds[0].test_fold = 1;
    detached.report_sha256 = detached.digest().unwrap();
    assert!(detached.validate().is_err());
}
