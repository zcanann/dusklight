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
const VERSION: u16 = 1;
const HEADER_SIZE: usize = 128;
const BLOCK_HEADER_SIZE: usize = 64;
const PAYLOAD_HEADER_SIZE: usize = 24;
const COMPLETE: u32 = 1;
const SUCCESS: u16 = 1;
const OBSERVATION_VERSION_V2: u16 = 2;
const OBSERVATION_VERSION_V3: u16 = 3;
const OBSERVATION_VERSION_V4: u16 = 4;
const ACTION_VERSION: u16 = 2;
const MAX_EPISODES: usize = 16_384;
const MAX_TICKS: usize = 4_096;
const MAX_ACTORS: usize = u16::MAX as usize;
const MAX_EXPANDED_BYTES: usize = 16 * 1024 * 1024 * 1024;

pub const NATIVE_EPISODE_SHARD_SCHEMA_V1: &str = "dusklight-native-episode-shard/v1";
pub const LEARNING_OBSERVATION_SCHEMA_V2: &str = "dusklight-learning-observation/v2";
pub const LEARNING_OBSERVATION_SCHEMA_V3: &str = "dusklight-learning-observation/v3";
pub const LEARNING_OBSERVATION_SCHEMA_V4: &str = "dusklight-learning-observation/v4";
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
    pub game_data_identity: Option<String>,
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
    pub parent_runtime_generation: u32,
    pub parameters: u32,
    pub status: u32,
    pub actor_name: i16,
    pub profile_name: i16,
    pub set_id: u16,
    pub home_room: i8,
    pub current_room: i8,
    pub group: u8,
    pub argument: i8,
    pub health: i16,
    pub position: [f32; 3],
    pub home_position: [f32; 3],
    pub velocity: [f32; 3],
    pub forward_speed: f32,
    pub current_angle: [i16; 3],
    pub shape_angle: [i16; 3],
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
    pub event_flags: Option<Vec<u8>>,
    pub temporary_flags: Option<Vec<u8>>,
    pub dungeon_flags: Option<Vec<u8>>,
    pub switch_flags: Option<Vec<u8>>,
    pub switch_flag_room: i8,
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

impl NativeEpisodeShard {
    pub fn read(path: impl AsRef<Path>) -> Result<Self, NativeEpisodeShardError> {
        let bytes =
            fs::read(path).map_err(|error| NativeEpisodeShardError::new(error.to_string()))?;
        Self::decode(&bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, NativeEpisodeShardError> {
        if bytes.len() < HEADER_SIZE || &bytes[..8] != MAGIC {
            return Err(NativeEpisodeShardError::new(
                "invalid native episode shard magic",
            ));
        }
        let mut header = Reader::new(bytes);
        header.bytes(8)?;
        if header.u16()? != VERSION || usize::from(header.u16()?) != HEADER_SIZE {
            return Err(NativeEpisodeShardError::new(
                "unsupported native episode shard version",
            ));
        }
        let flags = header.u32()?;
        let episode_count = usize::try_from(header.u32()?)
            .map_err(|_| NativeEpisodeShardError::new("episode count overflow"))?;
        if flags != COMPLETE || !(1..=MAX_EPISODES).contains(&episode_count) {
            return Err(NativeEpisodeShardError::new(
                "incomplete or oversized native episode shard",
            ));
        }
        let observation_version = header.u16()?;
        if !matches!(
            observation_version,
            OBSERVATION_VERSION_V2 | OBSERVATION_VERSION_V3 | OBSERVATION_VERSION_V4
        ) || header.u16()? != ACTION_VERSION
        {
            return Err(NativeEpisodeShardError::new(
                "unsupported observation or action schema",
            ));
        }
        let source_frame = header.u64()?;
        let maximum_ticks = header.u32()?;
        if maximum_ticks == 0 || maximum_ticks as usize > MAX_TICKS || header.u32()? != 0 {
            return Err(NativeEpisodeShardError::new("invalid shard tick bound"));
        }
        let metadata_offset = header.usize_u64()?;
        let metadata_length = header.usize_u64()?;
        let payload_offset = header.usize_u64()?;
        let payload_length = header.usize_u64()?;
        let expected_uncompressed = header.u64()?;
        let expected_compressed = header.u64()?;
        if metadata_offset != HEADER_SIZE
            || payload_offset
                != metadata_offset
                    .checked_add(metadata_length)
                    .ok_or_else(|| NativeEpisodeShardError::new("metadata range overflow"))?
            || payload_offset.checked_add(payload_length) != Some(bytes.len())
            || header
                .bytes(HEADER_SIZE - 88)?
                .iter()
                .any(|byte| *byte != 0)
            || expected_uncompressed as usize > MAX_EXPANDED_BYTES
        {
            return Err(NativeEpisodeShardError::new(
                "noncanonical native episode shard layout",
            ));
        }
        let metadata =
            decode_metadata(&bytes[metadata_offset..payload_offset], observation_version)?;
        let mut payload = Reader::new(&bytes[payload_offset..]);
        let mut episodes = Vec::with_capacity(episode_count);
        let mut uncompressed_total = 0_u64;
        let mut compressed_total = 0_u64;
        for _ in 0..episode_count {
            let (episode, expanded_size, compressed_size) = decode_episode(
                &mut payload,
                maximum_ticks,
                source_frame,
                observation_version,
            )?;
            uncompressed_total = uncompressed_total
                .checked_add(expanded_size)
                .ok_or_else(|| NativeEpisodeShardError::new("uncompressed byte total overflow"))?;
            compressed_total = compressed_total
                .checked_add(compressed_size)
                .ok_or_else(|| NativeEpisodeShardError::new("compressed byte total overflow"))?;
            episodes.push(episode);
        }
        if !payload.done()
            || uncompressed_total != expected_uncompressed
            || compressed_total != expected_compressed
        {
            return Err(NativeEpisodeShardError::new(
                "native episode shard byte totals disagree",
            ));
        }
        Ok(Self {
            content_sha256: Digest(Sha256::digest(bytes).into()),
            source_frame,
            maximum_ticks,
            metadata,
            episodes,
            uncompressed_bytes: uncompressed_total,
            compressed_bytes: compressed_total,
        })
    }
}

fn decode_metadata(
    bytes: &[u8],
    observation_version: u16,
) -> Result<NativeEpisodeShardMetadata, NativeEpisodeShardError> {
    let mut reader = Reader::new(bytes);
    if reader.u16()? != 12 {
        return Err(NativeEpisodeShardError::new(
            "unsupported shard metadata field count",
        ));
    }
    let mut fields = Vec::with_capacity(12);
    for _ in 0..12 {
        fields.push(reader.string16()?);
    }
    let expected_observation_schema = match observation_version {
        OBSERVATION_VERSION_V2 => LEARNING_OBSERVATION_SCHEMA_V2,
        OBSERVATION_VERSION_V3 => LEARNING_OBSERVATION_SCHEMA_V3,
        OBSERVATION_VERSION_V4 => LEARNING_OBSERVATION_SCHEMA_V4,
        _ => {
            return Err(NativeEpisodeShardError::new(
                "unsupported observation schema version",
            ));
        }
    };
    if !reader.done()
        || fields[0] != NATIVE_EPISODE_SHARD_SCHEMA_V1
        || fields[1] != expected_observation_schema
        || fields[2] != RAW_PAD_ACTION_SCHEMA_V2
        || fields[3].len() != 32
        || fields[4].len() != 32
        || fields[5].is_empty()
        || fields[6].len() != 32
        || fields[7].is_empty()
        || fields[9].is_empty()
        || fields[10].is_empty()
    {
        return Err(NativeEpisodeShardError::new(
            "invalid shard identity metadata",
        ));
    }
    Ok(NativeEpisodeShardMetadata {
        shard_schema: fields.remove(0),
        observation_schema: fields.remove(0),
        action_schema: fields.remove(0),
        source_boundary_fingerprint: fields.remove(0),
        checkpoint_identity: fields.remove(0),
        objective: fields.remove(0),
        objective_identity: fields.remove(0),
        build_revision: fields.remove(0),
        aurora_revision: fields.remove(0),
        feature_digest: fields.remove(0),
        fidelity_profile: fields.remove(0),
        game_data_identity: (!fields[0].is_empty()).then(|| fields.remove(0)),
    })
}

fn decode_episode(
    reader: &mut Reader<'_>,
    maximum_ticks: u32,
    source_frame: u64,
    observation_version: u16,
) -> Result<(NativeEpisode, u64, u64), NativeEpisodeShardError> {
    if reader.bytes(4)? != EPISODE_MAGIC || usize::from(reader.u16()?) != BLOCK_HEADER_SIZE {
        return Err(NativeEpisodeShardError::new("invalid episode block header"));
    }
    let flags = reader.u16()?;
    if flags & !SUCCESS != 0 {
        return Err(NativeEpisodeShardError::new("unknown episode block flags"));
    }
    let ticks_executed = reader.u32()?;
    let first_hit = reader.u32()?;
    let remaining_ticks = reader.u32()?;
    let id_length = usize::from(reader.u16()?);
    if reader.u16()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero episode block reserved field",
        ));
    }
    let expanded_size = reader.usize_u64()?;
    let compressed_size = reader.usize_u64()?;
    let payload_xxh3_128: [u8; 16] = reader.bytes(16)?.try_into().expect("exact length");
    if reader.u64()? != 0
        || ticks_executed == 0
        || ticks_executed > maximum_ticks
        || remaining_ticks != maximum_ticks - ticks_executed
        || id_length == 0
        || expanded_size > MAX_EXPANDED_BYTES
    {
        return Err(NativeEpisodeShardError::new(
            "invalid episode block descriptor",
        ));
    }
    let id = std::str::from_utf8(reader.bytes(id_length)?)
        .map_err(|_| NativeEpisodeShardError::new("episode id is not UTF-8"))?
        .to_owned();
    let compressed = reader.bytes(compressed_size)?;
    let expanded = zstd::bulk::decompress(compressed, expanded_size)
        .map_err(|error| NativeEpisodeShardError::new(error.to_string()))?;
    if xxhash_rust::xxh3::xxh3_128(&expanded).to_be_bytes() != payload_xxh3_128 {
        return Err(NativeEpisodeShardError::new(
            "episode payload digest mismatch",
        ));
    }
    let mut payload = Reader::new(&expanded);
    if payload.bytes(8)? != PAYLOAD_MAGIC
        || payload.u16()? != observation_version
        || usize::from(payload.u16()?) != PAYLOAD_HEADER_SIZE
        || payload.u32()? != ticks_executed
        || payload.u32()? != 0
        || payload.u32()? != 0
    {
        return Err(NativeEpisodeShardError::new(
            "invalid expanded episode header",
        ));
    }
    let mut steps = Vec::with_capacity(ticks_executed as usize);
    let success = flags & SUCCESS != 0;
    for step_index in 0..ticks_executed {
        let pre_input = decode_observation(&mut payload, observation_version)?;
        let chosen_pad = decode_pad(&mut payload)?;
        let consumed_pad = decode_pad(&mut payload)?;
        let post_simulation = decode_observation(&mut payload, observation_version)?;
        validate_step(
            steps.last(),
            &pre_input,
            consumed_pad,
            &post_simulation,
            step_index + 1 == ticks_executed,
            success,
        )?;
        steps.push(NativeEpisodeStep {
            pre_input,
            chosen_pad,
            consumed_pad,
            post_simulation,
        });
    }
    if !payload.done() {
        return Err(NativeEpisodeShardError::new(
            "trailing expanded episode bytes",
        ));
    }
    let first_hit_tick = (first_hit != u32::MAX).then_some(first_hit);
    if success != first_hit_tick.is_some()
        || first_hit_tick.is_some_and(|tick| tick + 1 != ticks_executed)
        || steps.first().is_none_or(|step| {
            step.pre_input.remaining_ticks != maximum_ticks
                || step.pre_input.tape_frame != source_frame
        })
        || steps
            .last()
            .is_none_or(|step| step.post_simulation.remaining_ticks != remaining_ticks)
        || steps
            .last()
            .is_none_or(|step| step.post_simulation.goal.reached != success)
    {
        return Err(NativeEpisodeShardError::new(
            "episode outcome disagrees with terminal boundary",
        ));
    }
    Ok((
        NativeEpisode {
            id,
            success,
            ticks_executed,
            first_hit_tick,
            remaining_ticks,
            payload_xxh3_128,
            steps,
        },
        expanded_size as u64,
        compressed_size as u64,
    ))
}

