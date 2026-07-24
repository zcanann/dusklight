//! Fixed, route-agnostic features for tactic-level value learning.
//!
//! The encoder consumes only the shared `FactSnapshot`. It deliberately omits
//! absolute tape, simulation, and boundary indices so a critic cannot mistake
//! replay position for gameplay progress.

use crate::artifact::Digest;
use crate::fact_snapshot::{
    ByteBankFactSnapshot, FactAvailability, FactSnapshot, FactSnapshotError,
};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::f32::consts::PI;
use std::fmt;

pub const TACTIC_FEATURE_SCHEMA_V1: &str = "dusklight-tactic-features/v1";

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
    "player_contacts_available",
    "player_contacts",
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
    "recent_option_available",
    "recent_option_ticks",
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
            schema: TACTIC_FEATURE_SCHEMA_V1.into(),
            schema_sha256,
            feature_names,
        }
    }

    pub fn feature_width(&self) -> usize {
        self.feature_names.len()
    }

    pub fn encode(&self, facts: &FactSnapshot) -> Result<Vec<f32>, TacticFeatureError> {
        facts.validate().map_err(TacticFeatureError::Facts)?;
        if self.schema != TACTIC_FEATURE_SCHEMA_V1
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
        push_optional_u8(&mut output, facts.player.contacts);
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
        match &facts.recent_option {
            Some(option) => output.extend([1.0, option.realized_ticks as f32]),
            None => output.extend([0.0, 0.0]),
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

fn symbol_feature(value: &str) -> f32 {
    let digest = Sha256::digest(value.as_bytes());
    let bucket = u32::from_le_bytes(digest[..4].try_into().unwrap());
    bucket as f32 / u32::MAX as f32
}

fn feature_schema_digest(names: &[&str]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(TACTIC_FEATURE_SCHEMA_V1.as_bytes());
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
}
