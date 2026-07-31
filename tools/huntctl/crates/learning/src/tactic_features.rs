//! Fixed, route-agnostic features for tactic-level value learning.
//!
//! The encoder consumes only the shared `FactSnapshot`. It deliberately omits
//! absolute tape, simulation, and boundary indices so a critic cannot mistake
//! replay position for gameplay progress.

use crate::artifact::Digest;
use crate::fact_snapshot::{
    ByteBankFactSnapshot, FRONT_ROLL_DO_STATUS, FactAvailability, FactSnapshot, FactSnapshotError,
};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::error::Error;
use std::f32::consts::PI;
use std::fmt;

pub const TACTIC_FEATURE_SCHEMA_V6: &str = "dusklight-tactic-features/v6";
pub const GOAL_CONDITIONED_TACTIC_FEATURE_SCHEMA_V6: &str =
    "dusklight-goal-conditioned-tactic-features/v6";
const GOAL_FEATURE_NAMES: &[&str] = &[
    "goal_dx",
    "goal_dy",
    "goal_dz",
    "goal_planar_distance",
    "goal_history_progress",
    "goal_history_progress_per_tick",
    "goal_trajectory_alignment",
    "goal_closing_speed",
];

const FEATURE_NAMES: &[&str] = &[
    "stage_hash",
    "room",
    "layer_available",
    "layer",
    "point_available",
    "point",
    "next_stage_available",
    "next_stage_hash",
    "next_room_available",
    "next_room",
    "player_present",
    "player_is_link_available",
    "player_is_link",
    "player_procedure_available",
    "player_procedure",
    "player_mode_available",
    "player_mode",
    "player_action_available",
    "player_do_prompt_available",
    "player_do_status",
    "player_do_status_bit_0",
    "player_do_status_bit_1",
    "player_do_status_bit_2",
    "player_do_status_bit_3",
    "player_do_status_bit_4",
    "player_do_status_bit_5",
    "player_do_status_bit_6",
    "player_do_status_bit_7",
    "player_front_roll_prompt_available",
    "player_damage_wait_timer",
    "player_sword_at_up_time",
    "player_ice_damage_wait_timer",
    "player_sword_change_wait_timer",
    "player_procedure_context_0",
    "player_procedure_context_1",
    "player_procedure_context_2",
    "player_procedure_context_3",
    "player_procedure_context_4",
    "player_procedure_context_5",
    "player_action_lane_count",
    "player_action_lane_frame_min",
    "player_action_lane_frame_mean",
    "player_action_lane_frame_max",
    "player_action_flag_0",
    "player_action_flag_1",
    "player_action_flag_2",
    "player_action_flag_3",
    "player_action_flag_4",
    "player_action_flag_5",
    "player_action_flag_6",
    "player_action_flag_7",
    "player_action_flag_8",
    "player_action_flag_9",
    "player_action_flag_10",
    "player_action_flag_11",
    "player_action_flag_12",
    "player_action_flag_13",
    "player_action_flag_14",
    "player_action_flag_15",
    "player_action_flag_16",
    "player_action_flag_17",
    "player_action_flag_18",
    "player_action_flag_19",
    "player_action_flag_20",
    "player_action_flag_21",
    "player_action_flag_22",
    "player_action_flag_23",
    "player_action_flag_24",
    "player_action_flag_25",
    "player_action_flag_26",
    "player_action_flag_27",
    "player_action_flag_28",
    "player_action_flag_29",
    "player_action_flag_30",
    "player_action_flag_31",
    "previous_pad_available",
    "previous_pad_button_0",
    "previous_pad_button_1",
    "previous_pad_button_2",
    "previous_pad_button_3",
    "previous_pad_button_4",
    "previous_pad_button_5",
    "previous_pad_button_6",
    "previous_pad_button_7",
    "previous_pad_button_8",
    "previous_pad_button_9",
    "previous_pad_button_10",
    "previous_pad_button_11",
    "previous_pad_button_12",
    "previous_pad_button_13",
    "previous_pad_button_14",
    "previous_pad_button_15",
    "previous_pad_stick_x",
    "previous_pad_stick_y",
    "previous_pad_stick_magnitude",
    "previous_pad_substick_x",
    "previous_pad_substick_y",
    "previous_pad_substick_magnitude",
    "previous_pad_trigger_left",
    "previous_pad_trigger_right",
    "previous_pad_analog_a",
    "previous_pad_analog_b",
    "previous_pad_connected",
    "previous_pad_error",
    "player_contacts_available",
    "player_contacts",
    "collision_correction_available",
    "collision_correction_magnitude",
    "player_x",
    "player_y",
    "player_z",
    "velocity_available",
    "velocity_x",
    "velocity_y",
    "velocity_z",
    "forward_speed_available",
    "forward_speed",
    "yaw_available",
    "yaw_sin",
    "yaw_cos",
    "camera_yaw_available",
    "camera_yaw_sin",
    "camera_yaw_cos",
    "ground_height_available",
    "ground_height",
    "roof_height_available",
    "roof_height",
    "event_available",
    "event_running",
    "event_id",
    "event_mode",
    "terminal_available",
    "terminal_reached",
    "terminal_hit_fraction",
    "terminal_stability_fraction",
    "actor_count",
    "same_room_actor_count",
    "portable_actor_fraction",
    "nearest_actor_available",
    "nearest_actor_name_hash",
    "nearest_actor_dx",
    "nearest_actor_dy",
    "nearest_actor_dz",
    "nearest_actor_distance",
    "mean_actor_dx",
    "mean_actor_dy",
    "mean_actor_dz",
    "mean_actor_speed",
    "known_actor_health_fraction",
    "mean_actor_health",
    "event_flag_bits_available",
    "event_flag_bits_set",
    "temporary_flag_bits_available",
    "temporary_flag_bits_set",
    "temporary_event_flag_bits_available",
    "temporary_event_flag_bits_set",
    "dungeon_flag_bits_available",
    "dungeon_flag_bits_set",
    "switch_flag_bits_available",
    "switch_flag_bits_set",
    "history_available",
    "history_elapsed_ticks",
    "history_stage_changed",
    "history_room_changed",
    "history_dx",
    "history_dy",
    "history_dz",
    "history_procedure_changed",
    "history_event_changed",
    "trajectory_available",
    "trajectory_elapsed_ticks",
    "trajectory_planar_path_length",
    "trajectory_planar_displacement",
    "trajectory_straightness",
    "trajectory_mean_planar_speed",
    "trajectory_commanded_fraction",
    "trajectory_stalled_command_fraction",
    "trajectory_speed_retention",
    "recent_option_available",
    "recent_option_ticks",
    "recent_option_trajectory_available",
    "recent_option_wall_contact_fraction",
    "recent_option_momentum_loss_per_tick",
    "recent_option_contact_slowdown_available",
    "recent_option_contact_commanded_fraction",
    "recent_option_contact_momentum_loss_per_command_tick",
    "recent_option_collision_correction_per_tick",
    "condition_true_count",
    "condition_false_count",
    "condition_unknown_count",
    "channel_available_count",
    "channel_absent_count",
    "channel_unavailable_count",
    "channel_not_sampled_count",
];

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticFeatureEncoder {
    pub schema: String,
    pub schema_sha256: Digest,
    pub feature_names: Vec<String>,
}