fn validate_step(
    prior: Option<&NativeEpisodeStep>,
    pre: &NativeLearningObservation,
    action: NativeRawPad,
    post: &NativeLearningObservation,
    final_step: bool,
    success: bool,
) -> Result<(), NativeEpisodeShardError> {
    if pre.phase != NativeObservationPhase::PreInput
        || post.phase != NativeObservationPhase::PostSimulation
        || pre.simulation_tick != post.simulation_tick
        || pre.tape_frame != post.tape_frame
        || post.boundary_index != pre.boundary_index + 1
        || post.remaining_ticks + 1 != pre.remaining_ticks
        || post.previous_input != action
        || pre.terminal_reason != NativeTerminalReason::None
        || (!final_step && post.terminal_reason != NativeTerminalReason::None)
        || (final_step
            && post.terminal_reason
                != if success {
                    NativeTerminalReason::GoalReached
                } else {
                    NativeTerminalReason::TickBudgetExhausted
                })
    {
        return Err(NativeEpisodeShardError::new(
            "action is not aligned to its observation boundaries",
        ));
    }
    if let Some(prior) = prior
        && (prior.post_simulation.state_identity != pre.state_identity
            || prior.post_simulation.boundary_index != pre.boundary_index
            || prior.post_simulation.remaining_ticks != pre.remaining_ticks
            || prior.post_simulation.simulation_tick + 1 != pre.simulation_tick
            || prior.post_simulation.tape_frame + 1 != pre.tape_frame
            || prior.consumed_pad != pre.previous_input)
    {
        return Err(NativeEpisodeShardError::new(
            "adjacent transition boundaries are discontinuous",
        ));
    }
    Ok(())
}

fn decode_channel_status(
    reader: &mut Reader<'_>,
) -> Result<NativeChannelStatus, NativeEpisodeShardError> {
    match reader.u8()? {
        0 => Ok(NativeChannelStatus::NotSampled),
        1 => Ok(NativeChannelStatus::Present),
        2 => Ok(NativeChannelStatus::Absent),
        3 => Ok(NativeChannelStatus::Unavailable),
        _ => Err(NativeEpisodeShardError::new(
            "invalid collision channel status",
        )),
    }
}

fn decode_camera(
    reader: &mut Reader<'_>,
) -> Result<NativeCameraObservation, NativeEpisodeShardError> {
    let view_yaw = reader.i16()?;
    let controlled_yaw = reader.i16()?;
    let bank = reader.i16()?;
    if reader.u16()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero camera reserved field",
        ));
    }
    Ok(NativeCameraObservation {
        view_yaw,
        controlled_yaw,
        bank,
        eye: reader.f32x3()?,
        center: reader.f32x3()?,
        up: reader.f32x3()?,
        fovy: reader.f32()?,
    })
}

fn decode_animation_lane(
    reader: &mut Reader<'_>,
) -> Result<NativeAnimationLane, NativeEpisodeShardError> {
    let resource_id = reader.u16()?;
    if reader.u16()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero animation-lane reserved field",
        ));
    }
    Ok(NativeAnimationLane {
        resource_id,
        frame: reader.f32()?,
        rate: reader.f32()?,
    })
}

fn decode_trace_actor_identity(
    reader: &mut Reader<'_>,
) -> Result<NativeTraceActorIdentity, NativeEpisodeShardError> {
    let runtime_generation = reader.u32()?;
    let actor_name = reader.i16()?;
    let set_id = reader.u16()?;
    let home_room = reader.i8()?;
    let current_room = reader.i8()?;
    if reader.u16()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero trace actor-identity reserved field",
        ));
    }
    Ok(NativeTraceActorIdentity {
        runtime_generation,
        actor_name,
        set_id,
        home_room,
        current_room,
        home_position: reader.f32x3()?,
    })
}

fn trace_actor_identity_is_absent(actor: &NativeTraceActorIdentity) -> bool {
    actor.runtime_generation == u32::MAX
        && actor.actor_name == -1
        && actor.set_id == u16::MAX
        && actor.home_room == -1
        && actor.current_room == -1
        && actor.home_position == [0.0; 3]
}

