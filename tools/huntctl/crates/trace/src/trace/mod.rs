use crate::scenario_fixture::ScenarioFixture;
use crate::tape::{RawPadState, TapeBoot};
use serde::Serialize;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

const V1_HEADER_SIZE: usize = 36;
const V1_RECORD_SIZE: usize = 102;
const V2_HEADER_SIZE: usize = 64;
const V3_HEADER_SIZE: usize = 128;
const V4_HEADER_SIZE: usize = 128;
const V5_HEADER_SIZE: usize = 128;
const V2_DIRECTORY_ENTRY_SIZE: usize = 64;
const MAGIC: &[u8; 8] = b"DUSKTRCE";
const MAX_TRACE_RECORDS: usize = 131_072;

const FILE_COMPLETE: u32 = 1 << 0;
const FILE_CAPACITY_EXHAUSTED: u32 = 1 << 1;
const FILE_TRIGGER_RETENTION: u32 = 1 << 2;
const RETENTION_CRASH: u32 = 1 << 0;
const RETENTION_NOVEL_CONTACT: u32 = 1 << 1;
const RETENTION_FLAG_CHANGE: u32 = 1 << 2;
const RETENTION_PREDICATE_HIT: u32 = 1 << 3;
const KNOWN_RETENTION_TRIGGERS: u32 =
    RETENTION_CRASH | RETENTION_NOVEL_CONTACT | RETENTION_FLAG_CHANGE | RETENTION_PREDICATE_HIT;
