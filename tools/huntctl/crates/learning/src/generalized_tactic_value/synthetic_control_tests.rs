use super::*;
use crate::double_q::{ConservativeQ, ConservativeQConfig, DoubleQ, DoubleQConfig};
use crate::fact_snapshot::{
    FactSnapshot, FactTerminalReason, OptionTrajectoryFactSnapshot, RecentOptionFactSnapshot,
};
use crate::fqi::{FittedQ, FqiConfig, Transition};
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
    after.player.action_state = before.player.action_state;
    after.event = before.event.clone();
    after.channels.player_action = before.channels.player_action;
    after.terminal.configured = Some(true);
    after.terminal.reached = Some(false);
    after.terminal.reason = FactTerminalReason::None;
    (before, after)
}

#[test]
fn generalized_context_ignores_absolute_replay_position() {
    let (facts, _) = native_fact_pair();
    let mut shifted = facts.clone();
    shifted.boundary_index += 10_000;
    shifted.simulation_tick += 10_000;
    shifted.tape_frame += 10_000;

    assert_eq!(
        GeneralizedTacticContext::from_facts(&facts)
            .unwrap()
            .values(),
        GeneralizedTacticContext::from_facts(&shifted)
            .unwrap()
            .values()
    );
}

#[test]
fn achieved_goal_relabeling_learns_direction_without_native_terminal_support() {
    let mut east = transition(
        "east",
        -std::f32::consts::FRAC_PI_2,
        [0.0, 0.0],
        [10.0, 0.0],
        3,
        3,
        -0.01,
        false,
        0,
    );
    let mut north = transition("north", 0.0, [0.0, 0.0], [0.0, 10.0], 3, 3, -0.01, false, 0);
    let target = east.after.player.position_f32_bits.map(f32::from_bits);
    let encoder = GoalConditionedTacticFeatureEncoder::new(target).unwrap();
    for row in [&mut east, &mut north] {
        row.feature_schema_sha256 = encoder.schema_sha256;
        row.value_sample.state = encoder.encode(&row.before).unwrap();
        row.value_sample.next_state = encoder.encode(&row.after).unwrap();
    }
    let state = encoder.encode(&east.before).unwrap();
    let context = GeneralizedTacticContext::from_facts(&east.before).unwrap();
    let actions = [
        east.value_sample.action.clone(),
        north.value_sample.action.clone(),
    ];

    let ranked = GeneralizedTacticValueModel::fit_achieved_goal_returns(
        &[east, north],
        encoder.goal_distance_feature(),
    )
    .unwrap()
    .rank(&state, &context, &actions)
    .unwrap();

    assert_eq!(ranked[0].descriptor.option_id, "east");
    assert_eq!(ranked[0].outcome.terminal, 0.0);
    assert!(ranked.iter().all(|estimate| {
        estimate.terminal_support_distance.is_none()
            && estimate.outcome.reward.is_finite()
            && estimate.outcome.reward < 0.0
    }));
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
    button_mask: u16,
) -> OptionTransitionSample {
    scheduled_transition(
        option_id,
        heading,
        before_position,
        after_position,
        before_procedure,
        after_procedure,
        reward,
        terminal,
        &[button_mask],
    )
}