fn decode_player_action(
    reader: &mut Reader<'_>,
) -> Result<NativePlayerActionObservation, NativeEpisodeShardError> {
    let procedure_id = reader.u16()?;
    if reader.u16()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero player-action reserved field",
        ));
    }
    let mode_flags = reader.u32()?;
    let procedure_context_raw = [
        reader.i16()?,
        reader.i16()?,
        reader.i16()?,
        reader.i16()?,
        reader.i16()?,
        reader.i16()?,
    ];
    let damage_wait_timer = reader.i16()?;
    let sword_at_up_time = reader.u16()?;
    let ice_damage_wait_timer = reader.i16()?;
    let sword_change_wait_timer = reader.u8()?;
    if reader.bytes(5)?.iter().any(|byte| *byte != 0) {
        return Err(NativeEpisodeShardError::new(
            "nonzero player-action padding",
        ));
    }
    let mut under_animations = Vec::with_capacity(3);
    let mut upper_animations = Vec::with_capacity(3);
    for _ in 0..3 {
        under_animations.push(decode_animation_lane(reader)?);
    }
    for _ in 0..3 {
        upper_animations.push(decode_animation_lane(reader)?);
    }
    let flags = reader.u32()?;
    let do_status = reader.u8()?;
    if flags & !0x3 != 0 || reader.u8()? != 0 || reader.u16()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "invalid player-action flags or reserved fields",
        ));
    }
    let talk_partner = decode_trace_actor_identity(reader)?;
    let grabbed_actor = decode_trace_actor_identity(reader)?;
    if (flags & 1 != 0) == trace_actor_identity_is_absent(&talk_partner)
        || (flags & 2 != 0) == trace_actor_identity_is_absent(&grabbed_actor)
    {
        return Err(NativeEpisodeShardError::new(
            "noncanonical player-action actor identity",
        ));
    }
    Ok(NativePlayerActionObservation {
        procedure_id,
        mode_flags,
        procedure_context_raw,
        damage_wait_timer,
        sword_at_up_time,
        ice_damage_wait_timer,
        sword_change_wait_timer,
        under_animations: under_animations.try_into().expect("three animation lanes"),
        upper_animations: upper_animations.try_into().expect("three animation lanes"),
        flags,
        do_status,
        talk_partner,
        grabbed_actor,
    })
}

fn decode_scene_exit(
    reader: &mut Reader<'_>,
    _status: NativeChannelStatus,
) -> Result<NativeSceneExitObservation, NativeEpisodeShardError> {
    let exit = NativeSceneExitObservation {
        runtime_generation: reader.u32()?,
        raw_parameters: reader.u32()?,
        flags: reader.u32()?,
        signed_distance_to_volume: reader.f32()?,
        actor_name: reader.i16()?,
        set_id: reader.u16()?,
        exit_id: reader.u8()?,
        path_id: reader.u8()?,
        argument1: reader.u8()?,
        switch_no: reader.u8()?,
        kind: reader.u8()?,
        observed_count: reader.u8()?,
        home_room: reader.i8()?,
        link_exit_direction: reader.u8()?,
        link_exit_id: reader.u16()?,
        shape_yaw: reader.i16()?,
        actor_action: reader.u8()?,
        player_local_position: {
            if reader.u8()? != 0 || reader.u16()? != 0 {
                return Err(NativeEpisodeShardError::new(
                    "nonzero scene-exit reserved fields",
                ));
            }
            reader.f32x3()?
        },
        volume_extent: reader.f32x3()?,
        home_position: reader.f32x3()?,
        destination_stage: reader.fixed_name()?,
        destination_room: reader.i8()?,
        destination_layer: reader.i8()?,
        destination_point: reader.i16()?,
        destination_wipe: reader.u8()?,
        destination_wipe_time: reader.u8()?,
        destination_time_hour: reader.i8()?,
    };
    if reader.u8()? != 0 || exit.flags & !0x7f != 0 {
        return Err(NativeEpisodeShardError::new(
            "invalid scene-exit flags or reserved fields",
        ));
    }
    Ok(exit)
}

fn decode_background_collision(
    reader: &mut Reader<'_>,
) -> Result<NativePlayerBackgroundCollision, NativeEpisodeShardError> {
    let flags = reader.u32()?;
    if flags & !0x0007_ffff != 0 {
        return Err(NativeEpisodeShardError::new(
            "unknown background collision flags",
        ));
    }
    let ground_height = reader.f32()?;
    let roof_height = reader.f32()?;
    let water_height = reader.f32()?;
    let ground_identity = [
        u32::from(reader.u16()?),
        u32::from(reader.u16()?),
        reader.u32()?,
    ];
    let ground_plane = reader.f32x4()?;
    let roof_identity = [
        u32::from(reader.u16()?),
        u32::from(reader.u16()?),
        reader.u32()?,
    ];
    let water_identity = [
        u32::from(reader.u16()?),
        u32::from(reader.u16()?),
        reader.u32()?,
    ];
    let mut walls = Vec::with_capacity(3);
    for _ in 0..3 {
        let wall = NativeCollisionWallObservation {
            bg_index: reader.u16()?,
            poly_index: reader.u16()?,
            owner_runtime_generation: reader.u32()?,
            angle_y: reader.i16()?,
            flags: reader.u16()?,
        };
        if wall.flags & !0x0007 != 0 {
            return Err(NativeEpisodeShardError::new("unknown collision wall flags"));
        }
        walls.push(wall);
    }
    let collision = NativePlayerBackgroundCollision {
        flags,
        ground_height,
        roof_height,
        water_height,
        ground_identity,
        ground_plane,
        roof_identity,
        water_identity,
        walls: walls.try_into().expect("three collision walls"),
        old_position: reader.f32x3()?,
        resolved_frame_displacement: reader.f32x3()?,
        final_position: reader.f32x3()?,
    };
    validate_background_collision(&collision)?;
    Ok(collision)
}

fn identity_is_coherent(identity: [u32; 3], identity_present: bool, owner_present: bool) -> bool {
    let bg_present = identity[0] != u32::from(u16::MAX);
    let polygon_present = identity[1] != u32::from(u16::MAX);
    let actual_owner_present = identity[2] != u32::MAX;
    bg_present == polygon_present
        && bg_present == identity_present
        && actual_owner_present == owner_present
        && (!actual_owner_present || identity_present)
}

fn validate_background_collision(
    collision: &NativePlayerBackgroundCollision,
) -> Result<(), NativeEpisodeShardError> {
    let has = |flag| collision.flags & flag != 0;
    let ground_valid = has(1 << 0);
    let ground_identity = has(1 << 16);
    let ground_owner = has(1 << 5);
    let roof_valid = has(1 << 7);
    let roof_identity = has(1 << 17);
    let roof_owner = has(1 << 9);
    let water_enabled = has(1 << 10);
    let water_found = has(1 << 11);
    let water_identity = has(1 << 18);
    let water_owner = has(1 << 13);
    if ground_valid
        != (collision.ground_height != -1_000_000_000.0 && collision.ground_height.is_finite())
        || (!ground_valid && (has(1 << 1) || has(1 << 2)))
        || !identity_is_coherent(collision.ground_identity, ground_identity, ground_owner)
        || (!ground_valid && collision.ground_identity[0] != u32::from(u16::MAX))
        || (has(1 << 4) && (!ground_valid || !has(1 << 1)))
        || (has(1 << 4) != (collision.ground_plane != [0.0; 4]))
        || roof_valid
            != (collision.roof_height != 1_000_000_000.0 && collision.roof_height.is_finite())
        || (has(1 << 8) && !roof_valid)
        || !identity_is_coherent(collision.roof_identity, roof_identity, roof_owner)
        || (!roof_valid && collision.roof_identity[0] != u32::from(u16::MAX))
        || water_found
            != (collision.water_height != -1_000_000_000.0 && collision.water_height.is_finite())
        || (water_found && !water_enabled)
        || (has(1 << 12) && !water_found)
        || !identity_is_coherent(collision.water_identity, water_identity, water_owner)
        || (!water_found && collision.water_identity[0] != u32::from(u16::MAX))
    {
        return Err(NativeEpisodeShardError::new(
            "inconsistent background collision payload",
        ));
    }

    let mut any_wall_hit = false;
    for wall in &collision.walls {
        let hit = wall.flags & 1 != 0;
        let owner = wall.flags & (1 << 1) != 0;
        let identity = wall.flags & (1 << 2) != 0;
        any_wall_hit |= hit;
        if !identity_is_coherent(
            [
                u32::from(wall.bg_index),
                u32::from(wall.poly_index),
                wall.owner_runtime_generation,
            ],
            identity,
            owner,
        ) || (!hit
            && (wall.bg_index != u16::MAX
                || wall.poly_index != u16::MAX
                || wall.owner_runtime_generation != u32::MAX
                || wall.angle_y != 0
                || wall.flags != 0))
        {
            return Err(NativeEpisodeShardError::new(
                "inconsistent background collision wall",
            ));
        }
    }
    if any_wall_hit != has(1 << 6) || (any_wall_hit && !has(1 << 14)) {
        return Err(NativeEpisodeShardError::new(
            "inconsistent background collision wall aggregate",
        ));
    }

    let trajectory_valid = has(1 << 15);
    if !trajectory_valid
        && (collision.old_position != [0.0; 3]
            || collision.resolved_frame_displacement != [0.0; 3]
            || collision.final_position != [0.0; 3])
    {
        return Err(NativeEpisodeShardError::new(
            "inconsistent background collision trajectory",
        ));
    }
    if trajectory_valid {
        for axis in 0..3 {
            let reconstructed =
                collision.old_position[axis] + collision.resolved_frame_displacement[axis];
            let tolerance = 0.0001 * collision.final_position[axis].abs().max(1.0);
            if (reconstructed - collision.final_position[axis]).abs() > tolerance {
                return Err(NativeEpisodeShardError::new(
                    "incoherent background collision trajectory",
                ));
            }
        }
    }
    Ok(())
}