const CHANNEL_REQUIRED: u32 = 1 << 0;
const CHANNEL_DENSE: u32 = 1 << 1;
const CORE_SIMULATION_TICK_VALID: u32 = 1 << 0;
const CORE_TAPE_FRAME_VALID: u32 = 1 << 1;
const INPUT_TAPE: u8 = 1 << 0;
const INPUT_CONTROLLER: u8 = 1 << 1;
const INPUT_LIVE: u8 = 1 << 2;
const KNOWN_INPUT_SOURCES: u8 = INPUT_TAPE | INPUT_CONTROLLER | INPUT_LIVE;
const PLAYER_IS_LINK: u32 = 1 << 0;
const EVENT_RUNNING: u32 = 1 << 0;
const EVENT_NAME_HASH_PRESENT: u32 = 1 << 1;
const SCENE_EXIT_VOLUME_VALID: u32 = 1 << 0;
const SCENE_EXIT_PLAYER_INSIDE: u32 = 1 << 1;
const SCENE_EXIT_PLAYER_LATCHED: u32 = 1 << 2;
const SCENE_EXIT_CHANGE_OK: u32 = 1 << 3;
const SCENE_EXIT_CHANGE_STARTED: u32 = 1 << 4;
const SCENE_EXIT_DESTINATION_VALID: u32 = 1 << 5;
const SCENE_EXIT_OBSERVED_COUNT_SATURATED: u32 = 1 << 6;
const SCENE_EXIT_KNOWN_FLAGS: u32 = 0x7f;
const COLLISION_GROUND_PROBE_VALID: u32 = 1 << 0;
const COLLISION_GROUND_CONTACT: u32 = 1 << 1;
const COLLISION_GROUND_LANDING: u32 = 1 << 2;
const COLLISION_GROUND_PLANE_VALID: u32 = 1 << 4;
const COLLISION_GROUND_OWNER_PRESENT: u32 = 1 << 5;
const COLLISION_WALL_CONTACT: u32 = 1 << 6;
const COLLISION_ROOF_PROBE_VALID: u32 = 1 << 7;
const COLLISION_ROOF_CONTACT: u32 = 1 << 8;
const COLLISION_ROOF_OWNER_PRESENT: u32 = 1 << 9;
const COLLISION_WATER_PROBE_ENABLED: u32 = 1 << 10;
const COLLISION_WATER_SURFACE_FOUND: u32 = 1 << 11;
const COLLISION_WATER_IN: u32 = 1 << 12;
const COLLISION_WATER_OWNER_PRESENT: u32 = 1 << 13;
const COLLISION_WALL_PROBE_ENABLED: u32 = 1 << 14;
const COLLISION_TRAJECTORY_VALID: u32 = 1 << 15;
const COLLISION_GROUND_IDENTITY_PRESENT: u32 = 1 << 16;
const COLLISION_ROOF_IDENTITY_PRESENT: u32 = 1 << 17;
const COLLISION_WATER_IDENTITY_PRESENT: u32 = 1 << 18;
const COLLISION_KNOWN_FLAGS: u32 = 0x7ffff;
const COLLISION_WALL_HIT: u16 = 1 << 0;
const COLLISION_WALL_OWNER_PRESENT: u16 = 1 << 1;
const COLLISION_WALL_IDENTITY_PRESENT: u16 = 1 << 2;
const COLLISION_WALL_KNOWN_FLAGS: u16 = 0x7;
const COLLISION_SURFACE_SET_ROOM_VALID: u32 = 1 << 0;
const COLLISION_SURFACE_SET_EXPLICIT_LINK_EXIT: u32 = 1 << 1;
const COLLISION_SURFACE_SET_NEXT_STAGE_PENDING: u32 = 1 << 2;
const COLLISION_SURFACE_SET_KNOWN_FLAGS: u32 = 0x7;
const COLLISION_SURFACE_IDENTITY_PRESENT: u32 = 1 << 0;
const COLLISION_SURFACE_OWNER_PRESENT: u32 = 1 << 1;
const COLLISION_SURFACE_BACKING_PRESENT: u32 = 1 << 2;
const COLLISION_SURFACE_CODES_PRESENT: u32 = 1 << 3;
const COLLISION_SURFACE_MATERIAL_PRESENT: u32 = 1 << 4;
const COLLISION_SURFACE_GROUP_PRESENT: u32 = 1 << 5;
const COLLISION_SURFACE_SOURCE_ROOM_PRESENT: u32 = 1 << 6;
const COLLISION_SURFACE_SOURCE_ROOM_EXACT: u32 = 1 << 7;
const COLLISION_SURFACE_SCLS_SOURCE_PRESENT: u32 = 1 << 8;
const COLLISION_SURFACE_DESTINATION_PRESENT: u32 = 1 << 9;
const COLLISION_SURFACE_PENDING_MATCH: u32 = 1 << 10;
const COLLISION_SURFACE_GEOMETRY_PRESENT: u32 = 1 << 11;
const COLLISION_SURFACE_KCL_HEIGHT_PRESENT: u32 = 1 << 12;
const COLLISION_SURFACE_KNOWN_FLAGS: u32 = 0x1fff;
const GOAL_CONFIGURED: u32 = 1 << 0;
const GOAL_REACHED: u32 = 1 << 1;
const GOAL_AUTHORED: u32 = 1 << 2;
const GOAL_FIRST_HIT_TICK_PRESENT: u32 = 1 << 3;
const GOAL_KNOWN_FLAGS: u32 = 0x0f;
const SELECTED_ACTORS_TRUNCATED: u32 = 1 << 0;
const TALK_PARTNER_PRESENT: u32 = 1 << 0;
const GRABBED_ACTOR_PRESENT: u32 = 1 << 1;
const PLAYER_ACTION_KNOWN_FLAGS: u32 = TALK_PARTNER_PRESENT | GRABBED_ACTOR_PRESENT;
const SELECTED_ACTOR_CAPACITY: usize = 16;
const INVALID_U16_ID: u16 = u16::MAX;
const INVALID_U32_ID: u32 = u32::MAX;
const INVALID_I8: i8 = i8::MIN;
const INVALID_I16: i16 = i16::MIN;

// Compatibility flags retained for the movement-v1 featurizer.
const LEGACY_PLAYER_PRESENT: u32 = 1 << 0;
const LEGACY_PLAYER_IS_LINK: u32 = 1 << 1;
const LEGACY_EVENT_RUNNING: u32 = 1 << 2;
const LEGACY_TAPE_PLAYING: u32 = 1 << 3;
const LEGACY_CONTROLLER_PLAYING: u32 = 1 << 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[repr(u16)]
#[serde(rename_all = "snake_case")]
pub enum TraceChannel {
    Core = 0,
    Stage = 1,
    AppliedPads = 2,
    PlayerMotion = 3,
    Event = 4,
    SceneExit = 5,
    Rng = 6,
    Camera = 7,
    PlayerAction = 8,
    PlayerBackgroundCollision = 9,
    PlayerCollisionSurfaces = 10,
    GoalProgress = 11,
    SelectedActors = 12,
}

