//! One learner-facing fact projection over existing native observation sources.
//!
//! The native wire observations remain authoritative. This view preserves exact
//! integer and IEEE-754 values, explicit missingness, complete actor identity,
//! terminal state, recent PAD/history, and optional condition evaluations.

use crate::artifact::Digest;
use crate::native_generic_tactic::{NativeTacticActionLane, NativeTacticObservation};
use dusklight_automation_contracts::actor_identity::PlacedActorSelector;
use dusklight_control::option_execution::{OptionCondition, OptionEndReason, OptionExecution};
use dusklight_evidence::native_episode_shard::{
    NativeActorObservation, NativeActorSelectionRule, NativeChannelStatus, NativeEpisodeStep,
    NativeLearningObservation, NativeObservationPhase, NativeRawPad, NativeTerminalReason,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const FACT_SNAPSHOT_SCHEMA_V1: &str = "dusklight-fact-snapshot/v1";
pub const FACT_SNAPSHOT_SCHEMA_V2: &str = "dusklight-fact-snapshot/v2";
pub const MAX_FACT_HISTORY: usize = 8;
pub const MAX_FACT_ACTORS: usize = 4_096;
pub const MAX_CONDITION_EVALUATIONS: usize = 256;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FactObservationSource {
    NativeLearning,
    NativeTactic,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FactPhase {
    PreInput,
    PostSimulation,
    TacticBoundary,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FactAvailability {
    Available,
    Absent,
    Unavailable,
    NotSampled,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FactSnapshot {
    pub schema: String,
    pub source: FactObservationSource,
    pub phase: FactPhase,
    pub boundary_index: u64,
    pub simulation_tick: u64,
    pub tape_frame: u64,
    pub state_identity: [u8; 16],
    pub world: WorldFactSnapshot,
    pub player: PlayerFactSnapshot,
    pub event: Option<EventFactSnapshot>,
    pub terminal: TerminalFactSnapshot,
    pub actors_complete: bool,
    pub actors: Vec<ActorFactSnapshot>,
    pub channels: ChannelFactSnapshot,
    pub flag_banks: FlagBankFactSnapshot,
    pub recent_history: Vec<HistoryFactSnapshot>,
    pub recent_option: Option<RecentOptionFactSnapshot>,
    pub conditions: Vec<ConditionFactSnapshot>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorldFactSnapshot {
    pub stage: String,
    pub room: i8,
    pub layer: Option<i8>,
    pub point: Option<i16>,
    pub next_stage: Option<String>,
    pub next_room: Option<i8>,
    pub next_layer: Option<i8>,
    pub next_point: Option<i16>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerFactSnapshot {
    pub present: bool,
    pub is_link: Option<bool>,
    pub process_id: Option<u32>,
    pub actor_name: Option<i16>,
    pub procedure: Option<u16>,
    pub mode_flags: Option<u32>,
    pub contacts: Option<u8>,
    pub position_f32_bits: [u32; 3],
    pub velocity_f32_bits: Option<[u32; 3]>,
    pub forward_speed_f32_bits: Option<u32>,
    pub current_angle: Option<[i16; 3]>,
    pub shape_angle: Option<[i16; 3]>,
    pub camera_yaw_radians_f32_bits: Option<u32>,
    pub ground_height_f32_bits: Option<u32>,
    pub roof_height_f32_bits: Option<u32>,
    pub collision_correction_f32_bits: Option<[u32; 2]>,
    pub action_lanes: Vec<ActionLaneFactSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_state: Option<PlayerActionFactSnapshot>,
    pub previous_pad: Option<PadFactSnapshot>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerActionFactSnapshot {
    pub procedure_context_raw: [i16; 6],
    pub damage_wait_timer: i16,
    pub sword_at_up_time: u16,
    pub ice_damage_wait_timer: i16,
    pub sword_change_wait_timer: u8,
    pub flags: u32,
    pub do_status: u8,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActionLaneFactSnapshot {
    pub resource_id: u16,
    pub frame_f32_bits: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PadFactSnapshot {
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EventFactSnapshot {
    pub running: bool,
    pub event_id: i16,
    pub mode: u8,
    pub status: u8,
    pub map_tool_id: u8,
    pub name_hash: Option<u32>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FactTerminalReason {
    None,
    GoalReached,
    TickBudgetExhausted,
    NotReported,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalFactSnapshot {
    pub reason: FactTerminalReason,
    pub configured: Option<bool>,
    pub reached: Option<bool>,
    pub requested_count: Option<u16>,
    pub hit_count: Option<u16>,
    pub stable_ticks: Option<u16>,
    pub consecutive_ticks: Option<u16>,
    pub first_hit_tick: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActorFactSnapshot {
    pub portable_selector: Option<PlacedActorSelector>,
    pub runtime_generation: u64,
    pub actor_name: i16,
    pub profile_name: Option<i16>,
    pub set_id: u16,
    pub home_room: i8,
    pub current_room: i8,
    pub position_f32_bits: [u32; 3],
    pub home_position_f32_bits: Option<[u32; 3]>,
    pub velocity_f32_bits: Option<[u32; 3]>,
    pub health: Option<i16>,
    pub status: Option<u32>,
    pub condition: Option<u32>,
    pub attention_present: Option<bool>,
    pub event_participation_present: Option<bool>,
    pub trigger_volume_present: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelFactSnapshot {
    pub camera: FactAvailability,
    pub player_action: FactAvailability,
    pub background_collision: FactAvailability,
    pub collision_surfaces: FactAvailability,
    pub scene_exit: FactAvailability,
    pub dynamic_colliders: FactAvailability,
    pub player_resources: FactAvailability,
    pub player_relationships: FactAvailability,
    pub collision_solver: FactAvailability,
    pub process_lifecycle: FactAvailability,
    pub event_transition: FactAvailability,
    pub room_load: FactAvailability,
    pub warp_session: FactAvailability,
    pub resource_loads: FactAvailability,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ByteBankFactSnapshot {
    pub availability: FactAvailability,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FlagBankFactSnapshot {
    pub event: ByteBankFactSnapshot,
    pub temporary: ByteBankFactSnapshot,
    pub temporary_event: ByteBankFactSnapshot,
    pub dungeon: ByteBankFactSnapshot,
    pub switch: ByteBankFactSnapshot,
    pub switch_room: Option<i8>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryFactSnapshot {
    pub boundary_index: u64,
    pub simulation_tick: u64,
    pub tape_frame: u64,
    pub stage: String,
    pub room: i8,
    pub player_position_f32_bits: [u32; 3],
    pub player_procedure: u16,
    pub event_running: bool,
    pub previous_pad: PadFactSnapshot,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecentOptionFactSnapshot {
    pub option_id: String,
    pub end_reason: OptionEndReason,
    pub realized_ticks: u32,
    pub tape_start: u64,
    pub tape_end_exclusive: u64,
    /// Motion accumulated over the complete realized option. This is
    /// transition evidence, not terminal authority. Older snapshots omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trajectory: Option<OptionTrajectoryFactSnapshot>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptionTrajectoryFactSnapshot {
    pub observed_ticks: u32,
    pub commanded_motion_ticks: u32,
    pub commanded_stall_ticks: u32,
    pub wall_contact_ticks: u32,
    pub collision_correction_ticks: u32,
    pub world_transition_ticks: u32,
    pub planar_path_length_f32_bits: u32,
    pub planar_displacement_f32_bits: u32,
    pub mean_planar_speed_f32_bits: u32,
    pub final_planar_velocity_f32_bits: u32,
    pub maximum_planar_velocity_f32_bits: u32,
    pub commanded_momentum_loss_f32_bits: u32,
    pub collision_correction_total_f32_bits: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConditionFactSnapshot {
    pub condition: OptionCondition,
    pub value: Option<bool>,
}

impl FactSnapshot {
    /// Reconstitutes the bounded observation view used by native-generic
    /// tactics. This keeps the selected tactic's first decision on the exact
    /// learner-visible state instead of spending an unselected probe input.
    pub fn to_native_tactic_observation(
        &self,
    ) -> Result<NativeTacticObservation, FactSnapshotError> {
        self.validate()?;
        let boundary_offset = u64::from(self.phase == FactPhase::PostSimulation);
        let simulation_tick = self
            .simulation_tick
            .checked_add(boundary_offset)
            .ok_or(FactSnapshotError::InvalidSnapshot)?;
        let tape_frame = self
            .tape_frame
            .checked_add(boundary_offset)
            .ok_or(FactSnapshotError::InvalidSnapshot)?;
        let current_angle = self
            .player
            .current_angle
            .ok_or(FactSnapshotError::IncompleteSource)?;
        let player_procedure = self
            .player
            .procedure
            .ok_or(FactSnapshotError::IncompleteSource)?;
        let player_mode_flags = self
            .player
            .mode_flags
            .ok_or(FactSnapshotError::IncompleteSource)?;
        let player_contacts = self
            .player
            .contacts
            .ok_or(FactSnapshotError::IncompleteSource)?;
        let actors = self
            .actors
            .iter()
            .filter_map(|actor| {
                actor.portable_selector.clone().map(|selector| {
                    crate::native_generic_tactic::NativeTacticActor {
                        selector,
                        runtime_generation: actor.runtime_generation,
                        current_room: actor.current_room,
                        position_f32_bits: actor.position_f32_bits,
                    }
                })
            })
            .collect();
        let observation = NativeTacticObservation {
            boundary_index: self.boundary_index,
            simulation_tick,
            tape_frame,
            state_identity: self.state_identity,
            stage: self.world.stage.clone(),
            room: self.world.room,
            player_position_f32_bits: self.player.position_f32_bits,
            player_yaw: current_angle[1],
            player_procedure,
            player_mode_flags,
            player_contacts,
            camera_yaw_radians_f32_bits: self.player.camera_yaw_radians_f32_bits,
            action_lanes: self
                .player
                .action_lanes
                .iter()
                .map(|lane| NativeTacticActionLane {
                    resource_id: lane.resource_id,
                    frame_f32_bits: lane.frame_f32_bits,
                })
                .collect(),
            actor_set_complete: self.actors_complete,
            actors,
        };
        observation
            .validate()
            .map_err(|error| FactSnapshotError::InvalidSource(error.to_string()))?;
        Ok(observation)
    }

    pub fn from_native_learning(
        observation: &NativeLearningObservation,
        prior: &[NativeLearningObservation],
        recent_option: Option<&OptionExecution>,
        conditions: Vec<ConditionFactSnapshot>,
    ) -> Result<Self, FactSnapshotError> {
        Self::from_native_learning_with_option_trajectory(
            observation,
            prior,
            recent_option,
            None,
            conditions,
        )
    }

    pub fn from_native_learning_with_option_trajectory(
        observation: &NativeLearningObservation,
        prior: &[NativeLearningObservation],
        recent_option: Option<&OptionExecution>,
        option_trajectory: Option<OptionTrajectoryFactSnapshot>,
        conditions: Vec<ConditionFactSnapshot>,
    ) -> Result<Self, FactSnapshotError> {
        validate_native_learning(observation)?;
        if prior.len() > MAX_FACT_HISTORY || conditions.len() > MAX_CONDITION_EVALUATIONS {
            return Err(FactSnapshotError::Capacity);
        }
        if option_trajectory.is_some() && recent_option.is_none() {
            return Err(FactSnapshotError::InvalidSnapshot);
        }
        let recent_history = prior
            .iter()
            .map(history_from_native)
            .collect::<Result<Vec<_>, _>>()?;
        validate_history(&recent_history, observation.boundary_index)?;
        let player_action_lanes = observation
            .player_action
            .as_ref()
            .map(|action| {
                action
                    .under_animations
                    .iter()
                    .chain(&action.upper_animations)
                    .map(|lane| {
                        Ok(ActionLaneFactSnapshot {
                            resource_id: lane.resource_id,
                            frame_f32_bits: finite_bits(lane.frame)?,
                        })
                    })
                    .collect::<Result<Vec<_>, FactSnapshotError>>()
            })
            .transpose()?
            .unwrap_or_default();
        let snapshot = Self {
            schema: FACT_SNAPSHOT_SCHEMA_V2.into(),
            source: FactObservationSource::NativeLearning,
            phase: match observation.phase {
                NativeObservationPhase::PreInput => FactPhase::PreInput,
                NativeObservationPhase::PostSimulation => FactPhase::PostSimulation,
            },
            boundary_index: observation.boundary_index,
            simulation_tick: observation.simulation_tick,
            tape_frame: observation.tape_frame,
            state_identity: observation.state_identity,
            world: WorldFactSnapshot {
                stage: observation.stage.clone(),
                room: observation.room,
                layer: Some(observation.layer),
                point: Some(observation.point),
                next_stage: observation.next_stage.clone(),
                next_room: observation
                    .next_stage
                    .as_ref()
                    .map(|_| observation.next_room),
                next_layer: observation
                    .next_stage
                    .as_ref()
                    .map(|_| observation.next_layer),
                next_point: observation
                    .next_stage
                    .as_ref()
                    .map(|_| observation.next_point),
            },
            player: PlayerFactSnapshot {
                present: observation.player_present,
                is_link: Some(observation.player_is_link),
                process_id: Some(observation.player_process_id),
                actor_name: Some(observation.player_actor_name),
                procedure: Some(observation.player_procedure),
                mode_flags: Some(observation.player_mode_flags),
                contacts: Some(observation.player_contacts),
                position_f32_bits: bits3(observation.player_position)?,
                velocity_f32_bits: Some(bits3(observation.player_velocity)?),
                forward_speed_f32_bits: Some(finite_bits(observation.player_forward_speed)?),
                current_angle: Some(observation.player_current_angle),
                shape_angle: Some(observation.player_shape_angle),
                camera_yaw_radians_f32_bits: observation
                    .camera_yaw_radians
                    .map(finite_bits)
                    .transpose()?,
                ground_height_f32_bits: observation
                    .player_ground_height
                    .map(finite_bits)
                    .transpose()?,
                roof_height_f32_bits: observation
                    .player_roof_height
                    .map(finite_bits)
                    .transpose()?,
                collision_correction_f32_bits: observation
                    .collision_correction
                    .map(bits2)
                    .transpose()?,
                action_lanes: player_action_lanes,
                action_state: observation.player_action.as_ref().map(|action| {
                    PlayerActionFactSnapshot {
                        procedure_context_raw: action.procedure_context_raw,
                        damage_wait_timer: action.damage_wait_timer,
                        sword_at_up_time: action.sword_at_up_time,
                        ice_damage_wait_timer: action.ice_damage_wait_timer,
                        sword_change_wait_timer: action.sword_change_wait_timer,
                        flags: action.flags,
                        do_status: action.do_status,
                    }
                }),
                previous_pad: Some(pad(observation.previous_input)),
            },
            event: Some(EventFactSnapshot {
                running: observation.event_running,
                event_id: observation.event_id,
                mode: observation.event_mode,
                status: observation.event_status,
                map_tool_id: observation.event_map_tool_id,
                name_hash: observation.event_name_hash,
            }),
            terminal: TerminalFactSnapshot {
                reason: terminal_reason(observation.terminal_reason),
                configured: Some(observation.goal.configured),
                reached: Some(observation.goal.reached),
                requested_count: Some(observation.goal.requested_count),
                hit_count: Some(observation.goal.hit_count),
                stable_ticks: Some(observation.goal.stable_ticks),
                consecutive_ticks: Some(observation.goal.consecutive_ticks),
                first_hit_tick: observation.goal.first_hit_tick,
            },
            actors_complete: true,
            actors: observation
                .actors
                .iter()
                .map(|actor| actor_from_native(&observation.stage, actor))
                .collect::<Result<Vec<_>, _>>()?,
            channels: channels(observation),
            flag_banks: FlagBankFactSnapshot {
                event: bank(&observation.event_flags),
                temporary: bank(&observation.temporary_flags),
                temporary_event: bank(&observation.temporary_event_bytes),
                dungeon: bank(&observation.dungeon_flags),
                switch: bank(&observation.switch_flags),
                switch_room: observation
                    .switch_flags
                    .as_ref()
                    .map(|_| observation.switch_flag_room),
            },
            recent_history,
            recent_option: recent_option.map(|execution| RecentOptionFactSnapshot {
                trajectory: option_trajectory,
                ..recent_option_fact(execution)
            }),
            conditions,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn from_native_tactic(
        observation: &NativeTacticObservation,
        conditions: Vec<ConditionFactSnapshot>,
    ) -> Result<Self, FactSnapshotError> {
        observation
            .validate()
            .map_err(|error| FactSnapshotError::InvalidSource(error.to_string()))?;
        if !observation.actor_set_complete
            || observation.actors.len() > MAX_FACT_ACTORS
            || conditions.len() > MAX_CONDITION_EVALUATIONS
        {
            return Err(FactSnapshotError::Capacity);
        }
        let snapshot = Self {
            schema: FACT_SNAPSHOT_SCHEMA_V2.into(),
            source: FactObservationSource::NativeTactic,
            phase: FactPhase::TacticBoundary,
            boundary_index: observation.boundary_index,
            simulation_tick: observation.simulation_tick,
            tape_frame: observation.tape_frame,
            state_identity: observation.state_identity,
            world: WorldFactSnapshot {
                stage: observation.stage.clone(),
                room: observation.room,
                layer: None,
                point: None,
                next_stage: None,
                next_room: None,
                next_layer: None,
                next_point: None,
            },
            player: PlayerFactSnapshot {
                present: true,
                is_link: None,
                process_id: None,
                actor_name: None,
                procedure: Some(observation.player_procedure),
                mode_flags: Some(observation.player_mode_flags),
                contacts: Some(observation.player_contacts),
                position_f32_bits: bits3(observation.player_position_f32_bits.map(f32::from_bits))?,
                velocity_f32_bits: None,
                forward_speed_f32_bits: None,
                current_angle: Some([0, observation.player_yaw, 0]),
                shape_angle: None,
                camera_yaw_radians_f32_bits: observation.camera_yaw_radians_f32_bits,
                ground_height_f32_bits: None,
                roof_height_f32_bits: None,
                collision_correction_f32_bits: None,
                action_lanes: observation
                    .action_lanes
                    .iter()
                    .map(action_lane)
                    .collect::<Result<Vec<_>, _>>()?,
                action_state: None,
                previous_pad: None,
            },
            event: None,
            terminal: TerminalFactSnapshot {
                reason: FactTerminalReason::NotReported,
                configured: None,
                reached: None,
                requested_count: None,
                hit_count: None,
                stable_ticks: None,
                consecutive_ticks: None,
                first_hit_tick: None,
            },
            actors_complete: true,
            actors: observation
                .actors
                .iter()
                .map(|actor| {
                    Ok(ActorFactSnapshot {
                        portable_selector: Some(actor.selector.clone()),
                        runtime_generation: actor.runtime_generation,
                        actor_name: 0,
                        profile_name: None,
                        set_id: 0,
                        home_room: 0,
                        current_room: actor.current_room,
                        position_f32_bits: actor.position_f32_bits,
                        home_position_f32_bits: None,
                        velocity_f32_bits: None,
                        health: None,
                        status: None,
                        condition: None,
                        attention_present: None,
                        event_participation_present: None,
                        trigger_volume_present: None,
                    })
                })
                .collect::<Result<Vec<_>, FactSnapshotError>>()?,
            channels: unavailable_channels(),
            flag_banks: unavailable_banks(),
            recent_history: Vec::new(),
            recent_option: None,
            conditions,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn validate(&self) -> Result<(), FactSnapshotError> {
        if !matches!(
            self.schema.as_str(),
            FACT_SNAPSHOT_SCHEMA_V1 | FACT_SNAPSHOT_SCHEMA_V2
        ) || self.world.stage.is_empty()
            || self.world.stage.len() > 64
            || self.actors.len() > MAX_FACT_ACTORS
            || self.recent_history.len() > MAX_FACT_HISTORY
            || self.conditions.len() > MAX_CONDITION_EVALUATIONS
            || !self.actors_complete
        {
            return Err(FactSnapshotError::InvalidSnapshot);
        }
        if self.schema == FACT_SNAPSHOT_SCHEMA_V1
            && self
                .recent_option
                .as_ref()
                .is_some_and(|option| option.trajectory.is_some())
        {
            return Err(FactSnapshotError::InvalidSnapshot);
        }
        validate_history(&self.recent_history, self.boundary_index)?;
        for condition in &self.conditions {
            dusklight_control::option_execution::validate_condition(&condition.condition)
                .map_err(|error| FactSnapshotError::InvalidSource(error.to_string()))?;
        }
        if let Some(option) = &self.recent_option
            && (option.option_id.is_empty()
                || option.realized_ticks == 0
                || option.tape_end_exclusive <= option.tape_start
                || option.tape_end_exclusive - option.tape_start
                    != u64::from(option.realized_ticks)
                || option
                    .trajectory
                    .is_some_and(|trajectory| !trajectory.is_valid(option.realized_ticks)))
        {
            return Err(FactSnapshotError::InvalidSnapshot);
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, FactSnapshotError> {
        self.validate()?;
        serde_json::to_vec(self)
            .map_err(|error| FactSnapshotError::Serialization(error.to_string()))
    }

    pub fn content_sha256(&self) -> Result<Digest, FactSnapshotError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

fn validate_native_learning(
    observation: &NativeLearningObservation,
) -> Result<(), FactSnapshotError> {
    if observation.actors_truncated
        || observation.actor_selection != NativeActorSelectionRule::Complete
        || observation.actor_observed_count as usize != observation.actors.len()
        || observation.actors.len() > MAX_FACT_ACTORS
        || observation.stage.is_empty()
    {
        return Err(FactSnapshotError::IncompleteSource);
    }
    bits3(observation.player_position)?;
    bits3(observation.player_velocity)?;
    finite_bits(observation.player_forward_speed)?;
    for actor in &observation.actors {
        bits3(actor.position)?;
        bits3(actor.home_position)?;
        bits3(actor.velocity)?;
    }
    Ok(())
}

fn actor_from_native(
    stage: &str,
    actor: &NativeActorObservation,
) -> Result<ActorFactSnapshot, FactSnapshotError> {
    Ok(ActorFactSnapshot {
        portable_selector: (actor.set_id != u16::MAX).then(|| PlacedActorSelector {
            stage: stage.into(),
            home_room: actor.home_room,
            set_id: actor.set_id,
            actor_name: actor.actor_name,
        }),
        runtime_generation: actor.runtime_generation,
        actor_name: actor.actor_name,
        profile_name: Some(actor.profile_name),
        set_id: actor.set_id,
        home_room: actor.home_room,
        current_room: actor.current_room,
        position_f32_bits: bits3(actor.position)?,
        home_position_f32_bits: Some(bits3(actor.home_position)?),
        velocity_f32_bits: Some(bits3(actor.velocity)?),
        health: Some(actor.health),
        status: Some(actor.status),
        condition: Some(actor.condition),
        attention_present: Some(actor.attention.is_some()),
        event_participation_present: Some(actor.event_participation.is_some()),
        trigger_volume_present: Some(actor.trigger_volume.is_some()),
    })
}

fn history_from_native(
    observation: &NativeLearningObservation,
) -> Result<HistoryFactSnapshot, FactSnapshotError> {
    Ok(HistoryFactSnapshot {
        boundary_index: observation.boundary_index,
        simulation_tick: observation.simulation_tick,
        tape_frame: observation.tape_frame,
        stage: observation.stage.clone(),
        room: observation.room,
        player_position_f32_bits: bits3(observation.player_position)?,
        player_procedure: observation.player_procedure,
        event_running: observation.event_running,
        previous_pad: pad(observation.previous_input),
    })
}

fn validate_history(
    history: &[HistoryFactSnapshot],
    current_boundary: u64,
) -> Result<(), FactSnapshotError> {
    let mut previous = None;
    for row in history {
        if row.boundary_index >= current_boundary
            || previous.is_some_and(|value| value >= row.boundary_index)
        {
            return Err(FactSnapshotError::InvalidHistory);
        }
        previous = Some(row.boundary_index);
    }
    Ok(())
}

fn action_lane(lane: &NativeTacticActionLane) -> Result<ActionLaneFactSnapshot, FactSnapshotError> {
    finite_bits(f32::from_bits(lane.frame_f32_bits))?;
    Ok(ActionLaneFactSnapshot {
        resource_id: lane.resource_id,
        frame_f32_bits: lane.frame_f32_bits,
    })
}

fn recent_option_fact(execution: &OptionExecution) -> RecentOptionFactSnapshot {
    RecentOptionFactSnapshot {
        option_id: execution.option_id.clone(),
        end_reason: execution.end_reason,
        realized_ticks: execution.duration.realized_ticks,
        tape_start: execution.realized_tape_range.start_frame,
        tape_end_exclusive: execution.realized_tape_range.end_frame_exclusive,
        trajectory: None,
    }
}

impl OptionTrajectoryFactSnapshot {
    pub fn from_native_steps(steps: &[NativeEpisodeStep]) -> Result<Self, FactSnapshotError> {
        let observed_ticks = u32::try_from(steps.len()).map_err(|_| FactSnapshotError::Capacity)?;
        if observed_ticks == 0 {
            return Err(FactSnapshotError::InvalidSnapshot);
        }
        let first = &steps[0].pre_input;
        let last = &steps[steps.len() - 1].post_simulation;
        let same_world = steps.iter().all(|step| {
            step.pre_input.stage == first.stage
                && step.pre_input.room == first.room
                && step.post_simulation.stage == first.stage
                && step.post_simulation.room == first.room
        });
        let mut planar_path_length = 0.0_f64;
        let mut commanded_motion_ticks = 0_u32;
        let mut commanded_stall_ticks = 0_u32;
        let mut wall_contact_ticks = 0_u32;
        let mut collision_correction_ticks = 0_u32;
        let mut world_transition_ticks = 0_u32;
        let mut maximum_planar_velocity = planar_velocity(first.player_velocity);
        let mut commanded_momentum_loss = 0.0_f64;
        let mut collision_correction_total = 0.0_f64;
        for step in steps {
            validate_motion_observation(&step.pre_input)?;
            validate_motion_observation(&step.post_simulation)?;
            let stayed_in_world = step.pre_input.stage == step.post_simulation.stage
                && step.pre_input.room == step.post_simulation.room;
            world_transition_ticks += u32::from(!stayed_in_world);
            let distance = if stayed_in_world {
                planar_distance(
                    step.pre_input.player_position,
                    step.post_simulation.player_position,
                )
            } else {
                0.0
            };
            planar_path_length += distance;
            let commanded = commanded_motion(step.consumed_pad);
            commanded_motion_ticks += u32::from(commanded);
            commanded_stall_ticks += u32::from(commanded && distance < 1.0);
            wall_contact_ticks += u32::from(
                step.post_simulation.player_contacts & (1 << 1) != 0
                    || step
                        .post_simulation
                        .player_collision_solver
                        .as_ref()
                        .is_some_and(|solver| {
                            solver.wall_circles.iter().any(|wall| wall.flags != 0)
                        }),
            );
            let before_velocity = planar_velocity(step.pre_input.player_velocity);
            let after_velocity = planar_velocity(step.post_simulation.player_velocity);
            maximum_planar_velocity = maximum_planar_velocity.max(after_velocity);
            if commanded {
                commanded_momentum_loss += (before_velocity - after_velocity).max(0.0);
            }
            if let Some(correction) = step.post_simulation.collision_correction {
                let magnitude = f64::from(correction[0]).hypot(f64::from(correction[1]));
                collision_correction_ticks += u32::from(magnitude > 0.0);
                collision_correction_total += magnitude;
            }
        }
        let planar_displacement = if same_world {
            planar_distance(first.player_position, last.player_position)
        } else {
            0.0
        };
        let mean_planar_speed = planar_path_length / f64::from(observed_ticks);
        let summary = Self {
            observed_ticks,
            commanded_motion_ticks,
            commanded_stall_ticks,
            wall_contact_ticks,
            collision_correction_ticks,
            world_transition_ticks,
            planar_path_length_f32_bits: finite_f64_bits(planar_path_length)?,
            planar_displacement_f32_bits: finite_f64_bits(planar_displacement)?,
            mean_planar_speed_f32_bits: finite_f64_bits(mean_planar_speed)?,
            final_planar_velocity_f32_bits: finite_f64_bits(planar_velocity(last.player_velocity))?,
            maximum_planar_velocity_f32_bits: finite_f64_bits(maximum_planar_velocity)?,
            commanded_momentum_loss_f32_bits: finite_f64_bits(commanded_momentum_loss)?,
            collision_correction_total_f32_bits: finite_f64_bits(collision_correction_total)?,
        };
        if summary.is_valid(observed_ticks) {
            Ok(summary)
        } else {
            Err(FactSnapshotError::InvalidSnapshot)
        }
    }

    pub(crate) fn is_valid(self, realized_ticks: u32) -> bool {
        let values = [
            self.planar_path_length_f32_bits,
            self.planar_displacement_f32_bits,
            self.mean_planar_speed_f32_bits,
            self.final_planar_velocity_f32_bits,
            self.maximum_planar_velocity_f32_bits,
            self.commanded_momentum_loss_f32_bits,
            self.collision_correction_total_f32_bits,
        ]
        .map(f32::from_bits);
        let tolerance = |value: f32| 8.0 * f32::EPSILON * value.max(1.0);
        self.observed_ticks == realized_ticks
            && self.commanded_motion_ticks <= self.observed_ticks
            && self.commanded_stall_ticks <= self.commanded_motion_ticks
            && self.wall_contact_ticks <= self.observed_ticks
            && self.collision_correction_ticks <= self.observed_ticks
            && self.world_transition_ticks <= self.observed_ticks
            && values
                .iter()
                .all(|value| value.is_finite() && *value >= 0.0)
            && values[1] <= values[0] + tolerance(values[0])
            && values[3] <= values[4] + tolerance(values[4])
            && (values[2] - values[0] / self.observed_ticks as f32).abs() <= tolerance(values[2])
            && (self.commanded_motion_ticks != 0 || values[5] == 0.0)
            && (self.world_transition_ticks == 0 || values[1] == 0.0)
    }
}

fn validate_motion_observation(
    observation: &NativeLearningObservation,
) -> Result<(), FactSnapshotError> {
    bits3(observation.player_position)?;
    bits3(observation.player_velocity)?;
    if let Some(correction) = observation.collision_correction {
        bits2(correction)?;
    }
    Ok(())
}

fn commanded_motion(pad: NativeRawPad) -> bool {
    f32::from(pad.stick_x).hypot(f32::from(pad.stick_y)) >= 32.0
}

fn planar_distance(left: [f32; 3], right: [f32; 3]) -> f64 {
    f64::from(right[0] - left[0]).hypot(f64::from(right[2] - left[2]))
}

fn planar_velocity(velocity: [f32; 3]) -> f64 {
    f64::from(velocity[0]).hypot(f64::from(velocity[2]))
}

fn finite_f64_bits(value: f64) -> Result<u32, FactSnapshotError> {
    let narrowed = value as f32;
    if value.is_finite() && narrowed.is_finite() && narrowed >= 0.0 {
        Ok(narrowed.to_bits())
    } else {
        Err(FactSnapshotError::NonFinite)
    }
}

fn pad(value: NativeRawPad) -> PadFactSnapshot {
    PadFactSnapshot {
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

fn bits3(values: [f32; 3]) -> Result<[u32; 3], FactSnapshotError> {
    if values.iter().any(|value| !value.is_finite()) {
        return Err(FactSnapshotError::NonFinite);
    }
    Ok(values.map(f32::to_bits))
}

fn bits2(values: [f32; 2]) -> Result<[u32; 2], FactSnapshotError> {
    if values.iter().any(|value| !value.is_finite()) {
        return Err(FactSnapshotError::NonFinite);
    }
    Ok(values.map(f32::to_bits))
}

fn finite_bits(value: f32) -> Result<u32, FactSnapshotError> {
    value
        .is_finite()
        .then(|| value.to_bits())
        .ok_or(FactSnapshotError::NonFinite)
}

fn terminal_reason(value: NativeTerminalReason) -> FactTerminalReason {
    match value {
        NativeTerminalReason::None => FactTerminalReason::None,
        NativeTerminalReason::GoalReached => FactTerminalReason::GoalReached,
        NativeTerminalReason::TickBudgetExhausted => FactTerminalReason::TickBudgetExhausted,
    }
}

fn availability(value: NativeChannelStatus) -> FactAvailability {
    match value {
        NativeChannelStatus::NotSampled => FactAvailability::NotSampled,
        NativeChannelStatus::Present => FactAvailability::Available,
        NativeChannelStatus::Absent => FactAvailability::Absent,
        NativeChannelStatus::Unavailable => FactAvailability::Unavailable,
    }
}

fn channels(observation: &NativeLearningObservation) -> ChannelFactSnapshot {
    ChannelFactSnapshot {
        camera: availability(observation.camera_status),
        player_action: availability(observation.player_action_status),
        background_collision: availability(observation.player_background_collision_status),
        collision_surfaces: availability(observation.player_collision_surfaces_status),
        scene_exit: availability(observation.scene_exit_status),
        dynamic_colliders: availability(observation.dynamic_colliders_status),
        player_resources: availability(observation.player_resources_status),
        player_relationships: availability(observation.player_relationships_status),
        collision_solver: availability(observation.player_collision_solver_status),
        process_lifecycle: availability(observation.process_lifecycle_status),
        event_transition: availability(observation.event_transition_status),
        room_load: availability(observation.room_load_status),
        warp_session: availability(observation.warp_session_status),
        resource_loads: availability(observation.resource_load_status),
    }
}

fn unavailable_channels() -> ChannelFactSnapshot {
    ChannelFactSnapshot {
        camera: FactAvailability::Unavailable,
        player_action: FactAvailability::Unavailable,
        background_collision: FactAvailability::Unavailable,
        collision_surfaces: FactAvailability::Unavailable,
        scene_exit: FactAvailability::Unavailable,
        dynamic_colliders: FactAvailability::Unavailable,
        player_resources: FactAvailability::Unavailable,
        player_relationships: FactAvailability::Unavailable,
        collision_solver: FactAvailability::Unavailable,
        process_lifecycle: FactAvailability::Unavailable,
        event_transition: FactAvailability::Unavailable,
        room_load: FactAvailability::Unavailable,
        warp_session: FactAvailability::Unavailable,
        resource_loads: FactAvailability::Unavailable,
    }
}

fn bank(value: &Option<Vec<u8>>) -> ByteBankFactSnapshot {
    match value {
        Some(bytes) => ByteBankFactSnapshot {
            availability: FactAvailability::Available,
            bytes: bytes.clone(),
        },
        None => ByteBankFactSnapshot {
            availability: FactAvailability::Unavailable,
            bytes: Vec::new(),
        },
    }
}

fn unavailable_banks() -> FlagBankFactSnapshot {
    let bank = || ByteBankFactSnapshot {
        availability: FactAvailability::Unavailable,
        bytes: Vec::new(),
    };
    FlagBankFactSnapshot {
        event: bank(),
        temporary: bank(),
        temporary_event: bank(),
        dungeon: bank(),
        switch: bank(),
        switch_room: None,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FactSnapshotError {
    Capacity,
    IncompleteSource,
    NonFinite,
    InvalidHistory,
    InvalidSource(String),
    InvalidSnapshot,
    Serialization(String),
}

impl fmt::Display for FactSnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Capacity => formatter.write_str("fact snapshot exceeds a finite capacity"),
            Self::IncompleteSource => formatter.write_str("fact snapshot source is incomplete"),
            Self::NonFinite => {
                formatter.write_str("fact snapshot source contains a non-finite value")
            }
            Self::InvalidHistory => formatter.write_str("fact snapshot history is not past-only"),
            Self::InvalidSource(message) => {
                write!(formatter, "fact snapshot source is invalid: {message}")
            }
            Self::InvalidSnapshot => formatter.write_str("fact snapshot is invalid"),
            Self::Serialization(message) => {
                write!(formatter, "fact snapshot serialization failed: {message}")
            }
        }
    }
}

impl Error for FactSnapshotError {}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_evidence::native_episode_shard::NativeEpisodeShard;

    #[test]
    fn current_native_observation_projects_to_one_stable_fact_snapshot() {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        let step = &shard.episodes[0].steps[0];
        let snapshot = FactSnapshot::from_native_learning(
            &step.post_simulation,
            &[step.pre_input.clone()],
            None,
            Vec::new(),
        )
        .unwrap();

        assert_eq!(snapshot.source, FactObservationSource::NativeLearning);
        assert_eq!(snapshot.world.stage, step.post_simulation.stage);
        assert_eq!(snapshot.actors.len(), step.post_simulation.actors.len());
        assert_eq!(snapshot.recent_history.len(), 1);
        assert_ne!(snapshot.content_sha256().unwrap(), Digest::ZERO);
        assert_eq!(
            snapshot.canonical_bytes().unwrap(),
            snapshot.canonical_bytes().unwrap()
        );
        let tactic = snapshot.to_native_tactic_observation().unwrap();
        assert_eq!(tactic.boundary_index, snapshot.boundary_index);
        assert_eq!(tactic.simulation_tick, snapshot.simulation_tick + 1);
        assert_eq!(tactic.tape_frame, snapshot.tape_frame + 1);
        assert_eq!(tactic.state_identity, snapshot.state_identity);
    }

    #[test]
    fn actors_without_portable_set_ids_do_not_block_non_actor_tactics() {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        let mut native = shard.episodes[0].steps[0].pre_input.clone();
        let actor = native
            .actors
            .first_mut()
            .expect("fixture must contain an actor");
        actor.set_id = u16::MAX;
        let snapshot = FactSnapshot::from_native_learning(&native, &[], None, Vec::new()).unwrap();
        assert!(
            snapshot
                .actors
                .iter()
                .any(|actor| actor.portable_selector.is_none())
        );
        let tactic = snapshot.to_native_tactic_observation().unwrap();
        let direct =
            crate::native_generic_tactic::NativeTacticObservation::from_native(&native).unwrap();
        assert!(tactic.actor_set_complete);
        assert_eq!(direct.actors, tactic.actors);
        assert_eq!(
            tactic.actors.len() + 1,
            snapshot.actors.len(),
            "only the unaddressable actor is omitted"
        );
    }

    #[test]
    fn history_must_be_strictly_past_only() {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        let current = &shard.episodes[0].steps[0].pre_input;
        assert_eq!(
            FactSnapshot::from_native_learning(current, &[current.clone()], None, Vec::new())
                .unwrap_err(),
            FactSnapshotError::InvalidHistory
        );
    }

    #[test]
    fn tactic_and_full_native_sources_share_the_same_core_facts() {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        let native = &shard.episodes[0].steps[0].pre_input;
        let tactic = NativeTacticObservation::from_native(native).unwrap();
        let full = FactSnapshot::from_native_learning(native, &[], None, Vec::new()).unwrap();
        let compact = FactSnapshot::from_native_tactic(&tactic, Vec::new()).unwrap();

        assert_eq!(compact.state_identity, full.state_identity);
        assert_eq!(compact.world.stage, full.world.stage);
        assert_eq!(compact.world.room, full.world.room);
        assert_eq!(
            compact.player.position_f32_bits,
            full.player.position_f32_bits
        );
        assert_eq!(compact.player.procedure, full.player.procedure);
        assert_eq!(compact.actors.len(), full.actors.len());
        assert_eq!(
            compact
                .actors
                .iter()
                .map(|actor| (actor.runtime_generation, actor.position_f32_bits))
                .collect::<Vec<_>>(),
            full.actors
                .iter()
                .map(|actor| (actor.runtime_generation, actor.position_f32_bits))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn complete_option_trajectory_retains_stalls_and_momentum_loss() {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        let template = shard.episodes[0].steps[0].clone();
        let mut first = template.clone();
        first.consumed_pad.stick_x = 127;
        first.pre_input.player_position = [0.0, 0.0, 0.0];
        first.pre_input.player_velocity = [0.0, 0.0, 0.0];
        first.post_simulation.player_position = [10.0, 0.0, 0.0];
        first.post_simulation.player_velocity = [10.0, 0.0, 0.0];
        first.post_simulation.collision_correction = None;
        first.post_simulation.player_contacts = 0;
        first.post_simulation.player_collision_solver = None;
        let mut second = template;
        second.consumed_pad.stick_x = 127;
        second.pre_input.player_position = [10.0, 0.0, 0.0];
        second.pre_input.player_velocity = [10.0, 0.0, 0.0];
        second.post_simulation.player_position = [20.0, 0.0, 0.0];
        second.post_simulation.player_velocity = [10.0, 0.0, 0.0];
        second.post_simulation.collision_correction = None;
        second.post_simulation.player_contacts = 0;
        second.post_simulation.player_collision_solver = None;

        let straight =
            OptionTrajectoryFactSnapshot::from_native_steps(&[first.clone(), second.clone()])
                .unwrap();
        assert_eq!(straight.observed_ticks, 2);
        assert_eq!(straight.commanded_stall_ticks, 0);
        assert_eq!(f32::from_bits(straight.planar_path_length_f32_bits), 20.0);
        assert_eq!(f32::from_bits(straight.planar_displacement_f32_bits), 20.0);
        assert_eq!(
            f32::from_bits(straight.commanded_momentum_loss_f32_bits),
            0.0
        );

        second.post_simulation.player_position = [10.0, 0.0, 0.0];
        second.post_simulation.player_velocity = [0.0, 0.0, 0.0];
        second.post_simulation.collision_correction = Some([2.0, 0.0]);
        second.post_simulation.player_contacts = 1 << 1;
        let bumped = OptionTrajectoryFactSnapshot::from_native_steps(&[first, second]).unwrap();
        assert_eq!(bumped.commanded_stall_ticks, 1);
        assert_eq!(bumped.wall_contact_ticks, 1);
        assert_eq!(bumped.collision_correction_ticks, 1);
        assert_eq!(f32::from_bits(bumped.mean_planar_speed_f32_bits), 5.0);
        assert_eq!(
            f32::from_bits(bumped.commanded_momentum_loss_f32_bits),
            10.0
        );
        assert_eq!(
            f32::from_bits(bumped.collision_correction_total_f32_bits),
            2.0
        );
        let mut tampered = bumped;
        tampered.mean_planar_speed_f32_bits = 99.0_f32.to_bits();
        assert!(!tampered.is_valid(2));
    }

    #[test]
    fn legacy_fact_snapshot_without_option_trajectory_still_decodes() {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        let mut snapshot = FactSnapshot::from_native_learning(
            &shard.episodes[0].steps[0].pre_input,
            &[],
            None,
            Vec::new(),
        )
        .unwrap();
        snapshot.schema = FACT_SNAPSHOT_SCHEMA_V1.into();
        let bytes = serde_json::to_vec(&snapshot).unwrap();
        let decoded: FactSnapshot = serde_json::from_slice(&bytes).unwrap();
        decoded.validate().unwrap();
        assert_eq!(decoded.schema, FACT_SNAPSHOT_SCHEMA_V1);
    }
}