#[allow(clippy::too_many_arguments)]
fn scheduled_transition(
    option_id: &str,
    heading: f32,
    before_position: [f32; 2],
    after_position: [f32; 2],
    before_procedure: u16,
    after_procedure: u16,
    reward: f32,
    terminal: bool,
    button_schedule: &[u16],
) -> OptionTransitionSample {
    assert!(!button_schedule.is_empty());
    let duration = u32::try_from(button_schedule.len()).unwrap();
    let (mut before, mut after) = native_fact_pair();
    after.simulation_tick = before.simulation_tick + u64::from(duration) - 1;
    after.tape_frame = before.tape_frame + u64::from(duration) - 1;
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
    for (offset, buttons) in button_schedule.iter().copied().enumerate() {
        let input = &mut tape.frames[before.tape_frame as usize + offset].pads[0];
        input.stick_x = (-heading.sin() * 127.0).round() as i8;
        input.stick_y = (heading.cos() * 127.0).round() as i8;
        input.buttons = buttons;
    }
    let button_mask = button_schedule
        .iter()
        .copied()
        .fold(0_u16, |mask, buttons| mask | buttons);
    let active_buttons = button_schedule
        .iter()
        .filter(|buttons| **buttons != 0)
        .count();
    let mut parameters = BTreeMap::from([
        (
            "command_initial_heading".into(),
            OptionParameter::F32Bits(heading.to_bits()),
        ),
        (
            "duration_ticks".into(),
            OptionParameter::Unsigned(u64::from(duration)),
        ),
    ]);
    if button_mask != 0 {
        parameters.insert(
            "command_button_mask".into(),
            OptionParameter::Unsigned(u64::from(button_mask)),
        );
        parameters.insert(
            "command_button_active_fraction".into(),
            OptionParameter::F32Bits(
                (active_buttons as f32 / button_schedule.len() as f32).to_bits(),
            ),
        );
        parameters.insert(
            "command_button_pulse_count".into(),
            OptionParameter::Unsigned(active_buttons as u64),
        );
        parameters.insert(
            "command_button_mean_interval_ticks".into(),
            OptionParameter::F32Bits((button_schedule.len() as f32).to_bits()),
        );
        parameters.insert(
            "button_pulse_phase_tick".into(),
            OptionParameter::Unsigned(
                button_schedule
                    .iter()
                    .position(|buttons| *buttons != 0)
                    .unwrap() as u64,
            ),
        );
    }
    let execution = OptionExecution::capture(
        option_id.into(),
        OptionType::Custom("synthetic-native-control".into()),
        parameters,
        duration,
        duration,
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
            0x0040,
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
            0x0040,
        ),
        transition(FORWARD, 0.0, [1.0, 0.0], [1.0, 1.0], 3, 3, 0.99, true, 0),
    ]
}

fn discrete_around_corner_replay(replay: &[OptionTransitionSample]) -> Vec<Transition> {
    replay
        .iter()
        .map(|transition| Transition {
            state: transition.value_sample.state.clone(),
            action: u32::from(transition.value_sample.action.option_id == TURN),
            duration: transition.value_sample.duration_ticks,
            reward: transition.value_sample.reward,
            next_state: transition.value_sample.next_state.clone(),
            terminal: transition.value_sample.terminal,
        })
        .collect()
}

fn assert_beats_equal_budget_controls(
    learned: &str,
    structured: &str,
    balanced_random: &[&str],
    optimal: &str,
) {
    let budget = balanced_random.len();
    let learned_successes = usize::from(learned == optimal) * budget;
    let structured_successes = usize::from(structured == optimal) * budget;
    let random_successes = balanced_random
        .iter()
        .filter(|action| **action == optimal)
        .count();
    assert!(learned_successes > structured_successes);
    assert!(learned_successes > random_successes);
}

fn with_trajectory(
    mut transition: OptionTransitionSample,
    trajectory: OptionTrajectoryFactSnapshot,
) -> OptionTransitionSample {
    transition.after.recent_option = Some(RecentOptionFactSnapshot {
        option_id: transition.execution.option_id.clone(),
        end_reason: transition.execution.end_reason,
        realized_ticks: transition.execution.duration.realized_ticks,
        tape_start: transition.execution.realized_tape_range.start_frame,
        tape_end_exclusive: transition.execution.realized_tape_range.end_frame_exclusive,
        trajectory: Some(trajectory),
    });
    transition.after_state_sha256 = transition.after.content_sha256().unwrap();
    transition.value_sample.after_state_sha256 = transition.after_state_sha256;
    transition.validate().unwrap();
    transition
}

fn motion(
    wall_contact: bool,
    path: f32,
    final_speed: f32,
    maximum_speed: f32,
    momentum_loss: f32,
) -> OptionTrajectoryFactSnapshot {
    OptionTrajectoryFactSnapshot {
        observed_ticks: 1,
        commanded_motion_ticks: 1,
        commanded_stall_ticks: u32::from(path < 1.0),
        wall_contact_ticks: u32::from(wall_contact),
        collision_correction_ticks: 0,
        world_transition_ticks: 0,
        planar_path_length_f32_bits: path.to_bits(),
        planar_displacement_f32_bits: path.to_bits(),
        mean_planar_speed_f32_bits: path.to_bits(),
        final_planar_velocity_f32_bits: final_speed.to_bits(),
        maximum_planar_velocity_f32_bits: maximum_speed.to_bits(),
        commanded_momentum_loss_f32_bits: momentum_loss.to_bits(),
        collision_correction_total_f32_bits: 0.0_f32.to_bits(),
    }
}

