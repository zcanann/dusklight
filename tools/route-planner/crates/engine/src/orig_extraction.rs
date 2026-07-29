//! Minimal, planner-owned extraction for immutable retail message archives.
//!
//! This module deliberately accepts bytes rather than filesystem paths. The
//! runtime CLI owns discovery and I/O; the engine owns bounded deterministic
//! decoding and portable extracted records.

use crate::PlannerContractError;
use serde::{Deserialize, Serialize};

mod structured;
#[cfg(test)]
use structured::hex_bytes;
pub use structured::{parse_event_list, parse_stage_data};

const MAX_DECODED_ARCHIVE_BYTES: usize = 64 * 1024 * 1024;
const RARC_FILE_ENTRY_SIZE: usize = 0x14;
const MAX_RARC_FILE_ENTRIES: usize = 100_000;
const MAX_STAGE_CHUNKS: usize = 4096;
const MAX_STAGE_RECORDS: usize = 1_000_000;
const MAX_EVENT_RECORDS: usize = 1_000_000;
pub const EXTRACTED_STAGE_DATA_SCHEMA: &str = "dusklight.route-planner.extracted-stage-data/v8";
pub const EXTRACTED_EVENT_LIST_SCHEMA: &str = "dusklight.route-planner.extracted-event-list/v1";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedStageData {
    pub chunks: Vec<ExtractedStageChunk>,
    pub stage_information: Option<ExtractedStageInformation>,
    pub room_transforms: Vec<ExtractedRoomTransform>,
    pub file_lists: Vec<ExtractedFileList>,
    pub room_read_table: Vec<ExtractedRoomRead>,
    pub cameras: Vec<ExtractedCamera>,
    pub camera_arrows: Vec<ExtractedCameraArrow>,
    pub paths: Vec<ExtractedPath>,
    pub path_points: Vec<ExtractedPathPoint>,
    pub scene_transitions: Vec<ExtractedSceneTransition>,
    pub map_events: Vec<ExtractedMapEvent>,
    pub demo_archive_banks: Vec<ExtractedDemoArchiveBank>,
    pub actor_placements: Vec<ExtractedActorPlacement>,
    pub treasure_placements: Vec<ExtractedActorPlacement>,
    pub player_spawns: Vec<ExtractedActorPlacement>,
}

/// One `RPAT` rail/path record. `point_offset` is relative to the paired
/// `RPPN` table and is also normalized to `first_point_index`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedPath {
    pub record_index: u32,
    pub point_count: u16,
    pub next_path_index: Option<u16>,
    pub path_argument: u8,
    pub closed: bool,
    pub closed_raw: u8,
    pub switch_no: Option<u8>,
    pub unknown_07: u8,
    pub point_offset: u32,
    pub first_point_index: u32,
    pub raw_hex: String,
}

/// One `RPPN` point record referenced by one or more `RPAT` paths.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedPathPoint {
    pub record_index: u32,
    pub arguments: [u8; 4],
    pub position: [f32; 3],
    pub raw_hex: String,
}

/// One `RCAM` map-tool camera record. The final field is `0xffff` when the
/// runtime resolves the camera implementation from `camera_type` instead.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedCamera {
    pub record_index: u32,
    pub camera_type: String,
    pub arrow_index: u8,
    pub field_of_view_y: u8,
    pub argument_0: u8,
    pub argument_1: u8,
    pub argument_2: u16,
    pub camera_type_index: Option<u16>,
    pub raw_hex: String,
}

/// One `RARO` map-tool camera/attention transform referenced by `RCAM`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedCameraArrow {
    pub record_index: u32,
    pub position: [f32; 3],
    pub angle: [i16; 3],
    pub trailing_i16: i16,
    pub raw_hex: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedRoomRead {
    pub room_index: u32,
    pub record_offset: u32,
    pub room_list_offset: u32,
    pub reverb: u8,
    pub reverb_raw: u8,
    pub time_pass: u8,
    pub vrbox_enabled: bool,
    pub flags_raw: u8,
    pub padding: u8,
    pub load_rooms: Vec<ExtractedLoadedRoom>,
    pub raw_header_hex: String,
    pub raw_room_list_hex: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedLoadedRoom {
    pub room: u8,
    pub load_background: bool,
    pub unknown_bit_6: bool,
    pub raw: u8,
}

/// One stage-level `MULT` record. Despite the original member name
/// `mTransY`, the room background actor applies the second translation to Z.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedRoomTransform {
    pub record_index: u32,
    pub room: u8,
    pub translation_xz: [f32; 2],
    pub angle_y: i16,
    pub trailing_byte: u8,
    pub raw_hex: String,
}