impl Default for TacticFeatureEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl TacticFeatureEncoder {
    pub fn new() -> Self {
        let feature_names = FEATURE_NAMES.iter().map(|name| (*name).into()).collect();
        let schema_sha256 = feature_schema_digest(FEATURE_NAMES);
        Self {
            schema: TACTIC_FEATURE_SCHEMA_V6.into(),
            schema_sha256,
            feature_names,
        }
    }

    pub fn feature_width(&self) -> usize {
        self.feature_names.len()
    }

    pub fn encode(&self, facts: &FactSnapshot) -> Result<Vec<f32>, TacticFeatureError> {
        facts.validate().map_err(TacticFeatureError::Facts)?;
        if self.schema != TACTIC_FEATURE_SCHEMA_V6
            || self.schema_sha256 != feature_schema_digest(FEATURE_NAMES)
            || self.feature_names.len() != FEATURE_NAMES.len()
            || self
                .feature_names
                .iter()
                .zip(FEATURE_NAMES)
                .any(|(actual, expected)| actual != expected)
        {
            return Err(TacticFeatureError::InvalidEncoder);
        }

        let player_position = finite_vec3(facts.player.position_f32_bits)?;
        let mut output = Vec::with_capacity(FEATURE_NAMES.len());
        output.push(symbol_feature(&facts.world.stage));
        output.push(f32::from(facts.world.room));
        push_optional_i8(&mut output, facts.world.layer);
        push_optional_i16(&mut output, facts.world.point);
        push_optional_symbol(&mut output, facts.world.next_stage.as_deref());
        push_optional_i8(&mut output, facts.world.next_room);
        output.push(bool_feature(facts.player.present));
        push_optional_bool(&mut output, facts.player.is_link);
        push_optional_u16(&mut output, facts.player.procedure);
        push_optional_u32(&mut output, facts.player.mode_flags);
        encode_player_action(&mut output, facts)?;
        encode_previous_pad(&mut output, facts);
        push_optional_u8(&mut output, facts.player.contacts);
        match facts.player.collision_correction_f32_bits {
            Some(bits) => {
                let correction = finite_vec2(bits)?;
                output.extend([1.0, correction[0].hypot(correction[1])]);
            }
            None => output.extend([0.0, 0.0]),
        }
        output.extend(player_position);
        match facts.player.velocity_f32_bits {
            Some(bits) => {
                output.push(1.0);
                output.extend(finite_vec3(bits)?);
            }
            None => {
                output.push(0.0);
                output.extend([0.0; 3]);
            }
        }
        push_optional_f32_bits(&mut output, facts.player.forward_speed_f32_bits)?;
        match facts.player.current_angle {
            Some(angle) => {
                let radians = f32::from(angle[1]) * (PI / 32768.0);
                output.extend([1.0, radians.sin(), radians.cos()]);
            }
            None => output.extend([0.0, 0.0, 0.0]),
        }
        match facts.player.camera_yaw_radians_f32_bits {
            Some(bits) => {
                let radians = finite_f32(bits)?;
                output.extend([1.0, radians.sin(), radians.cos()]);
            }
            None => output.extend([0.0, 0.0, 0.0]),
        }
        push_optional_f32_bits(&mut output, facts.player.ground_height_f32_bits)?;
        push_optional_f32_bits(&mut output, facts.player.roof_height_f32_bits)?;
        match &facts.event {
            Some(event) => output.extend([
                1.0,
                bool_feature(event.running),
                f32::from(event.event_id),
                f32::from(event.mode),
            ]),
            None => output.extend([0.0; 4]),
        }
        output.push(bool_feature(facts.terminal.reached.is_some()));
        output.push(optional_bool_feature(facts.terminal.reached));
        output.push(ratio(
            facts.terminal.hit_count,
            facts.terminal.requested_count,
        ));
        output.push(ratio(
            facts.terminal.consecutive_ticks,
            facts.terminal.stable_ticks,
        ));

        encode_actor_summary(&mut output, facts, player_position)?;
        encode_bank(&mut output, &facts.flag_banks.event);
        encode_bank(&mut output, &facts.flag_banks.temporary);
        encode_bank(&mut output, &facts.flag_banks.temporary_event);
        encode_bank(&mut output, &facts.flag_banks.dungeon);
        encode_bank(&mut output, &facts.flag_banks.switch);
        encode_history(&mut output, facts, player_position)?;
        output.extend(trajectory_summary(facts, player_position)?);
        match &facts.recent_option {
            Some(option) => {
                output.extend([1.0, option.realized_ticks as f32]);
                match option.trajectory {
                    Some(trajectory) => {
                        let ticks = trajectory.observed_ticks.max(1) as f32;
                        output.extend([
                            1.0,
                            trajectory.wall_contact_ticks as f32 / ticks,
                            finite_f32(trajectory.commanded_momentum_loss_f32_bits)? / ticks,
                        ]);
                        match (
                            trajectory.wall_contact_commanded_motion_ticks,
                            trajectory.wall_contact_commanded_momentum_loss_f32_bits,
                        ) {
                            (Some(contact_ticks), Some(contact_loss)) => output.extend([
                                1.0,
                                contact_ticks as f32
                                    / trajectory.commanded_motion_ticks.max(1) as f32,
                                finite_f32(contact_loss)? / contact_ticks.max(1) as f32,
                            ]),
                            (None, None) => output.extend([0.0; 3]),
                            _ => return Err(TacticFeatureError::InvalidOutput),
                        }
                        output.extend([finite_f32(
                            trajectory.collision_correction_total_f32_bits,
                        )? / ticks]);
                    }
                    None => output.extend([0.0; 7]),
                }
            }
            None => output.extend([0.0; 9]),
        }
        let mut condition_counts = [0_u32; 3];
        for condition in &facts.conditions {
            match condition.value {
                Some(true) => condition_counts[0] += 1,
                Some(false) => condition_counts[1] += 1,
                None => condition_counts[2] += 1,
            }
        }
        output.extend(condition_counts.map(|count| count as f32));
        output.extend(channel_counts(facts).map(|count| count as f32));

        if output.len() != FEATURE_NAMES.len() || output.iter().any(|value| !value.is_finite()) {
            return Err(TacticFeatureError::InvalidOutput);
        }
        Ok(output)
    }
}