#[test]
fn approximate_rejoin_cannot_close_a_censored_return() {
    let replay = around_corner_replay();
    assert_ne!(
        replay[1].after_state_sha256, replay[2].before_state_sha256,
        "the delayed-credit gate must require an approximate rejoin"
    );
    let targets = fitted_q::fit_transition_returns(&replay, 8, 0.99).unwrap();
    assert_eq!(targets.values[0], None);
    assert_eq!(targets.values[1], None);
    assert_eq!(
        targets.values[2],
        Some(-(replay[2].value_sample.duration_ticks as f32))
    );
    assert!(matches!(
        GeneralizedTacticValueModel::fit_fitted_q_transitions(&replay, 0, 8, 0.99),
        Err(GeneralizedTacticValueError::SampleCount)
    ));
}

#[test]
fn around_corner_gate_compares_discrete_q_and_conservative_controls() {
    let replay = around_corner_replay();
    let transitions = discrete_around_corner_replay(&replay);
    let state = &transitions[0].state;
    let actions = [0, 1];

    let fitted = FittedQ::fit(
        state.len(),
        &actions,
        &transitions,
        &FqiConfig {
            iterations: 16,
            trees_per_action: 7,
            max_tree_depth: 4,
            bootstrap: false,
            ..FqiConfig::default()
        },
    )
    .unwrap();
    let double_config = DoubleQConfig {
        epochs: 128,
        hidden_width: 16,
        learning_rate: 0.01,
        target_sync_steps: 16,
        seed: 7,
        ..DoubleQConfig::default()
    };
    let double = DoubleQ::fit(state.len(), &actions, &transitions, &double_config).unwrap();
    let conservative = ConservativeQ::fit(
        state.len(),
        &actions,
        &transitions,
        &ConservativeQConfig {
            double_q: DoubleQConfig {
                seed: 11,
                ..double_config
            },
            conservative_weight: 0.1,
            temperature: 1.0,
        },
    )
    .unwrap();

    let structured_action = 0;
    let fitted_action = fitted.rank_actions(state).unwrap()[0].action;
    let double_action = double.rank_actions(state).unwrap()[0].action;
    let conservative_action = conservative.rank_actions(state).unwrap()[0].action;
    assert_eq!(
        fitted_action, structured_action,
        "the exact-edge continuous FQI control remains trapped by the local straight-line optimum"
    );
    assert_eq!(double_action, 1);
    assert_eq!(conservative_action, 1);
    assert_ne!(double_action, structured_action);
    assert_ne!(conservative_action, structured_action);
}

#[test]
fn collision_gate_observes_slowdown_without_punishing_harmless_contact() {
    let clean = with_trajectory(
        transition("clean", 0.0, [0.0, 0.0], [0.0, 1.0], 3, 3, -0.01, false, 0),
        motion(false, 1.0, 10.0, 10.0, 0.0),
    );
    let harmless_contact = with_trajectory(
        transition(
            "harmless-contact",
            0.0,
            [0.0, 0.0],
            [0.0, 1.0],
            3,
            3,
            -0.01,
            false,
            0,
        ),
        motion(true, 1.0, 10.0, 10.0, 0.0),
    );
    let slowing_impact = with_trajectory(
        transition(
            "slowing-impact",
            0.0,
            [0.0, 0.0],
            [0.0, 0.2],
            3,
            3,
            -0.01,
            false,
            0,
        ),
        motion(true, 0.2, 2.0, 10.0, 8.0),
    );

    let clean = GeneralizedTacticOutcome::from_transition(&clean, 0).unwrap();
    let harmless_contact = GeneralizedTacticOutcome::from_transition(&harmless_contact, 0).unwrap();
    let slowing_impact = GeneralizedTacticOutcome::from_transition(&slowing_impact, 0).unwrap();

    assert_eq!(harmless_contact.wall_contact_fraction, 1.0);
    assert_eq!(harmless_contact.speed_retention, clean.speed_retention);
    assert_eq!(harmless_contact.momentum_loss_per_tick, 0.0);
    assert_eq!(
        compare_generalized_tactic_outcomes(&harmless_contact, &clean),
        std::cmp::Ordering::Equal,
        "contact without lost return is observation, not punishment"
    );

    assert_eq!(slowing_impact.wall_contact_fraction, 1.0);
    assert!(slowing_impact.speed_retention < harmless_contact.speed_retention);
    assert!(slowing_impact.momentum_loss_per_tick > harmless_contact.momentum_loss_per_tick);
    assert_eq!(
        compare_generalized_tactic_outcomes(&slowing_impact, &clean),
        std::cmp::Ordering::Equal,
        "slowdown is exposed for representation learning but cannot shape utility"
    );
}