fn decode_collision_surfaces(
    reader: &mut Reader<'_>,
    plane_mask: u8,
    status: NativeChannelStatus,
    next_stage: Option<&str>,
    next_room: i8,
    next_layer: i8,
    next_point: i16,
) -> Result<NativePlayerCollisionSurfaces, NativeEpisodeShardError> {
    let flags = reader.u32()?;
    let current_room = reader.i8()?;
    let identity_count = reader.u8()?;
    let backing_code_count = reader.u8()?;
    let destination_count = reader.u8()?;
    let raw_link_exit = reader.u16()?;
    let pending_stage_match_mask = reader.u8()?;
    let room_valid = flags & 1 != 0;
    let explicit_exit = flags & (1 << 1) != 0;
    let pending = flags & (1 << 2) != 0;
    if flags & !0x0007 != 0
        || pending_stage_match_mask & !0x3f != 0
        || reader.u8()? != 0
        || plane_mask & !0x3f != 0
        || (room_valid && !(-1..64).contains(&current_room))
        || (!room_valid && current_room != i8::MIN)
        || explicit_exit != (raw_link_exit != 0x003f)
        || (status == NativeChannelStatus::Present && pending != next_stage.is_some())
    {
        return Err(NativeEpisodeShardError::new(
            "invalid collision surface-set header",
        ));
    }
    let expected_kinds = [(1_u8, 0_u8), (2, 0), (3, 0), (4, 0), (4, 1), (4, 2)];
    let mut surfaces = Vec::with_capacity(6);
    for (index, (expected_kind, expected_wall_slot)) in expected_kinds.into_iter().enumerate() {
        let surface_flags = reader.u32()?;
        let kind = reader.u8()?;
        let wall_slot = reader.u8()?;
        let backing_format = reader.u8()?;
        let raw_code_presence_mask = reader.u8()?;
        if surface_flags & !0x0000_1fff != 0
            || kind != expected_kind
            || wall_slot != expected_wall_slot
            || backing_format > 2
            || raw_code_presence_mask & !0x1f != 0
        {
            return Err(NativeEpisodeShardError::new(
                "invalid collision surface identity",
            ));
        }
        let bg_index = reader.u16()?;
        let poly_index = reader.u16()?;
        let owner_runtime_generation = reader.u32()?;
        let material_index = reader.u16()?;
        let group_index = reader.u16()?;
        let raw_codes = [
            reader.u32()?,
            reader.u32()?,
            reader.u32()?,
            reader.u32()?,
            reader.u32()?,
        ];
        let raw_exit_id = reader.u8()?;
        let source_room = reader.i8()?;
        let scls_source_room = reader.i8()?;
        let destination_room = reader.i8()?;
        let destination_layer = reader.i8()?;
        let destination_wipe = reader.u8()?;
        let destination_wipe_time = reader.u8()?;
        let destination_time_hour = reader.i8()?;
        let destination_point = reader.i16()?;
        let geometry_count = usize::from(reader.u8()?);
        if geometry_count > 6 || reader.u8()? != 0 {
            return Err(NativeEpisodeShardError::new(
                "invalid collision surface geometry count",
            ));
        }
        let mut geometry_indices = [0_u16; 6];
        for geometry_index in &mut geometry_indices {
            *geometry_index = reader.u16()?;
        }
        let kcl_prism_height = reader.f32()?;
        let destination_stage = reader.fixed_name()?;
        let plane_values = reader.f32x4()?;
        let plane_present = plane_mask & (1 << index) != 0;
        let identity = surface_flags & 1 != 0;
        let owner = surface_flags & (1 << 1) != 0;
        let backing = surface_flags & (1 << 2) != 0;
        let codes = surface_flags & (1 << 3) != 0;
        let material = surface_flags & (1 << 4) != 0;
        let group = surface_flags & (1 << 5) != 0;
        let source_room_present = surface_flags & (1 << 6) != 0;
        let source_room_exact = surface_flags & (1 << 7) != 0;
        let scls_source = surface_flags & (1 << 8) != 0;
        let destination = surface_flags & (1 << 9) != 0;
        let destination_match = surface_flags & (1 << 10) != 0;
        let geometry = surface_flags & (1 << 11) != 0;
        let kcl_height = surface_flags & (1 << 12) != 0;
        let identity_tuple = [
            u32::from(bg_index),
            u32::from(poly_index),
            owner_runtime_generation,
        ];
        let destination_name_valid = !destination_stage.is_empty()
            && destination_stage
                .as_bytes()
                .iter()
                .all(|byte| (0x20..=0x7e).contains(byte));
        let destination_fields_valid = destination_name_valid
            && (-1..64).contains(&destination_room)
            && (-1..15).contains(&destination_layer)
            && destination_point >= 0
            && destination_wipe_time <= 7
            && (-1..31).contains(&destination_time_hour);
        let destination_fields_absent = destination_stage.is_empty()
            && destination_room == i8::MIN
            && destination_layer == i8::MIN
            && destination_point == i16::MIN
            && destination_wipe == u8::MAX
            && destination_wipe_time == u8::MAX
            && destination_time_hour == i8::MIN;
        let tuple_matches_pending = destination
            && pending
            && next_stage == Some(destination_stage.as_str())
            && destination_room == next_room
            && destination_layer == next_layer
            && destination_point == next_point;
        if !identity_is_coherent(identity_tuple, identity, owner)
            || (owner && !identity)
            || backing != (backing_format != 0)
            || (backing && !identity)
            || codes != (raw_code_presence_mask != 0)
            || (codes && (!backing || raw_code_presence_mask & 1 == 0))
            || material != (material_index != u16::MAX)
            || (material && !backing)
            || group != (group_index != u16::MAX)
            || (group && (!backing || backing_format != 1))
            || (source_room_present && (!identity || !(-1..64).contains(&source_room)))
            || (!source_room_present && source_room != i8::MIN)
            || (source_room_exact && !source_room_present)
            || (scls_source
                && (index != 0
                    || !identity
                    || !room_valid
                    || scls_source_room != current_room
                    || !(-1..64).contains(&scls_source_room)))
            || (!scls_source && scls_source_room != i8::MIN)
            || (destination
                && (index != 0 || !scls_source || !codes || matches!(raw_exit_id, 0x3f | 0xff)))
            || (destination_match && (!destination || !pending))
            || geometry != (geometry_count != 0)
            || (geometry && !backing)
            || (kcl_height && (!backing || backing_format != 2))
            || (!kcl_height && kcl_prism_height != 0.0)
            || raw_codes
                .iter()
                .enumerate()
                .any(|(word, code)| raw_code_presence_mask & (1 << word) == 0 && *code != 0)
            || geometry_indices
                .iter()
                .enumerate()
                .any(|(slot, value)| (geometry && slot < geometry_count) == (*value == u16::MAX))
            || (destination && !destination_fields_valid)
            || (!destination && !destination_fields_absent)
            || destination_match != tuple_matches_pending
        {
            return Err(NativeEpisodeShardError::new(
                "inconsistent collision surface payload",
            ));
        }
        if (plane_present && !identity) || (!plane_present && plane_values != [0.0; 4]) {
            return Err(NativeEpisodeShardError::new(
                "collision plane does not match realized surface identity",
            ));
        }
        surfaces.push(NativeCollisionSurfaceObservation {
            flags: surface_flags,
            kind,
            wall_slot,
            backing_format,
            raw_code_presence_mask,
            bg_index,
            poly_index,
            owner_runtime_generation,
            material_index,
            group_index,
            raw_codes,
            raw_exit_id,
            source_room,
            scls_source_room,
            destination_room,
            destination_layer,
            destination_wipe,
            destination_wipe_time,
            destination_time_hour,
            destination_point,
            source_geometry_indices: geometry_indices[..geometry_count].to_vec(),
            kcl_prism_height,
            destination_stage,
            plane: plane_present.then_some(plane_values),
        });
    }
    let observed_identity_count = surfaces
        .iter()
        .filter(|surface| surface.flags & 1 != 0)
        .count();
    let observed_backing_count = surfaces
        .iter()
        .filter(|surface| surface.flags & (1 << 2) != 0)
        .count();
    let observed_destination_count = surfaces
        .iter()
        .filter(|surface| surface.flags & (1 << 9) != 0)
        .count();
    let observed_match_mask = surfaces
        .iter()
        .enumerate()
        .fold(0_u8, |mask, (index, surface)| {
            mask | (((surface.flags & (1 << 10) != 0) as u8) << index)
        });
    if usize::from(identity_count) != observed_identity_count
        || usize::from(backing_code_count) != observed_backing_count
        || usize::from(destination_count) != observed_destination_count
        || pending_stage_match_mask != observed_match_mask
    {
        return Err(NativeEpisodeShardError::new(
            "collision surface counts disagree with entries",
        ));
    }
    Ok(NativePlayerCollisionSurfaces {
        flags,
        current_room,
        identity_count,
        backing_code_count,
        destination_count,
        raw_link_exit,
        pending_stage_match_mask,
        surfaces,
    })
}