/// The route-agnostic tactic features plus measurements relative to one
/// objective-derived world target.
///
/// The target is run context, not authored route progress. Its coordinates are
/// retained as exact bits so feature generation is deterministic and auditable.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GoalConditionedTacticFeatureEncoder {
    pub schema: String,
    pub schema_sha256: Digest,
    pub feature_names: Vec<String>,
    pub target_coordinate_f32_bits: [u32; 3],
    base: TacticFeatureEncoder,
}

impl GoalConditionedTacticFeatureEncoder {
    pub fn new(target: [f32; 3]) -> Result<Self, TacticFeatureError> {
        if target.iter().any(|value| !value.is_finite()) {
            return Err(TacticFeatureError::InvalidEncoder);
        }
        let base = TacticFeatureEncoder::new();
        let mut feature_names = base.feature_names.clone();
        feature_names.extend(GOAL_FEATURE_NAMES.iter().map(|name| (*name).into()));
        Ok(Self {
            schema: GOAL_CONDITIONED_TACTIC_FEATURE_SCHEMA_V6.into(),
            schema_sha256: goal_conditioned_feature_schema_digest(base.schema_sha256),
            feature_names,
            target_coordinate_f32_bits: target.map(f32::to_bits),
            base,
        })
    }

    pub fn feature_width(&self) -> usize {
        self.feature_names.len()
    }

    pub fn goal_distance_feature(&self) -> usize {
        self.base.feature_width() + 3
    }

    /// Gives each semantic feature family one unit of distance mass regardless
    /// of how many scalar columns encode it. This prevents variable-width
    /// bitsets and summaries from winning nearest-state lookup by cardinality.
    pub fn distance_weights(&self) -> Vec<f32> {
        let groups = self
            .feature_names
            .iter()
            .map(|name| distance_group(name))
            .collect::<Vec<_>>();
        let mut counts = BTreeMap::<&str, usize>::new();
        for group in &groups {
            *counts.entry(group).or_default() += 1;
        }
        groups
            .iter()
            .map(|group| 1.0 / *counts.get(group.as_str()).unwrap() as f32)
            .collect()
    }

    pub fn encode(&self, facts: &FactSnapshot) -> Result<Vec<f32>, TacticFeatureError> {
        if self.schema != GOAL_CONDITIONED_TACTIC_FEATURE_SCHEMA_V6
            || self.schema_sha256 != goal_conditioned_feature_schema_digest(self.base.schema_sha256)
            || self.feature_names.len() != self.base.feature_width() + GOAL_FEATURE_NAMES.len()
        {
            return Err(TacticFeatureError::InvalidEncoder);
        }
        let mut output = self.base.encode(facts)?;
        let player = finite_vec3(facts.player.position_f32_bits)?;
        let target = self.target_coordinate_f32_bits.map(f32::from_bits);
        if target.iter().any(|value| !value.is_finite()) {
            return Err(TacticFeatureError::InvalidEncoder);
        }
        let delta = [
            target[0] - player[0],
            target[1] - player[1],
            target[2] - player[2],
        ];
        let planar_distance = delta[0].hypot(delta[2]);
        output.extend(delta);
        output.push(planar_distance);
        output.extend(goal_trajectory_summary(facts, player, target)?);
        if output.len() != self.feature_width() || output.iter().any(|value| !value.is_finite()) {
            return Err(TacticFeatureError::InvalidOutput);
        }
        Ok(output)
    }
}

fn distance_group(name: &str) -> String {
    let group = if name.starts_with("player_do_status_bit_")
        || matches!(
            name,
            "player_do_prompt_available"
                | "player_do_status"
                | "player_front_roll_prompt_available"
        ) {
        "player_prompted_action"
    } else if name.starts_with("player_action_flag_") {
        "player_action_flags"
    } else if name.starts_with("player_procedure_context_") {
        "player_procedure_context"
    } else if name.starts_with("player_action_lane_") {
        "player_action_phase"
    } else if name.starts_with("previous_pad_") {
        "previous_input"
    } else if matches!(
        name,
        "actor_count"
            | "same_room_actor_count"
            | "portable_actor_fraction"
            | "nearest_actor_available"
            | "nearest_actor_name_hash"
            | "nearest_actor_dx"
            | "nearest_actor_dy"
            | "nearest_actor_dz"
            | "nearest_actor_distance"
            | "mean_actor_dx"
            | "mean_actor_dy"
            | "mean_actor_dz"
            | "mean_actor_speed"
            | "known_actor_health_fraction"
            | "mean_actor_health"
    ) {
        "actor_summary"
    } else if name.contains("_flag_bits_") {
        "flag_banks"
    } else if name.starts_with("channel_") {
        "channel_availability"
    } else if name.starts_with("trajectory_") {
        "recent_trajectory"
    } else if name.starts_with("recent_option_") {
        "recent_option"
    } else if matches!(name, "player_x" | "player_y" | "player_z") {
        "player_position"
    } else if matches!(
        name,
        "velocity_available" | "velocity_x" | "velocity_y" | "velocity_z"
    ) {
        "player_velocity"
    } else if matches!(name, "yaw_available" | "yaw_sin" | "yaw_cos") {
        "player_yaw"
    } else if name.starts_with("camera_yaw_") {
        "camera_yaw"
    } else if name.starts_with("terminal_") {
        "terminal_evidence"
    } else if matches!(name, "goal_dx" | "goal_dy" | "goal_dz") {
        "goal_delta"
    } else if name.starts_with("goal_history_")
        || name.starts_with("goal_trajectory_")
        || name == "goal_closing_speed"
    {
        "goal_motion"
    } else {
        return name.into();
    };
    group.into()
}

