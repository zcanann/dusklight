use super::*;
use crate::fact_snapshot::{FactPhase, FactTerminalReason};
use crate::tape::{InputFrame, InputTape};
use dusklight_control::option_execution::{
    OptionCondition, OptionEndReason, OptionExecution, OptionParameter, TapeRange,
};
use dusklight_evidence::native_episode_shard::NativeEpisodeShard;
use std::collections::BTreeMap;

fn boundary(base: &FactSnapshot, x: f32, elapsed: u64) -> FactSnapshot {
    let mut facts = base.clone();
    facts.phase = FactPhase::PreInput;
    facts.player.position_f32_bits = [x, 0.0, 0.0].map(f32::to_bits);
    facts.simulation_tick += elapsed;
    facts.tape_frame += elapsed;
    facts.boundary_index += elapsed;
    facts.terminal.configured = Some(true);
    facts.terminal.reached = Some(false);
    facts.terminal.reason = FactTerminalReason::None;
    facts
}

fn row(
    before: FactSnapshot,
    after: FactSnapshot,
    id: &str,
    heading: f32,
    encoder: &GoalConditionedTacticFeatureEncoder,
) -> OptionTransitionSample {
    let ticks = (after.tape_frame - before.tape_frame) as u32;
    let mut tape = InputTape {
        frames: vec![InputFrame::default(); after.tape_frame as usize],
        ..InputTape::default()
    };
    for frame in &mut tape.frames[before.tape_frame as usize..] {
        frame.pads[0].stick_x = (-heading.sin() * 100.0).round() as i8;
        frame.pads[0].stick_y = (heading.cos() * 100.0).round() as i8;
    }
    let execution = OptionExecution::capture(
        id.into(),
        OptionType::Move,
        BTreeMap::from([
            (
                "command_initial_heading".into(),
                OptionParameter::F32Bits(heading.to_bits()),
            ),
            (
                "duration_ticks".into(),
                OptionParameter::Unsigned(u64::from(ticks)),
            ),
        ]),
        ticks,
        ticks,
        OptionCondition::DurationElapsed,
        Vec::new(),
        OptionEndReason::Completed,
        &tape,
        TapeRange {
            start_frame: before.tape_frame,
            end_frame_exclusive: after.tape_frame,
        },
    )
    .unwrap();
    let terminal = after.terminal.reached == Some(true);
    OptionTransitionSample::capture(
        encoder.schema_sha256,
        Digest([2; 32]),
        Digest([3; 32]),
        before,
        after,
        execution,
        &tape,
        0.0,
        terminal,
        |facts| encoder.encode(facts),
    )
    .unwrap()
}

fn detour_replay(
    native_terminal: bool,
) -> (
    Vec<OptionTransitionSample>,
    GoalConditionedTacticFeatureEncoder,
) {
    let shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v28.dseps"
    ))
    .unwrap();
    let base = FactSnapshot::from_native_learning(
        &shard.episodes[0].steps[0].pre_input,
        &[],
        None,
        Vec::new(),
    )
    .unwrap();
    let encoder = GoalConditionedTacticFeatureEncoder::new([10.0, 0.0, 0.0]).unwrap();
    let root = boundary(&base, 0.0, 0);
    let away = boundary(&base, -1.0, 1);
    let toward = boundary(&base, 1.0, 1);
    let mut slow_end = boundary(&base, 10.0, 21);
    if native_terminal {
        slow_end.terminal.reached = Some(true);
        slow_end.terminal.reason = FactTerminalReason::GoalReached;
    }
    let replay = vec![
        row(
            root.clone(),
            away.clone(),
            "away",
            std::f32::consts::FRAC_PI_2,
            &encoder,
        ),
        row(
            away,
            boundary(&base, 10.0, 2),
            "detour-finish",
            -std::f32::consts::FRAC_PI_2,
            &encoder,
        ),
        row(
            root.clone(),
            toward.clone(),
            "toward",
            -std::f32::consts::FRAC_PI_2,
            &encoder,
        ),
        row(
            toward,
            slow_end,
            "slow-finish",
            -std::f32::consts::FRAC_PI_2,
            &encoder,
        ),
    ];
    (replay, encoder)
}

#[test]
fn hindsight_returns_choose_the_faster_detour_before_native_success() {
    let (replay, encoder) = detour_replay(false);
    let root = replay[0].before.clone();
    assert!(replay.iter().all(|row| !row.value_sample.terminal));
    let actions = [
        replay[0].value_sample.action.clone(),
        replay[2].value_sample.action.clone(),
    ];
    let features = encoder.encode(&root).unwrap();
    let context = GeneralizedTacticContext::from_facts(&root).unwrap();
    let motion = GeneralizedTacticValueModel::fit_achieved_goal_returns(
        &replay,
        encoder.goal_distance_feature(),
    )
    .unwrap();
    assert_eq!(
        motion
            .rank_goal_reachability(&features, &context, &actions)
            .unwrap()[0]
            .descriptor
            .option_id,
        "toward"
    );
    let delayed = GeneralizedTacticValueModel::fit_delayed_achieved_goal_returns(
        &replay,
        encoder.goal_distance_feature(),
    )
    .unwrap();
    let ranked = delayed.rank(&features, &context, &actions).unwrap();
    assert_eq!(ranked[0].descriptor.option_id, "away", "{ranked:#?}");
    assert!(ranked[0].outcome.reward > ranked[1].outcome.reward);
    assert!(
        delayed
            .samples
            .iter()
            .all(|sample| sample.outcome.reward < 0.0 && sample.outcome.terminal == 0.0)
    );
    assert!(
        ranked
            .iter()
            .all(|estimate| estimate.terminal_support_distance.is_none())
    );
}

#[test]
fn coordinate_only_success_cannot_alias_the_authored_goal() {
    let (replay, encoder) = detour_replay(true);
    let model = GeneralizedTacticValueModel::fit_delayed_achieved_goal_returns(
        &replay,
        encoder.goal_distance_feature(),
    )
    .unwrap();
    let actions = [
        replay[0].value_sample.action.clone(),
        replay[2].value_sample.action.clone(),
    ];
    let ranked = model
        .rank(
            &encoder.encode(&replay[0].before).unwrap(),
            &GeneralizedTacticContext::from_facts(&replay[0].before).unwrap(),
            &actions,
        )
        .unwrap();
    assert_eq!(ranked[0].descriptor.option_id, "toward", "{ranked:#?}");
    assert!(
        model
            .samples
            .iter()
            .any(|sample| sample.state.last() == Some(&GoalQueryKind::Authored.feature()))
    );
    assert!(
        model
            .samples
            .iter()
            .any(|sample| sample.state.last() == Some(&GoalQueryKind::Achieved.feature()))
    );
    assert!(
        model
            .samples
            .iter()
            .all(|sample| sample.outcome.reward < 0.0)
    );
}
