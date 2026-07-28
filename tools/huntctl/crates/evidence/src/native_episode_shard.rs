//! Decoder for native checkpoint-batch experience shards.
//!
//! The native writer emits one independently compressed episode per candidate.
//! This decoder validates every boundary/action join before the data may enter
//! replay or a learner view.

use crate::artifact::Digest;
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;

const MAGIC: &[u8; 8] = b"DUSKEPS\0";
const EPISODE_MAGIC: &[u8; 4] = b"EPIS";
const PAYLOAD_MAGIC: &[u8; 8] = b"DUSKEP\0\0";
const VERSION_V1: u16 = 1;
const VERSION_V2: u16 = 2;
const VERSION_V3: u16 = 3;
const HEADER_SIZE: usize = 128;
const BLOCK_HEADER_SIZE: usize = 64;
const PAYLOAD_HEADER_SIZE: usize = 24;
const COMPLETE: u32 = 1;
const SUCCESS: u16 = 1;
const OBSERVATION_VERSION_V2: u16 = 2;
const OBSERVATION_VERSION_V3: u16 = 3;
const OBSERVATION_VERSION_V4: u16 = 4;
const OBSERVATION_VERSION_V5: u16 = 5;
const OBSERVATION_VERSION_V6: u16 = 6;
const OBSERVATION_VERSION_V7: u16 = 7;
const OBSERVATION_VERSION_V8: u16 = 8;
const OBSERVATION_VERSION_V9: u16 = 9;
const OBSERVATION_VERSION_V10: u16 = 10;
const OBSERVATION_VERSION_V11: u16 = 11;
const OBSERVATION_VERSION_V12: u16 = 12;
const OBSERVATION_VERSION_V13: u16 = 13;
const OBSERVATION_VERSION_V14: u16 = 14;
const OBSERVATION_VERSION_V15: u16 = 15;
const OBSERVATION_VERSION_V16: u16 = 16;
const OBSERVATION_VERSION_V17: u16 = 17;
const OBSERVATION_VERSION_V18: u16 = 18;
const OBSERVATION_VERSION_V19: u16 = 19;
const OBSERVATION_VERSION_V20: u16 = 20;
const OBSERVATION_VERSION_V21: u16 = 21;
const OBSERVATION_VERSION_V22: u16 = 22;
const OBSERVATION_VERSION_V23: u16 = 23;
const OBSERVATION_VERSION_V24: u16 = 24;
const OBSERVATION_VERSION_V25: u16 = 25;
const OBSERVATION_VERSION_V26: u16 = 26;
const OBSERVATION_VERSION_V27: u16 = 27;
const OBSERVATION_VERSION_V28: u16 = 28;
const OBSERVATION_VERSION_V29: u16 = 29;
const ACTION_VERSION: u16 = 2;
const RNG_SNAPSHOT_VERSION: u32 = 1;
const RNG_ALGORITHM_VERSION: u32 = 1;
const ACTOR_NAME_DOOR20: i16 = 0x0e8;
const MAX_EPISODES: usize = 16_384;
const MAX_TICKS: usize = 4_096;
const MAX_ACTORS: usize = u16::MAX as usize;
const MAX_PENDING_PROCESS_RECORDS: usize = 1_000_000;
const MAX_EXPANDED_BYTES: usize = 16 * 1024 * 1024 * 1024;

#[path = "native_episode_relationships.rs"]
mod relationships;
use relationships::{decode_player_relationships, validate_player_relationship_joins};
#[path = "native_episode_collision_solver.rs"]
mod collision_solver;
use collision_solver::decode_player_collision_solver;
pub use collision_solver::{
    NativePlayerCollisionSolverObservation, NativePlayerCollisionSolverWall,
};
#[path = "native_episode_planner.rs"]
mod planner;
pub use planner::{
    NativeAttentionCandidateObservation, NativeAttentionCandidatesObservation,
    NativeCurrentEventObservation, NativeEventActorReferenceObservation,
    NativeEventHandoffObservation, NativeEventQueueObservation, NativeEventTransitionObservation,
    NativeMessageFlowObservation, NativeMessageSessionObservation,
    NativePendingEventOrderObservation, NativePendingStageObservation,
    NativePhysicalSlotObservation, NativePlayerControlObservation, NativeRestartObservation,
    NativeReturnPlaceObservation, NativeRuntimeFileObservation,
};

