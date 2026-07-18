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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TraceActorIdentity {
    pub session_process_id: u32,
    pub actor_name: i16,
    pub set_id: u16,
    pub home_room: i8,
    pub current_room: i8,
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
        (TraceChannel::PlayerBackgroundCollision, 1) => 128,
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

pub fn decode_and_summarize(bytes: &[u8]) -> Result<TraceSummary, TraceError> {
    let decoded = decode(bytes)?;
    Ok(summarize(decoded))
}

pub fn decode(bytes: &[u8]) -> Result<DecodedTrace, TraceError> {
    if bytes.len() < 12 {
        return Err(TraceError("truncated gameplay trace header".into()));
    }
    if &bytes[..8] != MAGIC {
        return Err(TraceError("bad gameplay trace magic".into()));
    }
    match u16_at(bytes, 8) {
        1 => decode_v1(bytes),
        2 => decode_columnar(bytes, 2, V2_HEADER_SIZE, TapeBoot::Process),
        3 => decode_columnar(bytes, 3, V3_HEADER_SIZE, decode_v3_boot(bytes)?),
        4 => {
            let (boot, data_end) = decode_v4_boot(bytes)?;
            decode_columnar(&bytes[..data_end], 4, V4_HEADER_SIZE, boot)
        }
        5 => {
            let (boot, data_end) = decode_v5_boot(bytes)?;
            decode_columnar(&bytes[..data_end], 5, V5_HEADER_SIZE, boot)
        }
        version => Err(TraceError(format!(
            "unsupported gameplay trace version {version}"
        ))),
    }
}

fn decode_v1(bytes: &[u8]) -> Result<DecodedTrace, TraceError> {
    if bytes.len() < V1_HEADER_SIZE {
        return Err(TraceError("truncated gameplay trace v1 header".into()));
    }
    if usize::from(u16_at(bytes, 10)) != V1_RECORD_SIZE {
        return Err(TraceError(
            "unsupported gameplay trace v1 record size".into(),
        ));
    }
    let count = count_at(bytes, 20)?;
    let expected = checked_region_end(V1_HEADER_SIZE, count, V1_RECORD_SIZE)?;
    if bytes.len() != expected {
        return Err(TraceError(format!(
            "gameplay trace size mismatch: expected {expected}, got {}",
            bytes.len()
        )));
    }
    if u32_at(bytes, 28) > 1 || u32_at(bytes, 32) != 0 {
        return Err(TraceError(
            "noncanonical gameplay trace v1 flags or reserved header".into(),
        ));
    }
    validate_tick_rate(bytes)?;
    let records = bytes[V1_HEADER_SIZE..]
        .chunks_exact(V1_RECORD_SIZE)
        .map(decode_v1_record)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(DecodedTrace {
        version: 1,
        boot: TapeBoot::Process,
        tick_rate_numerator: u32_at(bytes, 12),
        tick_rate_denominator: u32_at(bytes, 16),
        requested_channels: TraceChannel::Core.bit()
            | TraceChannel::Stage.bit()
            | TraceChannel::AppliedPads.bit()
            | TraceChannel::PlayerMotion.bit()
            | TraceChannel::Event.bit()
            | TraceChannel::SceneExit.bit(),
        capacity_exhausted: u32_at(bytes, 28) != 0,
        retention: None,
        channel_formats: BTreeMap::new(),
        records,
    })
}

fn decode_v1_record(bytes: &[u8]) -> Result<TraceRecord, TraceError> {
    if u16_at(bytes, 100) != 0 {
        return Err(TraceError(
            "nonzero gameplay trace v1 record reserved field".into(),
        ));
    }
    let raw_frame = u64_at(bytes, 8);
    let legacy_flags = u32_at(bytes, 28);
    let proc_id = u16_at(bytes, 70);
    let exit_actor = i16_at(bytes, 82);
    let exit_distance = f32_at(bytes, 96);
    let mut channel_status = BTreeMap::new();
    channel_status.insert(TraceChannel::Core, TraceChannelStatus::Present);
    channel_status.insert(TraceChannel::Stage, TraceChannelStatus::Present);
    channel_status.insert(TraceChannel::AppliedPads, TraceChannelStatus::Present);
    channel_status.insert(
        TraceChannel::PlayerMotion,
        if legacy_flags & LEGACY_PLAYER_PRESENT != 0 {
            TraceChannelStatus::Present
        } else {
            TraceChannelStatus::Absent
        },
    );
    channel_status.insert(TraceChannel::Event, TraceChannelStatus::Present);
    channel_status.insert(
        TraceChannel::SceneExit,
        if exit_actor != -1 {
            TraceChannelStatus::Present
        } else {
            TraceChannelStatus::Absent
        },
    );
    let simulation_tick = u64_at(bytes, 0);
    let boundary_index = simulation_tick
        .checked_add(1)
        .ok_or_else(|| TraceError("gameplay trace v1 simulation tick overflow".into()))?;
    Ok(TraceRecord {
        boundary_index,
        simulation_tick,
        tape_frame: (raw_frame != u64::MAX).then_some(raw_frame),
        observation_phase: TracePhase::PostSimulation,
        input_source: ((legacy_flags & LEGACY_TAPE_PLAYING != 0) as u8 * INPUT_TAPE)
            | ((legacy_flags & LEGACY_CONTROLLER_PLAYING != 0) as u8 * INPUT_CONTROLLER),
        channel_status,
        stage_name: decode_name(&bytes[16..24])?,
        room: bytes[24] as i8,
        layer: bytes[25] as i8,
        point: i16_at(bytes, 26),
        flags: legacy_flags,
        player_actor_name: i16_at(bytes, 32),
        current_angle: [0, i16_at(bytes, 34), 0],
        shape_angle: [0, i16_at(bytes, 36), 0],
        current_angle_y: i16_at(bytes, 34),
        shape_angle_y: i16_at(bytes, 36),
        buttons: u16_at(bytes, 38),
        stick_x: bytes[40] as i8,
        stick_y: bytes[41] as i8,
        position: [f32_at(bytes, 42), f32_at(bytes, 46), f32_at(bytes, 50)],
        velocity: [f32_at(bytes, 54), f32_at(bytes, 58), f32_at(bytes, 62)],
        forward_speed: f32_at(bytes, 66),
        player_proc_id: (proc_id != u16::MAX).then_some(proc_id),
        event_id: i16_at(bytes, 72),
        event_mode: bytes[74],
        event_status: bytes[75],
        event_map_tool_id: bytes[76],
        pad_error: bytes[77] as i8,
        event_name_hash: u32_at(bytes, 78),
        event_name_hash_present: true,
        nearest_scene_exit_actor_name: (exit_actor != -1).then_some(exit_actor),
        nearest_scene_exit_position: [f32_at(bytes, 84), f32_at(bytes, 88), f32_at(bytes, 92)],
        nearest_scene_exit_distance: (exit_distance >= 0.0).then_some(exit_distance),
        ..TraceRecord::default()
    })
}

fn decode_v3_boot(bytes: &[u8]) -> Result<TapeBoot, TraceError> {
    if bytes.len() < V3_HEADER_SIZE {
        return Err(TraceError("truncated gameplay trace v3 header".into()));
    }
    let extension = &bytes[V2_HEADER_SIZE..V3_HEADER_SIZE];
    match extension[0] {
        0 => {
            if extension.iter().any(|byte| *byte != 0) {
                return Err(TraceError(
                    "noncanonical process boot in gameplay trace v3".into(),
                ));
            }
            Ok(TapeBoot::Process)
        }
        1 => {
            let save_slot = extension[1];
            let stage_len = usize::from(extension[6]);
            if save_slot > 3
                || stage_len == 0
                || stage_len > 16
                || extension[7] != 0
                || extension[8 + stage_len..].iter().any(|byte| *byte != 0)
                || extension[8..8 + stage_len]
                    .iter()
                    .any(|byte| !(0x21..=0x7e).contains(byte) || *byte == b',')
            {
                return Err(TraceError(
                    "noncanonical stage boot in gameplay trace v3".into(),
                ));
            }
            let stage = String::from_utf8(extension[8..8 + stage_len].to_vec())
                .map_err(|_| TraceError("invalid stage name in gameplay trace v3".into()))?;
            Ok(TapeBoot::Stage {
                stage,
                room: extension[2] as i8,
                layer: extension[3] as i8,
                point: i16::from_le_bytes([extension[4], extension[5]]),
                save_slot: (save_slot != 0).then_some(save_slot),
                fixture: None,
            })
        }
        _ => Err(TraceError("unknown boot kind in gameplay trace v3".into())),
    }
}

fn decode_v4_boot(bytes: &[u8]) -> Result<(TapeBoot, usize), TraceError> {
    decode_v4_or_v5_boot(bytes, true)
}

fn decode_v5_boot(bytes: &[u8]) -> Result<(TapeBoot, usize), TraceError> {
    decode_v4_or_v5_boot(bytes, false)
}

fn decode_v4_or_v5_boot(
    bytes: &[u8],
    require_reserved_zero: bool,
) -> Result<(TapeBoot, usize), TraceError> {
    if bytes.len() < V4_HEADER_SIZE {
        return Err(TraceError("truncated gameplay trace v4/v5 header".into()));
    }
    let extension = &bytes[V2_HEADER_SIZE..88];
    let mut boot = match extension[0] {
        0 => {
            if extension.iter().any(|byte| *byte != 0) {
                return Err(TraceError(
                    "noncanonical process boot in gameplay trace v4".into(),
                ));
            }
            TapeBoot::Process
        }
        1 => {
            let save_slot = extension[1];
            let stage_len = usize::from(extension[6]);
            if save_slot > 3
                || stage_len == 0
                || stage_len > 16
                || extension[7] != 0
                || extension[8 + stage_len..].iter().any(|byte| *byte != 0)
                || extension[8..8 + stage_len]
                    .iter()
                    .any(|byte| !(0x21..=0x7e).contains(byte) || *byte == b',')
            {
                return Err(TraceError(
                    "noncanonical stage boot in gameplay trace v4".into(),
                ));
            }
            TapeBoot::Stage {
                stage: String::from_utf8(extension[8..8 + stage_len].to_vec())
                    .map_err(|_| TraceError("invalid stage name in gameplay trace v4".into()))?,
                room: extension[2] as i8,
                layer: extension[3] as i8,
                point: i16::from_le_bytes([extension[4], extension[5]]),
                save_slot: (save_slot != 0).then_some(save_slot),
                fixture: None,
            }
        }
        _ => return Err(TraceError("unknown boot kind in gameplay trace v4".into())),
    };
    if require_reserved_zero && bytes[100..V4_HEADER_SIZE].iter().any(|byte| *byte != 0) {
        return Err(TraceError(
            "nonzero gameplay trace v4 reserved field".into(),
        ));
    }
    let fixture_offset = usize_at_u64(bytes, 88)?;
    let fixture_size = usize::try_from(u32_at(bytes, 96))
        .map_err(|_| TraceError("gameplay trace fixture size overflow".into()))?;
    if fixture_size == 0 {
        if fixture_offset != 0 {
            return Err(TraceError(
                "gameplay trace v4 has an offset for an absent fixture".into(),
            ));
        }
        return Ok((boot, bytes.len()));
    }
    if fixture_offset < V4_HEADER_SIZE
        || fixture_offset
            .checked_add(fixture_size)
            .is_none_or(|end| end != bytes.len())
    {
        return Err(TraceError(
            "gameplay trace v4 fixture range is invalid".into(),
        ));
    }
    let fixture = ScenarioFixture::decode(&bytes[fixture_offset..])
        .map_err(|error| TraceError(format!("invalid gameplay trace fixture: {error}")))?;
    match &mut boot {
        TapeBoot::Stage {
            fixture: target, ..
        } => *target = Some(fixture),
        TapeBoot::Process => {
            return Err(TraceError(
                "process boot gameplay trace cannot carry a scenario fixture".into(),
            ));
        }
    }
    Ok((boot, fixture_offset))
}

fn decode_columnar(
    bytes: &[u8],
    version: u16,
    header_size: usize,
    boot: TapeBoot,
) -> Result<DecodedTrace, TraceError> {
    if bytes.len() < header_size {
        return Err(TraceError(format!(
            "truncated gameplay trace v{version} header"
        )));
    }
    if usize::from(u16_at(bytes, 10)) != header_size {
        return Err(TraceError(format!(
            "unsupported gameplay trace v{version} header size"
        )));
    }
    validate_tick_rate(bytes)?;
    let count = count_at(bytes, 20)?;
    let file_flags = u32_at(bytes, 28);
    let known_file_flags = if version >= 5 {
        FILE_COMPLETE | FILE_CAPACITY_EXHAUSTED | FILE_TRIGGER_RETENTION
    } else {
        FILE_COMPLETE | FILE_CAPACITY_EXHAUSTED
    };
    if file_flags & FILE_COMPLETE == 0 || file_flags & !known_file_flags != 0 {
        return Err(TraceError(
            "incomplete or noncanonical gameplay trace v2 flags".into(),
        ));
    }
    let retention = if version >= 5 {
        let configured_triggers = u32_at(bytes, 100);
        let observed_triggers = u32_at(bytes, 104);
        let pre_trigger_ticks = u32_at(bytes, 108);
        let post_trigger_ticks = u32_at(bytes, 112);
        let trigger_count = u32_at(bytes, 116);
        let observed_sample_count = u64_at(bytes, 120);
        let enabled = file_flags & FILE_TRIGGER_RETENTION != 0;
        if configured_triggers & !KNOWN_RETENTION_TRIGGERS != 0
            || observed_triggers & !configured_triggers != 0
            || ((trigger_count == 0) != (observed_triggers == 0))
            || observed_sample_count < count as u64
            || (enabled != (configured_triggers != 0))
            || (!enabled
                && (observed_triggers != 0
                    || pre_trigger_ticks != 0
                    || post_trigger_ticks != 0
                    || trigger_count != 0
                    || observed_sample_count != count as u64))
        {
            return Err(TraceError(
                "inconsistent gameplay trace v5 retention metadata".into(),
            ));
        }
        enabled.then_some(TraceRetention {
            configured_triggers,
            observed_triggers,
            pre_trigger_ticks,
            post_trigger_ticks,
            trigger_count,
            observed_sample_count,
        })
    } else {
        None
    };
    let channel_count = usize::from(u16_at(bytes, 32));
    if usize::from(u16_at(bytes, 34)) != V2_DIRECTORY_ENTRY_SIZE
        || usize_at_u64(bytes, 36)? != header_size
    {
        return Err(TraceError(
            "unsupported gameplay trace v2 directory layout".into(),
        ));
    }
    let directory_end = checked_region_end(header_size, channel_count, V2_DIRECTORY_ENTRY_SIZE)?;
    if usize_at_u64(bytes, 44)? != directory_end || directory_end > bytes.len() {
        return Err(TraceError("invalid gameplay trace v2 data offset".into()));
    }
    let requested_channels = u64_at(bytes, 52);
    if requested_channels & TraceChannel::Core.bit() == 0
        || requested_channels & !KNOWN_CHANNELS != 0
        || u32_at(bytes, 60) != 0
    {
        return Err(TraceError(
            "invalid gameplay trace v2 channel mask or reserved field".into(),
        ));
    }

    let mut descriptors = BTreeMap::<u16, ChannelDescriptor>::new();
    let mut regions = vec![(0_usize, directory_end)];
    for index in 0..channel_count {
        let offset = header_size + index * V2_DIRECTORY_ENTRY_SIZE;
        let id = u16_at(bytes, offset);
        let version = u16_at(bytes, offset + 2);
        let flags = u32_at(bytes, offset + 4);
        let stride = usize::try_from(u32_at(bytes, offset + 8))
            .map_err(|_| TraceError("gameplay trace stride overflow".into()))?;
        let status_stride = u32_at(bytes, offset + 12);
        let status_offset = usize_at_u64(bytes, offset + 16)?;
        let status_length = usize_at_u64(bytes, offset + 24)?;
        let payload_offset = usize_at_u64(bytes, offset + 32)?;
        let payload_length = usize_at_u64(bytes, offset + 40)?;
        let metadata_offset = usize_at_u64(bytes, offset + 48)?;
        let metadata_length = usize_at_u64(bytes, offset + 56)?;
        if flags & !(CHANNEL_REQUIRED | CHANNEL_DENSE) != 0
            || flags & CHANNEL_DENSE == 0
            || status_stride != 1
            || status_length != count
            || metadata_offset != 0
            || metadata_length != 0
        {
            return Err(TraceError(format!(
                "noncanonical gameplay trace channel {id}"
            )));
        }
        let expected_payload = count
            .checked_mul(stride)
            .ok_or_else(|| TraceError("gameplay trace payload size overflow".into()))?;
        if payload_length != expected_payload {
            return Err(TraceError(format!(
                "gameplay trace channel {id} length mismatch"
            )));
        }
        let status_end = status_offset
            .checked_add(status_length)
            .ok_or_else(|| TraceError("gameplay trace status range overflow".into()))?;
        let payload_end = payload_offset
            .checked_add(payload_length)
            .ok_or_else(|| TraceError("gameplay trace payload range overflow".into()))?;
        if status_offset < directory_end
            || payload_offset < directory_end
            || status_end > bytes.len()
            || payload_end > bytes.len()
        {
            return Err(TraceError(format!(
                "gameplay trace channel {id} is out of bounds"
            )));
        }
        regions.push((status_offset, status_end));
        regions.push((payload_offset, payload_end));
        let channel = TraceChannel::from_id(id);
        if let Some(channel) = channel {
            let definition = channel_definition(channel, version).ok_or_else(|| {
                TraceError(format!(
                    "unsupported gameplay trace channel {} version {version}",
                    channel.name()
                ))
            })?;
            if stride != definition.stride {
                return Err(TraceError(format!(
                    "unsupported gameplay trace channel {} version {version} stride {stride}; expected {}",
                    channel.name(),
                    definition.stride
                )));
            }
            if requested_channels & channel.bit() == 0 {
                return Err(TraceError(format!(
                    "unrequested known gameplay trace channel {} is present",
                    channel.name()
                )));
            }
        } else if flags & CHANNEL_REQUIRED != 0 {
            return Err(TraceError(format!(
                "unknown required gameplay trace channel {id}"
            )));
        }
        if descriptors
            .insert(
                id,
                ChannelDescriptor {
                    channel,
                    version,
                    flags,
                    stride,
                    status_offset,
                    status_length,
                    payload_offset,
                    payload_length,
                },
            )
            .is_some()
        {
            return Err(TraceError(format!("duplicate gameplay trace channel {id}")));
        }
    }
    regions.sort_unstable();
    if regions.windows(2).any(|pair| pair[0].1 != pair[1].0) {
        return Err(TraceError(
            "overlapping or unreferenced gameplay trace v2 regions".into(),
        ));
    }
    if descriptors
        .get(&(TraceChannel::Core as u16))
        .is_none_or(|descriptor| descriptor.flags & CHANNEL_REQUIRED == 0)
    {
        return Err(TraceError(
            "missing required gameplay trace core channel".into(),
        ));
    }
    if descriptors.contains_key(&(TraceChannel::PlayerCollisionSurfaces as u16))
        && !descriptors.contains_key(&(TraceChannel::Stage as u16))
    {
        return Err(TraceError(
            "player collision surfaces require the Stage channel".into(),
        ));
    }
    for channel in TraceChannel::ALL {
        if requested_channels & channel.bit() != 0 && !descriptors.contains_key(&(channel as u16)) {
            return Err(TraceError(format!(
                "requested channel {} is missing",
                channel.name()
            )));
        }
    }
    if regions.last().is_some_and(|region| region.1 != bytes.len()) {
        return Err(TraceError(
            "trailing or unreferenced gameplay trace v2 data".into(),
        ));
    }

    let mut records = vec![TraceRecord::default(); count];
    let mut channel_formats = BTreeMap::new();
    for descriptor in descriptors.values() {
        let Some(channel) = descriptor.channel else {
            continue;
        };
        channel_formats.insert(
            channel,
            TraceChannelWireFormat {
                version: descriptor.version,
                stride: descriptor.stride,
            },
        );
        debug_assert_eq!(descriptor.status_length, count);
        debug_assert_eq!(descriptor.payload_length, count * descriptor.stride);
        for (index, record) in records.iter_mut().enumerate() {
            let status = TraceChannelStatus::try_from(bytes[descriptor.status_offset + index])?;
            if channel == TraceChannel::Core && status != TraceChannelStatus::Present {
                return Err(TraceError("gameplay trace core is not present".into()));
            }
            validate_channel_status(channel, descriptor.version, status)?;
            record.channel_status.insert(channel, status);
            if status == TraceChannelStatus::Present || status == TraceChannelStatus::Truncated {
                let start = descriptor.payload_offset + index * descriptor.stride;
                decode_v2_channel(
                    channel,
                    descriptor.version,
                    &bytes[start..start + descriptor.stride],
                    record,
                )?;
            }
        }
    }
    for record in &records {
        if record.observation_phase != TracePhase::PostSimulation
            || record.simulation_tick.checked_add(1) != Some(record.boundary_index)
        {
            return Err(TraceError(
                "contradictory gameplay trace v2 boundary".into(),
            ));
        }
        validate_collision_surface_joins(record)?;
    }
    Ok(DecodedTrace {
        version,
        boot,
        tick_rate_numerator: u32_at(bytes, 12),
        tick_rate_denominator: u32_at(bytes, 16),
        requested_channels,
        capacity_exhausted: file_flags & FILE_CAPACITY_EXHAUSTED != 0,
        retention,
        channel_formats,
        records,
    })
}

fn validate_channel_status(
    channel: TraceChannel,
    version: u16,
    status: TraceChannelStatus,
) -> Result<(), TraceError> {
    let valid = status != TraceChannelStatus::NotSampled
        && match (channel, version) {
            (TraceChannel::SceneExit, 1) => matches!(
                status,
                TraceChannelStatus::Present | TraceChannelStatus::Absent
            ),
            (TraceChannel::SceneExit, 2) => matches!(
                status,
                TraceChannelStatus::Present
                    | TraceChannelStatus::Absent
                    | TraceChannelStatus::Unavailable
            ),
            (TraceChannel::PlayerBackgroundCollision, 1) => matches!(
                status,
                TraceChannelStatus::Present
                    | TraceChannelStatus::Absent
                    | TraceChannelStatus::Unavailable
            ),
            (TraceChannel::PlayerCollisionSurfaces, 1) => matches!(
                status,
                TraceChannelStatus::Present
                    | TraceChannelStatus::Absent
                    | TraceChannelStatus::Unavailable
            ),
            (TraceChannel::GoalProgress | TraceChannel::SelectedActors, 1) => {
                status == TraceChannelStatus::Present
            }
            _ => true,
        };
    if !valid {
        return Err(TraceError(format!(
            "invalid gameplay trace channel status {status:?} for {} version {version}",
            channel.name()
        )));
    }
    Ok(())
}

fn decode_v2_channel(
    channel: TraceChannel,
    version: u16,
    bytes: &[u8],
    record: &mut TraceRecord,
) -> Result<(), TraceError> {
    match channel {
        TraceChannel::Core => {
            if bytes[31] != 0 {
                return Err(TraceError(
                    "nonzero gameplay trace core reserved byte".into(),
                ));
            }
            let flags = u32_at(bytes, 24);
            if flags & CORE_SIMULATION_TICK_VALID == 0 || flags & !3 != 0 || bytes[29] != 1 {
                return Err(TraceError(
                    "invalid gameplay trace core flags or boundary".into(),
                ));
            }
            record.boundary_index = u64_at(bytes, 0);
            record.simulation_tick = u64_at(bytes, 8);
            record.tape_frame = (flags & CORE_TAPE_FRAME_VALID != 0).then(|| u64_at(bytes, 16));
            record.observation_phase = TracePhase::try_from(bytes[28])?;
            record.input_source = bytes[30];
            let wire_tape_frame = u64_at(bytes, 16);
            if record.input_source & !KNOWN_INPUT_SOURCES != 0
                || record.input_source.count_ones() > 1
                || (record.tape_frame.is_some() && wire_tape_frame == u64::MAX)
                || (record.tape_frame.is_none() && wire_tape_frame != u64::MAX)
            {
                return Err(TraceError(
                    "noncanonical gameplay trace core input or tape-frame state".into(),
                ));
            }
            if record.input_source & INPUT_TAPE != 0 {
                record.flags |= LEGACY_TAPE_PLAYING;
            }
            if record.input_source & INPUT_CONTROLLER != 0 {
                record.flags |= LEGACY_CONTROLLER_PLAYING;
            }
        }
        TraceChannel::Stage => {
            if u32_at(bytes, 28) != 0 {
                return Err(TraceError(
                    "nonzero gameplay trace stage reserved field".into(),
                ));
            }
            record.stage_name = decode_name(&bytes[0..8])?;
            record.room = bytes[8] as i8;
            record.layer = bytes[9] as i8;
            record.point = i16_at(bytes, 10);
            record.next_stage_name = decode_name(&bytes[12..20])?;
            record.next_room = bytes[20] as i8;
            record.next_layer = bytes[21] as i8;
            record.next_point = i16_at(bytes, 22);
            let flags = u32_at(bytes, 24);
            if flags & !1 != 0 {
                return Err(TraceError("unknown gameplay trace stage flags".into()));
            }
            record.next_stage_enabled = flags & 1 != 0;
        }
        TraceChannel::AppliedPads => {
            if u16_at(bytes, 2) != 0 || bytes[0] & !0x0f != 0 || bytes[1] & !0x0f != 0 {
                return Err(TraceError(
                    "invalid gameplay trace applied-pad header".into(),
                ));
            }
            let pads = [
                decode_pad(&bytes[4..16])?,
                decode_pad(&bytes[16..28])?,
                decode_pad(&bytes[28..40])?,
                decode_pad(&bytes[40..52])?,
            ];
            let connected_ports = pads.iter().enumerate().fold(0_u8, |mask, (port, pad)| {
                mask | if pad.connected { 1 << port } else { 0 }
            });
            if connected_ports != bytes[0] {
                return Err(TraceError(
                    "gameplay trace applied-pad validity disagrees with pad flags".into(),
                ));
            }
            record.buttons = pads[0].buttons;
            record.stick_x = pads[0].stick_x;
            record.stick_y = pads[0].stick_y;
            record.pad_error = pads[0].error;
            record.applied_pads = Some(TraceAppliedPads {
                valid_ports: bytes[0],
                owned_ports: bytes[1],
                pads,
            });
        }
        TraceChannel::PlayerMotion => {
            record.player_session_process_id = Some(u32_at(bytes, 0));
            record.player_actor_name = i16_at(bytes, 4);
            let procedure = u16_at(bytes, 6);
            record.player_proc_id = (procedure != u16::MAX).then_some(procedure);
            record.current_angle = [i16_at(bytes, 8), i16_at(bytes, 10), i16_at(bytes, 12)];
            record.shape_angle = [i16_at(bytes, 14), i16_at(bytes, 16), i16_at(bytes, 18)];
            record.current_angle_y = record.current_angle[1];
            record.shape_angle_y = record.shape_angle[1];
            record.position = [f32_at(bytes, 20), f32_at(bytes, 24), f32_at(bytes, 28)];
            record.velocity = [f32_at(bytes, 32), f32_at(bytes, 36), f32_at(bytes, 40)];
            record.forward_speed = f32_at(bytes, 44);
            let flags = u32_at(bytes, 48);
            if flags & !PLAYER_IS_LINK != 0 {
                return Err(TraceError("unknown gameplay trace player flags".into()));
            }
            record.flags |= LEGACY_PLAYER_PRESENT;
            if flags & PLAYER_IS_LINK != 0 {
                record.flags |= LEGACY_PLAYER_IS_LINK;
            }
        }
        TraceChannel::Event => {
            if bytes[9] != 0 || u16_at(bytes, 10) != 0 {
                return Err(TraceError(
                    "nonzero gameplay trace event reserved field".into(),
                ));
            }
            let flags = u32_at(bytes, 0);
            if flags & !(EVENT_RUNNING | EVENT_NAME_HASH_PRESENT) != 0 {
                return Err(TraceError("unknown gameplay trace event flags".into()));
            }
            if flags & EVENT_RUNNING != 0 {
                record.flags |= LEGACY_EVENT_RUNNING;
            }
            record.event_id = i16_at(bytes, 4);
            record.event_mode = bytes[6];
            record.event_status = bytes[7];
            record.event_map_tool_id = bytes[8];
            record.event_name_hash = u32_at(bytes, 12);
            record.event_name_hash_present = flags & EVENT_NAME_HASH_PRESENT != 0;
        }
        TraceChannel::SceneExit => match version {
            1 => decode_scene_exit_v1(bytes, record)?,
            2 => decode_scene_exit_v2(bytes, record)?,
            _ => unreachable!("channel version was validated"),
        },
        TraceChannel::Rng => {
            let primary = decode_rng_stream(&bytes[8..36])?;
            let secondary = decode_rng_stream(&bytes[36..64])?;
            let stream_count = u32_at(bytes, 4);
            if u32_at(bytes, 0) != 1
                || stream_count != 2
                || primary.id != 0
                || secondary.id != 1
                || primary.algorithm_version != 1
                || secondary.algorithm_version != 1
            {
                return Err(TraceError(
                    "invalid gameplay trace RNG stream identity".into(),
                ));
            }
            record.rng = Some(TraceRngSnapshot {
                version: u32_at(bytes, 0),
                stream_count,
                primary,
                secondary,
            });
        }
        TraceChannel::Camera => {
            if u16_at(bytes, 6) != 0 {
                return Err(TraceError(
                    "nonzero gameplay trace camera reserved field".into(),
                ));
            }
            record.camera = Some(TraceCamera {
                view_yaw: i16_at(bytes, 0),
                controlled_yaw: i16_at(bytes, 2),
                bank: i16_at(bytes, 4),
                eye: [f32_at(bytes, 8), f32_at(bytes, 12), f32_at(bytes, 16)],
                center: [f32_at(bytes, 20), f32_at(bytes, 24), f32_at(bytes, 28)],
                up: [f32_at(bytes, 32), f32_at(bytes, 36), f32_at(bytes, 40)],
                fovy: f32_at(bytes, 44),
            });
        }
        TraceChannel::PlayerAction => {
            if u16_at(bytes, 2) != 0 || bytes[27..32].iter().any(|value| *value != 0) {
                return Err(TraceError(
                    "nonzero gameplay trace player-action reserved field".into(),
                ));
            }
            let lane = |offset| TraceAnimationLane {
                resource_id: u16_at(bytes, offset),
                frame: f32_at(bytes, offset + 4),
                rate: f32_at(bytes, offset + 8),
            };
            for offset in (32..104).step_by(12) {
                if u16_at(bytes, offset + 2) != 0 {
                    return Err(TraceError("nonzero animation-lane reserved field".into()));
                }
            }
            let decode_identity = |offset: usize| -> Result<TraceActorIdentity, TraceError> {
                if u16_at(bytes, offset + 10) != 0 {
                    return Err(TraceError(
                        "nonzero gameplay trace actor-identity reserved field".into(),
                    ));
                }
                Ok(TraceActorIdentity {
                    session_process_id: u32_at(bytes, offset),
                    actor_name: i16_at(bytes, offset + 4),
                    set_id: u16_at(bytes, offset + 6),
                    home_room: bytes[offset + 8] as i8,
                    current_room: bytes[offset + 9] as i8,
                })
            };
            let (do_status, talk_partner, grabbed_actor) = if version == 2 {
                let flags = u32_at(bytes, 104);
                if flags & !PLAYER_ACTION_KNOWN_FLAGS != 0
                    || bytes[109..112].iter().any(|value| *value != 0)
                {
                    return Err(TraceError(
                        "invalid gameplay trace player-action interaction metadata".into(),
                    ));
                }
                let talk = decode_identity(112)?;
                let grabbed = decode_identity(124)?;
                let absent_is_canonical = |identity: &TraceActorIdentity| {
                    identity.session_process_id == u32::MAX
                        && identity.actor_name == -1
                        && identity.set_id == u16::MAX
                        && identity.home_room == -1
                        && identity.current_room == -1
                };
                if flags & TALK_PARTNER_PRESENT == 0 && !absent_is_canonical(&talk)
                    || flags & GRABBED_ACTOR_PRESENT == 0 && !absent_is_canonical(&grabbed)
                {
                    return Err(TraceError(
                        "noncanonical absent gameplay trace actor identity".into(),
                    ));
                }
                (
                    bytes[108],
                    (flags & TALK_PARTNER_PRESENT != 0).then_some(talk),
                    (flags & GRABBED_ACTOR_PRESENT != 0).then_some(grabbed),
                )
            } else {
                (0, None, None)
            };
            record.player_action = Some(TracePlayerAction {
                procedure_id: u16_at(bytes, 0),
                mode_flags: u32_at(bytes, 4),
                procedure_context_raw: std::array::from_fn(|index| i16_at(bytes, 8 + index * 2)),
                damage_wait_timer: i16_at(bytes, 20),
                sword_at_up_time: u16_at(bytes, 22),
                ice_damage_wait_timer: i16_at(bytes, 24),
                sword_change_wait_timer: bytes[26],
                under_animations: [lane(32), lane(44), lane(56)],
                upper_animations: [lane(68), lane(80), lane(92)],
                do_status,
                talk_partner,
                grabbed_actor,
            });
        }
        TraceChannel::PlayerBackgroundCollision => {
            decode_player_background_collision_v1(bytes, record)?
        }
        TraceChannel::PlayerCollisionSurfaces => {
            decode_player_collision_surfaces_v1(bytes, record)?
        }
        TraceChannel::GoalProgress => {
            let flags = u32_at(bytes, 0);
            let first_hit_tick = u64_at(bytes, 24);
            let configured = flags & GOAL_CONFIGURED != 0;
            let reached = flags & GOAL_REACHED != 0;
            let authored = flags & GOAL_AUTHORED != 0;
            let first_hit_present = flags & GOAL_FIRST_HIT_TICK_PRESENT != 0;
            let requested_count = u16_at(bytes, 8);
            let hit_count = u16_at(bytes, 10);
            let stable_ticks = u16_at(bytes, 12);
            let consecutive_ticks = u16_at(bytes, 14);
            let sequence_steps = bytes[16];
            let sequence_next_step = bytes[17];
            let sequence_within_ticks = u16_at(bytes, 18);
            let sequence_elapsed_ticks = u16_at(bytes, 20);
            if flags & !GOAL_KNOWN_FLAGS != 0
                || u16_at(bytes, 22) != 0
                || reached != first_hit_present
                || reached && !configured
                || authored && !configured
                || first_hit_present == (first_hit_tick == u64::MAX)
                || hit_count > requested_count
                || consecutive_ticks > stable_ticks
                || sequence_next_step > sequence_steps
                || (!configured
                    && (u32_at(bytes, 4) != 0
                        || stable_ticks != 0
                        || consecutive_ticks != 0
                        || sequence_steps != 0
                        || sequence_next_step != 0
                        || sequence_within_ticks != 0
                        || sequence_elapsed_ticks != 0))
            {
                return Err(TraceError(
                    "inconsistent gameplay trace goal-progress payload".into(),
                ));
            }
            record.goal_progress = Some(TraceGoalProgress {
                configured,
                reached,
                authored,
                goal_name_hash: configured.then(|| u32_at(bytes, 4)),
                requested_count,
                hit_count,
                stable_ticks,
                consecutive_ticks,
                sequence_steps,
                sequence_next_step,
                sequence_within_ticks,
                sequence_elapsed_ticks,
                first_hit_tick: first_hit_present.then_some(first_hit_tick),
            });
        }
        TraceChannel::SelectedActors => {
            let count = usize::from(u16_at(bytes, 0));
            let capacity = usize::from(u16_at(bytes, 2));
            let flags = u32_at(bytes, 4);
            let observed_count = u32_at(bytes, 8);
            let truncated = flags & SELECTED_ACTORS_TRUNCATED != 0;
            if capacity != SELECTED_ACTOR_CAPACITY
                || count > capacity
                || flags & !SELECTED_ACTORS_TRUNCATED != 0
                || u32_at(bytes, 12) != 0
                || observed_count < count as u32
                || truncated != (observed_count > count as u32)
            {
                return Err(TraceError(
                    "inconsistent gameplay trace selected-actor header".into(),
                ));
            }
            let mut actors = Vec::with_capacity(count);
            for index in 0..SELECTED_ACTOR_CAPACITY {
                let offset = 16 + index * 40;
                if index < count {
                    let actor = decode_selected_actor(&bytes[offset..offset + 40])?;
                    if actors.last().is_some_and(|previous: &TraceSelectedActor| {
                        previous.session_process_id >= actor.session_process_id
                    }) {
                        return Err(TraceError(
                            "gameplay trace selected actors are not strictly ordered".into(),
                        ));
                    }
                    actors.push(actor);
                } else if !unused_selected_actor_is_canonical(&bytes[offset..offset + 40]) {
                    return Err(TraceError(
                        "noncanonical unused gameplay trace selected-actor slot".into(),
                    ));
                }
            }
            record.selected_actors = Some(TraceSelectedActors {
                observed_count,
                truncated,
                actors,
            });
        }
    }
    Ok(())
}

fn decode_selected_actor(bytes: &[u8]) -> Result<TraceSelectedActor, TraceError> {
    let actor = TraceSelectedActor {
        session_process_id: u32_at(bytes, 0),
        actor_name: i16_at(bytes, 4),
        set_id: u16_at(bytes, 6),
        home_room: bytes[8] as i8,
        current_room: bytes[9] as i8,
        health: i16_at(bytes, 10),
        status: u32_at(bytes, 12),
        position: [f32_at(bytes, 16), f32_at(bytes, 20), f32_at(bytes, 24)],
        current_angle: [i16_at(bytes, 28), i16_at(bytes, 30), i16_at(bytes, 32)],
        shape_angle: [i16_at(bytes, 34), i16_at(bytes, 36), i16_at(bytes, 38)],
    };
    if actor.session_process_id == u32::MAX || actor.position.iter().any(|value| !value.is_finite())
    {
        return Err(TraceError(
            "invalid retained gameplay trace selected actor".into(),
        ));
    }
    Ok(actor)
}

fn unused_selected_actor_is_canonical(bytes: &[u8]) -> bool {
    u32_at(bytes, 0) == u32::MAX
        && i16_at(bytes, 4) == -1
        && u16_at(bytes, 6) == u16::MAX
        && bytes[8] as i8 == -1
        && bytes[9] as i8 == -1
        && i16_at(bytes, 10) == 0
        && u32_at(bytes, 12) == 0
        && bytes[16..40].iter().all(|byte| *byte == 0)
}

fn decode_scene_exit_v1(bytes: &[u8], record: &mut TraceRecord) -> Result<(), TraceError> {
    if u16_at(bytes, 6) != 0 {
        return Err(TraceError(
            "nonzero gameplay trace scene-exit v1 reserved field".into(),
        ));
    }
    let values = [
        f32_at(bytes, 8),
        f32_at(bytes, 12),
        f32_at(bytes, 16),
        f32_at(bytes, 20),
    ];
    if values.iter().any(|value| !value.is_finite()) {
        return Err(TraceError(
            "nonfinite gameplay trace scene-exit v1 value".into(),
        ));
    }
    record.nearest_scene_exit_session_process_id = Some(u32_at(bytes, 0));
    record.nearest_scene_exit_actor_name = Some(i16_at(bytes, 4));
    record.nearest_scene_exit_position = values[..3].try_into().expect("fixed slice");
    record.nearest_scene_exit_distance = Some(values[3]);
    Ok(())
}

fn decode_scene_exit_v2(bytes: &[u8], record: &mut TraceRecord) -> Result<(), TraceError> {
    let flags = u32_at(bytes, 8);
    if flags & !SCENE_EXIT_KNOWN_FLAGS != 0 || flags & SCENE_EXIT_VOLUME_VALID == 0 {
        return Err(TraceError(
            "invalid gameplay trace scene-exit v2 flags".into(),
        ));
    }
    if bytes[33..36].iter().any(|value| *value != 0) || bytes[87] != 0 {
        return Err(TraceError(
            "nonzero gameplay trace scene-exit v2 reserved field".into(),
        ));
    }
    let kind = match bytes[24] {
        1 => TraceSceneExitKind::OrientedBox,
        2 => TraceSceneExitKind::RadialXz,
        value => {
            return Err(TraceError(format!(
                "invalid gameplay trace scene-exit kind {value}"
            )));
        }
    };
    let observed_count = bytes[25];
    if observed_count == 0
        || flags & SCENE_EXIT_OBSERVED_COUNT_SATURATED != 0 && observed_count != u8::MAX
    {
        return Err(TraceError(
            "gameplay trace scene-exit has an invalid observed candidate count".into(),
        ));
    }
    let latched = flags & SCENE_EXIT_PLAYER_LATCHED != 0;
    let link_exit_direction = latched.then_some(bytes[27]);
    let raw_link_exit_id = u16_at(bytes, 28);
    let link_exit_id = (raw_link_exit_id != u16::MAX).then_some(raw_link_exit_id);
    if link_exit_id.is_some() != latched || (!latched && bytes[27] != u8::MAX) {
        return Err(TraceError(
            "inconsistent gameplay trace scene-exit Link exit sentinels".into(),
        ));
    }
    let raw_actor_action = bytes[32];
    let actor_action = (raw_actor_action != u8::MAX).then_some(raw_actor_action);
    if kind == TraceSceneExitKind::OrientedBox && actor_action.is_some() {
        return Err(TraceError(
            "box gameplay trace scene-exit has radial actor action".into(),
        ));
    }
    if kind == TraceSceneExitKind::RadialXz && flags & SCENE_EXIT_CHANGE_OK != 0 {
        return Err(TraceError(
            "radial gameplay trace scene-exit has box-only change-ok state".into(),
        ));
    }
    if kind == TraceSceneExitKind::RadialXz && latched {
        return Err(TraceError(
            "radial gameplay trace scene-exit cannot be Link's regular exit latch".into(),
        ));
    }
    if flags & SCENE_EXIT_CHANGE_OK != 0 && !latched {
        return Err(TraceError(
            "gameplay trace scene-exit change-ok state lacks Link latch".into(),
        ));
    }
    if kind == TraceSceneExitKind::RadialXz
        && (bytes[21] != u8::MAX
            || bytes[22] != u8::MAX
            || bytes[23] != u8::MAX
            || actor_action.is_none_or(|action| action > 3))
    {
        return Err(TraceError(
            "invalid gameplay trace radial scene-exit parameters".into(),
        ));
    }
    let raw_parameters = u32_at(bytes, 4);
    if bytes[20] != raw_parameters as u8
        || kind == TraceSceneExitKind::OrientedBox
            && (bytes[22] != (raw_parameters >> 8) as u8
                || bytes[21] != (raw_parameters >> 16) as u8
                || bytes[23] != (raw_parameters >> 24) as u8)
    {
        return Err(TraceError(
            "gameplay trace scene-exit fields disagree with raw parameters".into(),
        ));
    }
    if flags & SCENE_EXIT_CHANGE_STARTED != 0 && flags & SCENE_EXIT_PLAYER_LATCHED == 0 {
        return Err(TraceError(
            "gameplay trace scene-change start lacks selected exit latch".into(),
        ));
    }
    let player_local_position = [f32_at(bytes, 36), f32_at(bytes, 40), f32_at(bytes, 44)];
    let volume_extent = [f32_at(bytes, 48), f32_at(bytes, 52), f32_at(bytes, 56)];
    let home_position = [f32_at(bytes, 60), f32_at(bytes, 64), f32_at(bytes, 68)];
    if std::iter::once(f32_at(bytes, 12))
        .chain(player_local_position)
        .chain(volume_extent)
        .chain(home_position)
        .any(|value| !value.is_finite())
    {
        return Err(TraceError(
            "nonfinite gameplay trace scene-exit v2 geometry".into(),
        ));
    }
    let canonical_extent = match kind {
        TraceSceneExitKind::OrientedBox => volume_extent.iter().all(|value| *value >= 0.0),
        TraceSceneExitKind::RadialXz => {
            volume_extent[0] >= 0.0
                && volume_extent[1] == 0.0
                && volume_extent[2] == volume_extent[0]
        }
    };
    if !canonical_extent {
        return Err(TraceError(
            "invalid gameplay trace scene-exit volume extent".into(),
        ));
    }
    let geometrically_inside = match kind {
        TraceSceneExitKind::OrientedBox => f32_at(bytes, 12) <= 0.0,
        TraceSceneExitKind::RadialXz => f32_at(bytes, 12) < 0.0,
    };
    if geometrically_inside != (flags & SCENE_EXIT_PLAYER_INSIDE != 0) {
        return Err(TraceError(
            "gameplay trace scene-exit inside flag disagrees with signed distance".into(),
        ));
    }
    let destination_present = flags & SCENE_EXIT_DESTINATION_VALID != 0;
    let destination_name = decode_name(&bytes[72..80])?;
    let destination_wipe = bytes[84];
    let destination_wipe_time = bytes[85];
    let destination_time_hour = bytes[86] as i8;
    if destination_present {
        if destination_name.is_empty()
            || !(-1..=63).contains(&(bytes[80] as i8))
            || destination_wipe_time > 7
            || !(-1..=30).contains(&destination_time_hour)
            || !(bytes[81] as i8 == -1 || (0..=14).contains(&(bytes[81] as i8)))
            || i16_at(bytes, 82) < 0
        {
            return Err(TraceError(
                "invalid gameplay trace scene-exit destination".into(),
            ));
        }
    } else if !destination_name.is_empty()
        || bytes[80] as i8 != -1
        || bytes[81] as i8 != -1
        || i16_at(bytes, 82) != -1
        || destination_wipe != u8::MAX
        || destination_wipe_time != u8::MAX
        || destination_time_hour != -1
    {
        return Err(TraceError(
            "gameplay trace scene-exit destination sentinels disagree with flags".into(),
        ));
    }
    let destination = destination_present.then(|| TraceSceneExitDestination {
        stage_name: destination_name,
        room: bytes[80] as i8,
        layer: bytes[81] as i8,
        point: i16_at(bytes, 82),
        wipe: destination_wipe,
        wipe_time: destination_wipe_time,
        time_hour: destination_time_hour,
    });
    let scene_exit = TraceSceneExit {
        session_process_id: u32_at(bytes, 0),
        raw_parameters,
        flags,
        signed_distance_to_volume: f32_at(bytes, 12),
        actor_name: i16_at(bytes, 16),
        set_id: u16_at(bytes, 18),
        exit_id: bytes[20],
        path_id: bytes[21],
        argument_1: bytes[22],
        switch_no: bytes[23],
        kind,
        observed_count,
        observed_count_saturated: flags & SCENE_EXIT_OBSERVED_COUNT_SATURATED != 0,
        home_room: bytes[26] as i8,
        link_exit_direction,
        link_exit_id,
        shape_yaw: i16_at(bytes, 30),
        actor_action,
        player_local_position,
        volume_extent,
        home_position,
        destination,
    };
    record.nearest_scene_exit_session_process_id = Some(scene_exit.session_process_id);
    record.nearest_scene_exit_actor_name = Some(scene_exit.actor_name);
    // Preserve the old actor-origin projection for callers that only display it.
    // movement-state/v1 rejects SceneExit v2 before featurization because its
    // old Euclidean-distance slot has no equivalent in this signed-volume wire.
    record.nearest_scene_exit_position = scene_exit.home_position;
    record.scene_exit = Some(scene_exit);
    Ok(())
}

fn decode_player_background_collision_v1(
    bytes: &[u8],
    record: &mut TraceRecord,
) -> Result<(), TraceError> {
    let flags = u32_at(bytes, 0);
    if flags & !COLLISION_KNOWN_FLAGS != 0 {
        return Err(TraceError(
            "unknown gameplay trace player-background-collision flags".into(),
        ));
    }
    if flags & (COLLISION_GROUND_CONTACT | COLLISION_GROUND_LANDING) != 0
        && flags & COLLISION_GROUND_PROBE_VALID == 0
        || flags & COLLISION_ROOF_CONTACT != 0 && flags & COLLISION_ROOF_PROBE_VALID == 0
        || flags & COLLISION_WALL_CONTACT != 0 && flags & COLLISION_WALL_PROBE_ENABLED == 0
        || flags & COLLISION_WATER_SURFACE_FOUND != 0 && flags & COLLISION_WATER_PROBE_ENABLED == 0
        || flags & COLLISION_WATER_IN != 0 && flags & COLLISION_WATER_SURFACE_FOUND == 0
        || flags & COLLISION_WATER_OWNER_PRESENT != 0 && flags & COLLISION_WATER_SURFACE_FOUND == 0
        || flags & COLLISION_GROUND_PLANE_VALID != 0
            && flags & (COLLISION_GROUND_PROBE_VALID | COLLISION_GROUND_CONTACT)
                != (COLLISION_GROUND_PROBE_VALID | COLLISION_GROUND_CONTACT)
    {
        return Err(TraceError(
            "contradictory gameplay trace player-background-collision flags".into(),
        ));
    }

    let ground_bg = u16_at(bytes, 16);
    let ground_poly = u16_at(bytes, 18);
    let ground_owner = u32_at(bytes, 20);
    let ground_identity = flags & COLLISION_GROUND_IDENTITY_PRESENT != 0;
    validate_identity_pair(ground_bg, ground_poly, ground_identity, "ground")?;
    validate_owner(
        ground_owner,
        flags & COLLISION_GROUND_OWNER_PRESENT != 0,
        "ground",
    )?;
    if ground_identity && flags & COLLISION_GROUND_PROBE_VALID == 0
        || flags & COLLISION_GROUND_OWNER_PRESENT != 0 && !ground_identity
    {
        return Err(TraceError(
            "gameplay trace collision ground identity disagrees with flags".into(),
        ));
    }
    let ground_plane = [
        f32_at(bytes, 24),
        f32_at(bytes, 28),
        f32_at(bytes, 32),
        f32_at(bytes, 36),
    ];
    validate_plane(
        ground_plane,
        flags & COLLISION_GROUND_PLANE_VALID != 0,
        "ground",
    )?;

    let roof_bg = u16_at(bytes, 40);
    let roof_poly = u16_at(bytes, 42);
    let roof_owner = u32_at(bytes, 44);
    let roof_identity = flags & COLLISION_ROOF_IDENTITY_PRESENT != 0;
    validate_identity_pair(roof_bg, roof_poly, roof_identity, "roof")?;
    validate_owner(
        roof_owner,
        flags & COLLISION_ROOF_OWNER_PRESENT != 0,
        "roof",
    )?;
    if roof_identity && flags & COLLISION_ROOF_PROBE_VALID == 0
        || flags & COLLISION_ROOF_OWNER_PRESENT != 0 && !roof_identity
    {
        return Err(TraceError(
            "gameplay trace collision roof identity disagrees with flags".into(),
        ));
    }
    let water_bg = u16_at(bytes, 48);
    let water_poly = u16_at(bytes, 50);
    let water_owner = u32_at(bytes, 52);
    let water_identity = flags & COLLISION_WATER_IDENTITY_PRESENT != 0;
    validate_identity_pair(water_bg, water_poly, water_identity, "water")?;
    validate_owner(
        water_owner,
        flags & COLLISION_WATER_OWNER_PRESENT != 0,
        "water",
    )?;
    if water_identity && flags & COLLISION_WATER_SURFACE_FOUND == 0
        || flags & COLLISION_WATER_OWNER_PRESENT != 0 && !water_identity
    {
        return Err(TraceError(
            "gameplay trace collision water identity disagrees with flags".into(),
        ));
    }

    let walls: [TraceCollisionWall; 3] = (0..3)
        .map(|index| {
            let offset = 56 + index * 12;
            let wall_flags = u16_at(bytes, offset + 10);
            if wall_flags & !COLLISION_WALL_KNOWN_FLAGS != 0 {
                return Err(TraceError(format!(
                    "unknown gameplay trace collision wall {index} flags"
                )));
            }
            let bg = u16_at(bytes, offset);
            let poly = u16_at(bytes, offset + 2);
            let owner = u32_at(bytes, offset + 4);
            let identity = wall_flags & COLLISION_WALL_IDENTITY_PRESENT != 0;
            validate_identity_pair(bg, poly, identity, "wall")?;
            validate_owner(
                owner,
                wall_flags & COLLISION_WALL_OWNER_PRESENT != 0,
                "wall",
            )?;
            if identity && wall_flags & COLLISION_WALL_HIT == 0
                || wall_flags & COLLISION_WALL_OWNER_PRESENT != 0 && !identity
                || wall_flags & COLLISION_WALL_HIT == 0 && i16_at(bytes, offset + 8) != 0
            {
                return Err(TraceError(
                    "gameplay trace collision wall identity or angle disagrees with flags".into(),
                ));
            }
            Ok(TraceCollisionWall {
                identity_present: identity,
                bg_index: (bg != INVALID_U16_ID).then_some(bg),
                poly_index: (poly != INVALID_U16_ID).then_some(poly),
                owner_session_process_id: (owner != INVALID_U32_ID).then_some(owner),
                angle_y: i16_at(bytes, offset + 8),
                flags: wall_flags,
            })
        })
        .collect::<Result<Vec<_>, TraceError>>()?
        .try_into()
        .expect("three collision wall slots");
    let any_wall_hit = walls
        .iter()
        .any(|wall| wall.flags & COLLISION_WALL_HIT != 0);
    if any_wall_hit != (flags & COLLISION_WALL_CONTACT != 0) {
        return Err(TraceError(
            "gameplay trace aggregate wall contact disagrees with wall hits".into(),
        ));
    }
    let heights = [f32_at(bytes, 4), f32_at(bytes, 8), f32_at(bytes, 12)];
    let old_position = [f32_at(bytes, 92), f32_at(bytes, 96), f32_at(bytes, 100)];
    let resolved_frame_displacement = [f32_at(bytes, 104), f32_at(bytes, 108), f32_at(bytes, 112)];
    let final_position = [f32_at(bytes, 116), f32_at(bytes, 120), f32_at(bytes, 124)];
    if heights
        .iter()
        .chain(&old_position)
        .chain(&resolved_frame_displacement)
        .chain(&final_position)
        .any(|value| !value.is_finite())
        || (flags & COLLISION_GROUND_PROBE_VALID == 0 && heights[0] != -1.0e9)
        || (flags & COLLISION_GROUND_PROBE_VALID != 0 && heights[0] == -1.0e9)
        || (flags & COLLISION_ROOF_PROBE_VALID == 0 && heights[1] != 1.0e9)
        || (flags & COLLISION_ROOF_PROBE_VALID != 0 && heights[1] == 1.0e9)
        || (flags & COLLISION_WATER_SURFACE_FOUND == 0 && heights[2] != -1.0e9)
        || (flags & COLLISION_WATER_SURFACE_FOUND != 0 && heights[2] == -1.0e9)
        || (flags & COLLISION_TRAJECTORY_VALID == 0
            && old_position
                .iter()
                .chain(&resolved_frame_displacement)
                .chain(&final_position)
                .any(|value| *value != 0.0))
    {
        return Err(TraceError(
            "invalid gameplay trace player-background-collision height sentinel".into(),
        ));
    }
    if flags & COLLISION_TRAJECTORY_VALID != 0
        && (0..3).any(|axis| {
            let reconstructed = old_position[axis] + resolved_frame_displacement[axis];
            let tolerance = 1.0e-4 * final_position[axis].abs().max(1.0);
            (reconstructed - final_position[axis]).abs() > tolerance
        })
    {
        return Err(TraceError(
            "gameplay trace collision trajectory does not reconstruct final position".into(),
        ));
    }
    record.player_background_collision = Some(TracePlayerBackgroundCollision {
        flags,
        ground_height: heights[0],
        roof_height: heights[1],
        water_height: heights[2],
        ground_bg_index: (ground_bg != INVALID_U16_ID).then_some(ground_bg),
        ground_poly_index: (ground_poly != INVALID_U16_ID).then_some(ground_poly),
        ground_owner_session_process_id: (ground_owner != INVALID_U32_ID).then_some(ground_owner),
        ground_plane,
        ground_identity_present: ground_identity,
        roof_bg_index: (roof_bg != INVALID_U16_ID).then_some(roof_bg),
        roof_poly_index: (roof_poly != INVALID_U16_ID).then_some(roof_poly),
        roof_owner_session_process_id: (roof_owner != INVALID_U32_ID).then_some(roof_owner),
        roof_identity_present: roof_identity,
        water_bg_index: (water_bg != INVALID_U16_ID).then_some(water_bg),
        water_poly_index: (water_poly != INVALID_U16_ID).then_some(water_poly),
        water_owner_session_process_id: (water_owner != INVALID_U32_ID).then_some(water_owner),
        water_identity_present: water_identity,
        walls,
        old_position,
        resolved_frame_displacement,
        final_position,
    });
    Ok(())
}

fn decode_player_collision_surfaces_v1(
    bytes: &[u8],
    record: &mut TraceRecord,
) -> Result<(), TraceError> {
    let flags = u32_at(bytes, 0);
    if flags & !COLLISION_SURFACE_SET_KNOWN_FLAGS != 0
        || bytes[10] & !0x3f != 0
        || bytes[11..16].iter().any(|value| *value != 0)
    {
        return Err(TraceError(
            "invalid gameplay trace collision-surface set header".into(),
        ));
    }
    let room_valid = flags & COLLISION_SURFACE_SET_ROOM_VALID != 0;
    let raw_room = bytes[4] as i8;
    if room_valid != (raw_room != INVALID_I8) || room_valid && !(-1..=63).contains(&raw_room) {
        return Err(TraceError(
            "invalid gameplay trace collision-surface Link room".into(),
        ));
    }
    let raw_link_exit = u16_at(bytes, 8);
    if (flags & COLLISION_SURFACE_SET_EXPLICIT_LINK_EXIT != 0) != (raw_link_exit != 0x003f) {
        return Err(TraceError(
            "collision-surface explicit Link exit flag disagrees with raw field".into(),
        ));
    }

    let surfaces: [TraceCollisionSurface; 6] = (0..6)
        .map(|index| {
            decode_collision_surface(&bytes[16 + index * 80..16 + (index + 1) * 80], index)
        })
        .collect::<Result<Vec<_>, TraceError>>()?
        .try_into()
        .expect("six collision surface slots");
    let identity_count = surfaces
        .iter()
        .filter(|surface| surface.flags & COLLISION_SURFACE_IDENTITY_PRESENT != 0)
        .count() as u8;
    let backing_count = surfaces
        .iter()
        .filter(|surface| surface.flags & COLLISION_SURFACE_BACKING_PRESENT != 0)
        .count() as u8;
    let destination_count = surfaces
        .iter()
        .filter(|surface| surface.flags & COLLISION_SURFACE_DESTINATION_PRESENT != 0)
        .count() as u8;
    let pending_match_mask = surfaces
        .iter()
        .enumerate()
        .fold(0_u8, |mask, (index, surface)| {
            mask | (((surface.flags & COLLISION_SURFACE_PENDING_MATCH != 0) as u8) << index)
        });
    if bytes[5] != identity_count
        || bytes[6] != backing_count
        || bytes[7] != destination_count
        || bytes[10] != pending_match_mask
    {
        return Err(TraceError(
            "collision-surface set counts or pending-match mask disagree with slots".into(),
        ));
    }
    if flags & COLLISION_SURFACE_SET_EXPLICIT_LINK_EXIT != 0
        && surfaces[0].flags & COLLISION_SURFACE_PENDING_MATCH != 0
    {
        return Err(TraceError(
            "explicit Link exit cannot attribute the pending transition to ground collision".into(),
        ));
    }
    if surfaces
        .iter()
        .filter_map(|surface| surface.scls_source_room)
        .any(|room| !room_valid || room != raw_room)
    {
        return Err(TraceError(
            "collision-surface SCLS source disagrees with Link room".into(),
        ));
    }

    record.player_collision_surfaces = Some(TracePlayerCollisionSurfaces {
        flags,
        link_room: room_valid.then_some(raw_room),
        identity_count,
        backing_count,
        destination_count,
        raw_link_exit,
        pending_match_mask,
        surfaces,
    });
    Ok(())
}

fn decode_collision_surface(
    bytes: &[u8],
    expected_index: usize,
) -> Result<TraceCollisionSurface, TraceError> {
    let flags = u32_at(bytes, 0);
    if flags & !COLLISION_SURFACE_KNOWN_FLAGS != 0
        || bytes[51] != 0
        || bytes[76..80].iter().any(|value| *value != 0)
    {
        return Err(TraceError(format!(
            "invalid gameplay trace collision surface {expected_index} flags or reserved bytes"
        )));
    }
    let (expected_kind, expected_slot) = match expected_index {
        0 => (TraceCollisionSurfaceKind::Ground, 0),
        1 => (TraceCollisionSurfaceKind::Roof, 0),
        2 => (TraceCollisionSurfaceKind::Water, 0),
        3..=5 => (TraceCollisionSurfaceKind::Wall, (expected_index - 3) as u8),
        _ => unreachable!("bounded collision surface slot"),
    };
    let kind = match bytes[4] {
        1 => TraceCollisionSurfaceKind::Ground,
        2 => TraceCollisionSurfaceKind::Roof,
        3 => TraceCollisionSurfaceKind::Water,
        4 => TraceCollisionSurfaceKind::Wall,
        value => {
            return Err(TraceError(format!(
                "invalid gameplay trace collision surface kind {value}"
            )));
        }
    };
    if kind != expected_kind || bytes[5] != expected_slot {
        return Err(TraceError(format!(
            "collision surface {expected_index} has a noncanonical kind or wall slot"
        )));
    }

    let has = |flag| flags & flag != 0;
    let identity = has(COLLISION_SURFACE_IDENTITY_PRESENT);
    let owner_present = has(COLLISION_SURFACE_OWNER_PRESENT);
    let backing_present = has(COLLISION_SURFACE_BACKING_PRESENT);
    let codes_present = has(COLLISION_SURFACE_CODES_PRESENT);
    let material_present = has(COLLISION_SURFACE_MATERIAL_PRESENT);
    let group_present = has(COLLISION_SURFACE_GROUP_PRESENT);
    let source_room_present = has(COLLISION_SURFACE_SOURCE_ROOM_PRESENT);
    let source_room_exact = has(COLLISION_SURFACE_SOURCE_ROOM_EXACT);
    let scls_source_present = has(COLLISION_SURFACE_SCLS_SOURCE_PRESENT);
    let destination_present = has(COLLISION_SURFACE_DESTINATION_PRESENT);
    let pending_match = has(COLLISION_SURFACE_PENDING_MATCH);
    let geometry_present = has(COLLISION_SURFACE_GEOMETRY_PRESENT);
    let kcl_height_present = has(COLLISION_SURFACE_KCL_HEIGHT_PRESENT);
    if (flags & !COLLISION_SURFACE_IDENTITY_PRESENT) != 0 && !identity
        || source_room_exact && !source_room_present
        || pending_match && (!scls_source_present || !destination_present)
        || (scls_source_present || destination_present || pending_match)
            && kind != TraceCollisionSurfaceKind::Ground
    {
        return Err(TraceError(format!(
            "collision surface {expected_index} has incoherent presence or provenance flags"
        )));
    }

    let bg = u16_at(bytes, 8);
    let poly = u16_at(bytes, 10);
    validate_identity_pair(bg, poly, identity, "surface")?;
    let owner = u32_at(bytes, 12);
    validate_owner(owner, owner_present, "surface")?;
    let material = u16_at(bytes, 16);
    let group = u16_at(bytes, 18);
    if (material != INVALID_U16_ID) != material_present
        || (group != INVALID_U16_ID) != group_present
    {
        return Err(TraceError(format!(
            "collision surface {expected_index} row sentinels disagree with flags"
        )));
    }

    let backing_format = match bytes[6] {
        0 if !backing_present => None,
        1 if backing_present => Some(TraceCollisionBackingFormat::Dzb),
        2 if backing_present => Some(TraceCollisionBackingFormat::Kcl),
        value => {
            return Err(TraceError(format!(
                "collision surface {expected_index} has invalid backing format {value}"
            )));
        }
    };
    let raw_code_word_mask = bytes[7];
    if raw_code_word_mask & !0x1f != 0
        || codes_present != (raw_code_word_mask != 0)
        || codes_present && (!backing_present || raw_code_word_mask & 1 == 0)
    {
        return Err(TraceError(format!(
            "collision surface {expected_index} has invalid raw-code presence"
        )));
    }
    let raw_code_words = std::array::from_fn(|word| u32_at(bytes, 20 + word * 4));
    if raw_code_words.iter().enumerate().any(|(word, value)| {
        let present = raw_code_word_mask & (1 << word) != 0;
        !present && *value != 0
    }) {
        return Err(TraceError(format!(
            "collision surface {expected_index} has data in an absent raw-code word"
        )));
    }
    match backing_format {
        None => {
            if codes_present
                || material_present
                || group_present
                || geometry_present
                || kcl_height_present
            {
                return Err(TraceError(format!(
                    "collision surface {expected_index} has backing fields without backing"
                )));
            }
        }
        Some(TraceCollisionBackingFormat::Dzb) => {
            if kcl_height_present {
                return Err(TraceError(format!(
                    "collision surface {expected_index} has inconsistent DZB backing"
                )));
            }
        }
        Some(TraceCollisionBackingFormat::Kcl) => {
            if group_present {
                return Err(TraceError(format!(
                    "collision surface {expected_index} has inconsistent KCL backing"
                )));
            }
        }
    }

    let raw_exit = bytes[40];
    if codes_present {
        if raw_exit != (raw_code_words[0] & 0x3f) as u8 {
            return Err(TraceError(format!(
                "collision surface {expected_index} raw exit disagrees with collision code"
            )));
        }
    } else if raw_exit != u8::MAX {
        return Err(TraceError(format!(
            "collision surface {expected_index} has a raw exit without collision codes"
        )));
    }

    let raw_source_room = bytes[41] as i8;
    let raw_scls_room = bytes[42] as i8;
    if source_room_present != (raw_source_room != INVALID_I8)
        || source_room_present && !(-1..=63).contains(&raw_source_room)
        || scls_source_present != (raw_scls_room != INVALID_I8)
        || scls_source_present && !(-1..=63).contains(&raw_scls_room)
    {
        return Err(TraceError(format!(
            "collision surface {expected_index} has invalid room sentinels"
        )));
    }

    let destination_name = decode_name(&bytes[68..76])?;
    let destination_room = bytes[43] as i8;
    let destination_layer = bytes[44] as i8;
    let destination_wipe = bytes[45];
    let destination_wipe_time = bytes[46];
    let destination_time_hour = bytes[47] as i8;
    let destination_point = i16_at(bytes, 48);
    if destination_present {
        if !scls_source_present
            || !codes_present
            || raw_exit == 0x3f
            || raw_exit == u8::MAX
            || destination_name.is_empty()
            || !(-1..=63).contains(&destination_room)
            || !(destination_layer == -1 || (0..=14).contains(&destination_layer))
            || destination_point < 0
            || destination_wipe_time > 7
            || !(-1..=30).contains(&destination_time_hour)
        {
            return Err(TraceError(format!(
                "collision surface {expected_index} has an invalid destination"
            )));
        }
    } else if !destination_name.is_empty()
        || destination_room != INVALID_I8
        || destination_layer != INVALID_I8
        || destination_wipe != u8::MAX
        || destination_wipe_time != u8::MAX
        || destination_time_hour != INVALID_I8
        || destination_point != INVALID_I16
    {
        return Err(TraceError(format!(
            "collision surface {expected_index} destination sentinels disagree with flags"
        )));
    }

    let geometry_count = usize::from(bytes[50]);
    let geometry_indices: [u16; 6] = std::array::from_fn(|index| u16_at(bytes, 52 + index * 2));
    if geometry_present != (geometry_count != 0)
        || geometry_count > 6
        || geometry_indices
            .iter()
            .enumerate()
            .any(|(index, value)| (*value != INVALID_U16_ID) != (index < geometry_count))
    {
        return Err(TraceError(format!(
            "collision surface {expected_index} has invalid source geometry"
        )));
    }

    let kcl_prism_height = f32_at(bytes, 64);
    if !kcl_prism_height.is_finite()
        || kcl_height_present && backing_format != Some(TraceCollisionBackingFormat::Kcl)
        || !kcl_height_present && kcl_prism_height != 0.0
    {
        return Err(TraceError(format!(
            "collision surface {expected_index} has invalid KCL prism height"
        )));
    }

    Ok(TraceCollisionSurface {
        flags,
        kind,
        wall_slot: bytes[5],
        backing_format,
        raw_code_word_mask,
        bg_index: identity.then_some(bg),
        poly_index: identity.then_some(poly),
        owner_session_process_id: owner_present.then_some(owner),
        material_row: material_present.then_some(material),
        group_row: group_present.then_some(group),
        raw_code_words,
        raw_exit_id: codes_present.then_some(raw_exit),
        source_room: source_room_present.then_some(raw_source_room),
        source_room_exact,
        scls_source_room: scls_source_present.then_some(raw_scls_room),
        destination: destination_present.then_some(TraceCollisionSurfaceDestination {
            stage_name: destination_name,
            room: destination_room,
            layer: destination_layer,
            point: destination_point,
            wipe: destination_wipe,
            wipe_time: destination_wipe_time,
            time_hour: destination_time_hour,
        }),
        source_geometry_indices: geometry_indices[..geometry_count].to_vec(),
        kcl_prism_height: kcl_height_present.then_some(kcl_prism_height),
    })
}

fn validate_collision_surface_joins(record: &TraceRecord) -> Result<(), TraceError> {
    let Some(surfaces) = &record.player_collision_surfaces else {
        return Ok(());
    };
    let stage_present =
        record.channel_status.get(&TraceChannel::Stage) == Some(&TraceChannelStatus::Present);
    if !stage_present {
        return Err(TraceError(
            "player collision surfaces require present Stage observations".into(),
        ));
    }
    let pending = surfaces.flags & COLLISION_SURFACE_SET_NEXT_STAGE_PENDING != 0;
    if pending != record.next_stage_enabled {
        return Err(TraceError(
            "collision-surface pending-stage flag disagrees with Stage channel".into(),
        ));
    }
    for (index, surface) in surfaces.surfaces.iter().enumerate() {
        let matches_stage = pending
            && surface.destination.as_ref().is_some_and(|destination| {
                destination.stage_name == record.next_stage_name
                    && destination.room == record.next_room
                    && destination.layer == record.next_layer
                    && destination.point == record.next_point
            });
        if matches_stage != (surface.flags & COLLISION_SURFACE_PENDING_MATCH != 0) {
            return Err(TraceError(format!(
                "collision surface {index} pending-stage match disagrees with Stage channel"
            )));
        }
    }

    let Some(collision) = &record.player_background_collision else {
        return Ok(());
    };
    let wall_identity = |index: usize| {
        let wall = &collision.walls[index];
        (
            wall.bg_index,
            wall.poly_index,
            wall.owner_session_process_id,
        )
    };
    let expected: [(Option<u16>, Option<u16>, Option<u32>); 6] = [
        (
            collision.ground_bg_index,
            collision.ground_poly_index,
            collision.ground_owner_session_process_id,
        ),
        (
            collision.roof_bg_index,
            collision.roof_poly_index,
            collision.roof_owner_session_process_id,
        ),
        (
            collision.water_bg_index,
            collision.water_poly_index,
            collision.water_owner_session_process_id,
        ),
        wall_identity(0),
        wall_identity(1),
        wall_identity(2),
    ];
    for (index, (surface, expected)) in surfaces.surfaces.iter().zip(expected).enumerate() {
        let actual = (
            surface.bg_index,
            surface.poly_index,
            surface.owner_session_process_id,
        );
        if actual != expected {
            return Err(TraceError(format!(
                "collision surface {index} identity or owner disagrees with background collision"
            )));
        }
    }
    Ok(())
}

fn validate_identity_pair(bg: u16, poly: u16, present: bool, kind: &str) -> Result<(), TraceError> {
    if (bg != INVALID_U16_ID) != present || (poly != INVALID_U16_ID) != present {
        return Err(TraceError(format!(
            "invalid gameplay trace collision {kind} identity sentinel"
        )));
    }
    Ok(())
}

fn validate_owner(owner: u32, present: bool, kind: &str) -> Result<(), TraceError> {
    if (owner != INVALID_U32_ID) != present {
        return Err(TraceError(format!(
            "invalid gameplay trace collision {kind} owner sentinel"
        )));
    }
    Ok(())
}

fn validate_plane(plane: [f32; 4], present: bool, kind: &str) -> Result<(), TraceError> {
    if plane.iter().any(|value| !value.is_finite())
        || (!present && plane.iter().any(|value| *value != 0.0))
    {
        return Err(TraceError(format!(
            "invalid gameplay trace collision {kind} plane"
        )));
    }
    Ok(())
}

fn decode_rng_stream(bytes: &[u8]) -> Result<TraceRngStream, TraceError> {
    if bytes[1..4].iter().any(|value| *value != 0) {
        return Err(TraceError(
            "nonzero gameplay trace RNG reserved field".into(),
        ));
    }
    Ok(TraceRngStream {
        id: bytes[0],
        algorithm_version: u32_at(bytes, 4),
        state: [i32_at(bytes, 8), i32_at(bytes, 12), i32_at(bytes, 16)],
        call_count: u64_at(bytes, 20),
    })
}

fn decode_pad(bytes: &[u8]) -> Result<RawPadState, TraceError> {
    if bytes[10] & !1 != 0 {
        return Err(TraceError("unknown gameplay trace pad flags".into()));
    }
    Ok(RawPadState {
        buttons: u16_at(bytes, 0),
        stick_x: bytes[2] as i8,
        stick_y: bytes[3] as i8,
        substick_x: bytes[4] as i8,
        substick_y: bytes[5] as i8,
        trigger_left: bytes[6],
        trigger_right: bytes[7],
        analog_a: bytes[8],
        analog_b: bytes[9],
        connected: bytes[10] & 1 != 0,
        error: bytes[11] as i8,
    })
}

fn validate_tick_rate(bytes: &[u8]) -> Result<(), TraceError> {
    if u32_at(bytes, 12) == 0 || u32_at(bytes, 16) == 0 {
        return Err(TraceError("invalid gameplay trace tick rate".into()));
    }
    Ok(())
}

fn milestone(kind: &'static str, record: &TraceRecord) -> TraceMilestone {
    TraceMilestone {
        kind,
        simulation_tick: record.simulation_tick,
        tape_frame: record.tape_frame,
        location: record.location(),
        position: record.position,
        event_id: record.event_id,
        event_name_hash: record.event_name_hash,
        event_name_hash_present: record.event_name_hash_present,
    }
}

fn summarize(decoded: DecodedTrace) -> TraceSummary {
    let boot = decoded.boot.clone();
    let records = decoded.records;
    let playable_index = records.iter().position(|record| {
        record.stage_name == "F_SP103"
            && record.room == 1
            && record.point == 1
            && record.player_present()
            && record.player_is_link()
            && !record.event_running()
    });
    let loading_index = playable_index.and_then(|index| {
        let initial = records[index].location();
        records[index + 1..]
            .iter()
            .position(|record| record.location() != initial)
            .map(|relative| index + 1 + relative)
    });
    let opening_event_index = playable_index.and_then(|index| {
        records[index + 1..]
            .iter()
            .position(TraceRecord::event_running)
            .map(|relative| index + 1 + relative)
    });
    let route_control_index = opening_event_index.and_then(|index| {
        records[index + 1..]
            .iter()
            .position(|record| {
                record.stage_name == "F_SP103"
                    && record.room == 1
                    && record.player_is_link()
                    && !record.event_running()
            })
            .map(|relative| index + 1 + relative)
    });
    let loading_trigger_index = route_control_index.and_then(|index| {
        records[index + 1..]
            .iter()
            .position(|record| record.stage_name == "F_SP103" && record.event_running())
            .map(|relative| index + 1 + relative)
    });
    let post_load_index = loading_index.and_then(|index| {
        records[index..]
            .iter()
            .position(|record| {
                record.player_present() && record.player_is_link() && !record.event_running()
            })
            .map(|relative| index + relative)
    });
    let post_load_event_index = post_load_index.and_then(|index| {
        records[index + 1..]
            .iter()
            .position(TraceRecord::event_running)
            .map(|relative| index + 1 + relative)
    });
    let intro_cutscene_index = records.iter().position(|record| {
        record.stage_name == "F_SP104"
            && record.point == 26
            && record.event_running()
            && record.event_map_tool_id == 9
    });
    let requested_channels = TraceChannel::ALL
        .into_iter()
        .filter(|channel| decoded.requested_channels & channel.bit() != 0)
        .map(TraceChannel::name)
        .collect();

    TraceSummary {
        version: decoded.version,
        boot,
        requested_channels,
        record_count: records.len(),
        capacity_exhausted: decoded.capacity_exhausted,
        retention: decoded.retention,
        first_playable: playable_index.map(|index| milestone("first_playable", &records[index])),
        route_control: route_control_index.map(|index| milestone("route_control", &records[index])),
        first_loading_trigger: loading_trigger_index
            .map(|index| milestone("first_loading_trigger", &records[index])),
        first_loading_transition: loading_index
            .map(|index| milestone("first_loading_transition", &records[index])),
        post_load_playable: post_load_index
            .map(|index| milestone("post_load_playable", &records[index])),
        first_post_load_event: post_load_event_index
            .map(|index| milestone("first_post_load_event", &records[index])),
        intro_cutscene: intro_cutscene_index
            .map(|index| milestone("intro_cutscene", &records[index])),
        final_record: records.last().cloned(),
    }
}

fn decode_name(bytes: &[u8]) -> Result<String, TraceError> {
    let end = bytes
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(bytes.len());
    if bytes[end..].iter().any(|value| *value != 0)
        || bytes[..end].iter().any(|value| !value.is_ascii_graphic())
    {
        return Err(TraceError("invalid canonical gameplay trace name".into()));
    }
    Ok(String::from_utf8(bytes[..end].to_vec()).expect("validated ASCII"))
}

fn checked_region_end(start: usize, count: usize, stride: usize) -> Result<usize, TraceError> {
    start
        .checked_add(
            count
                .checked_mul(stride)
                .ok_or_else(|| TraceError("gameplay trace size overflow".into()))?,
        )
        .ok_or_else(|| TraceError("gameplay trace size overflow".into()))
}

fn count_at(bytes: &[u8], offset: usize) -> Result<usize, TraceError> {
    let count = usize::try_from(u64_at(bytes, offset))
        .map_err(|_| TraceError("gameplay trace record count is too large".into()))?;
    if count > MAX_TRACE_RECORDS {
        return Err(TraceError(format!(
            "gameplay trace record count exceeds {MAX_TRACE_RECORDS}"
        )));
    }
    Ok(count)
}

fn usize_at_u64(bytes: &[u8], offset: usize) -> Result<usize, TraceError> {
    usize::try_from(u64_at(bytes, offset))
        .map_err(|_| TraceError("gameplay trace offset is too large".into()))
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().expect("bounded field"))
}