fn encode_player_action(
    output: &mut Vec<f32>,
    facts: &FactSnapshot,
) -> Result<(), TacticFeatureError> {
    let action = facts.player.action_state;
    output.push(bool_feature(
        action.is_some() || !facts.player.action_lanes.is_empty(),
    ));
    if let Some(action) = action {
        let do_status = action.do_status;
        output.extend([bool_feature(do_status != 0), f32::from(do_status)]);
        output.extend((0..u8::BITS).map(|bit| bool_feature(do_status & (1_u8 << bit) != 0)));
        output.extend([
            bool_feature(do_status == FRONT_ROLL_DO_STATUS),
            f32::from(action.damage_wait_timer),
            f32::from(action.sword_at_up_time),
            f32::from(action.ice_damage_wait_timer),
            f32::from(action.sword_change_wait_timer),
        ]);
        output.extend(action.procedure_context_raw.map(f32::from));
    } else {
        output.extend([0.0; 21]);
    }

    let frames = facts
        .player
        .action_lanes
        .iter()
        .map(|lane| finite_f32(lane.frame_f32_bits))
        .collect::<Result<Vec<_>, _>>()?;
    let (minimum, mean, maximum) = if frames.is_empty() {
        (0.0, 0.0, 0.0)
    } else {
        let minimum = frames.iter().copied().min_by(f32::total_cmp).unwrap();
        let maximum = frames.iter().copied().max_by(f32::total_cmp).unwrap();
        let mean = frames.iter().sum::<f32>() / frames.len() as f32;
        (minimum, mean, maximum)
    };
    output.extend([frames.len() as f32, minimum, mean, maximum]);
    let flags = action.map_or(0, |action| action.flags);
    output.extend((0..32).map(|bit| bool_feature(flags & (1 << bit) != 0)));
    Ok(())
}

fn encode_previous_pad(output: &mut Vec<f32>, facts: &FactSnapshot) {
    let Some(pad) = facts.player.previous_pad else {
        output.extend([0.0; 29]);
        return;
    };
    output.push(1.0);
    output.extend((0..u16::BITS).map(|bit| bool_feature(pad.buttons & (1_u16 << bit) != 0)));
    let stick_x = f32::from(pad.stick_x);
    let stick_y = f32::from(pad.stick_y);
    let substick_x = f32::from(pad.substick_x);
    let substick_y = f32::from(pad.substick_y);
    output.extend([
        stick_x,
        stick_y,
        stick_x.hypot(stick_y),
        substick_x,
        substick_y,
        substick_x.hypot(substick_y),
        f32::from(pad.trigger_left),
        f32::from(pad.trigger_right),
        f32::from(pad.analog_a),
        f32::from(pad.analog_b),
        bool_feature(pad.connected),
        f32::from(pad.error),
    ]);
}

fn trajectory_summary(
    facts: &FactSnapshot,
    player: [f32; 3],
) -> Result<[f32; 9], TacticFeatureError> {
    let Some(oldest) = facts.recent_history.first() else {
        return Ok([0.0; 9]);
    };
    if facts
        .recent_history
        .iter()
        .any(|row| row.stage != facts.world.stage || row.room != facts.world.room)
    {
        return Ok([0.0; 9]);
    }
    let mut prior_position = finite_vec3(oldest.player_position_f32_bits)?;
    let mut prior_tick = oldest.simulation_tick;
    let mut path_length = 0.0_f32;
    let mut commanded_steps = 0_u32;
    let mut stalled_command_steps = 0_u32;
    let mut observed_steps = 0_u32;
    for row in facts.recent_history.iter().skip(1) {
        let position = finite_vec3(row.player_position_f32_bits)?;
        let elapsed = row.simulation_tick.saturating_sub(prior_tick).max(1) as f32;
        let distance = planar_distance(prior_position, position);
        let speed = distance / elapsed;
        let commanded = commanded_motion(row.previous_pad);
        path_length += distance;
        observed_steps += 1;
        commanded_steps += u32::from(commanded);
        stalled_command_steps += u32::from(commanded && speed < 1.0);
        prior_position = position;
        prior_tick = row.simulation_tick;
    }
    let elapsed = facts.simulation_tick.saturating_sub(prior_tick).max(1) as f32;
    let distance = planar_distance(prior_position, player);
    let speed = distance / elapsed;
    let commanded = facts.player.previous_pad.is_some_and(commanded_motion);
    path_length += distance;
    observed_steps += 1;
    commanded_steps += u32::from(commanded);
    stalled_command_steps += u32::from(commanded && speed < 1.0);
    let last_speed = speed;

    let oldest_position = finite_vec3(oldest.player_position_f32_bits)?;
    let window_ticks = facts
        .simulation_tick
        .saturating_sub(oldest.simulation_tick)
        .max(1) as f32;
    let displacement = planar_distance(oldest_position, player);
    let mean_speed = path_length / window_ticks;
    Ok([
        1.0,
        window_ticks,
        path_length,
        displacement,
        if path_length > f32::EPSILON {
            (displacement / path_length).clamp(0.0, 1.0)
        } else {
            0.0
        },
        mean_speed,
        commanded_steps as f32 / observed_steps as f32,
        if commanded_steps == 0 {
            0.0
        } else {
            stalled_command_steps as f32 / commanded_steps as f32
        },
        if mean_speed > f32::EPSILON {
            (last_speed / mean_speed).clamp(0.0, 4.0)
        } else {
            0.0
        },
    ])
}