pub const NATIVE_EPISODE_SHARD_SCHEMA_V1: &str = "dusklight-native-episode-shard/v1";
pub const NATIVE_EPISODE_SHARD_SCHEMA_V2: &str = "dusklight-native-episode-shard/v2";
pub const NATIVE_EPISODE_SHARD_SCHEMA_V3: &str = "dusklight-native-episode-shard/v3";
pub const LEARNING_OBSERVATION_SCHEMA_V2: &str = "dusklight-learning-observation/v2";
pub const LEARNING_OBSERVATION_SCHEMA_V3: &str = "dusklight-learning-observation/v3";
pub const LEARNING_OBSERVATION_SCHEMA_V4: &str = "dusklight-learning-observation/v4";
pub const LEARNING_OBSERVATION_SCHEMA_V5: &str = "dusklight-learning-observation/v5";
pub const LEARNING_OBSERVATION_SCHEMA_V6: &str = "dusklight-learning-observation/v6";
pub const LEARNING_OBSERVATION_SCHEMA_V7: &str = "dusklight-learning-observation/v7";
pub const LEARNING_OBSERVATION_SCHEMA_V8: &str = "dusklight-learning-observation/v8";
pub const LEARNING_OBSERVATION_SCHEMA_V9: &str = "dusklight-learning-observation/v9";
pub const LEARNING_OBSERVATION_SCHEMA_V10: &str = "dusklight-learning-observation/v10";
pub const LEARNING_OBSERVATION_SCHEMA_V11: &str = "dusklight-learning-observation/v11";
pub const LEARNING_OBSERVATION_SCHEMA_V12: &str = "dusklight-learning-observation/v12";
pub const LEARNING_OBSERVATION_SCHEMA_V13: &str = "dusklight-learning-observation/v13";
pub const LEARNING_OBSERVATION_SCHEMA_V14: &str = "dusklight-learning-observation/v14";
pub const LEARNING_OBSERVATION_SCHEMA_V15: &str = "dusklight-learning-observation/v15";
pub const LEARNING_OBSERVATION_SCHEMA_V16: &str = "dusklight-learning-observation/v16";
pub const LEARNING_OBSERVATION_SCHEMA_V17: &str = "dusklight-learning-observation/v17";
pub const LEARNING_OBSERVATION_SCHEMA_V18: &str = "dusklight-learning-observation/v18";
pub const LEARNING_OBSERVATION_SCHEMA_V19: &str = "dusklight-learning-observation/v19";
pub const LEARNING_OBSERVATION_SCHEMA_V20: &str = "dusklight-learning-observation/v20";
pub const LEARNING_OBSERVATION_SCHEMA_V21: &str = "dusklight-learning-observation/v21";
pub const LEARNING_OBSERVATION_SCHEMA_V22: &str = "dusklight-learning-observation/v22";
pub const LEARNING_OBSERVATION_SCHEMA_V23: &str = "dusklight-learning-observation/v23";
pub const LEARNING_OBSERVATION_SCHEMA_V24: &str = "dusklight-learning-observation/v24";
pub const LEARNING_OBSERVATION_SCHEMA_V25: &str = "dusklight-learning-observation/v25";
pub const LEARNING_OBSERVATION_SCHEMA_V26: &str = "dusklight-learning-observation/v26";
pub const LEARNING_OBSERVATION_SCHEMA_V27: &str = "dusklight-learning-observation/v27";
pub const LEARNING_OBSERVATION_SCHEMA_V28: &str = "dusklight-learning-observation/v28";
pub const LEARNING_OBSERVATION_SCHEMA_V29: &str = "dusklight-learning-observation/v29";
pub const RAW_PAD_ACTION_SCHEMA_V2: &str = "dusklight-raw-pad-action/v2";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeEpisodeShardMetadata {
    pub shard_schema: String,
    pub observation_schema: String,
    pub action_schema: String,
    pub source_boundary_fingerprint: String,
    pub checkpoint_identity: String,
    pub objective: String,
    pub objective_identity: String,
    pub build_revision: String,
    pub aurora_revision: String,
    pub feature_digest: String,
    pub fidelity_profile: String,
    pub game_data_sha256: Option<Digest>,
    pub card_fixture_identity: Option<String>,
    pub actor_profile_catalog_identity: Option<String>,
    pub world_context_sha256: Option<Digest>,
    pub policy_model: Option<NativeEpisodePolicyModelIdentity>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeEpisodePolicyModelIdentity {
    pub schema: String,
    pub model_xxh3_128: String,
    pub feature_schema_sha256: Digest,
    pub action_schema_sha256: Digest,
    pub objective_sha256: Digest,
    pub feature_width: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeEpisodeShard {
    pub content_sha256: Digest,
    pub source_frame: u64,
    pub maximum_ticks: u32,
    pub metadata: NativeEpisodeShardMetadata,
    pub episodes: Vec<NativeEpisode>,
    pub uncompressed_bytes: u64,
    pub compressed_bytes: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeEpisode {
    pub id: String,
    pub success: bool,
    pub ticks_executed: u32,
    pub first_hit_tick: Option<u32>,
    pub remaining_ticks: u32,
    pub payload_xxh3_128: [u8; 16],
    pub steps: Vec<NativeEpisodeStep>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeEpisodeStep {
    pub pre_input: NativeLearningObservation,
    pub chosen_pad: NativeRawPad,
    pub consumed_pad: NativeRawPad,
    pub post_simulation: NativeLearningObservation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeReturnRestartWriteTraceObservation {
    pub return_place_initialize_count: u16,
    pub return_place_set_count: u16,
    pub savmem_execute_count: u16,
    pub savmem_eligible_execute_count: u16,
    pub restart_place_set_count: u16,
    pub restart_start_point_set_count: u16,
    pub restart_room_parameter_set_count: u16,
    pub restart_last_scene_info_set_count: u16,
    pub return_place_value_change_count: u16,
    pub restart_place_value_change_count: u16,
    pub restart_start_point_value_change_count: u16,
    pub restart_room_parameter_value_change_count: u16,
    pub restart_last_scene_info_value_change_count: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeObservationPhase {
    PreInput,
    PostSimulation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeTerminalReason {
    None,
    GoalReached,
    TickBudgetExhausted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeActorSelectionRule {
    Complete,
    LowestRuntimeGeneration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeChannelStatus {
    NotSampled,
    Present,
    Absent,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeCameraObservation {
    pub view_yaw: i16,
    pub controlled_yaw: i16,
    pub bank: i16,
    pub eye: [f32; 3],
    pub center: [f32; 3],
    pub up: [f32; 3],
    pub fovy: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeAnimationLane {
    pub resource_id: u16,
    pub frame: f32,
    pub rate: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeTraceActorIdentity {
    pub runtime_generation: u32,
    pub actor_name: i16,
    pub set_id: u16,
    pub home_room: i8,
    pub current_room: i8,
    pub home_position: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativePlayerActionObservation {
    pub procedure_id: u16,
    pub mode_flags: u32,
    pub procedure_context_raw: [i16; 6],
    pub damage_wait_timer: i16,
    pub sword_at_up_time: u16,
    pub ice_damage_wait_timer: i16,
    pub sword_change_wait_timer: u8,
    pub under_animations: [NativeAnimationLane; 3],
    pub upper_animations: [NativeAnimationLane; 3],
    pub flags: u32,
    pub do_status: u8,
    pub talk_partner: NativeTraceActorIdentity,
    pub grabbed_actor: NativeTraceActorIdentity,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeSceneExitObservation {
    pub runtime_generation: u32,
    pub raw_parameters: u32,
    pub flags: u32,
    pub signed_distance_to_volume: f32,
    pub actor_name: i16,
    pub set_id: u16,
    pub exit_id: u8,
    pub path_id: u8,
    pub argument1: u8,
    pub switch_no: u8,
    pub kind: u8,
    pub observed_count: u8,
    pub home_room: i8,
    pub link_exit_direction: u8,
    pub link_exit_id: u16,
    pub shape_yaw: i16,
    pub actor_action: u8,
    pub player_local_position: [f32; 3],
    pub volume_extent: [f32; 3],
    pub home_position: [f32; 3],
    pub destination_stage: String,
    pub destination_room: i8,
    pub destination_layer: i8,
    pub destination_point: i16,
    pub destination_wipe: u8,
    pub destination_wipe_time: u8,
    pub destination_time_hour: i8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeRawPad {
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

#[derive(Clone, Debug, PartialEq)]
pub struct NativeActorIdentity {
    pub present: bool,
    pub runtime_generation: u32,
    pub actor_name: i16,
    pub set_id: u16,
    pub home_room: i8,
    pub current_room: i8,
    pub home_position: Option<[f32; 3]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeActorObservation {
    pub runtime_generation: u64,
    pub base_state_available: bool,
    pub actor_type: i32,
    pub process_subtype: i32,
    pub parent_runtime_generation: u32,
    pub parameters: u32,
    pub status: u32,
    pub condition: u32,
    pub actor_name: i16,
    pub profile_name: i16,
    pub set_id: u16,
    pub home_room: i8,
    pub old_room: i8,
    pub current_room: i8,
    pub group: u8,
    pub argument: i8,
    pub pause_flag: u8,
    pub process_init_state: i8,
    pub process_create_phase: u8,
    pub cull_type: u8,
    pub demo_actor_id: u8,
    pub carry_type: u8,
    pub heap_present: bool,
    pub model_present: bool,
    pub joint_collision_present: bool,
    pub health: i16,
    pub position: [f32; 3],
    pub home_position: [f32; 3],
    pub old_position: [f32; 3],
    pub velocity: [f32; 3],
    pub forward_speed: f32,
    pub scale: [f32; 3],
    pub gravity: f32,
    pub max_fall_speed: f32,
    pub eye_position: [f32; 3],
    pub home_angle: [i16; 3],
    pub old_angle: [i16; 3],
    pub current_angle: [i16; 3],
    pub shape_angle: [i16; 3],
    pub attention: Option<NativeActorAttentionComponent>,
    pub event_participation: Option<NativeActorEventParticipationComponent>,
    pub return_place_writer: Option<NativeReturnPlaceWriterComponent>,
    pub enemy_base: Option<NativeEnemyBaseComponent>,
    pub trigger_volume: Option<NativeTriggerVolumeComponent>,
    pub door20: Option<NativeDoor20Component>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeDoor20Action {
    Init,
    Wait,
    StopClose,
    Demo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeDoor20Side {
    Front,
    Back,
    Neither,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeDoor20StopperStatus {
    RoomUnavailable,
    Open,
    Closed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeDoor20Component {
    pub kind: u8,
    pub door_model: u8,
    pub front_option: u8,
    pub back_option: u8,
    pub front_room: u8,
    pub back_room: u8,
    pub exit_number: u8,
    pub message_door: bool,
    pub front_switch: u8,
    pub back_switch: u8,
    pub unlock_effect_switch: u8,
    pub front_switch_set: bool,
    pub back_switch_set: bool,
    pub unlock_effect_switch_set: bool,
    pub front_event: u8,
    pub back_event: u8,
    pub message_number: u16,
    pub action: NativeDoor20Action,
    pub active_side: NativeDoor20Side,
    pub event_variant: u8,
    pub locked: bool,
    pub background_collision_released: bool,
    pub unlock_effect_triggered: bool,
    pub key_type: u8,
    pub enemy_clear_debounce: u8,
    pub opening_active: bool,
    pub closing_active: bool,
    pub door_angle: i16,
    pub stopper_side: NativeDoor20Side,
    pub front_stopper_status: NativeDoor20StopperStatus,
    pub back_stopper_status: NativeDoor20StopperStatus,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeEnemyBaseComponent {
    pub flags: u16,
    pub throw_mode: u8,
    pub down_position: [f32; 3],
    pub head_lock_position: [f32; 3],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeTriggerVolumeKind {
    SceneExit,
    SceneExitCylinder,
    EventArea,
    ScriptedEvent,
    MappedEvent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeTriggerVolumeShape {
    Box,
    EllipticCylinder,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeTriggerVolumeComponent {
    pub kind: NativeTriggerVolumeKind,
    pub shape: NativeTriggerVolumeShape,
    pub enabled: bool,
    pub vertical_unbounded: bool,
    pub behavior: u16,
    pub center: [f32; 3],
    pub half_extent: [f32; 3],
    pub yaw: i16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeReturnPlaceWriterComponent {
    pub save_room: i8,
    pub save_point: u8,
    pub switch_room: i8,
    pub required_event_set: u16,
    pub required_event_unset: u16,
    pub required_switch_set: u8,
    pub required_switch_unset: u8,
    pub no_telop_clear: bool,
    pub event_set_satisfied: bool,
    pub event_unset_satisfied: bool,
    pub switch_set_satisfied: bool,
    pub switch_unset_satisfied: bool,
    pub eligible: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeActorAttentionComponent {
    pub flags: u32,
    pub position: [f32; 3],
    pub distance_indices: [u8; 9],
    pub auxiliary: i16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeActorEventParticipationComponent {
    pub command: u16,
    pub condition: u16,
    pub event_id: i16,
    pub map_tool_id: u8,
    pub index: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeDynamicColliderShape {
    Unknown,
    Sphere,
    Cylinder,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeDynamicColliderObservation {
    pub registration_index: u16,
    pub owner_runtime_generation: Option<u32>,
    pub attack_hit_owner_runtime_generation: Option<u32>,
    pub target_hit_owner_runtime_generation: Option<u32>,
    pub correction_hit_owner_runtime_generation: Option<u32>,
    pub status_present: bool,
    pub shape_present: bool,
    pub attack_set: bool,
    pub target_set: bool,
    pub correction_set: bool,
    pub attack_hit: bool,
    pub target_hit: bool,
    pub correction_hit: bool,
    pub shape: NativeDynamicColliderShape,
    pub attack_type: u32,
    pub target_type: u32,
    pub attack_source_parameters: u32,
    pub attack_result_parameters: u32,
    pub target_source_parameters: u32,
    pub target_result_parameters: u32,
    pub correction_source_parameters: u32,
    pub correction_result_parameters: u32,
    pub attack_power: u8,
    pub weight: u8,
    pub damage: u8,
    pub center: [f32; 3],
    pub radius: f32,
    pub height: f32,
    pub aabb_min: [f32; 3],
    pub aabb_max: [f32; 3],
    pub correction: [f32; 3],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NativePlayerResourcesObservation {
    pub maximum_life: u16,
    pub life: u16,
    pub rupees: u16,
    pub rupee_capacity: u16,
    pub maximum_oil: u16,
    pub oil: u16,
    pub maximum_magic: u8,
    pub magic: u8,
    pub wallet: u8,
    pub transform_status: u8,
    pub world_time: f32,
    pub date: u16,
    pub arrows: u8,
    pub arrow_capacity: u8,
    pub pachinko: u8,
    pub poe_souls: u8,
    pub small_keys: u8,
    pub dungeon_map: bool,
    pub dungeon_compass: bool,
    pub dungeon_boss_key: bool,
    pub dungeon_warp: bool,
    pub inventory: [u8; 24],
    pub selected_items: [u8; 4],
    pub mixed_items: [u8; 4],
    pub equipment: [u8; 6],
    pub bomb_counts: [u8; 3],
    pub bomb_capacities: [u8; 3],
    pub bottle_quantities: [u8; 4],
    pub acquired_item_bits: [u8; 32],
    pub collect_item_bits: [u8; 8],
    pub collected_crystal_bits: u8,
    pub collected_mirror_bits: u8,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NativePlayerRelationshipsObservation {
    pub targeted_actor: Option<NativeActorIdentity>,
    pub ride_actor: Option<NativeActorIdentity>,
    pub held_item_actor: Option<NativeActorIdentity>,
    pub grabbed_actor: Option<NativeActorIdentity>,
    pub thrown_boomerang_actor: Option<NativeActorIdentity>,
    pub copy_rod_actor: Option<NativeActorIdentity>,
    pub hookshot_roof_wait_actor: Option<NativeActorIdentity>,
    pub chain_grab_actor: Option<NativeActorIdentity>,
    pub attention_hint_actor: Option<NativeActorIdentity>,
    pub attention_catch_actor: Option<NativeActorIdentity>,
    pub attention_look_actor: Option<NativeActorIdentity>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeGoalObservation {
    pub configured: bool,
    pub reached: bool,
    pub requested_count: u16,
    pub hit_count: u16,
    pub stable_ticks: u16,
    pub consecutive_ticks: u16,
    pub sequence_steps: u8,
    pub sequence_next_step: u8,
    pub sequence_within_ticks: u16,
    pub sequence_elapsed_ticks: u16,
    pub first_hit_tick: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeCollisionWallObservation {
    pub bg_index: u16,
    pub poly_index: u16,
    pub owner_runtime_generation: u32,
    pub angle_y: i16,
    pub flags: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativePlayerBackgroundCollision {
    pub flags: u32,
    pub ground_height: f32,
    pub roof_height: f32,
    pub water_height: f32,
    pub ground_identity: [u32; 3],
    pub ground_plane: [f32; 4],
    pub roof_identity: [u32; 3],
    pub water_identity: [u32; 3],
    pub walls: [NativeCollisionWallObservation; 3],
    pub old_position: [f32; 3],
    pub resolved_frame_displacement: [f32; 3],
    pub final_position: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeCollisionSurfaceObservation {
    pub flags: u32,
    pub kind: u8,
    pub wall_slot: u8,
    pub backing_format: u8,
    pub raw_code_presence_mask: u8,
    pub bg_index: u16,
    pub poly_index: u16,
    pub owner_runtime_generation: u32,
    pub material_index: u16,
    pub group_index: u16,
    pub raw_codes: [u32; 5],
    pub raw_exit_id: u8,
    pub source_room: i8,
    pub scls_source_room: i8,
    pub destination_room: i8,
    pub destination_layer: i8,
    pub destination_wipe: u8,
    pub destination_wipe_time: u8,
    pub destination_time_hour: i8,
    pub destination_point: i16,
    pub source_geometry_indices: Vec<u16>,
    pub kcl_prism_height: f32,
    pub destination_stage: String,
    pub plane: Option<[f32; 4]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativePlayerCollisionSurfaces {
    pub flags: u32,
    pub current_room: i8,
    pub identity_count: u8,
    pub backing_code_count: u8,
    pub destination_count: u8,
    pub raw_link_exit: u16,
    pub pending_stage_match_mask: u8,
    pub surfaces: Vec<NativeCollisionSurfaceObservation>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeRngStream {
    pub id: u8,
    pub algorithm_version: u32,
    pub state: [i32; 3],
    pub call_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativePendingProcessState {
    pub runtime_generation: u32,
    pub process_name: i16,
    pub profile_name: i16,
    pub process_type: i32,
    pub process_subtype: i32,
    pub parameters: u32,
    pub init_state: i8,
    pub create_phase: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativePendingCreateObservation {
    pub runtime_generation: u32,
    pub doing: bool,
    pub cancelled: bool,
    pub process_status: NativeChannelStatus,
    pub process: Option<NativePendingProcessState>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativePendingDeleteObservation {
    pub process: NativePendingProcessState,
    pub timer: i16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeProcessLifecycleObservation {
    pub active_actor_count: u32,
    pub pending_create_count: u32,
    pub pending_delete_count: u32,
    pub pending_creates: Vec<NativePendingCreateObservation>,
    pub pending_deletes: Vec<NativePendingDeleteObservation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeClockDomainObservation {
    pub framework_frames: u32,
    pub gameplay_frames: u32,
    pub global_pause: bool,
    pub scene_paused: bool,
    pub scene_pause_timer: i8,
    pub scene_next_pause_timer: i8,
    pub overlap_request_active: bool,
    pub overlap_fadeout_peek: bool,
    pub demo_status: NativeChannelStatus,
    pub demo_mode: i32,
    pub demo_frame: u32,
    pub demo_frame_no_message: u32,
    pub demo_flags: u32,
    pub timer_status: NativeChannelStatus,
    pub timer_mode: i32,
    pub timer_now_ms: i32,
    pub timer_limit_ms: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeRoomLoadEntryObservation {
    pub room: u8,
    pub status_flags: u8,
    pub draw: bool,
    pub zone_count: i8,
    pub zone: i8,
    pub memory_block: i8,
    pub region: u8,
    pub scene_status: NativeChannelStatus,
    pub scene_phase: i32,
    pub scene_phase_active: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeRoomLoadObservation {
    pub room_read: i8,
    pub stay_room: i8,
    pub old_stay_room: i8,
    pub next_stay_room: i8,
    pub no_change_room: bool,
    pub time_pass: bool,
    pub rooms: Vec<NativeRoomLoadEntryObservation>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeWarpSelectionObservation {
    pub stage: String,
    pub position: [f32; 3],
    pub angle: i16,
    pub room: i8,
    pub parameter: u8,
    pub player: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeWarpReturnMarkObservation {
    pub stage: String,
    pub position: [f32; 3],
    pub angle: i16,
    pub room: i8,
    pub accept_stage: i8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeWarpSessionObservation {
    pub request_kind: u8,
    pub selection: Option<NativeWarpSelectionObservation>,
    pub return_mark: Option<NativeWarpReturnMarkObservation>,
    pub target_point: Option<u8>,
    pub selected_point: Option<u8>,
    pub transport_match: bool,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum NativeResourceArchiveKind {
    Object,
    Stage,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum NativeResourceLoadOutcome {
    Mounting,
    Ready,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeResourceLoadEntryObservation {
    pub kind: NativeResourceArchiveKind,
    pub slot: u8,
    pub outcome: NativeResourceLoadOutcome,
    pub mount_command_present: bool,
    pub archive_present: bool,
    pub data_heap_present: bool,
    pub resource_table_present: bool,
    pub reference_count: u16,
    pub archive_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeResourceLoadObservation {
    pub object_capacity: u16,
    pub stage_capacity: u16,
    pub object_count: u16,
    pub stage_count: u16,
    pub entries: Vec<NativeResourceLoadEntryObservation>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeLearningObservation {
    pub phase: NativeObservationPhase,
    pub terminal_reason: NativeTerminalReason,
    pub actor_selection: NativeActorSelectionRule,
    pub actors_truncated: bool,
    pub actor_observed_count: u32,
    pub boundary_index: u64,
    pub simulation_tick: u64,
    pub tape_frame: u64,
    pub remaining_ticks: u32,
    pub state_identity: [u8; 16],
    pub stage: String,
    pub room: i8,
    pub layer: i8,
    pub point: i16,
    pub next_stage: Option<String>,
    pub next_room: i8,
    pub next_layer: i8,
    pub next_point: i16,
    pub player_present: bool,
    pub player_is_link: bool,
    pub player_process_id: u32,
    pub player_actor_name: i16,
    pub player_procedure: u16,
    pub player_position: [f32; 3],
    pub player_velocity: [f32; 3],
    pub player_forward_speed: f32,
    pub player_current_angle: [i16; 3],
    pub player_shape_angle: [i16; 3],
    pub player_mode_flags: u32,
    pub player_damage_wait_timer: i16,
    pub player_ice_damage_wait_timer: i16,
    pub player_sword_change_wait_timer: u8,
    pub player_do_status: u8,
    pub player_contacts: u8,
    pub player_ground_height: Option<f32>,
    pub player_roof_height: Option<f32>,
    pub event_running: bool,
    pub event_id: i16,
    pub event_mode: u8,
    pub event_status: u8,
    pub event_map_tool_id: u8,
    pub event_name_hash: Option<u32>,
    pub menu_flags: u16,
    pub menu_procedures: [u8; 5],
    pub camera_yaw_radians: Option<f32>,
    pub collision_correction: Option<[f32; 2]>,
    pub camera_status: NativeChannelStatus,
    pub camera: Option<NativeCameraObservation>,
    pub player_action_status: NativeChannelStatus,
    pub player_action: Option<NativePlayerActionObservation>,
    pub player_background_collision_status: NativeChannelStatus,
    pub player_background_collision: Option<NativePlayerBackgroundCollision>,
    pub player_collision_surfaces_status: NativeChannelStatus,
    pub player_collision_surfaces: Option<NativePlayerCollisionSurfaces>,
    pub scene_exit_status: NativeChannelStatus,
    pub scene_exit: Option<NativeSceneExitObservation>,
    pub player_form_present: bool,
    pub player_is_wolf: bool,
    pub previous_input: NativeRawPad,
    pub rng_version: u32,
    pub rng_streams: Vec<NativeRngStream>,
    pub talk_partner: NativeActorIdentity,
    pub grabbed_actor: NativeActorIdentity,
    pub goal: NativeGoalObservation,
    pub actors: Vec<NativeActorObservation>,
    pub dynamic_colliders_status: NativeChannelStatus,
    pub dynamic_colliders: Vec<NativeDynamicColliderObservation>,
    pub player_resources_status: NativeChannelStatus,
    pub player_resources: Option<NativePlayerResourcesObservation>,
    pub player_relationships_status: NativeChannelStatus,
    pub player_relationships: Option<NativePlayerRelationshipsObservation>,
    pub player_collision_solver_status: NativeChannelStatus,
    pub player_collision_solver: Option<NativePlayerCollisionSolverObservation>,
    pub event_flags: Option<Vec<u8>>,
    pub temporary_flags: Option<Vec<u8>>,
    /// Exact 256-byte dSv_info_c::mTmp.mEvent register bank (v5+).
    pub temporary_event_bytes: Option<Vec<u8>>,
    pub dungeon_flags: Option<Vec<u8>>,
    pub switch_flags: Option<Vec<u8>>,
    pub switch_flag_room: i8,
    pub runtime_file_status: NativeChannelStatus,
    pub runtime_file: Option<NativeRuntimeFileObservation>,
    pub return_place_status: NativeChannelStatus,
    pub return_place: Option<NativeReturnPlaceObservation>,
    pub restart_status: NativeChannelStatus,
    pub restart: Option<NativeRestartObservation>,
    pub return_restart_write_trace_status: NativeChannelStatus,
    pub return_restart_write_trace: Option<NativeReturnRestartWriteTraceObservation>,
    pub event_handoff_status: NativeChannelStatus,
    pub event_handoff: Option<NativeEventHandoffObservation>,
    pub message_session_status: NativeChannelStatus,
    pub message_session: Option<NativeMessageSessionObservation>,
    pub event_queue_status: NativeChannelStatus,
    pub event_queue: Option<NativeEventQueueObservation>,
    pub process_lifecycle_status: NativeChannelStatus,
    pub process_lifecycle: Option<NativeProcessLifecycleObservation>,
    pub attention_candidates_status: NativeChannelStatus,
    pub attention_candidates: Option<NativeAttentionCandidatesObservation>,
    pub event_transition_status: NativeChannelStatus,
    pub event_transition: Option<NativeEventTransitionObservation>,
    pub clock_domains_status: NativeChannelStatus,
    pub clock_domains: Option<NativeClockDomainObservation>,
    pub room_load_status: NativeChannelStatus,
    pub room_load: Option<NativeRoomLoadObservation>,
    pub warp_session_status: NativeChannelStatus,
    pub warp_session: Option<NativeWarpSessionObservation>,
    pub resource_load_status: NativeChannelStatus,
    pub resource_loads: Option<NativeResourceLoadObservation>,
}

#[derive(Debug)]
pub struct NativeEpisodeShardError(String);

impl NativeEpisodeShardError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for NativeEpisodeShardError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeEpisodeShardError {}

mod actor_channels;
mod episode_decode;
mod objective_identity;
mod observation_decode;
mod planner_channels;
mod player_channels;
mod process_channels;
mod reader;
mod runtime_channels;
mod shard_decode;

use actor_channels::*;
use episode_decode::*;
pub use objective_identity::authored_milestone_objective_identity;
use observation_decode::*;
use planner_channels::*;
use player_channels::*;
use process_channels::*;
use reader::*;
use runtime_channels::*;

#[cfg(test)]
#[path = "native_episode_shard_tests.rs"]
mod tests;