impl TraceChannel {
    pub const ALL: [Self; 13] = [
        Self::Core,
        Self::Stage,
        Self::AppliedPads,
        Self::PlayerMotion,
        Self::Event,
        Self::SceneExit,
        Self::Rng,
        Self::Camera,
        Self::PlayerAction,
        Self::PlayerBackgroundCollision,
        Self::PlayerCollisionSurfaces,
        Self::GoalProgress,
        Self::SelectedActors,
    ];

    pub const fn bit(self) -> u64 {
        1_u64 << self as u16
    }

    fn from_id(id: u16) -> Option<Self> {
        Self::ALL.into_iter().find(|channel| *channel as u16 == id)
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Stage => "stage",
            Self::AppliedPads => "applied_pads",
            Self::PlayerMotion => "player_motion",
            Self::Event => "event",
            Self::SceneExit => "scene_exit",
            Self::Rng => "rng",
            Self::Camera => "camera",
            Self::PlayerAction => "player_action",
            Self::PlayerBackgroundCollision => "player_background_collision",
            Self::PlayerCollisionSurfaces => "player_collision_surfaces",
            Self::GoalProgress => "goal_progress",
            Self::SelectedActors => "selected_actors",
        }
    }
}

const KNOWN_CHANNELS: u64 = (1 << 13) - 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceChannelStatus {
    NotSampled,
    Present,
    Absent,
    Unavailable,
    Truncated,
}