fn i16_at(bytes: &[u8], offset: usize) -> i16 {
    i16::from_le_bytes(bytes[offset..offset + 2].try_into().expect("bounded field"))
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("bounded field"))
}

fn i32_at(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("bounded field"))
}

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("bounded field"))
}

fn f32_at(bytes: &[u8], offset: usize) -> f32 {
    f32::from_bits(u32_at(bytes, offset))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = vec![0; V1_HEADER_SIZE];
        bytes[..8].copy_from_slice(b"NOTTRACE");
        assert!(decode(&bytes).unwrap_err().to_string().contains("magic"));
    }

    #[test]
    fn v1_keeps_explicit_post_simulation_alignment() {
        let mut bytes = vec![0; V1_HEADER_SIZE + V1_RECORD_SIZE];
        bytes[..8].copy_from_slice(MAGIC);
        bytes[8..10].copy_from_slice(&1_u16.to_le_bytes());
        bytes[10..12].copy_from_slice(&(V1_RECORD_SIZE as u16).to_le_bytes());
        bytes[12..16].copy_from_slice(&30_u32.to_le_bytes());
        bytes[16..20].copy_from_slice(&1_u32.to_le_bytes());
        bytes[20..28].copy_from_slice(&1_u64.to_le_bytes());
        let record = &mut bytes[V1_HEADER_SIZE..];
        record[0..8].copy_from_slice(&41_u64.to_le_bytes());
        record[8..16].copy_from_slice(&9_u64.to_le_bytes());
        record[16..23].copy_from_slice(b"F_SP103");
        record[70..72].copy_from_slice(&u16::MAX.to_le_bytes());
        record[82..84].copy_from_slice(&(-1_i16).to_le_bytes());
        record[96..100].copy_from_slice(&(-1.0_f32).to_bits().to_le_bytes());
        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded.records[0].boundary_index, 42);
        assert_eq!(
            decoded.records[0].observation_phase,
            TracePhase::PostSimulation
        );
        assert_eq!(decoded.records[0].tape_frame, Some(9));
        assert!(decoded.channel_formats.is_empty());
    }

    #[test]
    fn v2_decodes_scene_exit_v1_and_retains_wire_format() {
        let mut payload = vec![0; 24];
        payload[0..4].copy_from_slice(&7_u32.to_le_bytes());
        payload[4..6].copy_from_slice(&(-13_i16).to_le_bytes());
        write_f32(&mut payload, 8, 10.0);
        write_f32(&mut payload, 12, 20.0);
        write_f32(&mut payload, 16, 30.0);
        write_f32(&mut payload, 20, 40.0);
        let decoded = build_v2_trace(vec![(
            TraceChannel::SceneExit,
            1,
            TraceChannelStatus::Present,
            payload,
        )]);
        let decoded = decode(&decoded).unwrap();
        assert_eq!(
            decoded.channel_formats[&TraceChannel::SceneExit],
            TraceChannelWireFormat {
                version: 1,
                stride: 24
            }
        );
        let record = &decoded.records[0];
        assert_eq!(record.nearest_scene_exit_session_process_id, Some(7));
        assert_eq!(record.nearest_scene_exit_actor_name, Some(-13));
        assert_eq!(record.nearest_scene_exit_position, [10.0, 20.0, 30.0]);
        assert_eq!(record.nearest_scene_exit_distance, Some(40.0));
        assert!(record.scene_exit.is_none());
    }

    #[test]
    fn v2_decodes_scene_exit_v2_destination_and_geometry() {
        let payload = scene_exit_v2_payload();
        let decoded = decode(&build_v2_trace(vec![(
            TraceChannel::SceneExit,
            2,
            TraceChannelStatus::Present,
            payload,
        )]))
        .unwrap();
        assert_eq!(
            decoded.channel_formats[&TraceChannel::SceneExit],
            TraceChannelWireFormat {
                version: 2,
                stride: 88
            }
        );
        let exit = decoded.records[0].scene_exit.as_ref().unwrap();
        assert_eq!(exit.session_process_id, 0x1234);
        assert_eq!(exit.kind, TraceSceneExitKind::OrientedBox);
        assert_eq!(exit.observed_count, 2);
        assert!(!exit.observed_count_saturated);
        assert_eq!(exit.signed_distance_to_volume, -0.25);
        assert_eq!(exit.home_position, [100.0, 200.0, 300.0]);
        let destination = exit.destination.as_ref().unwrap();
        assert_eq!(destination.stage_name, "F_SP103");
        assert_eq!(destination.room, 1);
        assert_eq!(destination.layer, -1);
        assert_eq!(destination.point, 4);
        assert_eq!(destination.wipe, 17);
        assert_eq!(destination.wipe_time, 3);
        assert_eq!(destination.time_hour, -1);
        assert_eq!(decoded.records[0].nearest_scene_exit_distance, None);
    }

    #[test]
    fn v2_scene_exit_latch_preserves_raw_ff_direction() {
        let mut payload = scene_exit_v2_payload();
        let flags = u32_at(&payload, 8) | SCENE_EXIT_PLAYER_LATCHED;
        payload[8..12].copy_from_slice(&flags.to_le_bytes());
        payload[27] = u8::MAX;
        payload[28..30].copy_from_slice(&7_u16.to_le_bytes());
        payload[8..12].copy_from_slice(&(flags & !SCENE_EXIT_DESTINATION_VALID).to_le_bytes());
        payload[72..80].fill(0);
        payload[80] = u8::MAX;
        payload[81] = u8::MAX;
        payload[82..84].copy_from_slice(&(-1_i16).to_le_bytes());
        payload[84..87].fill(u8::MAX);

        let decoded = decode(&build_v2_trace(vec![(
            TraceChannel::SceneExit,
            2,
            TraceChannelStatus::Present,
            payload,
        )]))
        .unwrap();
        let exit = decoded.records[0].scene_exit.as_ref().unwrap();
        assert_eq!(exit.link_exit_id, Some(7));
        assert_eq!(exit.link_exit_direction, Some(u8::MAX));
        assert!(exit.destination.is_none());
    }

    #[test]
    fn v2_scene_exit_preserves_saturated_observed_count() {
        let mut payload = scene_exit_v2_payload();
        let flags = u32_at(&payload, 8) | SCENE_EXIT_OBSERVED_COUNT_SATURATED;
        payload[8..12].copy_from_slice(&flags.to_le_bytes());
        payload[25] = u8::MAX;
        let decoded = decode(&build_v2_trace(vec![(
            TraceChannel::SceneExit,
            2,
            TraceChannelStatus::Present,
            payload,
        )]))
        .unwrap();
        let exit = decoded.records[0].scene_exit.as_ref().unwrap();
        assert_eq!(exit.observed_count, u8::MAX);
        assert!(exit.observed_count_saturated);
    }

    #[test]
    fn v2_decodes_player_background_collision_v1() {
        let decoded = decode(&build_v2_trace(vec![(
            TraceChannel::PlayerBackgroundCollision,
            1,
            TraceChannelStatus::Present,
            background_collision_v1_payload(),
        )]))
        .unwrap();
        assert_eq!(
            decoded.channel_formats[&TraceChannel::PlayerBackgroundCollision],
            TraceChannelWireFormat {
                version: 1,
                stride: 128
            }
        );
        let collision = decoded.records[0]
            .player_background_collision
            .as_ref()
            .unwrap();
        assert_eq!(collision.flags, COLLISION_TRAJECTORY_VALID);
        assert_eq!(collision.ground_height, -1.0e9);
        assert_eq!(collision.roof_height, 1.0e9);
        assert_eq!(collision.old_position, [1.0, 2.0, 3.0]);
        assert_eq!(collision.resolved_frame_displacement, [4.0, 5.0, 6.0]);
        assert_eq!(collision.final_position, [5.0, 7.0, 9.0]);
        assert!(collision.walls.iter().all(|wall| wall.bg_index.is_none()));
    }

    #[test]
    fn v2_decodes_collision_surfaces_and_pending_ground_destination() {
        let decoded = decode(&build_v2_trace(vec![
            (
                TraceChannel::Stage,
                1,
                TraceChannelStatus::Present,
                stage_payload(true),
            ),
            (
                TraceChannel::PlayerCollisionSurfaces,
                1,
                TraceChannelStatus::Present,
                collision_surfaces_pending_ground_payload(),
            ),
        ]))
        .unwrap();
        assert_eq!(
            decoded.channel_formats[&TraceChannel::PlayerCollisionSurfaces],
            TraceChannelWireFormat {
                version: 1,
                stride: 496
            }
        );
        let set = decoded.records[0]
            .player_collision_surfaces
            .as_ref()
            .unwrap();
        assert_eq!(set.link_room, Some(1));
        assert_eq!(set.identity_count, 1);
        assert_eq!(set.backing_count, 1);
        assert_eq!(set.destination_count, 1);
        assert_eq!(set.pending_match_mask, 1);
        let ground = &set.surfaces[0];
        assert_eq!(ground.kind, TraceCollisionSurfaceKind::Ground);
        assert_eq!(
            ground.backing_format,
            Some(TraceCollisionBackingFormat::Kcl)
        );
        assert_eq!(ground.bg_index, Some(7));
        assert_eq!(ground.poly_index, Some(2217));
        assert_eq!(ground.material_row, Some(19));
        assert_eq!(ground.raw_exit_id, Some(1));
        assert_eq!(ground.source_geometry_indices, vec![2, 3, 5, 7, 11]);
        assert_eq!(ground.kcl_prism_height, Some(42.5));
        assert_eq!(ground.destination.as_ref().unwrap().stage_name, "F_SP104");
        assert!(
            set.surfaces[1..]
                .iter()
                .all(|surface| surface.bg_index.is_none())
        );
    }

    #[test]
    fn v2_cross_checks_collision_surface_cache_identity() {
        let channels = vec![
            (
                TraceChannel::Stage,
                1,
                TraceChannelStatus::Present,
                stage_payload(true),
            ),
            (
                TraceChannel::PlayerBackgroundCollision,
                1,
                TraceChannelStatus::Present,
                background_collision_with_ground(7, 2217),
            ),
            (
                TraceChannel::PlayerCollisionSurfaces,
                1,
                TraceChannelStatus::Present,
                collision_surfaces_pending_ground_payload(),
            ),
        ];
        decode(&build_v2_trace(channels.clone())).unwrap();

        let mut mismatched = channels;
        mismatched[1].3[18..20].copy_from_slice(&841_u16.to_le_bytes());
        let error = decode(&build_v2_trace(mismatched)).unwrap_err();
        assert!(error.to_string().contains("identity or owner disagrees"));
    }

    #[test]
    fn v2_rejects_collision_surface_wire_corruption() {
        let build = |surface_payload| {
            build_v2_trace(vec![
                (
                    TraceChannel::Stage,
                    1,
                    TraceChannelStatus::Present,
                    stage_payload(true),
                ),
                (
                    TraceChannel::PlayerCollisionSurfaces,
                    1,
                    TraceChannelStatus::Present,
                    surface_payload,
                ),
            ])
        };

        let mut payload = collision_surfaces_pending_ground_payload();
        payload[16 + 40] = 2;
        assert!(
            decode(&build(payload))
                .unwrap_err()
                .to_string()
                .contains("raw exit disagrees")
        );

        let mut payload = collision_surfaces_pending_ground_payload();
        payload[10] = 0;
        assert!(
            decode(&build(payload))
                .unwrap_err()
                .to_string()
                .contains("pending-match mask")
        );

        let mut payload = collision_surfaces_pending_ground_payload();
        payload[16 + 76] = 1;
        assert!(
            decode(&build(payload))
                .unwrap_err()
                .to_string()
                .contains("reserved")
        );

        let mut payload = collision_surfaces_pending_ground_payload();
        write_f32(&mut payload, 16 + 64, f32::NAN);
        assert!(
            decode(&build(payload))
                .unwrap_err()
                .to_string()
                .contains("prism height")
        );

        let error = decode(&build_v2_trace(vec![(
            TraceChannel::PlayerCollisionSurfaces,
            1,
            TraceChannelStatus::Present,
            empty_collision_surfaces_payload(),
        )]))
        .unwrap_err();
        assert!(error.to_string().contains("require the Stage channel"));
    }

    #[test]
    fn v2_rejects_known_channel_version_stride_and_status_mismatches() {
        let wrong_version = build_v2_trace(vec![(
            TraceChannel::SceneExit,
            3,
            TraceChannelStatus::Present,
            vec![0; 88],
        )]);
        assert!(
            decode(&wrong_version)
                .unwrap_err()
                .to_string()
                .contains("scene_exit version 3")
        );

        let wrong_stride = build_v2_trace(vec![(
            TraceChannel::SceneExit,
            2,
            TraceChannelStatus::Present,
            vec![0; 72],
        )]);
        assert!(
            decode(&wrong_stride)
                .unwrap_err()
                .to_string()
                .contains("expected 88")
        );

        let truncated = build_v2_trace(vec![(
            TraceChannel::SceneExit,
            2,
            TraceChannelStatus::Truncated,
            scene_exit_v2_payload(),
        )]);
        assert!(
            decode(&truncated)
                .unwrap_err()
                .to_string()
                .contains("status Truncated")
        );

        let collision_truncated = build_v2_trace(vec![(
            TraceChannel::PlayerBackgroundCollision,
            1,
            TraceChannelStatus::Truncated,
            background_collision_v1_payload(),
        )]);
        assert!(
            decode(&collision_truncated)
                .unwrap_err()
                .to_string()
                .contains("status Truncated")
        );

        let collision_not_sampled = build_v2_trace(vec![(
            TraceChannel::PlayerBackgroundCollision,
            1,
            TraceChannelStatus::NotSampled,
            background_collision_v1_payload(),
        )]);
        assert!(
            decode(&collision_not_sampled)
                .unwrap_err()
                .to_string()
                .contains("status NotSampled")
        );

        let collision_surfaces_truncated = build_v2_trace(vec![
            (
                TraceChannel::Stage,
                1,
                TraceChannelStatus::Present,
                stage_payload(false),
            ),
            (
                TraceChannel::PlayerCollisionSurfaces,
                1,
                TraceChannelStatus::Truncated,
                empty_collision_surfaces_payload(),
            ),
        ]);
        assert!(
            decode(&collision_surfaces_truncated)
                .unwrap_err()
                .to_string()
                .contains("status Truncated")
        );
    }

    #[test]
    fn v2_rejects_scene_exit_and_collision_corruption() {
        let mut scene = scene_exit_v2_payload();
        scene[87] = 1;
        let error = decode(&build_v2_trace(vec![(
            TraceChannel::SceneExit,
            2,
            TraceChannelStatus::Present,
            scene,
        )]))
        .unwrap_err();
        assert!(error.to_string().contains("reserved"));

        let mut scene = scene_exit_v2_payload();
        let flags = u32_at(&scene, 8) | SCENE_EXIT_OBSERVED_COUNT_SATURATED;
        scene[8..12].copy_from_slice(&flags.to_le_bytes());
        let error = decode(&build_v2_trace(vec![(
            TraceChannel::SceneExit,
            2,
            TraceChannelStatus::Present,
            scene,
        )]))
        .unwrap_err();
        assert!(error.to_string().contains("candidate count"));

        let mut scene = scene_exit_v2_payload();
        scene[20] = 4;
        let error = decode(&build_v2_trace(vec![(
            TraceChannel::SceneExit,
            2,
            TraceChannelStatus::Present,
            scene,
        )]))
        .unwrap_err();
        assert!(error.to_string().contains("raw parameters"));

        let mut scene = scene_exit_v2_payload();
        let flags = u32_at(&scene, 8) | SCENE_EXIT_CHANGE_OK;
        scene[8..12].copy_from_slice(&flags.to_le_bytes());
        let error = decode(&build_v2_trace(vec![(
            TraceChannel::SceneExit,
            2,
            TraceChannelStatus::Present,
            scene,
        )]))
        .unwrap_err();
        assert!(error.to_string().contains("change-ok"));

        let mut scene = scene_exit_v2_payload();
        let flags = u32_at(&scene, 8) | SCENE_EXIT_PLAYER_LATCHED;
        scene[8..12].copy_from_slice(&flags.to_le_bytes());
        scene[24] = 2;
        scene[28..30].copy_from_slice(&7_u16.to_le_bytes());
        let error = decode(&build_v2_trace(vec![(
            TraceChannel::SceneExit,
            2,
            TraceChannelStatus::Present,
            scene,
        )]))
        .unwrap_err();
        assert!(error.to_string().contains("radial"));

        let mut collision = background_collision_v1_payload();
        collision[0..4].copy_from_slice(&(1_u32 << 31).to_le_bytes());
        let error = decode(&build_v2_trace(vec![(
            TraceChannel::PlayerBackgroundCollision,
            1,
            TraceChannelStatus::Present,
            collision,
        )]))
        .unwrap_err();
        assert!(error.to_string().contains("unknown"));

        let mut collision = background_collision_v1_payload();
        collision[0..4].copy_from_slice(
            &(COLLISION_TRAJECTORY_VALID | COLLISION_GROUND_IDENTITY_PRESENT).to_le_bytes(),
        );
        let error = decode(&build_v2_trace(vec![(
            TraceChannel::PlayerBackgroundCollision,
            1,
            TraceChannelStatus::Present,
            collision,
        )]))
        .unwrap_err();
        assert!(error.to_string().contains("identity sentinel"));

        let mut collision = background_collision_v1_payload();
        write_f32(&mut collision, 116, 6.0);
        let error = decode(&build_v2_trace(vec![(
            TraceChannel::PlayerBackgroundCollision,
            1,
            TraceChannelStatus::Present,
            collision,
        )]))
        .unwrap_err();
        assert!(error.to_string().contains("trajectory"));

        let mut collision = background_collision_v1_payload();
        collision[0..4].copy_from_slice(
            &(COLLISION_TRAJECTORY_VALID | COLLISION_WALL_PROBE_ENABLED | COLLISION_WALL_CONTACT)
                .to_le_bytes(),
        );
        let error = decode(&build_v2_trace(vec![(
            TraceChannel::PlayerBackgroundCollision,
            1,
            TraceChannelStatus::Present,
            collision,
        )]))
        .unwrap_err();
        assert!(error.to_string().contains("aggregate wall contact"));

        let mut collision = background_collision_v1_payload();
        collision[0..4].copy_from_slice(
            &(COLLISION_TRAJECTORY_VALID | COLLISION_WATER_PROBE_ENABLED | COLLISION_WATER_IN)
                .to_le_bytes(),
        );
        let error = decode(&build_v2_trace(vec![(
            TraceChannel::PlayerBackgroundCollision,
            1,
            TraceChannelStatus::Present,
            collision,
        )]))
        .unwrap_err();
        assert!(error.to_string().contains("contradictory"));

        let mut collision = background_collision_v1_payload();
        collision[0..4].copy_from_slice(
            &(COLLISION_TRAJECTORY_VALID | COLLISION_GROUND_PLANE_VALID).to_le_bytes(),
        );
        let error = decode(&build_v2_trace(vec![(
            TraceChannel::PlayerBackgroundCollision,
            1,
            TraceChannelStatus::Present,
            collision,
        )]))
        .unwrap_err();
        assert!(error.to_string().contains("contradictory"));
    }

    #[test]
    fn v2_decodes_goal_progress_and_bounded_selected_actors() {
        let mut goal = vec![0; 32];
        goal[0..4].copy_from_slice(
            &(GOAL_CONFIGURED | GOAL_REACHED | GOAL_AUTHORED | GOAL_FIRST_HIT_TICK_PRESENT)
                .to_le_bytes(),
        );
        goal[4..8].copy_from_slice(&0x1234_5678_u32.to_le_bytes());
        goal[8..10].copy_from_slice(&3_u16.to_le_bytes());
        goal[10..12].copy_from_slice(&2_u16.to_le_bytes());
        goal[12..14].copy_from_slice(&3_u16.to_le_bytes());
        goal[14..16].copy_from_slice(&3_u16.to_le_bytes());
        goal[16] = 2;
        goal[17] = 2;
        goal[18..20].copy_from_slice(&30_u16.to_le_bytes());
        goal[20..22].copy_from_slice(&7_u16.to_le_bytes());
        goal[24..32].copy_from_slice(&12_u64.to_le_bytes());

        let mut actors = vec![0; 656];
        actors[0..2].copy_from_slice(&1_u16.to_le_bytes());
        actors[2..4].copy_from_slice(&(SELECTED_ACTOR_CAPACITY as u16).to_le_bytes());
        actors[4..8].copy_from_slice(&SELECTED_ACTORS_TRUNCATED.to_le_bytes());
        actors[8..12].copy_from_slice(&2_u32.to_le_bytes());
        for index in 0..SELECTED_ACTOR_CAPACITY {
            let offset = 16 + index * 40;
            actors[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            actors[offset + 4..offset + 6].copy_from_slice(&(-1_i16).to_le_bytes());
            actors[offset + 6..offset + 8].copy_from_slice(&u16::MAX.to_le_bytes());
            actors[offset + 8] = -1_i8 as u8;
            actors[offset + 9] = -1_i8 as u8;
        }
        actors[16..20].copy_from_slice(&10_u32.to_le_bytes());
        actors[20..22].copy_from_slice(&77_i16.to_le_bytes());
        actors[22..24].copy_from_slice(&4_u16.to_le_bytes());
        actors[24] = 1;
        actors[25] = 2;
        actors[26..28].copy_from_slice(&9_i16.to_le_bytes());
        actors[28..32].copy_from_slice(&0x1234_u32.to_le_bytes());
        write_f32(&mut actors, 32, 1.0);
        write_f32(&mut actors, 36, 2.0);
        write_f32(&mut actors, 40, 3.0);
        actors[44..46].copy_from_slice(&4_i16.to_le_bytes());
        actors[50..52].copy_from_slice(&7_i16.to_le_bytes());

        let bytes = build_v2_trace(vec![
            (
                TraceChannel::GoalProgress,
                1,
                TraceChannelStatus::Present,
                goal.clone(),
            ),
            (
                TraceChannel::SelectedActors,
                1,
                TraceChannelStatus::Present,
                actors.clone(),
            ),
        ]);
        let decoded = decode(&bytes).unwrap();
        let record = &decoded.records[0];
        let decoded_goal = record.goal_progress.as_ref().unwrap();
        assert!(decoded_goal.reached && decoded_goal.authored);
        assert_eq!(decoded_goal.goal_name_hash, Some(0x1234_5678));
        assert_eq!(decoded_goal.first_hit_tick, Some(12));
        let decoded_actors = record.selected_actors.as_ref().unwrap();
        assert!(decoded_actors.truncated);
        assert_eq!(decoded_actors.observed_count, 2);
        assert_eq!(decoded_actors.actors[0].session_process_id, 10);
        assert_eq!(decoded_actors.actors[0].position, [1.0, 2.0, 3.0]);

        goal[24..32].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(
            decode(&build_v2_trace(vec![(
                TraceChannel::GoalProgress,
                1,
                TraceChannelStatus::Present,
                goal,
            )]))
            .unwrap_err()
            .to_string()
            .contains("goal-progress")
        );
        actors[56..60].copy_from_slice(&0_u32.to_le_bytes());
        assert!(
            decode(&build_v2_trace(vec![(
                TraceChannel::SelectedActors,
                1,
                TraceChannelStatus::Present,
                actors,
            )]))
            .unwrap_err()
            .to_string()
            .contains("unused")
        );
    }

    #[test]
    fn v2_decodes_portable_player_interaction_identities() {
        let mut action = vec![0; 136];
        action[104..108]
            .copy_from_slice(&(TALK_PARTNER_PRESENT | GRABBED_ACTOR_PRESENT).to_le_bytes());
        action[108] = 0x15;
        action[112..116].copy_from_slice(&11_u32.to_le_bytes());
        action[116..118].copy_from_slice(&42_i16.to_le_bytes());
        action[118..120].copy_from_slice(&7_u16.to_le_bytes());
        action[120] = 1;
        action[121] = 2;
        action[124..128].copy_from_slice(&12_u32.to_le_bytes());
        action[128..130].copy_from_slice(&43_i16.to_le_bytes());
        action[130..132].copy_from_slice(&8_u16.to_le_bytes());
        action[132] = 3;
        action[133] = 4;

        let decoded = decode(&build_v2_trace(vec![(
            TraceChannel::PlayerAction,
            2,
            TraceChannelStatus::Present,
            action.clone(),
        )]))
        .unwrap();
        assert_eq!(
            decoded.channel_formats[&TraceChannel::PlayerAction],
            TraceChannelWireFormat {
                version: 2,
                stride: 136,
            }
        );
        let player_action = decoded.records[0].player_action.as_ref().unwrap();
        assert_eq!(player_action.do_status, 0x15);
        assert_eq!(player_action.talk_partner.as_ref().unwrap().actor_name, 42);
        assert_eq!(player_action.talk_partner.as_ref().unwrap().set_id, 7);
        assert_eq!(player_action.grabbed_actor.as_ref().unwrap().actor_name, 43);
        assert_eq!(player_action.grabbed_actor.as_ref().unwrap().home_room, 3);

        action[104..108].copy_from_slice(&GRABBED_ACTOR_PRESENT.to_le_bytes());
        let error = decode(&build_v2_trace(vec![(
            TraceChannel::PlayerAction,
            2,
            TraceChannelStatus::Present,
            action,
        )]))
        .unwrap_err();
        assert!(error.to_string().contains("noncanonical absent"));
    }

    #[test]
    fn rejects_unknown_required_v2_channel() {
        let mut bytes = minimal_v2_header(2, TraceChannel::Core.bit());
        bytes.resize(V2_HEADER_SIZE + 2 * V2_DIRECTORY_ENTRY_SIZE, 0);
        write_empty_descriptor(&mut bytes[V2_HEADER_SIZE..], 0, 32, true);
        write_empty_descriptor(
            &mut bytes[V2_HEADER_SIZE + V2_DIRECTORY_ENTRY_SIZE..],
            15,
            1,
            true,
        );
        assert!(decode(&bytes).unwrap_err().to_string().contains("required"));
    }

    #[test]
    fn rejects_trace_record_count_above_global_bound_before_allocation() {
        let mut bytes = minimal_v2_header(0, TraceChannel::Core.bit());
        bytes[20..28].copy_from_slice(&((MAX_TRACE_RECORDS as u64) + 1).to_le_bytes());
        assert!(
            decode(&bytes)
                .unwrap_err()
                .to_string()
                .contains("record count exceeds")
        );
    }

    #[test]
    fn v3_authenticates_stage_boot_origin() {
        let mut bytes = build_v2_trace(Vec::new());
        let channel_count = usize::from(u16_at(&bytes, 32));
        bytes.splice(V2_HEADER_SIZE..V2_HEADER_SIZE, [0_u8; 64]);
        bytes[8..10].copy_from_slice(&3_u16.to_le_bytes());
        bytes[10..12].copy_from_slice(&(V3_HEADER_SIZE as u16).to_le_bytes());
        bytes[36..44].copy_from_slice(&(V3_HEADER_SIZE as u64).to_le_bytes());
        let data_offset = usize_at_u64(&bytes, 44).unwrap() + 64;
        bytes[44..52].copy_from_slice(&(data_offset as u64).to_le_bytes());
        for index in 0..channel_count {
            let descriptor = V3_HEADER_SIZE + index * V2_DIRECTORY_ENTRY_SIZE;
            for offset in [16, 32] {
                let value = usize_at_u64(&bytes, descriptor + offset).unwrap() + 64;
                bytes[descriptor + offset..descriptor + offset + 8]
                    .copy_from_slice(&(value as u64).to_le_bytes());
            }
        }
        bytes[64] = 1;
        bytes[65] = 2;
        bytes[66] = 1;
        bytes[67] = 3;
        bytes[68..70].copy_from_slice(&1_i16.to_le_bytes());
        bytes[70] = 7;
        bytes[72..79].copy_from_slice(b"F_SP103");

        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded.version, 3);
        assert_eq!(
            decoded.boot,
            TapeBoot::Stage {
                stage: "F_SP103".into(),
                room: 1,
                point: 1,
                layer: 3,
                save_slot: Some(2),
                fixture: None,
            }
        );

        bytes[79] = 1;
        assert!(
            decode(&bytes)
                .unwrap_err()
                .to_string()
                .contains("noncanonical stage boot")
        );
    }

    #[test]
    fn v4_authenticates_embedded_scenario_fixture() {
        use crate::scenario_fixture::{HealthFixture, PlayerForm, SCENARIO_FIXTURE_SCHEMA};

        let mut bytes = build_v2_trace(Vec::new());
        let channel_count = usize::from(u16_at(&bytes, 32));
        bytes.splice(V2_HEADER_SIZE..V2_HEADER_SIZE, [0_u8; 64]);
        bytes[8..10].copy_from_slice(&4_u16.to_le_bytes());
        bytes[10..12].copy_from_slice(&(V4_HEADER_SIZE as u16).to_le_bytes());
        bytes[36..44].copy_from_slice(&(V4_HEADER_SIZE as u64).to_le_bytes());
        let data_offset = usize_at_u64(&bytes, 44).unwrap() + 64;
        bytes[44..52].copy_from_slice(&(data_offset as u64).to_le_bytes());
        for index in 0..channel_count {
            let descriptor = V4_HEADER_SIZE + index * V2_DIRECTORY_ENTRY_SIZE;
            for offset in [16, 32] {
                let value = usize_at_u64(&bytes, descriptor + offset).unwrap() + 64;
                bytes[descriptor + offset..descriptor + offset + 8]
                    .copy_from_slice(&(value as u64).to_le_bytes());
            }
        }
        bytes[64] = 1;
        bytes[66] = 1;
        bytes[67] = 3;
        bytes[68..70].copy_from_slice(&1_i16.to_le_bytes());
        bytes[70] = 7;
        bytes[72..79].copy_from_slice(b"F_SP103");
        let fixture = ScenarioFixture {
            schema: SCENARIO_FIXTURE_SCHEMA.into(),
            name: "low-health wolf".into(),
            form: Some(PlayerForm::Wolf),
            health: Some(HealthFixture {
                current: 4,
                maximum: 20,
            }),
            rng: Vec::new(),
            video_mode: None,
            inventory: Vec::new(),
            equipment: Vec::new(),
            flags: Vec::new(),
            settings: Vec::new(),
        };
        let encoded = fixture.encode().unwrap();
        let fixture_offset = bytes.len();
        bytes[88..96].copy_from_slice(&(fixture_offset as u64).to_le_bytes());
        bytes[96..100].copy_from_slice(&(encoded.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&encoded);

        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded.version, 4);
        assert!(matches!(
            decoded.boot,
            TapeBoot::Stage {
                fixture: Some(value),
                ..
            } if value == fixture
        ));

        bytes[fixture_offset + 20] = 1;
        assert!(decode(&bytes).is_err());
    }

    #[test]
    fn v5_authenticates_trigger_retention_metadata() {
        let mut bytes = build_v2_trace(Vec::new());
        let channel_count = usize::from(u16_at(&bytes, 32));
        bytes.splice(V2_HEADER_SIZE..V2_HEADER_SIZE, [0_u8; 64]);
        bytes[8..10].copy_from_slice(&5_u16.to_le_bytes());
        bytes[10..12].copy_from_slice(&(V5_HEADER_SIZE as u16).to_le_bytes());
        bytes[28..32].copy_from_slice(&(FILE_COMPLETE | FILE_TRIGGER_RETENTION).to_le_bytes());
        bytes[36..44].copy_from_slice(&(V5_HEADER_SIZE as u64).to_le_bytes());
        let data_offset = usize_at_u64(&bytes, 44).unwrap() + 64;
        bytes[44..52].copy_from_slice(&(data_offset as u64).to_le_bytes());
        for index in 0..channel_count {
            let descriptor = V5_HEADER_SIZE + index * V2_DIRECTORY_ENTRY_SIZE;
            for offset in [16, 32] {
                let value = usize_at_u64(&bytes, descriptor + offset).unwrap() + 64;
                bytes[descriptor + offset..descriptor + offset + 8]
                    .copy_from_slice(&(value as u64).to_le_bytes());
            }
        }
        bytes[100..104].copy_from_slice(&KNOWN_RETENTION_TRIGGERS.to_le_bytes());
        bytes[104..108].copy_from_slice(&RETENTION_PREDICATE_HIT.to_le_bytes());
        bytes[108..112].copy_from_slice(&2_u32.to_le_bytes());
        bytes[112..116].copy_from_slice(&1_u32.to_le_bytes());
        bytes[116..120].copy_from_slice(&1_u32.to_le_bytes());
        bytes[120..128].copy_from_slice(&10_u64.to_le_bytes());

        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded.version, 5);
        assert_eq!(
            decoded.retention,
            Some(TraceRetention {
                configured_triggers: KNOWN_RETENTION_TRIGGERS,
                observed_triggers: RETENTION_PREDICATE_HIT,
                pre_trigger_ticks: 2,
                post_trigger_ticks: 1,
                trigger_count: 1,
                observed_sample_count: 10,
            })
        );

        bytes[104..108].copy_from_slice(&(1_u32 << 12).to_le_bytes());
        assert!(
            decode(&bytes)
                .unwrap_err()
                .to_string()
                .contains("retention")
        );
    }

    fn write_empty_descriptor(bytes: &mut [u8], id: u16, stride: u32, required: bool) {
        bytes[0..2].copy_from_slice(&id.to_le_bytes());
        bytes[2..4].copy_from_slice(&1_u16.to_le_bytes());
        let flags = CHANNEL_DENSE | if required { CHANNEL_REQUIRED } else { 0 };
        bytes[4..8].copy_from_slice(&flags.to_le_bytes());
        bytes[8..12].copy_from_slice(&stride.to_le_bytes());
        bytes[12..16].copy_from_slice(&1_u32.to_le_bytes());
        let data_offset = V2_HEADER_SIZE + 2 * V2_DIRECTORY_ENTRY_SIZE;
        bytes[16..24].copy_from_slice(&(data_offset as u64).to_le_bytes());
        bytes[32..40].copy_from_slice(&(data_offset as u64).to_le_bytes());
    }

    fn minimal_v2_header(channel_count: u16, requested: u64) -> Vec<u8> {
        let mut bytes = vec![0; V2_HEADER_SIZE];
        bytes[..8].copy_from_slice(MAGIC);
        bytes[8..10].copy_from_slice(&2_u16.to_le_bytes());
        bytes[10..12].copy_from_slice(&(V2_HEADER_SIZE as u16).to_le_bytes());
        bytes[12..16].copy_from_slice(&30_u32.to_le_bytes());
        bytes[16..20].copy_from_slice(&1_u32.to_le_bytes());
        bytes[28..32].copy_from_slice(&FILE_COMPLETE.to_le_bytes());
        bytes[32..34].copy_from_slice(&channel_count.to_le_bytes());
        bytes[34..36].copy_from_slice(&(V2_DIRECTORY_ENTRY_SIZE as u16).to_le_bytes());
        bytes[36..44].copy_from_slice(&(V2_HEADER_SIZE as u64).to_le_bytes());
        let data_offset = V2_HEADER_SIZE + usize::from(channel_count) * V2_DIRECTORY_ENTRY_SIZE;
        bytes[44..52].copy_from_slice(&(data_offset as u64).to_le_bytes());
        bytes[52..60].copy_from_slice(&requested.to_le_bytes());
        bytes
    }

    fn build_v2_trace(
        extra_channels: Vec<(TraceChannel, u16, TraceChannelStatus, Vec<u8>)>,
    ) -> Vec<u8> {
        let mut core = vec![0; 32];
        core[0..8].copy_from_slice(&1_u64.to_le_bytes());
        core[8..16].copy_from_slice(&0_u64.to_le_bytes());
        core[16..24].copy_from_slice(&u64::MAX.to_le_bytes());
        core[24..28].copy_from_slice(&CORE_SIMULATION_TICK_VALID.to_le_bytes());
        core[28] = 2;
        core[29] = 1;
        let mut channels = vec![(TraceChannel::Core, 1, TraceChannelStatus::Present, core)];
        channels.extend(extra_channels);
        let requested = channels
            .iter()
            .fold(0_u64, |mask, (channel, _, _, _)| mask | channel.bit());
        let mut bytes = minimal_v2_header(channels.len() as u16, requested);
        bytes[20..28].copy_from_slice(&1_u64.to_le_bytes());
        bytes.resize(V2_HEADER_SIZE + channels.len() * V2_DIRECTORY_ENTRY_SIZE, 0);
        for (index, (channel, version, status, payload)) in channels.into_iter().enumerate() {
            let descriptor = V2_HEADER_SIZE + index * V2_DIRECTORY_ENTRY_SIZE;
            let status_offset = bytes.len();
            bytes.push(match status {
                TraceChannelStatus::NotSampled => 0,
                TraceChannelStatus::Present => 1,
                TraceChannelStatus::Absent => 2,
                TraceChannelStatus::Unavailable => 3,
                TraceChannelStatus::Truncated => 4,
            });
            let payload_offset = bytes.len();
            bytes.extend_from_slice(&payload);
            bytes[descriptor..descriptor + 2].copy_from_slice(&(channel as u16).to_le_bytes());
            bytes[descriptor + 2..descriptor + 4].copy_from_slice(&version.to_le_bytes());
            let flags = CHANNEL_DENSE
                | if channel == TraceChannel::Core {
                    CHANNEL_REQUIRED
                } else {
                    0
                };
            bytes[descriptor + 4..descriptor + 8].copy_from_slice(&flags.to_le_bytes());
            bytes[descriptor + 8..descriptor + 12]
                .copy_from_slice(&(payload.len() as u32).to_le_bytes());
            bytes[descriptor + 12..descriptor + 16].copy_from_slice(&1_u32.to_le_bytes());
            bytes[descriptor + 16..descriptor + 24]
                .copy_from_slice(&(status_offset as u64).to_le_bytes());
            bytes[descriptor + 24..descriptor + 32].copy_from_slice(&1_u64.to_le_bytes());
            bytes[descriptor + 32..descriptor + 40]
                .copy_from_slice(&(payload_offset as u64).to_le_bytes());
            bytes[descriptor + 40..descriptor + 48]
                .copy_from_slice(&(payload.len() as u64).to_le_bytes());
        }
        bytes
    }

    fn scene_exit_v2_payload() -> Vec<u8> {
        let mut payload = vec![0; 88];
        payload[0..4].copy_from_slice(&0x1234_u32.to_le_bytes());
        payload[4..8].copy_from_slice(&0x0604_0503_u32.to_le_bytes());
        payload[8..12].copy_from_slice(
            &(SCENE_EXIT_VOLUME_VALID | SCENE_EXIT_PLAYER_INSIDE | SCENE_EXIT_DESTINATION_VALID)
                .to_le_bytes(),
        );
        write_f32(&mut payload, 12, -0.25);
        payload[16..18].copy_from_slice(&(-42_i16).to_le_bytes());
        payload[18..20].copy_from_slice(&9_u16.to_le_bytes());
        payload[20] = 3;
        payload[21] = 4;
        payload[22] = 5;
        payload[23] = 6;
        payload[24] = 1;
        payload[25] = 2;
        payload[26] = 1;
        payload[27] = u8::MAX;
        payload[28..30].copy_from_slice(&u16::MAX.to_le_bytes());
        payload[30..32].copy_from_slice(&0x123_i16.to_le_bytes());
        payload[32] = u8::MAX;
        for (offset, value) in [1.0, 2.0, 3.0, 10.0, 11.0, 12.0, 100.0, 200.0, 300.0]
            .into_iter()
            .enumerate()
        {
            write_f32(&mut payload, 36 + offset * 4, value);
        }
        payload[72..79].copy_from_slice(b"F_SP103");
        payload[80] = 1;
        payload[81] = -1_i8 as u8;
        payload[82..84].copy_from_slice(&4_i16.to_le_bytes());
        payload[84] = 17;
        payload[85] = 3;
        payload[86] = u8::MAX;
        payload
    }

    fn background_collision_v1_payload() -> Vec<u8> {
        let mut payload = vec![0; 128];
        payload[0..4].copy_from_slice(&COLLISION_TRAJECTORY_VALID.to_le_bytes());
        write_f32(&mut payload, 4, -1.0e9);
        write_f32(&mut payload, 8, 1.0e9);
        write_f32(&mut payload, 12, -1.0e9);
        for offset in [16, 18, 40, 42, 48, 50, 56, 58, 68, 70, 80, 82] {
            payload[offset..offset + 2].copy_from_slice(&u16::MAX.to_le_bytes());
        }
        for offset in [20, 44, 52, 60, 72, 84] {
            payload[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        }
        for (index, value) in [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 5.0, 7.0, 9.0]
            .into_iter()
            .enumerate()
        {
            write_f32(&mut payload, 92 + index * 4, value);
        }
        payload
    }

    fn stage_payload(next_pending: bool) -> Vec<u8> {
        let mut payload = vec![0; 32];
        payload[0..7].copy_from_slice(b"F_SP103");
        payload[8] = 1;
        payload[9] = u8::MAX;
        payload[10..12].copy_from_slice(&1_i16.to_le_bytes());
        if next_pending {
            payload[12..19].copy_from_slice(b"F_SP104");
            payload[20] = 1;
            payload[21] = u8::MAX;
            payload[22..24].copy_from_slice(&0_i16.to_le_bytes());
            payload[24..28].copy_from_slice(&1_u32.to_le_bytes());
        } else {
            payload[20] = u8::MAX;
            payload[21] = u8::MAX;
            payload[22..24].copy_from_slice(&(-1_i16).to_le_bytes());
        }
        payload
    }

    fn predicate_player_payload() -> Vec<u8> {
        let mut payload = vec![0; 52];
        payload[0..4].copy_from_slice(&19_u32.to_le_bytes());
        payload[4..6].copy_from_slice(&253_i16.to_le_bytes());
        payload[6..8].copy_from_slice(&7_u16.to_le_bytes());
        write_f32(&mut payload, 20, 666.0);
        write_f32(&mut payload, 24, 800.0);
        write_f32(&mut payload, 28, -2431.0);
        write_f32(&mut payload, 32, 1.0);
        write_f32(&mut payload, 44, 3.0);
        payload[48..52].copy_from_slice(&PLAYER_IS_LINK.to_le_bytes());
        payload
    }

    fn predicate_event_payload() -> Vec<u8> {
        let mut payload = vec![0; 16];
        payload[4..6].copy_from_slice(&(-1_i16).to_le_bytes());
        payload[8] = u8::MAX;
        payload
    }

    fn predicate_rng_payload() -> Vec<u8> {
        let mut payload = vec![0; 64];
        payload[0..4].copy_from_slice(&1_u32.to_le_bytes());
        payload[4..8].copy_from_slice(&2_u32.to_le_bytes());
        for (offset, id, states, calls) in [
            (8, 0_u8, [11_i32, 12, 13], 100_u64),
            (36, 1_u8, [21_i32, 22, 23], 200_u64),
        ] {
            payload[offset] = id;
            payload[offset + 4..offset + 8].copy_from_slice(&1_u32.to_le_bytes());
            for (index, state) in states.into_iter().enumerate() {
                payload[offset + 8 + index * 4..offset + 12 + index * 4]
                    .copy_from_slice(&state.to_le_bytes());
            }
            payload[offset + 20..offset + 28].copy_from_slice(&calls.to_le_bytes());
        }
        payload
    }

    #[test]
    fn authored_predicates_evaluate_against_a_decoded_recorded_trace_fixture() {
        let bytes = build_v2_trace(vec![
            (
                TraceChannel::Stage,
                1,
                TraceChannelStatus::Present,
                stage_payload(false),
            ),
            (
                TraceChannel::PlayerMotion,
                1,
                TraceChannelStatus::Present,
                predicate_player_payload(),
            ),
            (
                TraceChannel::Event,
                1,
                TraceChannelStatus::Present,
                predicate_event_payload(),
            ),
            (
                TraceChannel::Rng,
                1,
                TraceChannelStatus::Present,
                predicate_rng_payload(),
            ),
        ]);
        let trace = decode(&bytes).unwrap();
        let program = crate::milestone_dsl::parse(
            r#"milestones 1.3

milestone recorded_hit {
  phase post_sim
  when stage.name == "F_SP103" && stage.room == 1 &&
       player.exists && player.is_link && player.procedure == 7 &&
       player.position.x between 665.0 and 667.0 &&
       player.in_aabb(600.0, 700.0, -2500.0, 700.0, 900.0, -2400.0) &&
       !event.running && event.id == -1 && rng.primary.calls == 100
}

milestone unavailable_actor_catalog_cannot_guess {
  phase post_sim
  when actor.placed.exists("F_SP103", 1, 7, 42)
}
"#,
        )
        .unwrap();
        let hits = crate::milestone_dsl::evaluate_recorded_trace(&program, &trace).unwrap();
        let hit = hits["recorded_hit"].as_ref().unwrap();
        assert_eq!(hit.record_index, 0);
        assert_eq!(hit.boundary_index, 1);
        assert!(hits["unavailable_actor_catalog_cannot_guess"].is_none());
    }

    fn empty_collision_surface(payload: &mut [u8], index: usize) {
        let offset = 16 + index * 80;
        payload[offset + 4] = match index {
            0 => 1,
            1 => 2,
            2 => 3,
            3..=5 => 4,
            _ => unreachable!(),
        };
        payload[offset + 5] = index.saturating_sub(3) as u8;
        for field in [8, 10, 16, 18] {
            payload[offset + field..offset + field + 2].copy_from_slice(&u16::MAX.to_le_bytes());
        }
        payload[offset + 12..offset + 16].copy_from_slice(&u32::MAX.to_le_bytes());
        payload[offset + 40] = u8::MAX;
        payload[offset + 41] = INVALID_I8 as u8;
        payload[offset + 42] = INVALID_I8 as u8;
        payload[offset + 43] = INVALID_I8 as u8;
        payload[offset + 44] = INVALID_I8 as u8;
        payload[offset + 45] = u8::MAX;
        payload[offset + 46] = u8::MAX;
        payload[offset + 47] = INVALID_I8 as u8;
        payload[offset + 48..offset + 50].copy_from_slice(&INVALID_I16.to_le_bytes());
        for geometry in 0..6 {
            let field = offset + 52 + geometry * 2;
            payload[field..field + 2].copy_from_slice(&u16::MAX.to_le_bytes());
        }
    }

    fn empty_collision_surfaces_payload() -> Vec<u8> {
        let mut payload = vec![0; 496];
        payload[0..4].copy_from_slice(&COLLISION_SURFACE_SET_ROOM_VALID.to_le_bytes());
        payload[4] = 1;
        payload[8..10].copy_from_slice(&0x003f_u16.to_le_bytes());
        for index in 0..6 {
            empty_collision_surface(&mut payload, index);
        }
        payload
    }

    fn collision_surfaces_pending_ground_payload() -> Vec<u8> {
        let mut payload = empty_collision_surfaces_payload();
        payload[0..4].copy_from_slice(
            &(COLLISION_SURFACE_SET_ROOM_VALID | COLLISION_SURFACE_SET_NEXT_STAGE_PENDING)
                .to_le_bytes(),
        );
        payload[5] = 1;
        payload[6] = 1;
        payload[7] = 1;
        payload[10] = 1;
        let offset = 16;
        let flags = COLLISION_SURFACE_IDENTITY_PRESENT
            | COLLISION_SURFACE_BACKING_PRESENT
            | COLLISION_SURFACE_CODES_PRESENT
            | COLLISION_SURFACE_MATERIAL_PRESENT
            | COLLISION_SURFACE_SOURCE_ROOM_PRESENT
            | COLLISION_SURFACE_SOURCE_ROOM_EXACT
            | COLLISION_SURFACE_SCLS_SOURCE_PRESENT
            | COLLISION_SURFACE_DESTINATION_PRESENT
            | COLLISION_SURFACE_PENDING_MATCH
            | COLLISION_SURFACE_GEOMETRY_PRESENT
            | COLLISION_SURFACE_KCL_HEIGHT_PRESENT;
        payload[offset..offset + 4].copy_from_slice(&flags.to_le_bytes());
        payload[offset + 6] = 2;
        payload[offset + 7] = 0x1f;
        payload[offset + 8..offset + 10].copy_from_slice(&7_u16.to_le_bytes());
        payload[offset + 10..offset + 12].copy_from_slice(&2217_u16.to_le_bytes());
        payload[offset + 16..offset + 18].copy_from_slice(&19_u16.to_le_bytes());
        payload[offset + 20..offset + 24].copy_from_slice(&1_u32.to_le_bytes());
        payload[offset + 24..offset + 28].copy_from_slice(&0x1234_u32.to_le_bytes());
        payload[offset + 28..offset + 32].copy_from_slice(&0x5678_u32.to_le_bytes());
        payload[offset + 32..offset + 36].copy_from_slice(&0x9abc_u32.to_le_bytes());
        payload[offset + 36..offset + 40].copy_from_slice(&0xdef0_u32.to_le_bytes());
        payload[offset + 40] = 1;
        payload[offset + 41] = 1;
        payload[offset + 42] = 1;
        payload[offset + 43] = 1;
        payload[offset + 44] = u8::MAX;
        payload[offset + 45] = 0;
        payload[offset + 46] = 3;
        payload[offset + 47] = u8::MAX;
        payload[offset + 48..offset + 50].copy_from_slice(&0_i16.to_le_bytes());
        payload[offset + 50] = 5;
        for (geometry, value) in [2_u16, 3, 5, 7, 11].into_iter().enumerate() {
            let field = offset + 52 + geometry * 2;
            payload[field..field + 2].copy_from_slice(&value.to_le_bytes());
        }
        write_f32(&mut payload, offset + 64, 42.5);
        payload[offset + 68..offset + 75].copy_from_slice(b"F_SP104");
        payload
    }

    fn background_collision_with_ground(bg: u16, poly: u16) -> Vec<u8> {
        let mut payload = background_collision_v1_payload();
        let flags = u32_at(&payload, 0)
            | COLLISION_GROUND_PROBE_VALID
            | COLLISION_GROUND_CONTACT
            | COLLISION_GROUND_IDENTITY_PRESENT;
        payload[0..4].copy_from_slice(&flags.to_le_bytes());
        write_f32(&mut payload, 4, 0.0);
        payload[16..18].copy_from_slice(&bg.to_le_bytes());
        payload[18..20].copy_from_slice(&poly.to_le_bytes());
        payload
    }

    fn write_f32(bytes: &mut [u8], offset: usize, value: f32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_bits().to_le_bytes());
    }
}