fn collision_channels_agree(
    background: &NativePlayerBackgroundCollision,
    surfaces: &NativePlayerCollisionSurfaces,
) -> bool {
    let agrees = |surface: &NativeCollisionSurfaceObservation,
                  identity: [u32; 3],
                  identity_present: bool,
                  owner_present: bool| {
        (surface.flags & 1 != 0) == identity_present
            && (surface.flags & (1 << 1) != 0) == owner_present
            && (!identity_present
                || (u32::from(surface.bg_index) == identity[0]
                    && u32::from(surface.poly_index) == identity[1]))
            && (!owner_present || surface.owner_runtime_generation == identity[2])
    };
    agrees(
        &surfaces.surfaces[0],
        background.ground_identity,
        background.flags & (1 << 16) != 0,
        background.flags & (1 << 5) != 0,
    ) && agrees(
        &surfaces.surfaces[1],
        background.roof_identity,
        background.flags & (1 << 17) != 0,
        background.flags & (1 << 9) != 0,
    ) && agrees(
        &surfaces.surfaces[2],
        background.water_identity,
        background.flags & (1 << 18) != 0,
        background.flags & (1 << 13) != 0,
    ) && background.walls.iter().enumerate().all(|(index, wall)| {
        agrees(
            &surfaces.surfaces[index + 3],
            [
                u32::from(wall.bg_index),
                u32::from(wall.poly_index),
                wall.owner_runtime_generation,
            ],
            wall.flags & (1 << 2) != 0,
            wall.flags & (1 << 1) != 0,
        )
    })
}

fn decode_observation(
    reader: &mut Reader<'_>,
    observation_version: u16,
) -> Result<NativeLearningObservation, NativeEpisodeShardError> {
    let phase = match reader.u8()? {
        1 => NativeObservationPhase::PreInput,
        2 => NativeObservationPhase::PostSimulation,
        _ => return Err(NativeEpisodeShardError::new("invalid observation phase")),
    };
    let actor_selection = match reader.u8()? {
        0 => NativeActorSelectionRule::Complete,
        1 => NativeActorSelectionRule::LowestRuntimeGeneration,
        _ => return Err(NativeEpisodeShardError::new("invalid actor selection rule")),
    };
    let terminal_reason = match reader.u8()? {
        0 => NativeTerminalReason::None,
        1 => NativeTerminalReason::GoalReached,
        2 => NativeTerminalReason::TickBudgetExhausted,
        _ => return Err(NativeEpisodeShardError::new("invalid terminal reason")),
    };
    if reader.u8()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero observation reserved byte",
        ));
    }
    let actor_count = usize::from(reader.u16()?);
    let flags = reader.u32()?;
    let actor_observed_count = reader.u32()?;
    let remaining_ticks = reader.u32()?;
    let boundary_index = reader.u64()?;
    let simulation_tick = reader.u64()?;
    let tape_frame = reader.u64()?;
    let state_identity = reader.bytes(16)?.try_into().expect("exact length");
    if flags & !0x0fff != 0
        || actor_count > MAX_ACTORS
        || actor_observed_count < actor_count as u32
        || ((flags & (1 << 5) != 0) != (actor_observed_count > actor_count as u32))
        || (actor_selection == NativeActorSelectionRule::Complete) != (flags & (1 << 5) == 0)
    {
        return Err(NativeEpisodeShardError::new(
            "inconsistent observation header",
        ));
    }
    if observation_version >= OBSERVATION_VERSION_V4
        && (actor_selection != NativeActorSelectionRule::Complete
            || flags & (1 << 5) != 0
            || actor_observed_count != actor_count as u32)
    {
        return Err(NativeEpisodeShardError::new(
            "v4 observation does not contain the complete actor set",
        ));
    }
    let stage = reader.fixed_name()?;
    let room = reader.i8()?;
    let layer = reader.i8()?;
    let point = reader.i16()?;
    let next_stage_raw = reader.fixed_name()?;
    let next_room = reader.i8()?;
    let next_layer = reader.i8()?;
    let next_point = reader.i16()?;
    let player_process_id = reader.u32()?;
    let player_actor_name = reader.i16()?;
    let player_procedure = reader.u16()?;
    let player_position = reader.f32x3()?;
    let player_velocity = reader.f32x3()?;
    let player_forward_speed = reader.f32()?;
    let player_current_angle = reader.i16x3()?;
    let player_shape_angle = reader.i16x3()?;
    let player_mode_flags = reader.u32()?;
    let player_damage_wait_timer = reader.i16()?;
    let player_ice_damage_wait_timer = reader.i16()?;
    let player_sword_change_wait_timer = reader.u8()?;
    let player_do_status = reader.u8()?;
    let player_contacts = reader.u8()?;
    if player_contacts & !0x1f != 0 || reader.u8()? != 0 {
        return Err(NativeEpisodeShardError::new("invalid player contact bits"));
    }
    let ground_height = reader.f32()?;
    let roof_height = reader.f32()?;
    let event_running = reader.bool()?;
    let event_id = reader.i16()?;
    let event_mode = reader.u8()?;
    let event_status = reader.u8()?;
    let event_map_tool_id = reader.u8()?;
    let event_name_hash_raw = reader.u32()?;
    let menu_flags = reader.u16()?;
    if menu_flags & !0x0fff != 0 {
        return Err(NativeEpisodeShardError::new("invalid menu flags"));
    }
    let menu_procedures = [
        reader.u8()?,
        reader.u8()?,
        reader.u8()?,
        reader.u8()?,
        reader.u8()?,
    ];
    if reader.u8()? != 0 {
        return Err(NativeEpisodeShardError::new("nonzero menu reserved byte"));
    }
    let camera = reader.f32()?;
    let correction = [reader.f32()?, reader.f32()?];
    let (
        camera_status,
        mechanics_camera,
        player_action_status,
        player_action,
        player_background_collision_status,
        player_background_collision,
        player_collision_surfaces_status,
        player_collision_surfaces,
        scene_exit_status,
        scene_exit,
        player_form_present,
        player_is_wolf,
    ) = match observation_version {
        OBSERVATION_VERSION_V2 => (
            NativeChannelStatus::NotSampled,
            None,
            NativeChannelStatus::NotSampled,
            None,
            NativeChannelStatus::NotSampled,
            None,
            NativeChannelStatus::NotSampled,
            None,
            NativeChannelStatus::NotSampled,
            None,
            false,
            false,
        ),
        OBSERVATION_VERSION_V3 | OBSERVATION_VERSION_V4 => {
            let camera_status = decode_channel_status(reader)?;
            let action_status = decode_channel_status(reader)?;
            let background_status = decode_channel_status(reader)?;
            let surfaces_status = decode_channel_status(reader)?;
            let scene_exit_status = decode_channel_status(reader)?;
            let collision_plane_mask = reader.u8()?;
            let form_flags = reader.u8()?;
            if collision_plane_mask & !0x3f != 0
                || form_flags & !0x3 != 0
                || form_flags & 2 != 0 && form_flags & 1 == 0
                || reader.u8()? != 0
            {
                return Err(NativeEpisodeShardError::new(
                    "invalid mechanics observation header",
                ));
            }
            let mechanics_camera = decode_camera(reader)?;
            let player_action = decode_player_action(reader)?;
            let scene_exit = decode_scene_exit(reader, scene_exit_status)?;
            let background = decode_background_collision(reader)?;
            let surfaces = decode_collision_surfaces(
                reader,
                collision_plane_mask,
                surfaces_status,
                (flags & (1 << 2) != 0).then_some(next_stage_raw.as_str()),
                next_room,
                next_layer,
                next_point,
            )?;
            if surfaces_status != NativeChannelStatus::Present && collision_plane_mask != 0 {
                return Err(NativeEpisodeShardError::new(
                    "collision planes are present without a surface channel",
                ));
            }
            if background_status == NativeChannelStatus::Present
                && surfaces_status == NativeChannelStatus::Present
                && !collision_channels_agree(&background, &surfaces)
            {
                return Err(NativeEpisodeShardError::new(
                    "collision channels disagree on surface identities",
                ));
            }
            (
                camera_status,
                (camera_status == NativeChannelStatus::Present).then_some(mechanics_camera),
                action_status,
                (action_status == NativeChannelStatus::Present).then_some(player_action),
                background_status,
                (background_status == NativeChannelStatus::Present).then_some(background),
                surfaces_status,
                (surfaces_status == NativeChannelStatus::Present).then_some(surfaces),
                scene_exit_status,
                (scene_exit_status == NativeChannelStatus::Present).then_some(scene_exit),
                form_flags & 1 != 0,
                form_flags & 2 != 0,
            )
        }
        _ => {
            return Err(NativeEpisodeShardError::new(
                "unsupported observation schema version",
            ));
        }
    };
    let previous_input = decode_pad(reader)?;
    let rng_version = reader.u32()?;
    let rng_count = reader.u32()?;
    if rng_count != 2 {
        return Err(NativeEpisodeShardError::new("unsupported RNG stream count"));
    }
    let mut rng_streams = Vec::with_capacity(2);
    for expected_id in 0..2 {
        let id = reader.u8()?;
        if id != expected_id || reader.bytes(3)?.iter().any(|byte| *byte != 0) {
            return Err(NativeEpisodeShardError::new("noncanonical RNG stream"));
        }
        rng_streams.push(NativeRngStream {
            id,
            algorithm_version: reader.u32()?,
            state: [reader.i32()?, reader.i32()?, reader.i32()?],
            call_count: reader.u64()?,
        });
    }
    let talk_partner = decode_actor_identity(reader)?;
    let grabbed_actor = decode_actor_identity(reader)?;
    let goal = NativeGoalObservation {
        configured: flags & (1 << 7) != 0,
        reached: flags & (1 << 8) != 0,
        requested_count: reader.u16()?,
        hit_count: reader.u16()?,
        stable_ticks: reader.u16()?,
        consecutive_ticks: reader.u16()?,
        sequence_steps: reader.u8()?,
        sequence_next_step: reader.u8()?,
        sequence_within_ticks: reader.u16()?,
        sequence_elapsed_ticks: reader.u16()?,
        first_hit_tick: match reader.u64()? {
            u64::MAX => None,
            tick => Some(tick),
        },
    };
    if goal.reached != goal.first_hit_tick.is_some() || goal.hit_count > goal.requested_count {
        return Err(NativeEpisodeShardError::new(
            "inconsistent goal observation",
        ));
    }
    let mut actors = Vec::with_capacity(actor_count);
    for _ in 0..actor_count {
        actors.push(NativeActorObservation {
            runtime_generation: reader.u64()?,
            parent_runtime_generation: reader.u32()?,
            parameters: reader.u32()?,
            status: reader.u32()?,
            actor_name: reader.i16()?,
            profile_name: reader.i16()?,
            set_id: reader.u16()?,
            home_room: reader.i8()?,
            current_room: reader.i8()?,
            group: reader.u8()?,
            argument: reader.i8()?,
            health: reader.i16()?,
            position: reader.f32x3()?,
            home_position: reader.f32x3()?,
            velocity: reader.f32x3()?,
            forward_speed: reader.f32()?,
            current_angle: reader.i16x3()?,
            shape_angle: reader.i16x3()?,
        });
    }
    if actors
        .windows(2)
        .any(|pair| pair[0].runtime_generation >= pair[1].runtime_generation)
    {
        return Err(NativeEpisodeShardError::new(
            "actor set is not strictly ordered",
        ));
    }
    let flags_present = flags & (1 << 6) != 0;
    let event_flags = flags_present.then(|| reader.vec(822)).transpose()?;
    let temporary_flags = flags_present.then(|| reader.vec(185)).transpose()?;
    let dungeon_flags = flags_present.then(|| reader.vec(64)).transpose()?;
    let switch_flags = flags_present.then(|| reader.vec(240)).transpose()?;
    let switch_flag_room = reader.i8()?;
    Ok(NativeLearningObservation {
        phase,
        terminal_reason,
        actor_selection,
        actors_truncated: flags & (1 << 5) != 0,
        actor_observed_count,
        boundary_index,
        simulation_tick,
        tape_frame,
        remaining_ticks,
        state_identity,
        stage,
        room,
        layer,
        point,
        next_stage: (flags & (1 << 2) != 0).then_some(next_stage_raw),
        next_room,
        next_layer,
        next_point,
        player_present: flags & 1 != 0,
        player_is_link: flags & (1 << 1) != 0,
        player_process_id,
        player_actor_name,
        player_procedure,
        player_position,
        player_velocity,
        player_forward_speed,
        player_current_angle,
        player_shape_angle,
        player_mode_flags,
        player_damage_wait_timer,
        player_ice_damage_wait_timer,
        player_sword_change_wait_timer,
        player_do_status,
        player_contacts,
        player_ground_height: (flags & (1 << 9) != 0).then_some(ground_height),
        player_roof_height: (flags & (1 << 10) != 0).then_some(roof_height),
        event_running,
        event_id,
        event_mode,
        event_status,
        event_map_tool_id,
        event_name_hash: (flags & (1 << 11) != 0).then_some(event_name_hash_raw),
        menu_flags,
        menu_procedures,
        camera_yaw_radians: (flags & (1 << 3) != 0).then_some(camera),
        collision_correction: (flags & (1 << 4) != 0).then_some(correction),
        camera_status,
        camera: mechanics_camera,
        player_action_status,
        player_action,
        player_background_collision_status,
        player_background_collision,
        player_collision_surfaces_status,
        player_collision_surfaces,
        scene_exit_status,
        scene_exit,
        player_form_present,
        player_is_wolf,
        previous_input,
        rng_version,
        rng_streams,
        talk_partner,
        grabbed_actor,
        goal,
        actors,
        event_flags,
        temporary_flags,
        dungeon_flags,
        switch_flags,
        switch_flag_room,
    })
}

