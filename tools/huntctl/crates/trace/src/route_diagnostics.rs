//! Numeric, read-only route diagnostics derived from authenticated gameplay traces.

use crate::trace::{DecodedTrace, TraceChannel, TraceChannelStatus, TraceRecord};
use serde::Serialize;
use std::error::Error;
use std::f64::consts::PI;
use std::fmt;

pub const ROUTE_DIAGNOSTICS_SCHEMA_V1: &str = "dusklight-route-diagnostics/v1";
const FRONT_ROLL_PROCEDURE: u16 = 0x000e;
const BUTTON_A: u16 = 0x0100;

#[derive(Clone, Debug)]
pub struct RouteDiagnosticsConfig {
    pub source_boundary_frame: u64,
    pub terminal_frame: u64,
    /// Minimum absolute per-tick player-yaw change included in a turn episode.
    pub corner_yaw_threshold_s16: u16,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteDiagnostics {
    pub schema: &'static str,
    pub source_boundary_frame: u64,
    pub first_candidate_frame: u64,
    pub terminal_frame: u64,
    pub candidate_tick_count: u64,
    pub source_position: [f32; 3],
    pub terminal_position: [f32; 3],
    pub horizontal_path_distance: f64,
    pub horizontal_direct_distance: f64,
    pub horizontal_excess_distance: f64,
    pub horizontal_path_efficiency: f64,
    pub mean_facing_velocity_error_degrees: f64,
    pub maximum_facing_velocity_error_degrees: f64,
    pub total_absolute_yaw_change_degrees: f64,
    pub collision_correction_frames: u64,
    pub horizontal_collision_loss: f64,
    pub horizontal_collision_correction: f64,
    pub corner_yaw_threshold_s16: u16,
    pub corner_tick_count: u64,
    pub longest_corner_ticks: u64,
    pub corner_episodes: Vec<FrameEpisode>,
    pub roll_episodes: Vec<FrameEpisode>,
    pub roll_start_spacing_ticks: Vec<u64>,
    pub action_press_edges: Vec<u64>,
    pub action_press_to_roll_start_ticks: Vec<Option<u64>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FrameEpisode {
    pub start_frame: u64,
    pub end_frame: u64,
    pub ticks: u64,
}

pub fn analyze_route(
    trace: &DecodedTrace,
    config: &RouteDiagnosticsConfig,
) -> Result<RouteDiagnostics, RouteDiagnosticsError> {
    if config.terminal_frame <= config.source_boundary_frame || config.corner_yaw_threshold_s16 == 0
    {
        return Err(diagnostics_error(
            "route diagnostic boundaries and corner threshold must be positive and ordered",
        ));
    }
    if trace.capacity_exhausted || trace.retention.is_some() {
        return Err(diagnostics_error(
            "route diagnostics require a complete, unretained gameplay trace",
        ));
    }
    for channel in [
        TraceChannel::AppliedPads,
        TraceChannel::PlayerMotion,
        TraceChannel::Camera,
        TraceChannel::PlayerAction,
        TraceChannel::PlayerBackgroundCollision,
        TraceChannel::GoalProgress,
    ] {
        if !trace.requests(channel) {
            return Err(diagnostics_error(format!(
                "route trace did not request required channel {}",
                channel.name()
            )));
        }
    }

    let source_index = record_index(trace, config.source_boundary_frame)?;
    let terminal_index = record_index(trace, config.terminal_frame)?;
    if terminal_index <= source_index {
        return Err(diagnostics_error(
            "terminal record does not follow source boundary record",
        ));
    }
    let records = &trace.records[source_index..=terminal_index];
    for record in records {
        require_record_channels(record)?;
        if !record.player_present() || !record.player_is_link() {
            return Err(diagnostics_error(format!(
                "Link is not present at route frame {:?}",
                record.tape_frame
            )));
        }
    }
    let source = &records[0];
    let terminal = records.last().expect("route window is nonempty");

    let mut path_distance = 0.0;
    let mut facing_errors = Vec::new();
    let mut total_yaw_change = 0_u64;
    let mut corner_frames = Vec::new();
    let mut collision_correction_frames = 0_u64;
    let mut collision_loss = 0.0;
    let mut collision_correction = 0.0;
    for pair in records.windows(2) {
        let prior = &pair[0];
        let record = &pair[1];
        let dx = f64::from(record.position[0] - prior.position[0]);
        let dz = f64::from(record.position[2] - prior.position[2]);
        let distance = dx.hypot(dz);
        path_distance += distance;
        if distance > 1.0e-6 {
            let movement_yaw = dx.atan2(dz);
            let facing_yaw = s16_radians(record.current_angle_y);
            facing_errors.push(radians_degrees(
                angle_difference(movement_yaw, facing_yaw).abs(),
            ));
        }

        let yaw_delta = s16_delta(prior.current_angle_y, record.current_angle_y);
        let absolute_yaw_delta = u64::from(yaw_delta.unsigned_abs());
        total_yaw_change = total_yaw_change.saturating_add(absolute_yaw_delta);
        if absolute_yaw_delta >= u64::from(config.corner_yaw_threshold_s16) {
            corner_frames.push(record.tape_frame.expect("window records have tape frames"));
        }

        let collision = record
            .player_background_collision
            .as_ref()
            .expect("required collision channel is present");
        let intended_x = f64::from(record.velocity[0]);
        let intended_z = f64::from(record.velocity[2]);
        let resolved_x = f64::from(collision.resolved_frame_displacement[0]);
        let resolved_z = f64::from(collision.resolved_frame_displacement[2]);
        let intended_distance = intended_x.hypot(intended_z);
        let resolved_distance = resolved_x.hypot(resolved_z);
        collision_loss += (intended_distance - resolved_distance).max(0.0);
        let correction = (resolved_x - intended_x).hypot(resolved_z - intended_z);
        collision_correction += correction;
        if correction > 0.01 {
            collision_correction_frames += 1;
        }
    }

    let direct_x = f64::from(terminal.position[0] - source.position[0]);
    let direct_z = f64::from(terminal.position[2] - source.position[2]);
    let direct_distance = direct_x.hypot(direct_z);
    let excess_distance = (path_distance - direct_distance).max(0.0);
    let path_efficiency = if path_distance > 0.0 {
        direct_distance / path_distance
    } else {
        1.0
    };
    let mean_facing_error = if facing_errors.is_empty() {
        0.0
    } else {
        facing_errors.iter().sum::<f64>() / facing_errors.len() as f64
    };
    let maximum_facing_error = facing_errors.into_iter().fold(0.0_f64, f64::max);

    let corner_episodes = episodes_from_frames(&corner_frames);
    let roll_episodes = front_roll_cycles(records);
    let roll_start_spacing_ticks = roll_episodes
        .windows(2)
        .map(|pair| pair[1].start_frame - pair[0].start_frame)
        .collect::<Vec<_>>();
    let mut action_press_edges = Vec::new();
    for pair in records.windows(2) {
        if pair[0].buttons & BUTTON_A == 0 && pair[1].buttons & BUTTON_A != 0 {
            action_press_edges.push(pair[1].tape_frame.expect("window records have tape frames"));
        }
    }
    let action_press_to_roll_start_ticks = roll_episodes
        .iter()
        .map(|roll| {
            action_press_edges
                .iter()
                .rev()
                .copied()
                .find(|press| *press <= roll.start_frame && roll.start_frame - *press <= 8)
                .map(|press| roll.start_frame - press)
        })
        .collect();

    Ok(RouteDiagnostics {
        schema: ROUTE_DIAGNOSTICS_SCHEMA_V1,
        source_boundary_frame: config.source_boundary_frame,
        first_candidate_frame: config.source_boundary_frame + 1,
        terminal_frame: config.terminal_frame,
        candidate_tick_count: config.terminal_frame - config.source_boundary_frame,
        source_position: source.position,
        terminal_position: terminal.position,
        horizontal_path_distance: path_distance,
        horizontal_direct_distance: direct_distance,
        horizontal_excess_distance: excess_distance,
        horizontal_path_efficiency: path_efficiency,
        mean_facing_velocity_error_degrees: mean_facing_error,
        maximum_facing_velocity_error_degrees: maximum_facing_error,
        total_absolute_yaw_change_degrees: total_yaw_change as f64 * 360.0 / 65_536.0,
        collision_correction_frames,
        horizontal_collision_loss: collision_loss,
        horizontal_collision_correction: collision_correction,
        corner_yaw_threshold_s16: config.corner_yaw_threshold_s16,
        corner_tick_count: corner_frames.len() as u64,
        longest_corner_ticks: corner_episodes
            .iter()
            .map(|episode| episode.ticks)
            .max()
            .unwrap_or(0),
        corner_episodes,
        roll_episodes,
        roll_start_spacing_ticks,
        action_press_edges,
        action_press_to_roll_start_ticks,
    })
}

fn record_index(trace: &DecodedTrace, frame: u64) -> Result<usize, RouteDiagnosticsError> {
    trace
        .records
        .binary_search_by_key(&Some(frame), |record| record.tape_frame)
        .map_err(|_| diagnostics_error(format!("trace has no record for tape frame {frame}")))
}

fn require_record_channels(record: &TraceRecord) -> Result<(), RouteDiagnosticsError> {
    for channel in [
        TraceChannel::AppliedPads,
        TraceChannel::PlayerMotion,
        TraceChannel::Camera,
        TraceChannel::PlayerAction,
        TraceChannel::PlayerBackgroundCollision,
        TraceChannel::GoalProgress,
    ] {
        if record.channel_status.get(&channel) != Some(&TraceChannelStatus::Present) {
            return Err(diagnostics_error(format!(
                "route frame {:?} does not contain required channel {}",
                record.tape_frame,
                channel.name()
            )));
        }
    }
    if record.applied_pads.is_none()
        || record.camera.is_none()
        || record.player_action.is_none()
        || record.player_background_collision.is_none()
        || record.goal_progress.is_none()
    {
        return Err(diagnostics_error(format!(
            "route frame {:?} omitted required channel payload",
            record.tape_frame
        )));
    }
    Ok(())
}

fn episodes_from_frames(frames: &[u64]) -> Vec<FrameEpisode> {
    let mut episodes = Vec::new();
    let Some(&first) = frames.first() else {
        return episodes;
    };
    let mut start = first;
    let mut end = first;
    for &frame in &frames[1..] {
        if frame == end + 1 {
            end = frame;
        } else {
            episodes.push(FrameEpisode {
                start_frame: start,
                end_frame: end,
                ticks: end - start + 1,
            });
            start = frame;
            end = frame;
        }
    }
    episodes.push(FrameEpisode {
        start_frame: start,
        end_frame: end,
        ticks: end - start + 1,
    });
    episodes
}

fn front_roll_cycles(records: &[TraceRecord]) -> Vec<FrameEpisode> {
    let mut starts = Vec::new();
    for pair in records.windows(2) {
        let prior = &pair[0];
        let record = &pair[1];
        if record.player_proc_id != Some(FRONT_ROLL_PROCEDURE) {
            continue;
        }
        let entered_roll = prior.player_proc_id != Some(FRONT_ROLL_PROCEDURE);
        let animation_restarted = match (&prior.player_action, &record.player_action) {
            (Some(prior), Some(current)) => {
                current.under_animations[0].resource_id == prior.under_animations[0].resource_id
                    && current.under_animations[0].frame + 0.01 < prior.under_animations[0].frame
            }
            _ => false,
        };
        if entered_roll || animation_restarted {
            starts.push(record.tape_frame.expect("window records have tape frames"));
        }
    }
    starts
        .iter()
        .enumerate()
        .map(|(index, start)| {
            let next_start = starts.get(index + 1).copied();
            let end = records
                .iter()
                .skip_while(|record| record.tape_frame.is_some_and(|frame| frame < *start))
                .take_while(|record| {
                    record.player_proc_id == Some(FRONT_ROLL_PROCEDURE)
                        && next_start
                            .is_none_or(|next| record.tape_frame.is_some_and(|frame| frame < next))
                })
                .filter_map(|record| record.tape_frame)
                .last()
                .unwrap_or(*start);
            FrameEpisode {
                start_frame: *start,
                end_frame: end,
                ticks: end - start + 1,
            }
        })
        .collect()
}

fn s16_delta(from: i16, to: i16) -> i16 {
    to.wrapping_sub(from)
}

fn s16_radians(value: i16) -> f64 {
    f64::from(value) * 2.0 * PI / 65_536.0
}

fn angle_difference(left: f64, right: f64) -> f64 {
    (left - right + PI).rem_euclid(2.0 * PI) - PI
}

fn radians_degrees(value: f64) -> f64 {
    value * 180.0 / PI
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteDiagnosticsError(String);

impl fmt::Display for RouteDiagnosticsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for RouteDiagnosticsError {}

fn diagnostics_error(message: impl Into<String>) -> RouteDiagnosticsError {
    RouteDiagnosticsError(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tape::{RawPadState, TapeBoot};
    use crate::trace::{
        TraceAppliedPads, TraceCamera, TraceGoalProgress, TracePhase, TracePlayerAction,
        TracePlayerBackgroundCollision,
    };
    use std::collections::BTreeMap;

    fn record(frame: u64, x: f32, z: f32, yaw: i16, procedure: u16, buttons: u16) -> TraceRecord {
        let mut statuses = BTreeMap::new();
        for channel in [
            TraceChannel::AppliedPads,
            TraceChannel::PlayerMotion,
            TraceChannel::Camera,
            TraceChannel::PlayerAction,
            TraceChannel::PlayerBackgroundCollision,
            TraceChannel::GoalProgress,
        ] {
            statuses.insert(channel, TraceChannelStatus::Present);
        }
        TraceRecord {
            boundary_index: frame + 1,
            simulation_tick: frame,
            tape_frame: Some(frame),
            observation_phase: TracePhase::PostSimulation,
            channel_status: statuses,
            flags: 3,
            player_session_process_id: Some(1),
            current_angle_y: yaw,
            shape_angle_y: yaw,
            current_angle: [0, yaw, 0],
            shape_angle: [0, yaw, 0],
            position: [x, 0.0, z],
            velocity: [x, 0.0, z],
            buttons,
            player_proc_id: Some(procedure),
            applied_pads: Some(TraceAppliedPads {
                valid_ports: 1,
                owned_ports: 1,
                pads: [RawPadState::default(); 4],
            }),
            camera: Some(TraceCamera {
                view_yaw: 0,
                controlled_yaw: 0,
                bank: 0,
                eye: [0.0; 3],
                center: [0.0; 3],
                up: [0.0, 1.0, 0.0],
                fovy: 60.0,
            }),
            player_action: Some(TracePlayerAction {
                procedure_id: procedure,
                mode_flags: 0,
                procedure_context_raw: [0; 6],
                damage_wait_timer: 0,
                sword_at_up_time: 0,
                ice_damage_wait_timer: 0,
                sword_change_wait_timer: 0,
                under_animations: std::array::from_fn(|_| crate::trace::TraceAnimationLane {
                    resource_id: u16::MAX,
                    frame: 0.0,
                    rate: 0.0,
                }),
                upper_animations: std::array::from_fn(|_| crate::trace::TraceAnimationLane {
                    resource_id: u16::MAX,
                    frame: 0.0,
                    rate: 0.0,
                }),
                do_status: 0,
                talk_partner: None,
                grabbed_actor: None,
            }),
            player_background_collision: Some(TracePlayerBackgroundCollision {
                flags: 0,
                ground_height: 0.0,
                roof_height: 0.0,
                water_height: 0.0,
                ground_bg_index: None,
                ground_poly_index: None,
                ground_owner_session_process_id: None,
                ground_plane: [0.0; 4],
                ground_identity_present: false,
                roof_bg_index: None,
                roof_poly_index: None,
                roof_owner_session_process_id: None,
                roof_identity_present: false,
                water_bg_index: None,
                water_poly_index: None,
                water_owner_session_process_id: None,
                water_identity_present: false,
                walls: std::array::from_fn(|_| crate::trace::TraceCollisionWall {
                    identity_present: false,
                    bg_index: None,
                    poly_index: None,
                    owner_session_process_id: None,
                    angle_y: 0,
                    flags: 0,
                }),
                old_position: [0.0; 3],
                resolved_frame_displacement: [x, 0.0, z],
                final_position: [x, 0.0, z],
                solver: None,
            }),
            goal_progress: Some(TraceGoalProgress {
                configured: true,
                reached: false,
                authored: true,
                goal_name_hash: Some(1),
                requested_count: 1,
                hit_count: 0,
                stable_ticks: 1,
                consecutive_ticks: 0,
                sequence_steps: 0,
                sequence_next_step: 0,
                sequence_within_ticks: 0,
                sequence_elapsed_ticks: 0,
                first_hit_tick: None,
            }),
            ..TraceRecord::default()
        }
    }

    #[test]
    fn reports_path_turn_collision_and_roll_timing() {
        let records = vec![
            record(10, 0.0, 0.0, 0, 4, 0),
            record(11, 0.0, 1.0, 0, 14, BUTTON_A),
            record(12, 0.0, 2.0, 1024, 14, 0),
            record(13, 1.0, 2.0, 16_384, 4, 0),
            record(14, 2.0, 2.0, 16_384, 14, BUTTON_A),
        ];
        let mut requested_channels = 0;
        for channel in [
            TraceChannel::AppliedPads,
            TraceChannel::PlayerMotion,
            TraceChannel::Camera,
            TraceChannel::PlayerAction,
            TraceChannel::PlayerBackgroundCollision,
            TraceChannel::GoalProgress,
        ] {
            requested_channels |= channel.bit();
        }
        let trace = DecodedTrace {
            version: 5,
            boot: TapeBoot::Process,
            tick_rate_numerator: 30,
            tick_rate_denominator: 1,
            requested_channels,
            capacity_exhausted: false,
            retention: None,
            channel_formats: BTreeMap::new(),
            records,
        };
        let report = analyze_route(
            &trace,
            &RouteDiagnosticsConfig {
                source_boundary_frame: 10,
                terminal_frame: 14,
                corner_yaw_threshold_s16: 512,
            },
        )
        .unwrap();
        assert_eq!(report.candidate_tick_count, 4);
        assert_eq!(report.corner_tick_count, 2);
        assert_eq!(report.roll_episodes.len(), 2);
        assert_eq!(report.roll_start_spacing_ticks, vec![3]);
        assert_eq!(report.action_press_edges, vec![11, 14]);
        assert_eq!(
            report.action_press_to_roll_start_ticks,
            vec![Some(0), Some(0)]
        );
        assert!(report.horizontal_excess_distance > 1.0);
    }
}