#[test]
fn prompted_roll_gate_learns_available_timing_and_never_hallucinates_the_action() {
    let early_roll = scheduled_transition(
        "roll-early",
        0.0,
        [0.0, 0.0],
        [1.0, 1.0],
        3,
        3,
        0.98,
        true,
        &[0x0100, 0],
    );
    let late_roll = scheduled_transition(
        "roll-late",
        0.0,
        [0.0, 0.0],
        [1.0, 1.0],
        3,
        3,
        0.97,
        true,
        &[0, 0x0100, 0],
    );
    let continuous_only = scheduled_transition(
        "continuous-only",
        0.0,
        [0.0, 0.0],
        [0.0, 0.5],
        3,
        3,
        -0.02,
        false,
        &[0, 0],
    );
    let context = GeneralizedTacticContext::from_facts(&early_roll.before).unwrap();
    let early_factors =
        generalized_tactic_action_factors(&context, &early_roll.value_sample.action).unwrap();
    let late_factors =
        generalized_tactic_action_factors(&context, &late_roll.value_sample.action).unwrap();
    assert!(early_factors.rolling);
    assert!(late_factors.rolling);
    assert!(early_factors.button_phase_fraction < late_factors.button_phase_fraction);

    let replay = [early_roll, late_roll, continuous_only];
    let model = GeneralizedTacticValueModel::fit_transitions(&replay, 0).unwrap();
    let state = replay[0].value_sample.state.clone();
    let available = replay
        .iter()
        .map(|transition| transition.value_sample.action.clone())
        .collect::<Vec<_>>();
    let ranked = model.rank(&state, &context, &available).unwrap();
    assert_eq!(ranked[0].descriptor.option_id, "roll-early");
    assert_beats_equal_budget_controls(
        &ranked[0].descriptor.option_id,
        "continuous-only",
        &["continuous-only", "roll-late", "roll-early"],
        "roll-early",
    );

    let unavailable = model
        .rank(
            &state,
            &context,
            std::slice::from_ref(&replay[2].value_sample.action),
        )
        .unwrap();
    assert_eq!(unavailable.len(), 1);
    assert_eq!(unavailable[0].descriptor.option_id, "continuous-only");
}

#[test]
fn demonstration_does_not_authenticate_an_unconnected_shortcut() {
    let demonstration = scheduled_transition(
        "human-demonstration",
        0.0,
        [0.0, 0.0],
        [1.0, 1.0],
        3,
        3,
        0.95,
        true,
        &[0, 0, 0, 0, 0],
    );
    let shortcut_setup = transition(
        "learned-camera-lock-shortcut",
        std::f32::consts::FRAC_PI_2,
        [0.0, 0.0],
        [0.95, 0.0],
        3,
        3,
        -0.01,
        false,
        0x0040,
    );
    let shortcut_finish = transition(
        "shortcut-finish",
        0.0,
        [1.0, 0.0],
        [1.0, 1.0],
        3,
        3,
        0.99,
        true,
        0,
    );
    assert_ne!(
        shortcut_setup.after_state_sha256,
        shortcut_finish.before_state_sha256
    );
    let replay = [demonstration, shortcut_setup, shortcut_finish];
    let scratch_replay = [replay[1].clone(), replay[2].clone()];
    assert!(matches!(
        GeneralizedTacticValueModel::fit_fitted_q_transitions(&scratch_replay, 0, 8, 0.99),
        Err(GeneralizedTacticValueError::SampleCount)
    ));
}