fn decode_actor_identity(
    reader: &mut Reader<'_>,
) -> Result<NativeActorIdentity, NativeEpisodeShardError> {
    let present = reader.bool()?;
    let runtime_generation = reader.u32()?;
    let actor_name = reader.i16()?;
    let set_id = reader.u16()?;
    let home_room = reader.i8()?;
    let current_room = reader.i8()?;
    let home_present = reader.bool()?;
    if reader.u8()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "nonzero actor-identity reserved byte",
        ));
    }
    let position = reader.f32x3()?;
    if present != home_present {
        return Err(NativeEpisodeShardError::new(
            "actor identity has inconsistent presence",
        ));
    }
    Ok(NativeActorIdentity {
        present,
        runtime_generation,
        actor_name,
        set_id,
        home_room,
        current_room,
        home_position: home_present.then_some(position),
    })
}

fn decode_pad(reader: &mut Reader<'_>) -> Result<NativeRawPad, NativeEpisodeShardError> {
    let start = reader.offset;
    let buttons = reader.u16()?;
    let stick_x = reader.i8()?;
    let stick_y = reader.i8()?;
    let substick_x = reader.i8()?;
    let substick_y = reader.i8()?;
    let trigger_left = reader.u8()?;
    let trigger_right = reader.u8()?;
    let analog_a = reader.u8()?;
    let analog_b = reader.u8()?;
    let connection = reader.u8()?;
    let connected = match connection {
        0 => false,
        1 => true,
        _ => {
            let wire = reader.bytes.get(start..start + 12).unwrap_or_default();
            return Err(NativeEpisodeShardError::new(format!(
                "invalid raw PAD flags {connection:#04x} at payload offset {} (wire={wire:02x?})",
                start + 10,
            )));
        }
    };
    let pad = NativeRawPad {
        buttons,
        stick_x,
        stick_y,
        substick_x,
        substick_y,
        trigger_left,
        trigger_right,
        analog_a,
        analog_b,
        connected,
        error: reader.i8()?,
    };
    Ok(pad)
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }
    fn done(&self) -> bool {
        self.offset == self.bytes.len()
    }
    fn bytes(&mut self, count: usize) -> Result<&'a [u8], NativeEpisodeShardError> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or_else(|| NativeEpisodeShardError::new("native episode offset overflow"))?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| NativeEpisodeShardError::new("truncated native episode shard"))?;
        self.offset = end;
        Ok(value)
    }
    fn u8(&mut self) -> Result<u8, NativeEpisodeShardError> {
        Ok(self.bytes(1)?[0])
    }
    fn i8(&mut self) -> Result<i8, NativeEpisodeShardError> {
        Ok(self.u8()? as i8)
    }
    fn bool(&mut self) -> Result<bool, NativeEpisodeShardError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(NativeEpisodeShardError::new("noncanonical boolean")),
        }
    }
    fn u16(&mut self) -> Result<u16, NativeEpisodeShardError> {
        Ok(u16::from_le_bytes(
            self.bytes(2)?.try_into().expect("exact length"),
        ))
    }
    fn i16(&mut self) -> Result<i16, NativeEpisodeShardError> {
        Ok(i16::from_le_bytes(
            self.bytes(2)?.try_into().expect("exact length"),
        ))
    }
    fn u32(&mut self) -> Result<u32, NativeEpisodeShardError> {
        Ok(u32::from_le_bytes(
            self.bytes(4)?.try_into().expect("exact length"),
        ))
    }
    fn i32(&mut self) -> Result<i32, NativeEpisodeShardError> {
        Ok(i32::from_le_bytes(
            self.bytes(4)?.try_into().expect("exact length"),
        ))
    }
    fn u64(&mut self) -> Result<u64, NativeEpisodeShardError> {
        Ok(u64::from_le_bytes(
            self.bytes(8)?.try_into().expect("exact length"),
        ))
    }
    fn usize_u64(&mut self) -> Result<usize, NativeEpisodeShardError> {
        usize::try_from(self.u64()?)
            .map_err(|_| NativeEpisodeShardError::new("native episode size overflow"))
    }
    fn f32(&mut self) -> Result<f32, NativeEpisodeShardError> {
        let value = f32::from_bits(self.u32()?);
        if !value.is_finite() || (value == 0.0 && value.is_sign_negative()) {
            return Err(NativeEpisodeShardError::new(
                "noncanonical observation float",
            ));
        }
        Ok(value)
    }
    fn f32x3(&mut self) -> Result<[f32; 3], NativeEpisodeShardError> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }
    fn f32x4(&mut self) -> Result<[f32; 4], NativeEpisodeShardError> {
        Ok([self.f32()?, self.f32()?, self.f32()?, self.f32()?])
    }
    fn i16x3(&mut self) -> Result<[i16; 3], NativeEpisodeShardError> {
        Ok([self.i16()?, self.i16()?, self.i16()?])
    }
    fn fixed_name(&mut self) -> Result<String, NativeEpisodeShardError> {
        let bytes = self.bytes(8)?;
        let end = bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(bytes.len());
        if bytes[end..].iter().any(|byte| *byte != 0) {
            return Err(NativeEpisodeShardError::new("noncanonical fixed string"));
        }
        std::str::from_utf8(&bytes[..end])
            .map(str::to_owned)
            .map_err(|_| NativeEpisodeShardError::new("fixed string is not UTF-8"))
    }
    fn string16(&mut self) -> Result<String, NativeEpisodeShardError> {
        let count = usize::from(self.u16()?);
        std::str::from_utf8(self.bytes(count)?)
            .map(str::to_owned)
            .map_err(|_| NativeEpisodeShardError::new("metadata string is not UTF-8"))
    }
    fn vec(&mut self, count: usize) -> Result<Vec<u8>, NativeEpisodeShardError> {
        Ok(self.bytes(count)?.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn golden() -> &'static [u8] {
        include_bytes!("../../../../../tests/fixtures/automation/native_episode_v2.dseps")
    }

    fn golden_v3() -> &'static [u8] {
        include_bytes!("../../../../../tests/fixtures/automation/native_episode_v3.dseps")
    }

    fn golden_v4() -> &'static [u8] {
        include_bytes!("../../../../../tests/fixtures/automation/native_episode_v4.dseps")
    }

    fn read_u16(bytes: &[u8], offset: usize) -> u16 {
        u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
    }

    fn read_u64(bytes: &[u8], offset: usize) -> u64 {
        u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
    }

    fn write_u64(bytes: &mut [u8], offset: usize, value: u64) {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn mutate_first_episode(mutator: impl FnOnce(&mut [u8])) -> Vec<u8> {
        mutate_first_episode_in(golden(), mutator)
    }

    fn mutate_first_v3_episode(mutator: impl FnOnce(&mut [u8])) -> Vec<u8> {
        mutate_first_episode_in(golden_v3(), mutator)
    }

    fn mutate_first_v4_episode(mutator: impl FnOnce(&mut [u8])) -> Vec<u8> {
        mutate_first_episode_in(golden_v4(), mutator)
    }

    fn mutate_first_episode_in(source: &[u8], mutator: impl FnOnce(&mut [u8])) -> Vec<u8> {
        let mut shard = source.to_vec();
        let payload_offset = read_u64(&shard, 56) as usize;
        let id_length = usize::from(read_u16(&shard, payload_offset + 20));
        let expanded_size = read_u64(&shard, payload_offset + 24) as usize;
        let old_compressed_size = read_u64(&shard, payload_offset + 32) as usize;
        let compressed_offset = payload_offset + BLOCK_HEADER_SIZE + id_length;
        let mut expanded = zstd::bulk::decompress(
            &shard[compressed_offset..compressed_offset + old_compressed_size],
            expanded_size,
        )
        .unwrap();
        mutator(&mut expanded);
        let compressed = zstd::bulk::compress(&expanded, 0).unwrap();
        let new_compressed_size = compressed.len();
        shard.splice(
            compressed_offset..compressed_offset + old_compressed_size,
            compressed,
        );
        write_u64(&mut shard, payload_offset + 32, new_compressed_size as u64);
        shard[payload_offset + 40..payload_offset + 56]
            .copy_from_slice(&xxhash_rust::xxh3::xxh3_128(&expanded).to_be_bytes());
        let delta = new_compressed_size as i64 - old_compressed_size as i64;
        write_u64(
            &mut shard,
            64,
            read_u64(source, 64).checked_add_signed(delta).unwrap(),
        );
        write_u64(
            &mut shard,
            80,
            read_u64(source, 80).checked_add_signed(delta).unwrap(),
        );
        shard
    }

    fn first_step_offsets(expanded: &[u8]) -> (usize, usize) {
        let mut reader = Reader::new(expanded);
        reader.bytes(8).unwrap();
        let observation_version = reader.u16().unwrap();
        reader.bytes(PAYLOAD_HEADER_SIZE - 10).unwrap();
        let pre_input = reader.offset;
        decode_observation(&mut reader, observation_version).unwrap();
        reader.bytes(24).unwrap();
        (pre_input, reader.offset)
    }

    #[test]
    fn rejects_incomplete_header_before_allocating() {
        let mut bytes = vec![0; HEADER_SIZE];
        bytes[..8].copy_from_slice(MAGIC);
        bytes[8..10].copy_from_slice(&VERSION.to_le_bytes());
        bytes[10..12].copy_from_slice(&(HEADER_SIZE as u16).to_le_bytes());
        let error = NativeEpisodeShard::decode(&bytes).unwrap_err();
        assert!(error.to_string().contains("incomplete"));
    }

    #[test]
    fn raw_pad_rejects_unknown_connection_flags() {
        let mut bytes = [0_u8; 12];
        bytes[10] = 2;
        assert!(decode_pad(&mut Reader::new(&bytes)).is_err());
    }

    #[test]
    fn decodes_native_cpp_golden_shard_with_exact_phase_joins() {
        let shard = NativeEpisodeShard::decode(golden()).unwrap();
        assert_eq!(shard.source_frame, 440);
        assert_eq!(shard.maximum_ticks, 1);
        assert_eq!(shard.episodes.len(), 2);
        let episode = &shard.episodes[0];
        assert_eq!(episode.id, "failure-0");
        assert!(!episode.success);
        assert_eq!(episode.steps.len(), 1);
        let step = &episode.steps[0];
        assert_eq!(step.pre_input.phase, NativeObservationPhase::PreInput);
        assert_eq!(step.pre_input.terminal_reason, NativeTerminalReason::None);
        assert_eq!(
            step.post_simulation.phase,
            NativeObservationPhase::PostSimulation
        );
        assert_eq!(step.consumed_pad.buttons, 0x0100);
        assert_eq!(step.consumed_pad.stick_x, 100);
        assert_eq!(step.chosen_pad, step.consumed_pad);
        assert_eq!(
            step.post_simulation.terminal_reason,
            NativeTerminalReason::TickBudgetExhausted
        );
        assert_eq!(step.post_simulation.previous_input, step.consumed_pad);
        assert_eq!(step.pre_input.actors.len(), 1);
        assert_eq!(step.pre_input.actor_observed_count, 1);
        assert_eq!(step.pre_input.actors[0].parameters, 0x12345678);
        assert_eq!(step.pre_input.actors[0].velocity, [0.25, 0.0, 0.0]);
        assert_eq!(step.pre_input.event_flags.as_ref().unwrap()[3], 1);
        assert_eq!(
            step.pre_input.camera_status,
            NativeChannelStatus::NotSampled
        );
        assert!(step.pre_input.camera.is_none());
        assert!(step.pre_input.player_collision_surfaces.is_none());

        let success = &shard.episodes[1];
        assert_eq!(success.id, "success-0");
        assert!(success.success);
        assert_eq!(success.first_hit_tick, Some(0));
        assert_eq!(success.steps.len(), 1);
        assert_eq!(
            success.steps[0].post_simulation.terminal_reason,
            NativeTerminalReason::GoalReached
        );
        assert!(success.steps[0].post_simulation.goal.reached);
    }

    #[test]
    fn decodes_v3_mechanics_and_collision_channels() {
        let shard = NativeEpisodeShard::decode(golden_v3()).unwrap();
        let observation = &shard.episodes[0].steps[0].pre_input;
        assert_eq!(observation.camera_status, NativeChannelStatus::Present);
        assert_eq!(observation.camera.as_ref().unwrap().view_yaw, 0x1200);
        assert_eq!(
            observation.player_action.as_ref().unwrap().procedure_id,
            0x42
        );
        assert_eq!(
            observation.scene_exit.as_ref().unwrap().destination_stage,
            "F_SP104"
        );
        assert!(observation.player_form_present);
        assert!(!observation.player_is_wolf);
        let background = observation.player_background_collision.as_ref().unwrap();
        assert_eq!(background.ground_identity, [1, 17, u32::MAX]);
        assert_eq!(background.ground_plane, [0.0, 1.0, 0.0, -2.0]);
        let surfaces = observation.player_collision_surfaces.as_ref().unwrap();
        assert_eq!(surfaces.identity_count, 1);
        assert_eq!(surfaces.surfaces[0].source_geometry_indices, [10, 11, 12]);
        assert_eq!(surfaces.surfaces[0].plane, Some([0.0, 1.0, 0.0, -2.0]));
    }

    #[test]
    fn decodes_v4_complete_actor_contract() {
        let shard = NativeEpisodeShard::decode(golden_v4()).unwrap();
        assert_eq!(
            shard.metadata.observation_schema,
            LEARNING_OBSERVATION_SCHEMA_V4
        );
        assert_eq!(shard.episodes[0].steps[0].pre_input.actors.len(), 257);
        for observation in shard.episodes.iter().flat_map(|episode| {
            episode
                .steps
                .iter()
                .flat_map(|step| [&step.pre_input, &step.post_simulation])
        }) {
            assert_eq!(
                observation.actor_selection,
                NativeActorSelectionRule::Complete
            );
            assert!(!observation.actors_truncated);
            assert_eq!(
                observation.actor_observed_count as usize,
                observation.actors.len()
            );
        }
    }

    #[test]
    fn v4_rejects_an_explicitly_truncated_actor_subset() {
        let shard = mutate_first_v4_episode(|expanded| {
            let (pre_input, _) = first_step_offsets(expanded);
            expanded[pre_input + 1] = 1;
            expanded[pre_input + 6] |= 1 << 5;
            expanded[pre_input + 10..pre_input + 14].copy_from_slice(&258_u32.to_le_bytes());
        });
        assert!(
            NativeEpisodeShard::decode(&shard)
                .unwrap_err()
                .to_string()
                .contains("does not contain the complete actor set")
        );
    }

    #[test]
    fn rejects_terminal_label_leakage_into_pre_input() {
        let shard = mutate_first_episode(|expanded| {
            let (pre_input, _) = first_step_offsets(expanded);
            expanded[pre_input + 2] = 1;
        });
        assert!(
            NativeEpisodeShard::decode(&shard)
                .unwrap_err()
                .to_string()
                .contains("action is not aligned")
        );
    }

    #[test]
    fn rejects_noncanonical_v3_mechanics_header() {
        let shard = mutate_first_v3_episode(|expanded| {
            let (pre_input, _) = first_step_offsets(expanded);
            expanded[pre_input + 186] = 2;
        });
        assert!(
            NativeEpisodeShard::decode(&shard)
                .unwrap_err()
                .to_string()
                .contains("invalid mechanics observation header")
        );
    }

    #[test]
    fn rejects_actor_completeness_that_masquerades_as_complete() {
        let shard = mutate_first_episode(|expanded| {
            let (pre_input, _) = first_step_offsets(expanded);
            expanded[pre_input + 10..pre_input + 14].copy_from_slice(&0_u32.to_le_bytes());
        });
        assert!(
            NativeEpisodeShard::decode(&shard)
                .unwrap_err()
                .to_string()
                .contains("inconsistent observation header")
        );
    }

    #[test]
    fn rejects_post_phase_and_boundary_discontinuity() {
        let wrong_phase = mutate_first_episode(|expanded| {
            let (_, post_simulation) = first_step_offsets(expanded);
            expanded[post_simulation] = 1;
        });
        assert!(
            NativeEpisodeShard::decode(&wrong_phase)
                .unwrap_err()
                .to_string()
                .contains("action is not aligned")
        );

        let wrong_boundary = mutate_first_episode(|expanded| {
            let (_, post_simulation) = first_step_offsets(expanded);
            let boundary = read_u64(expanded, post_simulation + 18);
            write_u64(expanded, post_simulation + 18, boundary + 1);
        });
        assert!(
            NativeEpisodeShard::decode(&wrong_boundary)
                .unwrap_err()
                .to_string()
                .contains("action is not aligned")
        );
    }

    #[test]
    fn rejects_episode_payload_corruption() {
        let mut shard = golden().to_vec();
        let payload_offset = read_u64(&shard, 56) as usize;
        let id_length = usize::from(read_u16(&shard, payload_offset + 20));
        let compressed_offset = payload_offset + BLOCK_HEADER_SIZE + id_length;
        shard[compressed_offset] ^= 0x40;
        assert!(NativeEpisodeShard::decode(&shard).is_err());
    }

    #[test]
    fn decodes_requested_live_native_batch() {
        let Some(path) = std::env::var_os("DUSK_NATIVE_EPISODE_SHARD") else {
            return;
        };
        let shard = NativeEpisodeShard::read(path).expect("decode live native episode shard");
        assert!(!shard.episodes.is_empty());
        if let Some(expected) = std::env::var_os("DUSK_EXPECTED_GAME_DATA_IDENTITY") {
            assert_eq!(
                shard.metadata.game_data_identity.as_deref(),
                expected.to_str(),
                "live shard did not bind the declared game-data fixture identity"
            );
        }
        assert!(shard.episodes.iter().all(|episode| {
            episode.steps.len() == episode.ticks_executed as usize
                && episode
                    .steps
                    .iter()
                    .all(|step| step.chosen_pad == step.consumed_pad)
        }));
        let source_identity = shard.episodes[0].steps[0].pre_input.state_identity;
        assert!(
            shard
                .episodes
                .iter()
                .all(|episode| episode.steps[0].pre_input.state_identity == source_identity)
        );
        assert!(shard.episodes.iter().all(|episode| {
            episode.steps.last().is_some_and(|step| {
                step.post_simulation.terminal_reason
                    == if episode.success {
                        NativeTerminalReason::GoalReached
                    } else {
                        NativeTerminalReason::TickBudgetExhausted
                    }
            })
        }));
        if matches!(
            shard.metadata.observation_schema.as_str(),
            LEARNING_OBSERVATION_SCHEMA_V3 | LEARNING_OBSERVATION_SCHEMA_V4
        ) {
            let observations = shard.episodes.iter().flat_map(|episode| {
                episode
                    .steps
                    .iter()
                    .flat_map(|step| [&step.pre_input, &step.post_simulation])
            });
            let observations: Vec<_> = observations.collect();
            assert!(observations.iter().all(|observation| {
                observation.camera_status == NativeChannelStatus::Present
                    && observation.player_action_status == NativeChannelStatus::Present
                    && observation.player_background_collision_status
                        == NativeChannelStatus::Present
                    && observation.player_collision_surfaces_status == NativeChannelStatus::Present
                    && observation.scene_exit_status != NativeChannelStatus::NotSampled
                    && observation.player_form_present
            }));
            assert!(observations.iter().any(|observation| {
                observation
                    .player_collision_surfaces
                    .as_ref()
                    .is_some_and(|surfaces| {
                        surfaces
                            .surfaces
                            .iter()
                            .any(|surface| surface.plane.is_some())
                    })
            }));
        }
    }

    #[test]
    fn rejects_action_shift_and_terminal_label_leakage() {
        let bytes =
            include_bytes!("../../../../../tests/fixtures/automation/native_episode_v2.dseps");
        let shard = NativeEpisodeShard::decode(bytes).unwrap();
        let step = &shard.episodes[1].steps[0];

        let mut leaked_pre = step.pre_input.clone();
        leaked_pre.terminal_reason = NativeTerminalReason::GoalReached;
        assert!(
            validate_step(
                None,
                &leaked_pre,
                step.consumed_pad,
                &step.post_simulation,
                true,
                true,
            )
            .is_err()
        );

        let mut shifted_action = step.consumed_pad;
        shifted_action.buttons ^= 1;
        assert!(
            validate_step(
                None,
                &step.pre_input,
                shifted_action,
                &step.post_simulation,
                true,
                true,
            )
            .is_err()
        );

        let mut missing_terminal = step.post_simulation.clone();
        missing_terminal.terminal_reason = NativeTerminalReason::None;
        assert!(
            validate_step(
                None,
                &step.pre_input,
                step.consumed_pad,
                &missing_terminal,
                true,
                true,
            )
            .is_err()
        );
    }
}
