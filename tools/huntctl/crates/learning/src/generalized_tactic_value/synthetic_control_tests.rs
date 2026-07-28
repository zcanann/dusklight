use super::*;
use crate::fact_snapshot::{FactSnapshot, FactTerminalReason};
use crate::tape::{InputFrame, InputTape};
use dusklight_control::option_execution::{
    OptionCondition, OptionEndReason, OptionExecution, OptionParameter, OptionType, TapeRange,
};
use dusklight_evidence::native_episode_shard::NativeEpisodeShard;
use std::collections::BTreeMap;

const STRAIGHT: &str = "greedy-straight";
const TURN: &str = "turn-away";
const FORWARD: &str = "forward-after-corner";

fn native_fact_pair() -> (FactSnapshot, FactSnapshot) {
    let shard = NativeEpisodeShard::decode(include_bytes!(
        "../../../../../../tests/fixtures/automation/native_episode_v28.dseps"
    ))
    .unwrap();
    let step = &shard.episodes[0].steps[0];
    let mut before =
        FactSnapshot::from_native_learning(&step.pre_input, &[], None, Vec::new()).unwrap();
    let mut after = FactSnapshot::from_native_learning(
        &step.post_simulation,
        &[step.pre_input.clone()],
        None,
        Vec::new(),
    )
    .unwrap();
    before.terminal.configured = Some(true);
    before.terminal.reached = Some(false);
    before.terminal.reason = FactTerminalReason::None;
    // Keep the typed compatibility fields stable so the gate varies only the
    // continuous movement state and the explicit dead-end procedure.
    after.world = before.world.clone();
    after.player.present = before.player.present;
    after.player.is_link = before.player.is_link;
    after.player.procedure = before.player.procedure;
    after.player.mode_flags = before.player.mode_flags;
    after.player.action_lanes = before.player.action_lanes.clone();
    after.event = before.event.clone();
    after.channels.player_action = before.channels.player_action;
    after.terminal.configured = Some(true);
    after.terminal.reached = Some(false);
    after.terminal.reason = FactTerminalReason::None;
    (before, after)
}

fn set_player_state(facts: &mut FactSnapshot, x: f32, z: f32, procedure: u16) {
    facts.player.position_f32_bits[0] = x.to_bits();
    facts.player.position_f32_bits[2] = z.to_bits();
    facts.player.procedure = Some(procedure);
}

fn features(facts: &FactSnapshot) -> Vec<f32> {
    let x = f32::from_bits(facts.player.position_f32_bits[0]);
    let z = f32::from_bits(facts.player.position_f32_bits[2]);
    let goal_distance = ((1.0 - x).powi(2) + (1.0 - z).powi(2)).sqrt();
    vec![goal_distance, x, z]
}

#[allow(clippy::too_many_arguments)]
fn transition(
    option_id: &str,
    heading: f32,
    before_position: [f32; 2],
    after_position: [f32; 2],
    before_procedure: u16,
    after_procedure: u16,
    reward: f32,
    terminal: bool,
    camera_lock: bool,
) -> OptionTransitionSample {
    let (mut before, mut after) = native_fact_pair();
    set_player_state(
        &mut before,
        before_position[0],
        before_position[1],
        before_procedure,
    );
    set_player_state(
        &mut after,
        after_position[0],
        after_position[1],
        after_procedure,
    );
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
    let input = &mut tape.frames[before.tape_frame as usize].pads[0];
    input.stick_x = (-heading.sin() * 127.0).round() as i8;
    input.stick_y = (heading.cos() * 127.0).round() as i8;
    input.buttons = if camera_lock { 0x0040 } else { 0 };
    let mut parameters = BTreeMap::from([
        (
            "command_initial_heading".into(),
            OptionParameter::F32Bits(heading.to_bits()),
        ),
        ("duration_ticks".into(), OptionParameter::Unsigned(1)),
    ]);
    if camera_lock {
        parameters.insert(
            "command_button_mask".into(),
            OptionParameter::Unsigned(0x0040),
        );
    }
    let execution = OptionExecution::capture(
        option_id.into(),
        OptionType::Custom("synthetic-native-control".into()),
        parameters,
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
        reward,
        terminal,
        |facts| Ok::<_, &'static str>(features(facts)),
    )
    .unwrap()
}

fn around_corner_replay() -> Vec<OptionTransitionSample> {
    vec![
        // Euclidean goal distance makes this look locally attractive, but the
        // changed native procedure represents the unrecoverable wall pocket.
        transition(
            STRAIGHT,
            0.0,
            [0.0, 0.0],
            [0.0, 0.9],
            3,
            99,
            -0.01,
            false,
            true,
        ),
        // The initially worse heading reaches a state just shy of the observed
        // post-corner state. Their fact hashes intentionally do not match.
        transition(
            TURN,
            std::f32::consts::FRAC_PI_2,
            [0.0, 0.0],
            [0.95, 0.0],
            3,
            3,
            -0.01,
            false,
            true,
        ),
        transition(
            FORWARD,
            0.0,
            [1.0, 0.0],
            [1.0, 1.0],
            3,
            3,
            0.99,
            true,
            false,
        ),
    ]
}

fn succeeds(option_id: &str) -> bool {
    option_id == TURN
}

#[test]
fn learned_policy_beats_greedy_and_balanced_random_on_around_corner_gate() {
    let replay = around_corner_replay();
    assert_ne!(
        replay[1].after_state_sha256, replay[2].before_state_sha256,
        "the delayed-credit gate must require an approximate rejoin"
    );
    let state = replay[0].value_sample.state.clone();
    let context = GeneralizedTacticContext::from_facts(&replay[0].before).unwrap();
    let actions = [
        replay[0].value_sample.action.clone(),
        replay[1].value_sample.action.clone(),
    ];

    let immediate_only = GeneralizedTacticValueModel::fit_transitions(&replay, 0)
        .unwrap()
        .rank(&state, &context, &actions)
        .unwrap();
    assert_eq!(
        immediate_only[0].descriptor.option_id, STRAIGHT,
        "greedy local evidence should select the apparent straight-line gain"
    );

    let learned = GeneralizedTacticValueModel::fit_fitted_q_transitions(&replay, 0, 8, 0.99)
        .unwrap()
        .rank(&state, &context, &actions)
        .unwrap();
    assert_eq!(learned[0].descriptor.option_id, TURN);
    assert!(learned[0].outcome.reward > learned[1].outcome.reward);
    assert_eq!(
        learned[0].outcome.terminal, 0.0,
        "approximate credit must not fabricate exact terminal support"
    );

    let learned_successes = usize::from(succeeds(&learned[0].descriptor.option_id)) * 2;
    let structured_successes = usize::from(succeeds(STRAIGHT)) * 2;
    let balanced_random_successes = [STRAIGHT, TURN]
        .into_iter()
        .filter(|action| succeeds(action))
        .count();
    assert!(learned_successes > structured_successes);
    assert!(learned_successes > balanced_random_successes);
}