/// One normal stage/room `FILI` record. Field-map DZS resources reinterpret
/// the same tag with a distinct layout and are outside normal archive discovery.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedFileList {
    pub record_index: u32,
    pub parameters: u32,
    pub sea_level: f32,
    pub unknown_float_08: f32,
    pub unknown_float_0c: f32,
    pub unknown_bytes_10_19_hex: String,
    pub minimap_style: u8,
    pub enemy_appear_flag: bool,
    pub global_wind_level: u8,
    pub global_wind_direction: u8,
    pub grass_light: u8,
    pub default_camera: u8,
    pub bit_switch: u8,
    pub message_id: u16,
    pub raw_hex: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedMapEvent {
    pub record_index: u32,
    pub event_type: u8,
    pub map_tool_id: u8,
    pub priority: u8,
    pub normal_exit_id: Option<u8>,
    pub skip_exit_id: Option<u8>,
    pub event_name: Option<String>,
    pub switch_no: Option<u8>,
    pub raw_hex: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedDemoArchiveBank {
    pub layer: u8,
    pub bank: Option<u8>,
    pub bank2: Option<u8>,
    pub archive_name: Option<String>,
    pub raw_hex: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedEventList {
    pub resource_size: u32,
    pub events: Vec<ExtractedEvent>,
    pub staff: Vec<ExtractedEventStaff>,
    pub cuts: Vec<ExtractedEventCut>,
    pub data: Vec<ExtractedEventData>,
    pub float_data_bits: Vec<u32>,
    pub integer_data: Vec<i32>,
    pub string_data_hex: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedEvent {
    pub index: u32,
    pub name: String,
    pub priority: i32,
    pub staff_indices: Vec<u32>,
    pub finish_flags: [i32; 3],
    pub raw_hex: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedEventStaff {
    pub index: u32,
    pub name: String,
    pub tag_id: i32,
    pub flag_id: u32,
    pub staff_type: i32,
    pub start_cut_index: u32,
    pub raw_hex: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedEventCut {
    pub index: u32,
    pub name: String,
    pub tag_id: u32,
    pub start_flags: [i32; 3],
    pub flag_id: u32,
    pub data_index: Option<u32>,
    pub next_cut_index: Option<u32>,
    pub raw_hex: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedEventData {
    pub index: u32,
    pub name: String,
    pub data_type: i32,
    pub value_index: u32,
    pub value_count: u32,
    pub next_data_index: Option<u32>,
    pub value: ExtractedEventDataValue,
    pub raw_hex: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExtractedEventDataValue {
    FloatBits {
        values: Vec<u32>,
    },
    VectorBits {
        values: Vec<u32>,
    },
    UnknownFloatBits {
        values: Vec<u32>,
    },
    Integers {
        values: Vec<i32>,
    },
    StringBytes {
        raw_hex: String,
        ascii: Option<String>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedStageInformation {
    pub message_group: u8,
    pub raw_hex: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedSceneTransition {
    /// Zero-based index consumed by `dStage_changeScene`.
    pub exit_id: u32,
    pub destination_stage: String,
    pub destination_spawn: u8,
    pub destination_room: i8,
    pub scene_layer: Option<u8>,
    pub time_hour: Option<u8>,
    pub wipe: u8,
    pub wipe_time: u8,
    pub raw_hex: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedStageChunk {
    pub tag: String,
    pub record_count: u32,
    pub data_offset: u32,
    pub recognized_record_size: Option<u8>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedActorPlacement {
    pub chunk_tag: String,
    pub record_index: u32,
    pub layer: Option<u8>,
    pub name: String,
    pub parameters: u32,
    pub position: [f32; 3],
    pub angle: [i16; 3],
    pub set_id: u16,
    pub scale_raw: Option<[u8; 3]>,
    pub raw_hex: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedMessageFlow {
    /// The retail header can stop at the end of MID1 even though FLW1/FLI1
    /// follow in the same physical RARC resource.
    pub header_declared_size: u32,
    pub resource_size: u32,
    pub node_count: u16,
    pub branch_target_count: u16,
    pub labels: Vec<MessageFlowLabel>,
    pub nodes: Vec<MessageFlowNode>,
    pub branch_targets: Vec<u16>,
    pub temporary_flag_accesses: Vec<MessageFlowTemporaryFlagAccess>,
    pub persistent_flag_accesses: Vec<MessageFlowPersistentFlagAccess>,
    pub switch_accesses: Vec<MessageFlowSwitchAccess>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MessageFlowLabel {
    pub flow_id: u16,
    pub node_index: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MessageFlowNode {
    Message {
        index: u16,
        flags: u8,
        message_index: u16,
        next_node_index: u16,
        unknown: u16,
    },
    Branch {
        index: u16,
        flags: u8,
        /// The on-disc index into `dMsgFlow_c::mQueryList`.
        raw_query_index: u16,
        /// The numbered query handler reached through the retail dispatch table.
        query_handler_index: Option<u16>,
        parameter: u16,
        next_target_index: u16,
    },
    Event {
        index: u16,
        event_index: u8,
        next_target_index: u16,
        parameter_0: u16,
        parameter_1: u16,
        raw_parameter_u32: u32,
        raw_parameters: [u8; 4],
    },
    Unknown {
        index: u16,
        node_type: u8,
        raw: [u8; 8],
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageFlowTemporaryFlagOperation {
    Set,
    Clear,
    /// `query011` returns the true branch when this bit is clear.
    BranchTrueWhenClear,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MessageFlowTemporaryFlagAccess {
    pub node_index: u16,
    pub operation: MessageFlowTemporaryFlagOperation,
    pub parameter_ordinal: u8,
    pub label_index: u16,
    /// Known packed byte/bit coordinate from `tempBitLabels`; absent when this
    /// minimal extractor has not imported that label's source definition yet.
    pub packed_backing_coordinate: Option<u16>,
    pub friendly_name: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageFlowPersistentFlagOperation {
    Set,
    Clear,
    /// `query001` returns the true branch when this bit is clear.
    BranchTrueWhenClear,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MessageFlowPersistentFlagAccess {
    pub node_index: u16,
    pub operation: MessageFlowPersistentFlagOperation,
    pub parameter_ordinal: u8,
    pub label_index: u16,
    pub packed_backing_coordinate: Option<u16>,
    pub friendly_name: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageFlowSwitchOperation {
    Set,
    Clear,
    BranchTrueWhenClear,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageFlowSwitchStore {
    /// The current stage's loaded `dSv_memBit_c` bank. It can later be
    /// projected to that stage's persistent save table; it is not a generic
    /// process-global switch store.
    LoadedStageMemory,
    Dungeon,
    Zone,
    OneZone,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MessageFlowSwitchAccess {
    pub node_index: u16,
    pub operation: MessageFlowSwitchOperation,
    pub store: MessageFlowSwitchStore,
    pub switch_index: u16,
}

/// Translate the raw BMG branch index through the retail query dispatch table.
/// The first eight entries are reordered; entries 8..=52 dispatch to handlers
/// 9..=53 respectively.
pub fn message_query_handler_index(raw_query_index: u16) -> Option<u16> {
    const REORDERED: [u16; 8] = [5, 1, 2, 3, 6, 7, 4, 8];
    match raw_query_index {
        0..=7 => Some(REORDERED[usize::from(raw_query_index)]),
        8..=52 => Some(raw_query_index + 1),
        _ => None,
    }
}

fn generic_message_flag(label_index: u16) -> Option<(u16, &'static str)> {
    Some(match label_index {
        11 => (0x0004, "message_flow_control_a"),
        12 => (0x0002, "message_flow_control_b"),
        13 => (0x0001, "message_flow_control_c"),
        14 => (0x0180, "message_flow_control_d"),
        15 => (0x0140, "message_flow_control_e"),
        51 => (0x0508, "message_flow_control_f"),
        52 => (0x0504, "message_flow_control_g"),
        53 => (0x0502, "message_flow_control_h"),
        54 => (0x0501, "message_flow_control_i"),
        55 => (0x0680, "message_flow_control_j"),
        _ => return None,
    })
}

fn persistent_message_flag(label_index: u16) -> Option<(u16, &'static str)> {
    Some(match label_index {
        6 => (0x0004, "lost_first_gor_coron_match"),
        62 => (0x0704, "won_gor_coron_match"),
        63 => (0x0702, "first_gor_coron_conversation"),
        64 => (0x0701, "goron_mines_clear"),
        66 => (0x0840, "start_carriage_guarding_game"),
        115 => (0x0e20, "spoke_with_spring_goron_a"),
        152 => (0x1201, "lost_gor_coron_match_with_iron_boots"),
        154 => (0x1340, "lost_gor_coron_match_again"),
        615 => (0x4b04, "received_lanayru_vessel"),
        _ => return None,
    })
}

pub fn extract_unique_rarc_resource(
    input: &[u8],
    resource_name: &str,
) -> Result<Vec<u8>, PlannerContractError> {
    if resource_name.is_empty()
        || resource_name.as_bytes().contains(&0)
        || resource_name.contains(['/', '\\'])
    {
        return Err(PlannerContractError::new(
            "orig.resource_name",
            "must be one nonempty basename without NUL or path separators",
        ));
    }
    let decoded = decode_archive(input)?;
    extract_uncompressed_rarc_resource(&decoded, resource_name)
}

/// List file basenames present in a bounded RARC/Yaz0 archive. Duplicate
/// basenames remain duplicated so callers cannot mistake an ambiguous archive
/// for one with a unique resource.
pub fn list_rarc_resource_names(input: &[u8]) -> Result<Vec<String>, PlannerContractError> {
    let decoded = decode_archive(input)?;
    let mut names = rarc_resource_entries(&decoded)?
        .into_iter()
        .map(|entry| {
            std::str::from_utf8(entry.name)
                .map(str::to_owned)
                .map_err(|_| PlannerContractError::new("orig.rarc.entry.name", "must be UTF-8"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    Ok(names)
}

/// Parse authored actor placements directly from a DZS/DZR resource. Unknown
/// chunk types remain listed but are not guessed at.
pub fn parse_message_flow(input: &[u8]) -> Result<ExtractedMessageFlow, PlannerContractError> {
    require_range(input, 0, 0x20, "orig.bmg.header")?;
    if &input[0..8] != b"MESGbmg1" {
        return Err(PlannerContractError::new(
            "orig.bmg.magic",
            "must equal MESGbmg1",
        ));
    }
    let declared_size = read_u32(input, 8, "orig.bmg.size")? as usize;
    if declared_size > input.len() || declared_size < 0x20 {
        return Err(PlannerContractError::new(
            "orig.bmg.size",
            format!(
                "declared {declared_size:#x} bytes outside resource size {:#x}",
                input.len()
            ),
        ));
    }
    let section_count = read_u32(input, 12, "orig.bmg.section_count")? as usize;
    let mut offset = 0x20_usize;
    let mut flow_section = None;
    let mut label_section = None;
    for section_index in 0..section_count {
        require_range(input, offset, 8, "orig.bmg.section")?;
        let size = read_u32(input, offset + 4, "orig.bmg.section.size")? as usize;
        if size < 8 {
            return Err(PlannerContractError::new(
                "orig.bmg.section.size",
                "must be at least eight bytes",
            ));
        }
        let available_size = input.len() - offset;
        let physical_size = size.min(available_size);
        if size > available_size
            && (section_index + 1 != section_count || size - available_size >= 0x20)
        {
            return Err(PlannerContractError::new(
                "orig.bmg.section.size",
                "exceeds the resource by more than final alignment padding",
            ));
        }
        match &input[offset..offset + 4] {
            b"FLW1" => {
                if flow_section.replace((offset, physical_size)).is_some() {
                    return Err(PlannerContractError::new("orig.bmg.flw1", "must be unique"));
                }
            }
            b"FLI1" => {
                if label_section.replace((offset, physical_size)).is_some() {
                    return Err(PlannerContractError::new("orig.bmg.fli1", "must be unique"));
                }
            }
            _ => {}
        }
        offset = offset
            .checked_add(size)
            .ok_or_else(|| PlannerContractError::new("orig.bmg.section", "offset overflow"))?;
    }
    if offset < input.len() && input[offset..].iter().any(|byte| *byte != 0) {
        return Err(PlannerContractError::new(
            "orig.bmg.sections",
            "leave nonzero bytes after the section sequence",
        ));
    }

    let (flow_offset, flow_size) = flow_section
        .ok_or_else(|| PlannerContractError::new("orig.bmg.flw1", "section is missing"))?;
    let (label_offset, label_size) = label_section
        .ok_or_else(|| PlannerContractError::new("orig.bmg.fli1", "section is missing"))?;
    require_range(input, flow_offset, 0x10, "orig.bmg.flw1.header")?;
    require_range(input, label_offset, 0x10, "orig.bmg.fli1.header")?;
    let node_count = read_u16(input, flow_offset + 8, "orig.bmg.flw1.node_count")?;
    let branch_target_count =
        read_u16(input, flow_offset + 10, "orig.bmg.flw1.branch_target_count")?;
    let nodes_start = flow_offset + 0x10;
    let node_bytes = usize::from(node_count)
        .checked_mul(8)
        .ok_or_else(|| PlannerContractError::new("orig.bmg.flw1.nodes", "size overflow"))?;
    let targets_start = nodes_start
        .checked_add(node_bytes)
        .ok_or_else(|| PlannerContractError::new("orig.bmg.flw1.targets", "offset overflow"))?;
    let target_bytes = usize::from(branch_target_count)
        .checked_mul(2)
        .ok_or_else(|| PlannerContractError::new("orig.bmg.flw1.targets", "size overflow"))?;
    if targets_start + target_bytes > flow_offset + flow_size {
        return Err(PlannerContractError::new(
            "orig.bmg.flw1",
            "node and target tables exceed the section",
        ));
    }

    let mut nodes = Vec::with_capacity(usize::from(node_count));
    for index in 0..node_count {
        let start = nodes_start + usize::from(index) * 8;
        let raw: [u8; 8] = input[start..start + 8].try_into().unwrap();
        let node = match raw[0] {
            1 => MessageFlowNode::Message {
                index,
                flags: raw[1],
                message_index: u16::from_be_bytes([raw[2], raw[3]]),
                next_node_index: u16::from_be_bytes([raw[4], raw[5]]),
                unknown: u16::from_be_bytes([raw[6], raw[7]]),
            },
            2 => MessageFlowNode::Branch {
                index,
                flags: raw[1],
                raw_query_index: u16::from_be_bytes([raw[2], raw[3]]),
                query_handler_index: message_query_handler_index(u16::from_be_bytes([
                    raw[2], raw[3],
                ])),
                parameter: u16::from_be_bytes([raw[4], raw[5]]),
                next_target_index: u16::from_be_bytes([raw[6], raw[7]]),
            },
            3 => MessageFlowNode::Event {
                index,
                event_index: raw[1],
                next_target_index: u16::from_be_bytes([raw[2], raw[3]]),
                parameter_0: u16::from_be_bytes([raw[4], raw[5]]),
                parameter_1: u16::from_be_bytes([raw[6], raw[7]]),
                raw_parameter_u32: u32::from_be_bytes(raw[4..8].try_into().unwrap()),
                raw_parameters: raw[4..8].try_into().unwrap(),
            },
            node_type => MessageFlowNode::Unknown {
                index,
                node_type,
                raw,
            },
        };
        nodes.push(node);
    }
    let mut branch_targets = Vec::with_capacity(usize::from(branch_target_count));
    for index in 0..branch_target_count {
        branch_targets.push(read_u16(
            input,
            targets_start + usize::from(index) * 2,
            "orig.bmg.flw1.target",
        )?);
    }

    let label_count = read_u16(input, label_offset + 8, "orig.bmg.fli1.label_count")?;
    let labels_end = label_offset
        .checked_add(0x10 + usize::from(label_count) * 8)
        .ok_or_else(|| PlannerContractError::new("orig.bmg.fli1", "size overflow"))?;
    if labels_end > label_offset + label_size {
        return Err(PlannerContractError::new(
            "orig.bmg.fli1",
            "label table exceeds the section",
        ));
    }
    let mut labels = Vec::with_capacity(usize::from(label_count));
    for index in 0..label_count {
        let start = label_offset + 0x10 + usize::from(index) * 8;
        let flow_id = (read_u32(input, start, "orig.bmg.fli1.label")? >> 16) as u16;
        let node_index = read_u16(input, start + 4, "orig.bmg.fli1.node")?;
        if node_index != u16::MAX && node_index >= node_count {
            return Err(PlannerContractError::new(
                "orig.bmg.fli1.node",
                format!("label {flow_id} references node {node_index} outside {node_count}"),
            ));
        }
        labels.push(MessageFlowLabel {
            flow_id,
            node_index,
        });
    }
    labels.sort_by_key(|label| (label.flow_id, label.node_index));
    if labels
        .windows(2)
        .any(|pair| pair[0].flow_id == pair[1].flow_id)
    {
        return Err(PlannerContractError::new(
            "orig.bmg.fli1.flow_id",
            "must be unique",
        ));
    }

    let mut temporary_flag_accesses = Vec::new();
    let mut persistent_flag_accesses = Vec::new();
    let mut switch_accesses = Vec::new();
    for node in &nodes {
        match *node {
            MessageFlowNode::Event {
                index,
                event_index: 10 | 11,
                parameter_0,
                parameter_1,
                ..
            } => {
                let operation = if matches!(
                    node,
                    MessageFlowNode::Event {
                        event_index: 10,
                        ..
                    }
                ) {
                    MessageFlowTemporaryFlagOperation::Set
                } else {
                    MessageFlowTemporaryFlagOperation::Clear
                };
                for (parameter_ordinal, label_index) in [(0_u8, parameter_0), (1_u8, parameter_1)] {
                    // Both generic handlers treat zero as a no-op sentinel.
                    if label_index != 0 {
                        temporary_flag_accesses.push(temporary_flag_access(
                            index,
                            operation,
                            parameter_ordinal,
                            label_index,
                        ));
                    }
                }
            }
            MessageFlowNode::Branch {
                index,
                query_handler_index: Some(11),
                parameter,
                ..
            } => temporary_flag_accesses.push(temporary_flag_access(
                index,
                MessageFlowTemporaryFlagOperation::BranchTrueWhenClear,
                0,
                parameter,
            )),
            MessageFlowNode::Event {
                index,
                event_index: 0 | 1,
                parameter_0,
                parameter_1,
                ..
            } => {
                let operation = if matches!(node, MessageFlowNode::Event { event_index: 0, .. }) {
                    MessageFlowPersistentFlagOperation::Set
                } else {
                    MessageFlowPersistentFlagOperation::Clear
                };
                for (parameter_ordinal, label_index) in [(0_u8, parameter_0), (1_u8, parameter_1)] {
                    if label_index != 0 {
                        persistent_flag_accesses.push(persistent_flag_access(
                            index,
                            operation,
                            parameter_ordinal,
                            label_index,
                        ));
                    }
                }
            }
            MessageFlowNode::Branch {
                index,
                query_handler_index: Some(1),
                parameter,
                ..
            } => persistent_flag_accesses.push(persistent_flag_access(
                index,
                MessageFlowPersistentFlagOperation::BranchTrueWhenClear,
                0,
                parameter,
            )),
            MessageFlowNode::Event {
                index,
                event_index: 14 | 15,
                parameter_0,
                parameter_1,
                ..
            } => {
                if let Some(store) = switch_store(parameter_0) {
                    switch_accesses.push(MessageFlowSwitchAccess {
                        node_index: index,
                        operation: if matches!(
                            node,
                            MessageFlowNode::Event {
                                event_index: 14,
                                ..
                            }
                        ) {
                            MessageFlowSwitchOperation::Set
                        } else {
                            MessageFlowSwitchOperation::Clear
                        },
                        store,
                        switch_index: parameter_1,
                    });
                }
            }
            MessageFlowNode::Branch {
                index,
                query_handler_index: Some(handler @ (13 | 15 | 17 | 19)),
                parameter,
                ..
            } => switch_accesses.push(MessageFlowSwitchAccess {
                node_index: index,
                operation: MessageFlowSwitchOperation::BranchTrueWhenClear,
                store: match handler {
                    13 => MessageFlowSwitchStore::LoadedStageMemory,
                    15 => MessageFlowSwitchStore::Dungeon,
                    17 => MessageFlowSwitchStore::Zone,
                    19 => MessageFlowSwitchStore::OneZone,
                    _ => unreachable!(),
                },
                switch_index: parameter,
            }),
            _ => {}
        }
    }

    Ok(ExtractedMessageFlow {
        header_declared_size: declared_size as u32,
        resource_size: input.len() as u32,
        node_count,
        branch_target_count,
        labels,
        nodes,
        branch_targets,
        temporary_flag_accesses,
        persistent_flag_accesses,
        switch_accesses,
    })
}

fn temporary_flag_access(
    node_index: u16,
    operation: MessageFlowTemporaryFlagOperation,
    parameter_ordinal: u8,
    label_index: u16,
) -> MessageFlowTemporaryFlagAccess {
    let known = generic_message_flag(label_index);
    MessageFlowTemporaryFlagAccess {
        node_index,
        operation,
        parameter_ordinal,
        label_index,
        packed_backing_coordinate: known.map(|entry| entry.0),
        friendly_name: known.map(|entry| entry.1.to_owned()),
    }
}

fn persistent_flag_access(
    node_index: u16,
    operation: MessageFlowPersistentFlagOperation,
    parameter_ordinal: u8,
    label_index: u16,
) -> MessageFlowPersistentFlagAccess {
    let known = persistent_message_flag(label_index);
    MessageFlowPersistentFlagAccess {
        node_index,
        operation,
        parameter_ordinal,
        label_index,
        packed_backing_coordinate: known.map(|entry| entry.0),
        friendly_name: known.map(|entry| entry.1.to_owned()),
    }
}

fn switch_store(selector: u16) -> Option<MessageFlowSwitchStore> {
    Some(match selector {
        0 => MessageFlowSwitchStore::LoadedStageMemory,
        1 => MessageFlowSwitchStore::Dungeon,
        2 => MessageFlowSwitchStore::Zone,
        3 => MessageFlowSwitchStore::OneZone,
        _ => return None,
    })
}

fn decode_yaz0(input: &[u8]) -> Result<Vec<u8>, PlannerContractError> {
    require_range(input, 0, 16, "orig.yaz0.header")?;
    if &input[0..4] != b"Yaz0" {
        return Err(PlannerContractError::new(
            "orig.yaz0.magic",
            "must equal Yaz0",
        ));
    }
    let output_size = read_u32(input, 4, "orig.yaz0.output_size")? as usize;
    if output_size > MAX_DECODED_ARCHIVE_BYTES {
        return Err(PlannerContractError::new(
            "orig.yaz0.output_size",
            format!("exceeds bounded limit {MAX_DECODED_ARCHIVE_BYTES}"),
        ));
    }
    let mut output = Vec::with_capacity(output_size);
    let mut cursor = 16_usize;
    while output.len() < output_size {
        let code = *input
            .get(cursor)
            .ok_or_else(|| PlannerContractError::new("orig.yaz0", "truncated code byte"))?;
        cursor += 1;
        for bit in 0..8 {
            if output.len() == output_size {
                break;
            }
            if code & (0x80 >> bit) != 0 {
                output.push(
                    *input.get(cursor).ok_or_else(|| {
                        PlannerContractError::new("orig.yaz0", "truncated literal")
                    })?,
                );
                cursor += 1;
                continue;
            }
            let first = *input.get(cursor).ok_or_else(|| {
                PlannerContractError::new("orig.yaz0", "truncated back-reference")
            })?;
            let second = *input.get(cursor + 1).ok_or_else(|| {
                PlannerContractError::new("orig.yaz0", "truncated back-reference")
            })?;
            cursor += 2;
            let distance = usize::from(((u16::from(first) & 0x0f) << 8) | u16::from(second)) + 1;
            if distance > output.len() {
                return Err(PlannerContractError::new(
                    "orig.yaz0.back_reference",
                    "distance precedes decoded output",
                ));
            }
            let mut length = usize::from(first >> 4);
            if length == 0 {
                length = usize::from(*input.get(cursor).ok_or_else(|| {
                    PlannerContractError::new("orig.yaz0", "truncated long length")
                })?) + 0x12;
                cursor += 1;
            } else {
                length += 2;
            }
            if output.len() + length > output_size {
                return Err(PlannerContractError::new(
                    "orig.yaz0.back_reference",
                    "exceeds declared output size",
                ));
            }
            for _ in 0..length {
                let value = output[output.len() - distance];
                output.push(value);
            }
        }
    }
    Ok(output)
}

fn decode_archive(input: &[u8]) -> Result<Vec<u8>, PlannerContractError> {
    if input.starts_with(b"Yaz0") {
        decode_yaz0(input)
    } else {
        Ok(input.to_vec())
    }
}

struct RarcResourceEntry<'a> {
    name: &'a [u8],
    offset: usize,
    size: usize,
}

fn rarc_resource_entries(
    archive: &[u8],
) -> Result<Vec<RarcResourceEntry<'_>>, PlannerContractError> {
    require_range(archive, 0, 0x40, "orig.rarc.header")?;
    if &archive[0..4] != b"RARC" {
        return Err(PlannerContractError::new(
            "orig.rarc.magic",
            "archive is neither RARC nor Yaz0-wrapped RARC",
        ));
    }
    let declared_size = read_u32(archive, 4, "orig.rarc.size")? as usize;
    if declared_size != archive.len() {
        return Err(PlannerContractError::new(
            "orig.rarc.size",
            "does not equal decoded archive size",
        ));
    }
    let info_base = 0x20_usize;
    let file_count = read_u32(archive, info_base + 8, "orig.rarc.file_count")? as usize;
    if file_count > MAX_RARC_FILE_ENTRIES {
        return Err(PlannerContractError::new(
            "orig.rarc.file_count",
            format!("exceeds bounded limit {MAX_RARC_FILE_ENTRIES}"),
        ));
    }
    let file_table = relative_offset(
        info_base,
        read_u32(archive, info_base + 12, "orig.rarc.file_table")?,
        "orig.rarc.file_table",
    )?;
    let string_table = relative_offset(
        info_base,
        read_u32(archive, info_base + 20, "orig.rarc.string_table")?,
        "orig.rarc.string_table",
    )?;
    let data_base = relative_offset(
        info_base,
        read_u32(archive, 12, "orig.rarc.data_base")?,
        "orig.rarc.data_base",
    )?;
    require_range(
        archive,
        file_table,
        file_count
            .checked_mul(RARC_FILE_ENTRY_SIZE)
            .ok_or_else(|| PlannerContractError::new("orig.rarc.file_table", "size overflow"))?,
        "orig.rarc.file_table",
    )?;
    if string_table >= archive.len() || data_base > archive.len() {
        return Err(PlannerContractError::new(
            "orig.rarc.offset",
            "table or data offset is outside the archive",
        ));
    }

    let mut resources = Vec::new();
    for index in 0..file_count {
        let entry = file_table + index * RARC_FILE_ENTRY_SIZE;
        let flags = read_u16(archive, entry + 4, "orig.rarc.entry.flags")?;
        if flags & 0x0100 == 0 {
            continue;
        }
        let name_offset = usize::from(read_u16(archive, entry + 6, "orig.rarc.entry.name_offset")?);
        let name_start = string_table
            .checked_add(name_offset)
            .ok_or_else(|| PlannerContractError::new("orig.rarc.entry.name", "offset overflow"))?;
        let name = nul_terminated(archive, name_start, "orig.rarc.entry.name")?;
        let offset = relative_offset(
            data_base,
            read_u32(archive, entry + 8, "orig.rarc.entry.offset")?,
            "orig.rarc.entry.offset",
        )?;
        let size = read_u32(archive, entry + 12, "orig.rarc.entry.size")? as usize;
        require_range(archive, offset, size, "orig.rarc.entry.data")?;
        resources.push(RarcResourceEntry { name, offset, size });
    }
    Ok(resources)
}

fn extract_uncompressed_rarc_resource(
    archive: &[u8],
    resource_name: &str,
) -> Result<Vec<u8>, PlannerContractError> {
    let matches = rarc_resource_entries(archive)?
        .into_iter()
        .filter(|entry| entry.name == resource_name.as_bytes())
        .collect::<Vec<_>>();
    let [matched] = matches.as_slice() else {
        if matches.len() > 1 {
            return Err(PlannerContractError::new(
                "orig.rarc.resource",
                format!("contains multiple files named {resource_name:?}"),
            ));
        }
        return Err(PlannerContractError::new(
            "orig.rarc.resource",
            format!("{resource_name:?} was not found"),
        ));
    };
    Ok(archive[matched.offset..matched.offset + matched.size].to_vec())
}

fn read_u16(input: &[u8], offset: usize, field: &str) -> Result<u16, PlannerContractError> {
    require_range(input, offset, 2, field)?;
    Ok(u16::from_be_bytes(
        input[offset..offset + 2].try_into().unwrap(),
    ))
}

fn read_u32(input: &[u8], offset: usize, field: &str) -> Result<u32, PlannerContractError> {
    require_range(input, offset, 4, field)?;
    Ok(u32::from_be_bytes(
        input[offset..offset + 4].try_into().unwrap(),
    ))
}

fn read_i32(input: &[u8], offset: usize, field: &str) -> Result<i32, PlannerContractError> {
    require_range(input, offset, 4, field)?;
    Ok(i32::from_be_bytes(
        input[offset..offset + 4].try_into().unwrap(),
    ))
}

fn read_i16(input: &[u8], offset: usize, field: &str) -> Result<i16, PlannerContractError> {
    require_range(input, offset, 2, field)?;
    Ok(i16::from_be_bytes(
        input[offset..offset + 2].try_into().unwrap(),
    ))
}

fn read_f32(input: &[u8], offset: usize, field: &str) -> Result<f32, PlannerContractError> {
    Ok(f32::from_bits(read_u32(input, offset, field)?))
}

fn relative_offset(base: usize, relative: u32, field: &str) -> Result<usize, PlannerContractError> {
    base.checked_add(relative as usize)
        .ok_or_else(|| PlannerContractError::new(field, "offset overflow"))
}

fn require_range(
    input: &[u8],
    offset: usize,
    size: usize,
    field: &str,
) -> Result<(), PlannerContractError> {
    let end = offset
        .checked_add(size)
        .ok_or_else(|| PlannerContractError::new(field, "range overflow"))?;
    if end > input.len() {
        return Err(PlannerContractError::new(field, "range exceeds input"));
    }
    Ok(())
}

fn nul_terminated<'a>(
    input: &'a [u8],
    offset: usize,
    field: &str,
) -> Result<&'a [u8], PlannerContractError> {
    let tail = input
        .get(offset..)
        .ok_or_else(|| PlannerContractError::new(field, "offset exceeds input"))?;
    let length = tail
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| PlannerContractError::new(field, "is not NUL terminated"))?;
    Ok(&tail[..length])
}

#[cfg(test)]
#[path = "orig_extraction_tests.rs"]
mod tests;
