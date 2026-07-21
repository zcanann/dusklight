//! Phase-correct, bounded collision history derived from authenticated episodes.
//!
//! Decision rows contain only the current pre-input snapshot and transitions
//! completed before that decision. Realized pre-to-post transitions are kept in
//! a separate auxiliary-target table so callers cannot accidentally treat
//! future collision state as an input feature.

use crate::artifact::Digest;
use dusklight_evidence::native_episode_shard::{
    NativeChannelStatus, NativeEpisodeShard, NativeLearningObservation, NativeRawPad,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const NATIVE_COLLISION_HISTORY_SCHEMA_V2: &str = "dusklight-native-collision-history/v2";
pub const DEFAULT_COLLISION_HISTORY_DEPTH: usize = 4;
pub const MAX_COLLISION_HISTORY_DEPTH: usize = 32;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CollisionChannelStatus {
    NotSampled,
    Unavailable,
    Absent,
    Present,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CollisionHistoryPad {
    pub buttons: u16,
    pub stick_x: i8,
    pub stick_y: i8,
    pub substick_x: i8,
    pub substick_y: i8,
    pub trigger_left: u8,
    pub trigger_right: u8,
    pub analog_a: u8,
    pub analog_b: u8,
    pub connected: bool,
    pub error: i8,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CollisionBackgroundState {
    pub flags: u32,
    pub ground_height: f32,
    pub roof_height: f32,
    pub water_height: f32,
    pub wall_flags: [u16; 3],
    pub wall_angles_y: [i16; 3],
    pub resolved_frame_displacement: [f32; 3],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CollisionSolverWallState {
    pub flags: u32,
    pub angle_y: i16,
    pub wall_radius_squared: f32,
    pub wall_height: f32,
    pub wall_radius: f32,
    pub direct_wall_height: f32,
    pub realized_center: [f32; 3],
    pub realized_radius: f32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CollisionSolverState {
    pub flags: u32,
    pub wall_table_size: i32,
    pub water_mode: u8,
    pub line_start: [f32; 3],
    pub line_end: [f32; 3],
    pub wall_cylinder_center: [f32; 3],
    pub wall_cylinder_radius: f32,
    pub wall_cylinder_height: f32,
    pub ground_check_offset: f32,
    pub roof_correction_height: f32,
    pub water_check_offset: f32,
    pub walls: [CollisionSolverWallState; 3],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CollisionSnapshot {
    pub boundary_index: u64,
    pub state_identity_xxh3_128: String,
    pub player_present: bool,
    pub player_contacts: Option<u8>,
    pub background_status: CollisionChannelStatus,
    pub background: Option<CollisionBackgroundState>,
    pub solver_status: CollisionChannelStatus,
    pub solver: Option<CollisionSolverState>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CollisionBackgroundDelta {
    pub flags_activated: u32,
    pub flags_cleared: u32,
    pub ground_height_delta: f32,
    pub roof_height_delta: f32,
    pub water_height_delta: f32,
    pub wall_flags_activated: [u16; 3],
    pub wall_flags_cleared: [u16; 3],
    pub wall_angle_delta_y: [i16; 3],
    pub resolved_displacement_delta: [f32; 3],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CollisionSolverWallDelta {
    pub flags_activated: u32,
    pub flags_cleared: u32,
    pub angle_delta_y: i16,
    pub wall_radius_squared_delta: f32,
    pub wall_height_delta: f32,
    pub wall_radius_delta: f32,
    pub direct_wall_height_delta: f32,
    pub realized_center_delta: [f32; 3],
    pub realized_radius_delta: f32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CollisionSolverDelta {
    pub flags_activated: u32,
    pub flags_cleared: u32,
    pub wall_table_size_delta: i64,
    pub water_mode_changed: bool,
    pub line_start_delta: [f32; 3],
    pub line_end_delta: [f32; 3],
    pub wall_cylinder_center_delta: [f32; 3],
    pub wall_cylinder_radius_delta: f32,
    pub wall_cylinder_height_delta: f32,
    pub ground_check_offset_delta: f32,
    pub roof_correction_height_delta: f32,
    pub water_check_offset_delta: f32,
    pub walls: [CollisionSolverWallDelta; 3],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CollisionTransitionDelta {
    pub player_contacts_activated: Option<u8>,
    pub player_contacts_cleared: Option<u8>,
    pub background: Option<CollisionBackgroundDelta>,
    pub solver: Option<CollisionSolverDelta>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CollisionTransitionTarget {
    pub episode_id: String,
    pub step_index: u32,
    pub consumed_pad: CollisionHistoryPad,
    pub before_snapshot_index: u32,
    pub after_snapshot_index: u32,
    pub delta: CollisionTransitionDelta,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CollisionHistoryDecision {
    pub episode_id: String,
    pub step_index: u32,
    pub current_snapshot_index: u32,
    /// Oldest to newest indices into `auxiliary_targets`. Every referenced
    /// transition is complete and strictly earlier than this decision.
    pub completed_transition_indices: Vec<u32>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCollisionHistoryView {
    pub schema: String,
    pub native_shard_sha256: Digest,
    pub observation_schema: String,
    pub history_depth: usize,
    pub snapshots: Vec<CollisionSnapshot>,
    pub decisions: Vec<CollisionHistoryDecision>,
    pub auxiliary_targets: Vec<CollisionTransitionTarget>,
    pub view_sha256: Digest,
}

impl NativeCollisionHistoryView {
    pub fn build(
        shard: &NativeEpisodeShard,
        history_depth: usize,
    ) -> Result<Self, NativeCollisionHistoryError> {
        if shard.content_sha256 == Digest::ZERO
            || shard.episodes.is_empty()
            || !(1..=MAX_COLLISION_HISTORY_DEPTH).contains(&history_depth)
        {
            return Err(NativeCollisionHistoryError::new(
                "collision history requires an authenticated shard and bounded nonzero depth",
            ));
        }
        let mut snapshots = Vec::new();
        let mut decisions = Vec::new();
        let mut auxiliary_targets = Vec::new();
        for episode in &shard.episodes {
            let mut completed = Vec::<u32>::new();
            let mut prior_after_snapshot_index = None;
            for (step_index, step) in episode.steps.iter().enumerate() {
                let step_index = u32::try_from(step_index)
                    .map_err(|_| NativeCollisionHistoryError::new("step index overflowed"))?;
                let before = snapshot(&step.pre_input)?;
                let after = snapshot(&step.post_simulation)?;
                let delta = transition_delta(&before, &after);
                let before_snapshot_index = if let Some(index) = prior_after_snapshot_index
                    && snapshots.get(index as usize) == Some(&before)
                {
                    index
                } else {
                    push_snapshot(&mut snapshots, before)?
                };
                let after_snapshot_index = push_snapshot(&mut snapshots, after)?;
                prior_after_snapshot_index = Some(after_snapshot_index);
                let target_index = u32::try_from(auxiliary_targets.len()).map_err(|_| {
                    NativeCollisionHistoryError::new("transition target index overflowed")
                })?;
                let retained_from = completed.len().saturating_sub(history_depth);
                decisions.push(CollisionHistoryDecision {
                    episode_id: episode.id.clone(),
                    step_index,
                    current_snapshot_index: before_snapshot_index,
                    completed_transition_indices: completed[retained_from..].to_vec(),
                });
                let consumed_pad = pad(step.consumed_pad);
                auxiliary_targets.push(CollisionTransitionTarget {
                    episode_id: episode.id.clone(),
                    step_index,
                    consumed_pad,
                    before_snapshot_index,
                    after_snapshot_index,
                    delta,
                });
                completed.push(target_index);
            }
        }
        let mut view = Self {
            schema: NATIVE_COLLISION_HISTORY_SCHEMA_V2.into(),
            native_shard_sha256: shard.content_sha256,
            observation_schema: shard.metadata.observation_schema.clone(),
            history_depth,
            snapshots,
            decisions,
            auxiliary_targets,
            view_sha256: Digest::ZERO,
        };
        view.view_sha256 = view.compute_identity()?;
        view.validate()?;
        Ok(view)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, NativeCollisionHistoryError> {
        self.validate()?;
        serde_json::to_vec(self)
            .map_err(|error| NativeCollisionHistoryError::new(error.to_string()))
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, NativeCollisionHistoryError> {
        let view: Self = serde_json::from_slice(bytes)
            .map_err(|error| NativeCollisionHistoryError::new(error.to_string()))?;
        view.validate()?;
        if view.canonical_bytes()? != bytes {
            return Err(NativeCollisionHistoryError::new(
                "collision history bytes are not canonical",
            ));
        }
        Ok(view)
    }

    pub fn resolved_history(
        &self,
        decision_index: usize,
    ) -> Result<Vec<&CollisionTransitionTarget>, NativeCollisionHistoryError> {
        self.validate()?;
        let decision = self.decisions.get(decision_index).ok_or_else(|| {
            NativeCollisionHistoryError::new("collision history decision index is out of range")
        })?;
        Ok(decision
            .completed_transition_indices
            .iter()
            .map(|index| &self.auxiliary_targets[*index as usize])
            .collect())
    }

    pub fn current_snapshot(
        &self,
        decision_index: usize,
    ) -> Result<&CollisionSnapshot, NativeCollisionHistoryError> {
        self.validate()?;
        let decision = self.decisions.get(decision_index).ok_or_else(|| {
            NativeCollisionHistoryError::new("collision history decision index is out of range")
        })?;
        Ok(&self.snapshots[decision.current_snapshot_index as usize])
    }

    pub fn validate(&self) -> Result<(), NativeCollisionHistoryError> {
        self.validate_content()?;
        if self.view_sha256 != self.compute_identity()? {
            return Err(NativeCollisionHistoryError::new(
                "collision history seal is invalid",
            ));
        }
        Ok(())
    }

    fn validate_content(&self) -> Result<(), NativeCollisionHistoryError> {
        if self.schema != NATIVE_COLLISION_HISTORY_SCHEMA_V2
            || self.native_shard_sha256 == Digest::ZERO
            || self.observation_schema.is_empty()
            || !(1..=MAX_COLLISION_HISTORY_DEPTH).contains(&self.history_depth)
            || self.snapshots.is_empty()
            || self.decisions.is_empty()
            || self.decisions.len() != self.auxiliary_targets.len()
        {
            return Err(NativeCollisionHistoryError::new(
                "collision history envelope is invalid",
            ));
        }
        let mut prior_episode: Option<&str> = None;
        let mut completed = Vec::<u32>::new();
        let mut prior_after_snapshot_index = None;
        for (target_index, (decision, target)) in self
            .decisions
            .iter()
            .zip(&self.auxiliary_targets)
            .enumerate()
        {
            if prior_episode != Some(decision.episode_id.as_str()) {
                completed.clear();
                prior_after_snapshot_index = None;
                prior_episode = Some(&decision.episode_id);
            }
            let retained_from = completed.len().saturating_sub(self.history_depth);
            let before = self
                .snapshots
                .get(target.before_snapshot_index as usize)
                .ok_or_else(|| {
                    NativeCollisionHistoryError::new("before snapshot index is invalid")
                })?;
            let after = self
                .snapshots
                .get(target.after_snapshot_index as usize)
                .ok_or_else(|| {
                    NativeCollisionHistoryError::new("after snapshot index is invalid")
                })?;
            if decision.episode_id.is_empty()
                || decision.episode_id != target.episode_id
                || decision.step_index != target.step_index
                || usize::try_from(decision.step_index).ok() != Some(completed.len())
                || decision.current_snapshot_index != target.before_snapshot_index
                || decision.completed_transition_indices != completed[retained_from..]
                || prior_after_snapshot_index
                    .is_some_and(|index| index != target.before_snapshot_index)
                || target.delta != transition_delta(before, after)
                || before.boundary_index >= after.boundary_index
            {
                return Err(NativeCollisionHistoryError::new(
                    "collision history ordering, phase, or auxiliary target is invalid",
                ));
            }
            validate_delta(&target.delta)?;
            prior_after_snapshot_index = Some(target.after_snapshot_index);
            completed.push(u32::try_from(target_index).map_err(|_| {
                NativeCollisionHistoryError::new("transition target index overflowed")
            })?);
        }
        Ok(())
    }

    fn compute_identity(&self) -> Result<Digest, NativeCollisionHistoryError> {
        self.validate_hashable_payload()?;
        let mut canonical = self.clone();
        canonical.view_sha256 = Digest::ZERO;
        let bytes = serde_json::to_vec(&canonical)
            .map_err(|error| NativeCollisionHistoryError::new(error.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.native-collision-history/v2\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }

    fn validate_hashable_payload(&self) -> Result<(), NativeCollisionHistoryError> {
        for snapshot in &self.snapshots {
            validate_snapshot(snapshot)?;
        }
        for target in &self.auxiliary_targets {
            validate_delta(&target.delta)?;
        }
        Ok(())
    }
}

fn push_snapshot(
    snapshots: &mut Vec<CollisionSnapshot>,
    snapshot: CollisionSnapshot,
) -> Result<u32, NativeCollisionHistoryError> {
    let index = u32::try_from(snapshots.len())
        .map_err(|_| NativeCollisionHistoryError::new("collision snapshot index overflowed"))?;
    snapshots.push(snapshot);
    Ok(index)
}

fn snapshot(
    source: &NativeLearningObservation,
) -> Result<CollisionSnapshot, NativeCollisionHistoryError> {
    let background =
        source
            .player_background_collision
            .as_ref()
            .map(|value| CollisionBackgroundState {
                flags: value.flags,
                ground_height: value.ground_height,
                roof_height: value.roof_height,
                water_height: value.water_height,
                wall_flags: value.walls.each_ref().map(|wall| wall.flags),
                wall_angles_y: value.walls.each_ref().map(|wall| wall.angle_y),
                resolved_frame_displacement: value.resolved_frame_displacement,
            });
    let solver = source
        .player_collision_solver
        .as_ref()
        .map(|value| CollisionSolverState {
            flags: value.flags,
            wall_table_size: value.wall_table_size,
            water_mode: value.water_mode,
            line_start: value.line_start,
            line_end: value.line_end,
            wall_cylinder_center: value.wall_cylinder_center,
            wall_cylinder_radius: value.wall_cylinder_radius,
            wall_cylinder_height: value.wall_cylinder_height,
            ground_check_offset: value.ground_check_offset,
            roof_correction_height: value.roof_correction_height,
            water_check_offset: value.water_check_offset,
            walls: value
                .wall_circles
                .each_ref()
                .map(|wall| CollisionSolverWallState {
                    flags: wall.flags,
                    angle_y: wall.angle_y,
                    wall_radius_squared: wall.wall_radius_squared,
                    wall_height: wall.wall_height,
                    wall_radius: wall.wall_radius,
                    direct_wall_height: wall.direct_wall_height,
                    realized_center: wall.realized_center,
                    realized_radius: wall.realized_radius,
                }),
        });
    let snapshot = CollisionSnapshot {
        boundary_index: source.boundary_index,
        state_identity_xxh3_128: lower_hex(&source.state_identity),
        player_present: source.player_present,
        player_contacts: source.player_present.then_some(source.player_contacts),
        background_status: status(source.player_background_collision_status),
        background,
        solver_status: status(source.player_collision_solver_status),
        solver,
    };
    validate_snapshot(&snapshot)?;
    Ok(snapshot)
}

fn transition_delta(
    before: &CollisionSnapshot,
    after: &CollisionSnapshot,
) -> CollisionTransitionDelta {
    let contacts = before.player_contacts.zip(after.player_contacts);
    CollisionTransitionDelta {
        player_contacts_activated: contacts.map(|(left, right)| right & !left),
        player_contacts_cleared: contacts.map(|(left, right)| left & !right),
        background: before
            .background
            .as_ref()
            .zip(after.background.as_ref())
            .map(|(left, right)| CollisionBackgroundDelta {
                flags_activated: right.flags & !left.flags,
                flags_cleared: left.flags & !right.flags,
                ground_height_delta: right.ground_height - left.ground_height,
                roof_height_delta: right.roof_height - left.roof_height,
                water_height_delta: right.water_height - left.water_height,
                wall_flags_activated: std::array::from_fn(|index| {
                    right.wall_flags[index] & !left.wall_flags[index]
                }),
                wall_flags_cleared: std::array::from_fn(|index| {
                    left.wall_flags[index] & !right.wall_flags[index]
                }),
                wall_angle_delta_y: std::array::from_fn(|index| {
                    right.wall_angles_y[index].wrapping_sub(left.wall_angles_y[index])
                }),
                resolved_displacement_delta: subtract(
                    right.resolved_frame_displacement,
                    left.resolved_frame_displacement,
                ),
            }),
        solver: before
            .solver
            .as_ref()
            .zip(after.solver.as_ref())
            .map(|(left, right)| CollisionSolverDelta {
                flags_activated: right.flags & !left.flags,
                flags_cleared: left.flags & !right.flags,
                wall_table_size_delta: i64::from(right.wall_table_size)
                    - i64::from(left.wall_table_size),
                water_mode_changed: left.water_mode != right.water_mode,
                line_start_delta: subtract(right.line_start, left.line_start),
                line_end_delta: subtract(right.line_end, left.line_end),
                wall_cylinder_center_delta: subtract(
                    right.wall_cylinder_center,
                    left.wall_cylinder_center,
                ),
                wall_cylinder_radius_delta: right.wall_cylinder_radius - left.wall_cylinder_radius,
                wall_cylinder_height_delta: right.wall_cylinder_height - left.wall_cylinder_height,
                ground_check_offset_delta: right.ground_check_offset - left.ground_check_offset,
                roof_correction_height_delta: right.roof_correction_height
                    - left.roof_correction_height,
                water_check_offset_delta: right.water_check_offset - left.water_check_offset,
                walls: std::array::from_fn(|index| {
                    wall_delta(&left.walls[index], &right.walls[index])
                }),
            }),
    }
}

fn wall_delta(
    left: &CollisionSolverWallState,
    right: &CollisionSolverWallState,
) -> CollisionSolverWallDelta {
    CollisionSolverWallDelta {
        flags_activated: right.flags & !left.flags,
        flags_cleared: left.flags & !right.flags,
        angle_delta_y: right.angle_y.wrapping_sub(left.angle_y),
        wall_radius_squared_delta: right.wall_radius_squared - left.wall_radius_squared,
        wall_height_delta: right.wall_height - left.wall_height,
        wall_radius_delta: right.wall_radius - left.wall_radius,
        direct_wall_height_delta: right.direct_wall_height - left.direct_wall_height,
        realized_center_delta: subtract(right.realized_center, left.realized_center),
        realized_radius_delta: right.realized_radius - left.realized_radius,
    }
}

fn validate_snapshot(value: &CollisionSnapshot) -> Result<(), NativeCollisionHistoryError> {
    if !is_lower_hex(&value.state_identity_xxh3_128, 32)
        || value.player_present != value.player_contacts.is_some()
        || (value.background_status == CollisionChannelStatus::Present)
            != value.background.is_some()
        || (value.solver_status == CollisionChannelStatus::Present) != value.solver.is_some()
    {
        return Err(NativeCollisionHistoryError::new(
            "collision snapshot masks are invalid",
        ));
    }
    if let Some(background) = &value.background
        && background
            .resolved_frame_displacement
            .iter()
            .chain([
                &background.ground_height,
                &background.roof_height,
                &background.water_height,
            ])
            .any(|value| !value.is_finite())
    {
        return Err(NativeCollisionHistoryError::new(
            "nonfinite background collision state",
        ));
    }
    if let Some(solver) = &value.solver {
        let scalar = [
            solver.wall_cylinder_radius,
            solver.wall_cylinder_height,
            solver.ground_check_offset,
            solver.roof_correction_height,
            solver.water_check_offset,
        ];
        if solver
            .line_start
            .iter()
            .chain(&solver.line_end)
            .chain(&solver.wall_cylinder_center)
            .chain(&scalar)
            .chain(solver.walls.iter().flat_map(|wall| {
                wall.realized_center.iter().chain([
                    &wall.wall_radius_squared,
                    &wall.wall_height,
                    &wall.wall_radius,
                    &wall.direct_wall_height,
                    &wall.realized_radius,
                ])
            }))
            .any(|value| !value.is_finite())
        {
            return Err(NativeCollisionHistoryError::new(
                "nonfinite collision solver state",
            ));
        }
    }
    Ok(())
}

fn validate_delta(value: &CollisionTransitionDelta) -> Result<(), NativeCollisionHistoryError> {
    let background_finite = value.background.as_ref().is_none_or(|delta| {
        [
            delta.ground_height_delta,
            delta.roof_height_delta,
            delta.water_height_delta,
        ]
        .iter()
        .chain(&delta.resolved_displacement_delta)
        .all(|value| value.is_finite())
    });
    let solver_finite = value.solver.as_ref().is_none_or(|delta| {
        delta
            .line_start_delta
            .iter()
            .chain(&delta.line_end_delta)
            .chain(&delta.wall_cylinder_center_delta)
            .chain([
                &delta.wall_cylinder_radius_delta,
                &delta.wall_cylinder_height_delta,
                &delta.ground_check_offset_delta,
                &delta.roof_correction_height_delta,
                &delta.water_check_offset_delta,
            ])
            .chain(delta.walls.iter().flat_map(|wall| {
                wall.realized_center_delta.iter().chain([
                    &wall.wall_radius_squared_delta,
                    &wall.wall_height_delta,
                    &wall.wall_radius_delta,
                    &wall.direct_wall_height_delta,
                    &wall.realized_radius_delta,
                ])
            }))
            .all(|value| value.is_finite())
    });
    if !background_finite || !solver_finite {
        return Err(NativeCollisionHistoryError::new(
            "nonfinite collision transition delta",
        ));
    }
    Ok(())
}

fn status(value: NativeChannelStatus) -> CollisionChannelStatus {
    match value {
        NativeChannelStatus::NotSampled => CollisionChannelStatus::NotSampled,
        NativeChannelStatus::Unavailable => CollisionChannelStatus::Unavailable,
        NativeChannelStatus::Absent => CollisionChannelStatus::Absent,
        NativeChannelStatus::Present => CollisionChannelStatus::Present,
    }
}

fn pad(value: NativeRawPad) -> CollisionHistoryPad {
    CollisionHistoryPad {
        buttons: value.buttons,
        stick_x: value.stick_x,
        stick_y: value.stick_y,
        substick_x: value.substick_x,
        substick_y: value.substick_y,
        trigger_left: value.trigger_left,
        trigger_right: value.trigger_right,
        analog_a: value.analog_a,
        analog_b: value.analog_b,
        connected: value.connected,
        error: value.error,
    }
}

fn subtract(right: [f32; 3], left: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|index| right[index] - left[index])
}

fn lower_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(DIGITS[usize::from(byte >> 4)] as char);
        encoded.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    encoded
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeCollisionHistoryError(String);

impl NativeCollisionHistoryError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for NativeCollisionHistoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeCollisionHistoryError {}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_evidence::native_episode_shard::{NativeObservationPhase, NativeTerminalReason};

    fn shard_v11_with_history() -> NativeEpisodeShard {
        let mut shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v11.dseps"
        ))
        .unwrap();
        shard.episodes.truncate(2);
        for episode in &mut shard.episodes {
            let first = episode.steps[0].clone();
            let mut second = first.clone();
            second.pre_input = first.post_simulation.clone();
            second.pre_input.phase = NativeObservationPhase::PreInput;
            second.pre_input.terminal_reason = NativeTerminalReason::None;
            second.post_simulation = second.pre_input.clone();
            second.post_simulation.phase = NativeObservationPhase::PostSimulation;
            second.post_simulation.boundary_index += 1;
            second.post_simulation.simulation_tick += 1;
            second.post_simulation.state_identity[0] ^= 0x55;
            let solver = second
                .post_simulation
                .player_collision_solver
                .as_mut()
                .unwrap();
            solver.flags ^= 0x20;
            solver.wall_table_size += 1;
            solver.wall_circles[0].angle_y = solver.wall_circles[0].angle_y.wrapping_add(7);
            second.post_simulation.player_contacts ^= 0x2;
            second.consumed_pad.buttons ^= 0x40;
            episode.steps.push(second);
        }
        shard
    }

    #[test]
    fn bounded_history_is_past_only_and_resets_between_episodes() {
        let view = NativeCollisionHistoryView::build(&shard_v11_with_history(), 4).unwrap();
        assert_eq!(view.decisions.len(), 4);
        assert!(view.decisions[0].completed_transition_indices.is_empty());
        assert_eq!(view.decisions[1].completed_transition_indices, [0]);
        assert!(view.decisions[2].completed_transition_indices.is_empty());
        assert_eq!(view.decisions[3].completed_transition_indices, [2]);
        assert_eq!(
            view.auxiliary_targets[1].delta.player_contacts_activated,
            Some(2)
        );
        assert_eq!(
            view.auxiliary_targets[1]
                .delta
                .solver
                .as_ref()
                .unwrap()
                .wall_table_size_delta,
            1
        );
        let resolved = view.resolved_history(1).unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].step_index, 0);
        assert_ne!(
            view.auxiliary_targets[1].consumed_pad,
            resolved[0].consumed_pad
        );
        let bytes = view.canonical_bytes().unwrap();
        assert_eq!(
            NativeCollisionHistoryView::decode_canonical(&bytes).unwrap(),
            view
        );
    }

    #[test]
    fn legacy_solver_missingness_is_not_fabricated() {
        let mut shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v10.dseps"
        ))
        .unwrap();
        shard.episodes.truncate(1);
        let view = NativeCollisionHistoryView::build(&shard, 2).unwrap();
        assert!(view.decisions.iter().all(|decision| {
            let current = &view.snapshots[decision.current_snapshot_index as usize];
            current.solver_status == CollisionChannelStatus::NotSampled && current.solver.is_none()
        }));
        assert!(
            view.auxiliary_targets
                .iter()
                .all(|target| target.delta.solver.is_none())
        );
    }

    #[test]
    fn future_leakage_delta_tampering_and_nonfinite_state_fail_closed() {
        let view = NativeCollisionHistoryView::build(&shard_v11_with_history(), 4).unwrap();

        let mut future = view.clone();
        future.decisions[0].completed_transition_indices.push(0);
        future.view_sha256 = future.compute_identity().unwrap();
        assert!(future.validate().is_err());

        let mut changed_delta = view.clone();
        changed_delta.auxiliary_targets[0]
            .delta
            .solver
            .as_mut()
            .unwrap()
            .flags_activated ^= 2;
        changed_delta.view_sha256 = changed_delta.compute_identity().unwrap();
        assert!(changed_delta.validate().is_err());

        let mut nonfinite = view;
        let current_index = nonfinite.decisions[0].current_snapshot_index as usize;
        nonfinite.snapshots[current_index]
            .solver
            .as_mut()
            .unwrap()
            .line_start[0] = f32::NAN;
        assert!(validate_snapshot(&nonfinite.snapshots[current_index]).is_err());
        assert!(nonfinite.compute_identity().is_err());
    }
}