fn goal_trajectory_summary(
    facts: &FactSnapshot,
    player: [f32; 3],
    target: [f32; 3],
) -> Result<[f32; 4], TacticFeatureError> {
    let Some(oldest) = facts.recent_history.first() else {
        return Ok([0.0; 4]);
    };
    if facts
        .recent_history
        .iter()
        .any(|row| row.stage != facts.world.stage || row.room != facts.world.room)
    {
        return Ok([0.0; 4]);
    }
    let oldest_position = finite_vec3(oldest.player_position_f32_bits)?;
    let old_distance = planar_distance(oldest_position, target);
    let current_distance = planar_distance(player, target);
    let progress = old_distance - current_distance;
    let elapsed = facts
        .simulation_tick
        .saturating_sub(oldest.simulation_tick)
        .max(1) as f32;
    let displacement = [
        player[0] - oldest_position[0],
        player[2] - oldest_position[2],
    ];
    let goal_delta = [
        target[0] - oldest_position[0],
        target[2] - oldest_position[2],
    ];
    let displacement_length = displacement[0].hypot(displacement[1]);
    let goal_length = goal_delta[0].hypot(goal_delta[1]);
    let alignment = if displacement_length > f32::EPSILON && goal_length > f32::EPSILON {
        ((displacement[0] * goal_delta[0] + displacement[1] * goal_delta[1])
            / (displacement_length * goal_length))
            .clamp(-1.0, 1.0)
    } else {
        0.0
    };
    let closing_speed = facts
        .player
        .velocity_f32_bits
        .map(finite_vec3)
        .transpose()?
        .map_or(0.0, |velocity| {
            if current_distance > f32::EPSILON {
                (velocity[0] * (target[0] - player[0]) + velocity[2] * (target[2] - player[2]))
                    / current_distance
            } else {
                0.0
            }
        });
    Ok([progress, progress / elapsed, alignment, closing_speed])
}

fn commanded_motion(pad: crate::fact_snapshot::PadFactSnapshot) -> bool {
    f32::from(pad.stick_x).hypot(f32::from(pad.stick_y)) >= 32.0
}

fn planar_distance(left: [f32; 3], right: [f32; 3]) -> f32 {
    (right[0] - left[0]).hypot(right[2] - left[2])
}

fn goal_conditioned_feature_schema_digest(base: Digest) -> Digest {
    let bytes = serde_json::to_vec(&(
        GOAL_CONDITIONED_TACTIC_FEATURE_SCHEMA_V6,
        base,
        GOAL_FEATURE_NAMES,
    ))
    .expect("fixed goal-conditioned feature schema serializes");
    Digest(Sha256::digest(bytes).into())
}

fn encode_actor_summary(
    output: &mut Vec<f32>,
    facts: &FactSnapshot,
    player: [f32; 3],
) -> Result<(), TacticFeatureError> {
    let actor_count = facts.actors.len();
    let mut same_room = 0_usize;
    let mut portable = 0_usize;
    let mut nearest: Option<(f32, i16, [f32; 3])> = None;
    let mut relative_sum = [0.0_f64; 3];
    let mut speed_sum = 0.0_f64;
    let mut health_sum = 0.0_f64;
    let mut health_count = 0_usize;
    for actor in &facts.actors {
        same_room += usize::from(actor.current_room == facts.world.room);
        portable += usize::from(actor.portable_selector.is_some());
        let position = finite_vec3(actor.position_f32_bits)?;
        let relative = [
            position[0] - player[0],
            position[1] - player[1],
            position[2] - player[2],
        ];
        let distance =
            (relative[0] * relative[0] + relative[1] * relative[1] + relative[2] * relative[2])
                .sqrt();
        if nearest
            .as_ref()
            .is_none_or(|(best, name, _)| (distance, actor.actor_name) < (*best, *name))
        {
            nearest = Some((distance, actor.actor_name, relative));
        }
        for (sum, value) in relative_sum.iter_mut().zip(relative) {
            *sum += f64::from(value);
        }
        if let Some(bits) = actor.velocity_f32_bits {
            let velocity = finite_vec3(bits)?;
            speed_sum += f64::from(
                (velocity[0] * velocity[0] + velocity[1] * velocity[1] + velocity[2] * velocity[2])
                    .sqrt(),
            );
        }
        if let Some(health) = actor.health {
            health_sum += f64::from(health);
            health_count += 1;
        }
    }
    let count = actor_count.max(1) as f64;
    output.extend([
        actor_count as f32,
        same_room as f32,
        if actor_count == 0 {
            0.0
        } else {
            portable as f32 / actor_count as f32
        },
    ]);
    match nearest {
        Some((distance, name, relative)) => output.extend([
            1.0,
            symbol_feature(&name.to_string()),
            relative[0],
            relative[1],
            relative[2],
            distance,
        ]),
        None => output.extend([0.0; 6]),
    }
    output.extend([
        (relative_sum[0] / count) as f32,
        (relative_sum[1] / count) as f32,
        (relative_sum[2] / count) as f32,
        (speed_sum / count) as f32,
        if actor_count == 0 {
            0.0
        } else {
            health_count as f32 / actor_count as f32
        },
        if health_count == 0 {
            0.0
        } else {
            (health_sum / health_count as f64) as f32
        },
    ]);
    Ok(())
}

fn encode_bank(output: &mut Vec<f32>, bank: &ByteBankFactSnapshot) {
    let available = bank.availability == FactAvailability::Available;
    output.extend([
        bool_feature(available),
        if available {
            bank.bytes.iter().map(|byte| byte.count_ones()).sum::<u32>() as f32
        } else {
            0.0
        },
    ]);
}

fn encode_history(
    output: &mut Vec<f32>,
    facts: &FactSnapshot,
    player: [f32; 3],
) -> Result<(), TacticFeatureError> {
    let Some(previous) = facts.recent_history.last() else {
        output.extend([0.0; 9]);
        return Ok(());
    };
    let old_position = finite_vec3(previous.player_position_f32_bits)?;
    output.extend([
        1.0,
        facts
            .simulation_tick
            .saturating_sub(previous.simulation_tick) as f32,
        bool_feature(previous.stage != facts.world.stage),
        bool_feature(previous.room != facts.world.room),
        player[0] - old_position[0],
        player[1] - old_position[1],
        player[2] - old_position[2],
        bool_feature(facts.player.procedure != Some(previous.player_procedure)),
        bool_feature(
            facts.event.as_ref().is_some_and(|event| event.running) != previous.event_running,
        ),
    ]);
    Ok(())
}

