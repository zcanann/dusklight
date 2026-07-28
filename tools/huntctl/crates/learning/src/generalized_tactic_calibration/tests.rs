use super::*;
use crate::fact_snapshot::{FactSnapshot, FactTerminalReason};
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