impl TryFrom<u8> for TraceChannelStatus {
    type Error = TraceError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::NotSampled),
            1 => Ok(Self::Present),
            2 => Ok(Self::Absent),
            3 => Ok(Self::Unavailable),
            4 => Ok(Self::Truncated),
            _ => Err(TraceError(format!(
                "invalid gameplay trace channel status {value}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TracePhase {
    PreInput,
    PostSimulation,
}

impl TryFrom<u8> for TracePhase {
    type Error = TraceError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::PreInput),
            2 => Ok(Self::PostSimulation),
            _ => Err(TraceError(format!("invalid gameplay trace phase {value}"))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TraceRngStream {
    pub id: u8,
    pub algorithm_version: u32,
    pub state: [i32; 3],
    pub call_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TraceRngSnapshot {
    pub version: u32,
    pub stream_count: u32,
    pub primary: TraceRngStream,
    pub secondary: TraceRngStream,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TraceCamera {
    pub view_yaw: i16,
    pub controlled_yaw: i16,
    pub bank: i16,
    pub eye: [f32; 3],
    pub center: [f32; 3],
    pub up: [f32; 3],
    pub fovy: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TraceAnimationLane {
    pub resource_id: u16,
    pub frame: f32,
    pub rate: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TraceActorIdentity {
    pub session_process_id: u32,
    pub actor_name: i16,
    pub set_id: u16,
    pub home_room: i8,
    pub current_room: i8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub home_position: Option<[f32; 3]>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TracePlayerAction {
    pub procedure_id: u16,
    pub mode_flags: u32,
    pub procedure_context_raw: [i16; 6],
    pub damage_wait_timer: i16,
    pub sword_at_up_time: u16,
    pub ice_damage_wait_timer: i16,
    pub sword_change_wait_timer: u8,
    pub under_animations: [TraceAnimationLane; 3],
    pub upper_animations: [TraceAnimationLane; 3],
    pub do_status: u8,
    pub talk_partner: Option<TraceActorIdentity>,
    pub grabbed_actor: Option<TraceActorIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TraceAppliedPads {
    pub valid_ports: u8,
    pub owned_ports: u8,
    pub pads: [RawPadState; 4],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceSceneExitKind {
    OrientedBox,
    RadialXz,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TraceSceneExit {
    pub session_process_id: u32,
    pub raw_parameters: u32,
    pub flags: u32,
    pub signed_distance_to_volume: f32,
    pub actor_name: i16,
    pub set_id: u16,
    pub exit_id: u8,
    pub path_id: u8,
    pub argument_1: u8,
    pub switch_no: u8,
    pub kind: TraceSceneExitKind,
    pub observed_count: u8,
    pub observed_count_saturated: bool,
    pub home_room: i8,
    pub link_exit_direction: Option<u8>,
    pub link_exit_id: Option<u16>,
    pub shape_yaw: i16,
    pub actor_action: Option<u8>,
    pub player_local_position: [f32; 3],
    pub volume_extent: [f32; 3],
    pub home_position: [f32; 3],
    pub destination: Option<TraceSceneExitDestination>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TraceSceneExitDestination {
    pub stage_name: String,
    pub room: i8,
    pub layer: i8,
    pub point: i16,
    pub wipe: u8,
    pub wipe_time: u8,
    pub time_hour: i8,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TraceCollisionWall {
    pub identity_present: bool,
    pub bg_index: Option<u16>,
    pub poly_index: Option<u16>,
    pub owner_session_process_id: Option<u32>,
    pub angle_y: i16,
    pub flags: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TraceCollisionSolverWall {
    pub flags: u32,
    pub angle_y: i16,
    pub wall_radius_squared: f32,
    pub wall_height: f32,
    pub wall_radius: f32,
    pub direct_wall_height: f32,
    pub realized_center: [f32; 3],
    pub realized_radius: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TracePlayerCollisionSolver {
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
    pub walls: [TraceCollisionSolverWall; 3],
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TracePlayerBackgroundCollision {
    pub flags: u32,
    pub ground_height: f32,
    pub roof_height: f32,
    pub water_height: f32,
    pub ground_bg_index: Option<u16>,
    pub ground_poly_index: Option<u16>,
    pub ground_owner_session_process_id: Option<u32>,
    pub ground_plane: [f32; 4],
    pub ground_identity_present: bool,
    pub roof_bg_index: Option<u16>,
    pub roof_poly_index: Option<u16>,
    pub roof_owner_session_process_id: Option<u32>,
    pub roof_identity_present: bool,
    pub water_bg_index: Option<u16>,
    pub water_poly_index: Option<u16>,
    pub water_owner_session_process_id: Option<u32>,
    pub water_identity_present: bool,
    pub walls: [TraceCollisionWall; 3],
    pub old_position: [f32; 3],
    pub resolved_frame_displacement: [f32; 3],
    pub final_position: [f32; 3],
    pub solver: Option<TracePlayerCollisionSolver>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceCollisionSurfaceKind {
    Ground,
    Roof,
    Water,
    Wall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceCollisionBackingFormat {
    Dzb,
    Kcl,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TraceCollisionSurfaceDestination {
    pub stage_name: String,
    pub room: i8,
    pub layer: i8,
    pub point: i16,
    pub wipe: u8,
    pub wipe_time: u8,
    pub time_hour: i8,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TraceCollisionSurface {
    pub flags: u32,
    pub kind: TraceCollisionSurfaceKind,
    pub wall_slot: u8,
    pub backing_format: Option<TraceCollisionBackingFormat>,
    pub raw_code_word_mask: u8,
    pub bg_index: Option<u16>,
    pub poly_index: Option<u16>,
    pub owner_session_process_id: Option<u32>,
    pub material_row: Option<u16>,
    pub group_row: Option<u16>,
    pub raw_code_words: [u32; 5],
    pub raw_exit_id: Option<u8>,
    pub source_room: Option<i8>,
    pub source_room_exact: bool,
    pub scls_source_room: Option<i8>,
    pub destination: Option<TraceCollisionSurfaceDestination>,
    pub source_geometry_indices: Vec<u16>,
    pub kcl_prism_height: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TracePlayerCollisionSurfaces {
    pub flags: u32,
    pub link_room: Option<i8>,
    pub identity_count: u8,
    pub backing_count: u8,
    pub destination_count: u8,
    pub raw_link_exit: u16,
    pub pending_match_mask: u8,
    pub surfaces: [TraceCollisionSurface; 6],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TraceGoalProgress {
    pub configured: bool,
    pub reached: bool,
    pub authored: bool,
    pub goal_name_hash: Option<u32>,
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

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TraceSelectedActor {
    pub session_process_id: u32,
    pub actor_name: i16,
    pub set_id: u16,
    pub home_room: i8,
    pub current_room: i8,
    pub health: i16,
    pub status: u32,
    pub position: [f32; 3],
    pub current_angle: [i16; 3],
    pub shape_angle: [i16; 3],
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TraceSelectedActors {
    pub observed_count: u32,
    pub truncated: bool,
    pub actors: Vec<TraceSelectedActor>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TraceRecord {
    pub boundary_index: u64,
    pub simulation_tick: u64,
    pub tape_frame: Option<u64>,
    pub observation_phase: TracePhase,
    pub input_source: u8,
    pub channel_status: BTreeMap<TraceChannel, TraceChannelStatus>,
    pub stage_name: String,
    pub room: i8,
    pub layer: i8,
    pub point: i16,
    pub next_stage_name: String,
    pub next_room: i8,
    pub next_layer: i8,
    pub next_point: i16,
    pub next_stage_enabled: bool,
    pub flags: u32,
    pub player_session_process_id: Option<u32>,
    pub player_actor_name: i16,
    pub current_angle: [i16; 3],
    pub shape_angle: [i16; 3],
    pub current_angle_y: i16,
    pub shape_angle_y: i16,
    pub buttons: u16,
    pub stick_x: i8,
    pub stick_y: i8,
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub forward_speed: f32,
    pub player_proc_id: Option<u16>,
    pub event_id: i16,
    pub event_mode: u8,
    pub event_status: u8,
    pub event_map_tool_id: u8,
    pub pad_error: i8,
    pub event_name_hash: u32,
    pub event_name_hash_present: bool,
    pub nearest_scene_exit_session_process_id: Option<u32>,
    pub nearest_scene_exit_actor_name: Option<i16>,
    pub nearest_scene_exit_position: [f32; 3],
    pub nearest_scene_exit_distance: Option<f32>,
    pub scene_exit: Option<TraceSceneExit>,
    pub applied_pads: Option<TraceAppliedPads>,
    pub rng: Option<TraceRngSnapshot>,
    pub camera: Option<TraceCamera>,
    pub player_action: Option<TracePlayerAction>,
    pub player_background_collision: Option<TracePlayerBackgroundCollision>,
    pub player_collision_surfaces: Option<TracePlayerCollisionSurfaces>,
    pub goal_progress: Option<TraceGoalProgress>,
    pub selected_actors: Option<TraceSelectedActors>,
}

impl Default for TraceRecord {
    fn default() -> Self {
        Self {
            boundary_index: 0,
            simulation_tick: 0,
            tape_frame: None,
            observation_phase: TracePhase::PostSimulation,
            input_source: 0,
            channel_status: BTreeMap::new(),
            stage_name: String::new(),
            room: 0,
            layer: -1,
            point: 0,
            next_stage_name: String::new(),
            next_room: 0,
            next_layer: -1,
            next_point: 0,
            next_stage_enabled: false,
            flags: 0,
            player_session_process_id: None,
            player_actor_name: -1,
            current_angle: [0; 3],
            shape_angle: [0; 3],
            current_angle_y: 0,
            shape_angle_y: 0,
            buttons: 0,
            stick_x: 0,
            stick_y: 0,
            position: [0.0; 3],
            velocity: [0.0; 3],
            forward_speed: 0.0,
            player_proc_id: None,
            event_id: -1,
            event_mode: 0,
            event_status: 0,
            event_map_tool_id: 0xff,
            pad_error: -1,
            event_name_hash: 0,
            event_name_hash_present: false,
            nearest_scene_exit_session_process_id: None,
            nearest_scene_exit_actor_name: None,
            nearest_scene_exit_position: [0.0; 3],
            nearest_scene_exit_distance: None,
            scene_exit: None,
            applied_pads: None,
            rng: None,
            camera: None,
            player_action: None,
            player_background_collision: None,
            player_collision_surfaces: None,
            goal_progress: None,
            selected_actors: None,
        }
    }
}

impl TraceRecord {
    pub fn player_present(&self) -> bool {
        self.flags & LEGACY_PLAYER_PRESENT != 0
    }

    pub fn player_is_link(&self) -> bool {
        self.flags & LEGACY_PLAYER_IS_LINK != 0
    }

    pub fn event_running(&self) -> bool {
        self.flags & LEGACY_EVENT_RUNNING != 0
    }

    pub fn tape_input_applied(&self) -> bool {
        self.input_source & INPUT_TAPE != 0
    }

    pub fn controller_input_applied(&self) -> bool {
        self.input_source & INPUT_CONTROLLER != 0
    }

    fn location(&self) -> TraceLocation {
        TraceLocation {
            stage_name: self.stage_name.clone(),
            room: self.room,
            point: self.point,
            layer: self.layer,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TraceLocation {
    pub stage_name: String,
    pub room: i8,
    pub point: i16,
    pub layer: i8,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TraceMilestone {
    pub kind: &'static str,
    pub simulation_tick: u64,
    pub tape_frame: Option<u64>,
    pub location: TraceLocation,
    pub position: [f32; 3],
    pub event_id: i16,
    pub event_name_hash: u32,
    pub event_name_hash_present: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TraceSummary {
    pub version: u16,
    pub boot: TapeBoot,
    pub requested_channels: Vec<&'static str>,
    pub record_count: usize,
    pub capacity_exhausted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retention: Option<TraceRetention>,
    pub first_playable: Option<TraceMilestone>,
    pub route_control: Option<TraceMilestone>,
    pub first_loading_trigger: Option<TraceMilestone>,
    pub first_loading_transition: Option<TraceMilestone>,
    pub post_load_playable: Option<TraceMilestone>,
    pub first_post_load_event: Option<TraceMilestone>,
    pub intro_cutscene: Option<TraceMilestone>,
    pub final_record: Option<TraceRecord>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecodedTrace {
    pub version: u16,
    pub boot: TapeBoot,
    pub tick_rate_numerator: u32,
    pub tick_rate_denominator: u32,
    pub requested_channels: u64,
    pub capacity_exhausted: bool,
    pub retention: Option<TraceRetention>,
    pub channel_formats: BTreeMap<TraceChannel, TraceChannelWireFormat>,
    pub records: Vec<TraceRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TraceRetention {
    pub configured_triggers: u32,
    pub observed_triggers: u32,
    pub pre_trigger_ticks: u32,
    pub post_trigger_ticks: u32,
    pub trigger_count: u32,
    pub observed_sample_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TraceChannelWireFormat {
    pub version: u16,
    pub stride: usize,
}

impl DecodedTrace {
    pub fn requests(&self, channel: TraceChannel) -> bool {
        self.requested_channels & channel.bit() != 0
    }
}

#[derive(Debug)]
pub struct TraceError(String);

impl fmt::Display for TraceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for TraceError {}

#[derive(Clone, Copy)]
struct ChannelDefinition {
    stride: usize,
}

fn channel_definition(channel: TraceChannel, version: u16) -> Option<ChannelDefinition> {
    let stride = match (channel, version) {
        (TraceChannel::Core, 1) => 32,
        (TraceChannel::Stage, 1) => 32,
        (TraceChannel::AppliedPads, 1) => 52,
        (TraceChannel::PlayerMotion, 1) => 52,
        (TraceChannel::Event, 1) => 16,
        (TraceChannel::SceneExit, 1) => 24,
        (TraceChannel::SceneExit, 2) => 88,
        (TraceChannel::Rng, 1) => 64,
        (TraceChannel::Camera, 1) => 48,
        (TraceChannel::PlayerAction, 1) => 104,
        (TraceChannel::PlayerAction, 2) => 136,
        (TraceChannel::PlayerAction, 3) => 160,
        (TraceChannel::PlayerBackgroundCollision, 1) => 128,
        (TraceChannel::PlayerBackgroundCollision, 2) => 316,
        (TraceChannel::PlayerCollisionSurfaces, 1) => 496,
        (TraceChannel::GoalProgress, 1) => 32,
        (TraceChannel::SelectedActors, 1) => 656,
        _ => return None,
    };
    Some(ChannelDefinition { stride })
}

#[derive(Clone)]
struct ChannelDescriptor {
    channel: Option<TraceChannel>,
    version: u16,
    flags: u32,
    stride: usize,
    status_offset: usize,
    status_length: usize,
    payload_offset: usize,
    payload_length: usize,
}

mod columnar;
mod decode;
mod summary;
mod wire;

use columnar::decode_columnar;
use summary::{summarize, validate_tick_rate};
use wire::*;

pub use decode::{decode, decode_and_summarize};

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