fn channel_counts(facts: &FactSnapshot) -> [u32; 4] {
    let channels = [
        facts.channels.camera,
        facts.channels.player_action,
        facts.channels.background_collision,
        facts.channels.collision_surfaces,
        facts.channels.scene_exit,
        facts.channels.dynamic_colliders,
        facts.channels.player_resources,
        facts.channels.player_relationships,
        facts.channels.collision_solver,
        facts.channels.process_lifecycle,
        facts.channels.event_transition,
        facts.channels.room_load,
        facts.channels.warp_session,
        facts.channels.resource_loads,
    ];
    let mut counts = [0_u32; 4];
    for channel in channels {
        counts[match channel {
            FactAvailability::Available => 0,
            FactAvailability::Absent => 1,
            FactAvailability::Unavailable => 2,
            FactAvailability::NotSampled => 3,
        }] += 1;
    }
    counts
}

fn push_optional_bool(output: &mut Vec<f32>, value: Option<bool>) {
    match value {
        Some(value) => output.extend([1.0, bool_feature(value)]),
        None => output.extend([0.0, 0.0]),
    }
}

fn push_optional_i8(output: &mut Vec<f32>, value: Option<i8>) {
    match value {
        Some(value) => output.extend([1.0, f32::from(value)]),
        None => output.extend([0.0, 0.0]),
    }
}

fn push_optional_i16(output: &mut Vec<f32>, value: Option<i16>) {
    match value {
        Some(value) => output.extend([1.0, f32::from(value)]),
        None => output.extend([0.0, 0.0]),
    }
}

fn push_optional_u8(output: &mut Vec<f32>, value: Option<u8>) {
    match value {
        Some(value) => output.extend([1.0, f32::from(value)]),
        None => output.extend([0.0, 0.0]),
    }
}

fn push_optional_u16(output: &mut Vec<f32>, value: Option<u16>) {
    match value {
        Some(value) => output.extend([1.0, f32::from(value)]),
        None => output.extend([0.0, 0.0]),
    }
}

fn push_optional_u32(output: &mut Vec<f32>, value: Option<u32>) {
    match value {
        Some(value) => output.extend([1.0, value as f32]),
        None => output.extend([0.0, 0.0]),
    }
}

fn push_optional_f32_bits(
    output: &mut Vec<f32>,
    value: Option<u32>,
) -> Result<(), TacticFeatureError> {
    match value {
        Some(bits) => output.extend([1.0, finite_f32(bits)?]),
        None => output.extend([0.0, 0.0]),
    }
    Ok(())
}

fn push_optional_symbol(output: &mut Vec<f32>, value: Option<&str>) {
    match value {
        Some(value) => output.extend([1.0, symbol_feature(value)]),
        None => output.extend([0.0, 0.0]),
    }
}

fn ratio(numerator: Option<u16>, denominator: Option<u16>) -> f32 {
    match (numerator, denominator) {
        (Some(numerator), Some(denominator)) if denominator != 0 => {
            f32::from(numerator) / f32::from(denominator)
        }
        _ => 0.0,
    }
}

fn optional_bool_feature(value: Option<bool>) -> f32 {
    value.map_or(0.0, bool_feature)
}

fn bool_feature(value: bool) -> f32 {
    if value { 1.0 } else { 0.0 }
}

fn finite_vec3(bits: [u32; 3]) -> Result<[f32; 3], TacticFeatureError> {
    Ok([
        finite_f32(bits[0])?,
        finite_f32(bits[1])?,
        finite_f32(bits[2])?,
    ])
}

fn finite_f32(bits: u32) -> Result<f32, TacticFeatureError> {
    let value = f32::from_bits(bits);
    if value.is_finite() {
        Ok(value)
    } else {
        Err(TacticFeatureError::NonFinite)
    }
}

fn finite_vec2(bits: [u32; 2]) -> Result<[f32; 2], TacticFeatureError> {
    Ok([finite_f32(bits[0])?, finite_f32(bits[1])?])
}

fn symbol_feature(value: &str) -> f32 {
    let digest = Sha256::digest(value.as_bytes());
    let bucket = u32::from_le_bytes(digest[..4].try_into().unwrap());
    bucket as f32 / u32::MAX as f32
}

fn feature_schema_digest(names: &[&str]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(TACTIC_FEATURE_SCHEMA_V6.as_bytes());
    for name in names {
        hasher.update((name.len() as u64).to_le_bytes());
        hasher.update(name.as_bytes());
    }
    Digest(hasher.finalize().into())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TacticFeatureError {
    Facts(FactSnapshotError),
    InvalidEncoder,
    InvalidOutput,
    NonFinite,
}

impl fmt::Display for TacticFeatureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Facts(error) => write!(formatter, "tactic facts are invalid: {error}"),
            Self::InvalidEncoder => formatter.write_str("tactic feature encoder is detached"),
            Self::InvalidOutput => formatter.write_str("tactic feature output has invalid shape"),
            Self::NonFinite => formatter.write_str("tactic facts contain a non-finite number"),
        }
    }
}

