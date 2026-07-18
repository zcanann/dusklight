use crate::tape::RawPadState;
use serde::Serialize;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

const V1_HEADER_SIZE: usize = 36;
const V1_RECORD_SIZE: usize = 102;
const V2_HEADER_SIZE: usize = 64;
const V2_DIRECTORY_ENTRY_SIZE: usize = 64;
const MAGIC: &[u8; 8] = b"DUSKTRCE";

const FILE_COMPLETE: u32 = 1 << 0;
const FILE_CAPACITY_EXHAUSTED: u32 = 1 << 1;
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
}

impl TraceChannel {
    pub const ALL: [Self; 9] = [
        Self::Core,
        Self::Stage,
        Self::AppliedPads,
        Self::PlayerMotion,
        Self::Event,
        Self::SceneExit,
        Self::Rng,
        Self::Camera,
        Self::PlayerAction,
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
        }
    }
}

const KNOWN_CHANNELS: u64 = (1 << 9) - 1;

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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TraceAppliedPads {
    pub valid_ports: u8,
    pub owned_ports: u8,
    pub pads: [RawPadState; 4],
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
    pub applied_pads: Option<TraceAppliedPads>,
    pub rng: Option<TraceRngSnapshot>,
    pub camera: Option<TraceCamera>,
    pub player_action: Option<TracePlayerAction>,
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
            applied_pads: None,
            rng: None,
            camera: None,
            player_action: None,
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
    pub requested_channels: Vec<&'static str>,
    pub record_count: usize,
    pub capacity_exhausted: bool,
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
    pub tick_rate_numerator: u32,
    pub tick_rate_denominator: u32,
    pub requested_channels: u64,
    pub capacity_exhausted: bool,
    pub records: Vec<TraceRecord>,
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
    version: u16,
    stride: usize,
}

fn channel_definition(channel: TraceChannel) -> ChannelDefinition {
    match channel {
        TraceChannel::Core => ChannelDefinition {
            version: 1,
            stride: 32,
        },
        TraceChannel::Stage => ChannelDefinition {
            version: 1,
            stride: 32,
        },
        TraceChannel::AppliedPads => ChannelDefinition {
            version: 1,
            stride: 52,
        },
        TraceChannel::PlayerMotion => ChannelDefinition {
            version: 1,
            stride: 52,
        },
        TraceChannel::Event => ChannelDefinition {
            version: 1,
            stride: 16,
        },
        TraceChannel::SceneExit => ChannelDefinition {
            version: 1,
            stride: 24,
        },
        TraceChannel::Rng => ChannelDefinition {
            version: 1,
            stride: 64,
        },
        TraceChannel::Camera => ChannelDefinition {
            version: 1,
            stride: 48,
        },
        TraceChannel::PlayerAction => ChannelDefinition {
            version: 1,
            stride: 104,
        },
    }
}

#[derive(Clone)]
struct ChannelDescriptor {
    channel: Option<TraceChannel>,
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
        2 => decode_v2(bytes),
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
        tick_rate_numerator: u32_at(bytes, 12),
        tick_rate_denominator: u32_at(bytes, 16),
        requested_channels: TraceChannel::Core.bit()
            | TraceChannel::Stage.bit()
            | TraceChannel::AppliedPads.bit()
            | TraceChannel::PlayerMotion.bit()
            | TraceChannel::Event.bit()
            | TraceChannel::SceneExit.bit(),
        capacity_exhausted: u32_at(bytes, 28) != 0,
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

fn decode_v2(bytes: &[u8]) -> Result<DecodedTrace, TraceError> {
    if bytes.len() < V2_HEADER_SIZE {
        return Err(TraceError("truncated gameplay trace v2 header".into()));
    }
    if usize::from(u16_at(bytes, 10)) != V2_HEADER_SIZE {
        return Err(TraceError(
            "unsupported gameplay trace v2 header size".into(),
        ));
    }
    validate_tick_rate(bytes)?;
    let count = count_at(bytes, 20)?;
    let file_flags = u32_at(bytes, 28);
    if file_flags & FILE_COMPLETE == 0
        || file_flags & !(FILE_COMPLETE | FILE_CAPACITY_EXHAUSTED) != 0
    {
        return Err(TraceError(
            "incomplete or noncanonical gameplay trace v2 flags".into(),
        ));
    }
    let channel_count = usize::from(u16_at(bytes, 32));
    if usize::from(u16_at(bytes, 34)) != V2_DIRECTORY_ENTRY_SIZE
        || usize_at_u64(bytes, 36)? != V2_HEADER_SIZE
    {
        return Err(TraceError(
            "unsupported gameplay trace v2 directory layout".into(),
        ));
    }
    let directory_end = checked_region_end(V2_HEADER_SIZE, channel_count, V2_DIRECTORY_ENTRY_SIZE)?;
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
        let offset = V2_HEADER_SIZE + index * V2_DIRECTORY_ENTRY_SIZE;
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
            let definition = channel_definition(channel);
            if version != definition.version || stride != definition.stride {
                return Err(TraceError(format!(
                    "unsupported gameplay trace channel {} version {version} / stride {stride}",
                    channel.name()
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
    for descriptor in descriptors.values() {
        let Some(channel) = descriptor.channel else {
            continue;
        };
        debug_assert_eq!(descriptor.status_length, count);
        debug_assert_eq!(descriptor.payload_length, count * descriptor.stride);
        for (index, record) in records.iter_mut().enumerate() {
            let status = TraceChannelStatus::try_from(bytes[descriptor.status_offset + index])?;
            if channel == TraceChannel::Core && status != TraceChannelStatus::Present {
                return Err(TraceError("gameplay trace core is not present".into()));
            }
            record.channel_status.insert(channel, status);
            if status == TraceChannelStatus::Present || status == TraceChannelStatus::Truncated {
                let start = descriptor.payload_offset + index * descriptor.stride;
                decode_v2_channel(channel, &bytes[start..start + descriptor.stride], record)?;
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
    }
    Ok(DecodedTrace {
        version: 2,
        tick_rate_numerator: u32_at(bytes, 12),
        tick_rate_denominator: u32_at(bytes, 16),
        requested_channels,
        capacity_exhausted: file_flags & FILE_CAPACITY_EXHAUSTED != 0,
        records,
    })
}

fn decode_v2_channel(
    channel: TraceChannel,
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
        TraceChannel::SceneExit => {
            if u16_at(bytes, 6) != 0 {
                return Err(TraceError(
                    "nonzero gameplay trace scene-exit reserved field".into(),
                ));
            }
            record.nearest_scene_exit_session_process_id = Some(u32_at(bytes, 0));
            record.nearest_scene_exit_actor_name = Some(i16_at(bytes, 4));
            record.nearest_scene_exit_position =
                [f32_at(bytes, 8), f32_at(bytes, 12), f32_at(bytes, 16)];
            record.nearest_scene_exit_distance = Some(f32_at(bytes, 20));
        }
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
            });
        }
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
        requested_channels,
        record_count: records.len(),
        capacity_exhausted: decoded.capacity_exhausted,
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
    usize::try_from(u64_at(bytes, offset))
        .map_err(|_| TraceError("gameplay trace record count is too large".into()))
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
}