impl Error for TacticFeatureError {}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_evidence::native_episode_shard::NativeEpisodeShard;

    #[test]
    fn route_agnostic_features_are_fixed_finite_and_ignore_absolute_replay_position() {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        let mut facts = FactSnapshot::from_native_learning(
            &shard.episodes[0].steps[0].pre_input,
            &[],
            None,
            Vec::new(),
        )
        .unwrap();
        let encoder = TacticFeatureEncoder::new();
        let baseline = encoder.encode(&facts).unwrap();
        assert_eq!(baseline.len(), encoder.feature_width());
        assert!(baseline.iter().all(|value| value.is_finite()));

        facts.boundary_index += 100;
        facts.simulation_tick += 100;
        facts.tape_frame += 100;
        let shifted = encoder.encode(&facts).unwrap();
        assert_eq!(baseline, shifted);

        facts.player.position_f32_bits[0] = 123.0_f32.to_bits();
        assert_ne!(baseline, encoder.encode(&facts).unwrap());
    }

    #[test]
    fn actor_summary_is_permutation_invariant() {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        let facts = FactSnapshot::from_native_learning(
            &shard.episodes[0].steps[0].pre_input,
            &[],
            None,
            Vec::new(),
        )
        .unwrap();
        let mut reversed = facts.clone();
        reversed.actors.reverse();
        let encoder = TacticFeatureEncoder::new();
        assert_eq!(
            encoder.encode(&facts).unwrap(),
            encoder.encode(&reversed).unwrap()
        );
    }

    #[test]
    fn player_action_features_expose_prompt_availability_phase_and_flags() {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        let mut facts = FactSnapshot::from_native_learning(
            &shard.episodes[0].steps[0].pre_input,
            &[],
            None,
            Vec::new(),
        )
        .unwrap();
        facts.player.action_state = Some(crate::fact_snapshot::PlayerActionFactSnapshot {
            procedure_context_raw: [1, 2, 3, 4, 5, 6],
            damage_wait_timer: 7,
            sword_at_up_time: 8,
            ice_damage_wait_timer: 9,
            sword_change_wait_timer: 10,
            flags: (1 << 3) | (1 << 19),
            do_status: 4,
        });
        facts.player.action_lanes = vec![
            crate::fact_snapshot::ActionLaneFactSnapshot {
                resource_id: 11,
                frame_f32_bits: 1.5_f32.to_bits(),
            },
            crate::fact_snapshot::ActionLaneFactSnapshot {
                resource_id: 12,
                frame_f32_bits: 3.5_f32.to_bits(),
            },
        ];
        let encoder = TacticFeatureEncoder::new();
        let encoded = encoder.encode(&facts).unwrap();
        let feature = |name: &str| {
            encoded[encoder
                .feature_names
                .iter()
                .position(|candidate| candidate == name)
                .unwrap()]
        };

        assert_eq!(feature("player_action_available"), 1.0);
        assert_eq!(feature("player_do_prompt_available"), 1.0);
        assert_eq!(feature("player_do_status"), 4.0);
        assert_eq!(feature("player_do_status_bit_2"), 1.0);
        assert_eq!(feature("player_do_status_bit_3"), 0.0);
        assert_eq!(feature("player_front_roll_prompt_available"), 0.0);
        assert_eq!(feature("player_procedure_context_5"), 6.0);
        assert_eq!(feature("player_action_lane_count"), 2.0);
        assert_eq!(feature("player_action_lane_frame_min"), 1.5);
        assert_eq!(feature("player_action_lane_frame_mean"), 2.5);
        assert_eq!(feature("player_action_lane_frame_max"), 3.5);
        assert_eq!(feature("player_action_flag_3"), 1.0);
        assert_eq!(feature("player_action_flag_19"), 1.0);
        assert_eq!(feature("player_action_flag_20"), 0.0);

        facts.player.action_state.as_mut().unwrap().do_status = FRONT_ROLL_DO_STATUS;
        let front_roll = encoder.encode(&facts).unwrap();
        let feature = |name: &str| {
            front_roll[encoder
                .feature_names
                .iter()
                .position(|candidate| candidate == name)
                .unwrap()]
        };
        assert_eq!(feature("player_do_status"), 121.0);
        assert_eq!(feature("player_do_status_bit_0"), 1.0);
        assert_eq!(feature("player_do_status_bit_1"), 0.0);
        assert_eq!(feature("player_do_status_bit_3"), 1.0);
        assert_eq!(feature("player_do_status_bit_7"), 0.0);
        assert_eq!(feature("player_front_roll_prompt_available"), 1.0);
    }

    #[test]
    fn previous_input_and_terminal_missingness_are_explicit_model_state() {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        let mut facts = FactSnapshot::from_native_learning(
            &shard.episodes[0].steps[0].pre_input,
            &[],
            None,
            Vec::new(),
        )
        .unwrap();
        facts.player.previous_pad = Some(crate::fact_snapshot::PadFactSnapshot {
            buttons: 0x0140,
            stick_x: -100,
            stick_y: 50,
            substick_x: 25,
            substick_y: -75,
            trigger_left: 9,
            trigger_right: 10,
            analog_a: 11,
            analog_b: 12,
            connected: true,
            error: -3,
        });
        facts.terminal.reached = Some(false);
        let encoder = TacticFeatureEncoder::new();
        let encoded = encoder.encode(&facts).unwrap();
        let feature = |name: &str| {
            encoded[encoder
                .feature_names
                .iter()
                .position(|candidate| candidate == name)
                .unwrap()]
        };
        assert_eq!(feature("previous_pad_available"), 1.0);
        assert_eq!(feature("previous_pad_button_6"), 1.0);
        assert_eq!(feature("previous_pad_button_8"), 1.0);
        assert_eq!(feature("previous_pad_button_7"), 0.0);
        assert_eq!(feature("previous_pad_stick_x"), -100.0);
        assert_eq!(feature("previous_pad_stick_y"), 50.0);
        assert_eq!(feature("previous_pad_substick_x"), 25.0);
        assert_eq!(feature("previous_pad_trigger_right"), 10.0);
        assert_eq!(feature("previous_pad_analog_b"), 12.0);
        assert_eq!(feature("previous_pad_connected"), 1.0);
        assert_eq!(feature("previous_pad_error"), -3.0);
        assert_eq!(feature("terminal_available"), 1.0);
        assert_eq!(feature("terminal_reached"), 0.0);

        facts.player.previous_pad = None;
        facts.terminal.reached = None;
        let missing = encoder.encode(&facts).unwrap();
        let missing_feature = |name: &str| {
            missing[encoder
                .feature_names
                .iter()
                .position(|candidate| candidate == name)
                .unwrap()]
        };
        assert_eq!(missing_feature("previous_pad_available"), 0.0);
        assert_eq!(missing_feature("previous_pad_button_8"), 0.0);
        assert_eq!(missing_feature("terminal_available"), 0.0);
        assert_eq!(missing_feature("terminal_reached"), 0.0);
    }

    #[test]
    fn goal_conditioned_features_measure_objective_relative_progress() {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        let facts = FactSnapshot::from_native_learning(
            &shard.episodes[0].steps[0].pre_input,
            &[],
            None,
            Vec::new(),
        )
        .unwrap();
        let player = facts.player.position_f32_bits.map(f32::from_bits);
        let target = [player[0] + 30.0, player[1] + 7.0, player[2] + 40.0];
        let encoder = GoalConditionedTacticFeatureEncoder::new(target).unwrap();
        let encoded = encoder.encode(&facts).unwrap();

        assert_eq!(
            encoded.len(),
            TacticFeatureEncoder::new().feature_width() + GOAL_FEATURE_NAMES.len()
        );
        assert_eq!(
            encoded[encoder.goal_distance_feature()].to_bits(),
            50.0_f32.to_bits()
        );
        assert_ne!(
            encoder.schema_sha256,
            TacticFeatureEncoder::new().schema_sha256
        );
        assert!(GoalConditionedTacticFeatureEncoder::new([f32::NAN, 0.0, 0.0]).is_err());
    }

    #[test]
    fn distance_weights_are_balanced_by_semantic_family() {
        let encoder = GoalConditionedTacticFeatureEncoder::new([0.0; 3]).unwrap();
        let weights = encoder.distance_weights();
        assert_eq!(weights.len(), encoder.feature_width());
        let group_weight = |prefix: &str| {
            encoder
                .feature_names
                .iter()
                .zip(&weights)
                .filter(|(name, _)| name.starts_with(prefix))
                .map(|(_, weight)| *weight)
                .sum::<f32>()
        };

        assert!((group_weight("player_action_flag_") - 1.0).abs() < 1.0e-6);
        assert!((group_weight("player_procedure_context_") - 1.0).abs() < 1.0e-6);
        assert!((group_weight("player_action_lane_") - 1.0).abs() < 1.0e-6);
        assert!(
            weights
                .iter()
                .all(|weight| weight.is_finite() && *weight > 0.0)
        );
    }

    #[test]
    fn trajectory_features_summarize_recent_native_motion() {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        let mut facts = FactSnapshot::from_native_learning(
            &shard.episodes[0].steps[0].pre_input,
            &[],
            None,
            Vec::new(),
        )
        .unwrap();
        let mut commanded_pad = facts.player.previous_pad.unwrap();
        commanded_pad.stick_x = 127;
        commanded_pad.stick_y = 0;
        facts.boundary_index = 4;
        facts.simulation_tick = 4;
        facts.tape_frame = 4;
        facts.player.position_f32_bits = [30.0_f32.to_bits(), 0.0_f32.to_bits(), 0.0_f32.to_bits()];
        facts.player.previous_pad = Some(commanded_pad);
        facts.recent_history = [0.0_f32, 10.0, 20.0]
            .into_iter()
            .enumerate()
            .map(|(index, x)| crate::fact_snapshot::HistoryFactSnapshot {
                boundary_index: index as u64 + 1,
                simulation_tick: index as u64 + 1,
                tape_frame: index as u64 + 1,
                stage: facts.world.stage.clone(),
                room: facts.world.room,
                player_position_f32_bits: [x.to_bits(), 0.0_f32.to_bits(), 0.0_f32.to_bits()],
                player_procedure: facts.player.procedure.unwrap(),
                event_running: facts.event.as_ref().is_some_and(|event| event.running),
                previous_pad: commanded_pad,
            })
            .collect();
        facts.recent_option = Some(crate::fact_snapshot::RecentOptionFactSnapshot {
            option_id: "move".into(),
            end_reason: dusklight_control::option_execution::OptionEndReason::Completed,
            realized_ticks: 4,
            tape_start: 0,
            tape_end_exclusive: 4,
            trajectory: Some(crate::fact_snapshot::OptionTrajectoryFactSnapshot {
                observed_ticks: 4,
                commanded_motion_ticks: 4,
                commanded_stall_ticks: 1,
                wall_contact_ticks: 2,
                collision_correction_ticks: 0,
                world_transition_ticks: 0,
                planar_path_length_f32_bits: 30.0_f32.to_bits(),
                planar_displacement_f32_bits: 30.0_f32.to_bits(),
                mean_planar_speed_f32_bits: 7.5_f32.to_bits(),
                final_planar_velocity_f32_bits: 8.0_f32.to_bits(),
                maximum_planar_velocity_f32_bits: 10.0_f32.to_bits(),
                commanded_momentum_loss_f32_bits: 4.0_f32.to_bits(),
                wall_contact_commanded_motion_ticks: Some(2),
                wall_contact_commanded_momentum_loss_f32_bits: Some(3.0_f32.to_bits()),
                collision_correction_total_f32_bits: 0.0_f32.to_bits(),
            }),
        });
        let encoder = TacticFeatureEncoder::new();
        let encoded = encoder.encode(&facts).unwrap();
        let feature = |name: &str| {
            let index = encoder
                .feature_names
                .iter()
                .position(|candidate| candidate == name)
                .unwrap();
            encoded[index]
        };

        assert_eq!(feature("trajectory_available"), 1.0);
        assert_eq!(feature("trajectory_elapsed_ticks"), 3.0);
        assert_eq!(feature("trajectory_planar_path_length"), 30.0);
        assert_eq!(feature("trajectory_planar_displacement"), 30.0);
        assert_eq!(feature("trajectory_straightness"), 1.0);
        assert_eq!(feature("trajectory_mean_planar_speed"), 10.0);
        assert_eq!(feature("trajectory_commanded_fraction"), 1.0);
        assert_eq!(feature("trajectory_stalled_command_fraction"), 0.0);
        assert_eq!(feature("trajectory_speed_retention"), 1.0);
        assert_eq!(feature("recent_option_contact_slowdown_available"), 1.0);
        assert_eq!(feature("recent_option_contact_commanded_fraction"), 0.5);
        assert_eq!(
            feature("recent_option_contact_momentum_loss_per_command_tick"),
            1.5
        );
    }
}
