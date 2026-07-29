//! Minimal, planner-owned extraction for immutable retail message archives.
//!
//! This module deliberately accepts bytes rather than filesystem paths. The
//! runtime CLI owns discovery and I/O; the engine owns bounded deterministic
//! decoding and portable extracted records.

use crate::PlannerContractError;
use serde::{Deserialize, Serialize};

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
pub fn parse_stage_data(input: &[u8]) -> Result<ExtractedStageData, PlannerContractError> {
    let chunk_count = read_u32(input, 0, "orig.stage.chunk_count")? as usize;
    if chunk_count > MAX_STAGE_CHUNKS {
        return Err(PlannerContractError::new(
            "orig.stage.chunk_count",
            format!("exceeds bounded limit {MAX_STAGE_CHUNKS}"),
        ));
    }
    let header_bytes = chunk_count
        .checked_mul(12)
        .ok_or_else(|| PlannerContractError::new("orig.stage.headers", "size overflow"))?;
    require_range(input, 4, header_bytes, "orig.stage.headers")?;
    let records_floor = 4 + header_bytes;
    let mut chunks = Vec::with_capacity(chunk_count);
    let mut stage_information = None;
    let mut room_transforms = Vec::new();
    let mut file_lists = Vec::new();
    let mut room_read_table = Vec::new();
    let mut cameras = Vec::new();
    let mut camera_arrows = Vec::new();
    let mut paths = Vec::new();
    let mut path_points = Vec::new();
    let mut saw_path_table = false;
    let mut saw_path_point_table = false;
    let mut saw_room_read_table = false;
    let mut scene_transitions = Vec::new();
    let mut map_events = Vec::new();
    let mut demo_archive_banks = Vec::new();
    let mut actor_placements = Vec::new();
    let mut treasure_placements = Vec::new();
    let mut player_spawns = Vec::new();
    let mut recognized_ranges = Vec::new();
    let mut total_records = 0_usize;

    for chunk_index in 0..chunk_count {
        let header = 4 + chunk_index * 12;
        let tag_bytes = &input[header..header + 4];
        if !tag_bytes.iter().all(u8::is_ascii_graphic) {
            return Err(PlannerContractError::new(
                "orig.stage.chunk.tag",
                "must contain four printable ASCII bytes",
            ));
        }
        let tag = std::str::from_utf8(tag_bytes)
            .map_err(|_| PlannerContractError::new("orig.stage.chunk.tag", "must be UTF-8"))?
            .to_owned();
        let record_count = read_u32(input, header + 4, "orig.stage.chunk.record_count")?;
        let data_offset = read_u32(input, header + 8, "orig.stage.chunk.data_offset")?;
        let actor_layout = actor_record_layout(&tag);
        let record_size = actor_layout
            .map(|layout| layout.0)
            .or_else(|| recognized_stage_record_size(&tag));
        chunks.push(ExtractedStageChunk {
            tag: tag.clone(),
            record_count,
            data_offset,
            recognized_record_size: record_size.map(|size| size as u8),
        });
        if tag == "RTBL" {
            total_records = total_records
                .checked_add(record_count as usize)
                .ok_or_else(|| PlannerContractError::new("orig.stage.records", "count overflow"))?;
            if total_records > MAX_STAGE_RECORDS {
                return Err(PlannerContractError::new(
                    "orig.stage.records",
                    format!("exceeds bounded limit {MAX_STAGE_RECORDS}"),
                ));
            }
            if saw_room_read_table {
                return Err(PlannerContractError::new(
                    "orig.stage.rtbl",
                    "must contain one unique chunk",
                ));
            }
            saw_room_read_table = true;
            room_read_table = parse_room_read_table(
                input,
                data_offset as usize,
                record_count as usize,
                records_floor,
            )?;
            continue;
        }
        let Some(record_size) = record_size else {
            continue;
        };
        total_records = total_records
            .checked_add(record_count as usize)
            .ok_or_else(|| PlannerContractError::new("orig.stage.records", "count overflow"))?;
        if total_records > MAX_STAGE_RECORDS {
            return Err(PlannerContractError::new(
                "orig.stage.records",
                format!("exceeds bounded limit {MAX_STAGE_RECORDS}"),
            ));
        }
        let start = data_offset as usize;
        if start < records_floor {
            return Err(PlannerContractError::new(
                "orig.stage.chunk.data_offset",
                "overlaps the chunk header table",
            ));
        }
        let bytes = (record_count as usize)
            .checked_mul(record_size)
            .ok_or_else(|| PlannerContractError::new("orig.stage.records", "size overflow"))?;
        require_range(input, start, bytes, "orig.stage.records")?;
        recognized_ranges.push((start, start + bytes, tag.clone()));

        if tag == "STAG" {
            if record_count != 1 || stage_information.is_some() {
                return Err(PlannerContractError::new(
                    "orig.stage.stag",
                    "must contain exactly one unique record",
                ));
            }
            let record = &input[start..start + record_size];
            stage_information = Some(ExtractedStageInformation {
                message_group: record[0x28],
                raw_hex: hex_bytes(record),
            });
            continue;
        }

        if tag == "SCLS" {
            for exit_id in 0..record_count {
                let offset = start + exit_id as usize * record_size;
                let record = &input[offset..offset + record_size];
                let name_end = record[..8].iter().position(|byte| *byte == 0).unwrap_or(8);
                if name_end == 0 || !record[..name_end].iter().all(u8::is_ascii_graphic) {
                    return Err(PlannerContractError::new(
                        "orig.stage.scls.destination_stage",
                        "must contain a nonempty printable ASCII stage name",
                    ));
                }
                let destination_stage = std::str::from_utf8(&record[..name_end])
                    .map_err(|_| {
                        PlannerContractError::new(
                            "orig.stage.scls.destination_stage",
                            "must be UTF-8",
                        )
                    })?
                    .to_owned();
                let raw_layer = record[0x0b] & 0x0f;
                let raw_time = ((record[0x0a] >> 4) & 0x0f) | (record[0x0b] & 0x10);
                scene_transitions.push(ExtractedSceneTransition {
                    exit_id,
                    destination_stage,
                    destination_spawn: record[0x08],
                    destination_room: record[0x09] as i8,
                    scene_layer: (raw_layer < 15).then_some(raw_layer),
                    time_hour: (raw_time < 31).then_some(raw_time),
                    wipe: record[0x0c],
                    wipe_time: (record[0x0b] >> 5) & 7,
                    raw_hex: hex_bytes(record),
                });
            }
            continue;
        }

        if tag == "MULT" {
            for record_index in 0..record_count {
                let offset = start + record_index as usize * record_size;
                let record = &input[offset..offset + record_size];
                let translation_xz = [
                    read_f32(record, 0, "orig.stage.mult.translation_x")?,
                    read_f32(record, 4, "orig.stage.mult.translation_z")?,
                ];
                if !translation_xz
                    .iter()
                    .all(|coordinate| coordinate.is_finite())
                {
                    return Err(PlannerContractError::new(
                        "orig.stage.mult.translation_xz",
                        "must be finite",
                    ));
                }
                room_transforms.push(ExtractedRoomTransform {
                    record_index,
                    room: record[0x0a],
                    translation_xz,
                    angle_y: read_i16(record, 8, "orig.stage.mult.angle_y")?,
                    trailing_byte: record[0x0b],
                    raw_hex: hex_bytes(record),
                });
            }
            continue;
        }

        if tag == "FILI" {
            for record_index in 0..record_count {
                let offset = start + record_index as usize * record_size;
                let record = &input[offset..offset + record_size];
                let parameters = read_u32(record, 0, "orig.stage.fili.parameters")?;
                let sea_level = read_f32(record, 4, "orig.stage.fili.sea_level")?;
                let unknown_float_08 = read_f32(record, 8, "orig.stage.fili.unknown_float_08")?;
                let unknown_float_0c = read_f32(record, 12, "orig.stage.fili.unknown_float_0c")?;
                if ![sea_level, unknown_float_08, unknown_float_0c]
                    .iter()
                    .all(|value| value.is_finite())
                {
                    return Err(PlannerContractError::new(
                        "orig.stage.fili.floats",
                        "must be finite",
                    ));
                }
                let default_camera = record[0x1a];
                let bit_switch = record[0x1b];
                let message_id = read_u16(record, 0x1c, "orig.stage.fili.message_id")?;
                file_lists.push(ExtractedFileList {
                    record_index,
                    parameters,
                    sea_level,
                    unknown_float_08,
                    unknown_float_0c,
                    unknown_bytes_10_19_hex: hex_bytes(&record[0x10..0x1a]),
                    minimap_style: ((parameters >> 3) & 7) as u8,
                    enemy_appear_flag: parameters & 0x2000_0000 != 0,
                    global_wind_level: ((parameters >> 18) & 3) as u8,
                    global_wind_direction: ((parameters >> 15) & 7) as u8,
                    grass_light: ((parameters >> 7) & 0xff) as u8,
                    default_camera,
                    bit_switch,
                    message_id,
                    raw_hex: hex_bytes(record),
                });
            }
            continue;
        }

        if tag == "RCAM" {
            for record_index in 0..record_count {
                let offset = start + record_index as usize * record_size;
                let record = &input[offset..offset + record_size];
                let raw_type_index = read_u16(record, 0x16, "orig.stage.rcam.camera_type_index")?;
                cameras.push(ExtractedCamera {
                    record_index,
                    camera_type: parse_fixed_ascii(
                        &record[..0x10],
                        "orig.stage.rcam.camera_type",
                        false,
                    )?,
                    arrow_index: record[0x10],
                    field_of_view_y: record[0x11],
                    argument_0: record[0x12],
                    argument_1: record[0x13],
                    argument_2: read_u16(record, 0x14, "orig.stage.rcam.argument_2")?,
                    camera_type_index: (raw_type_index != u16::MAX).then_some(raw_type_index),
                    raw_hex: hex_bytes(record),
                });
            }
            continue;
        }

        if tag == "RARO" {
            for record_index in 0..record_count {
                let offset = start + record_index as usize * record_size;
                let record = &input[offset..offset + record_size];
                let position = [
                    read_f32(record, 0, "orig.stage.raro.position_x")?,
                    read_f32(record, 4, "orig.stage.raro.position_y")?,
                    read_f32(record, 8, "orig.stage.raro.position_z")?,
                ];
                if !position.iter().all(|coordinate| coordinate.is_finite()) {
                    return Err(PlannerContractError::new(
                        "orig.stage.raro.position",
                        "must be finite",
                    ));
                }
                camera_arrows.push(ExtractedCameraArrow {
                    record_index,
                    position,
                    angle: [
                        read_i16(record, 0x0c, "orig.stage.raro.angle_x")?,
                        read_i16(record, 0x0e, "orig.stage.raro.angle_y")?,
                        read_i16(record, 0x10, "orig.stage.raro.angle_z")?,
                    ],
                    trailing_i16: read_i16(record, 0x12, "orig.stage.raro.trailing_i16")?,
                    raw_hex: hex_bytes(record),
                });
            }
            continue;
        }

        if tag == "RPAT" {
            if saw_path_table {
                return Err(PlannerContractError::new(
                    "orig.stage.rpat",
                    "must contain one unique chunk",
                ));
            }
            saw_path_table = true;
            for record_index in 0..record_count {
                let offset = start + record_index as usize * record_size;
                let record = &input[offset..offset + record_size];
                let point_offset = read_u32(record, 8, "orig.stage.rpat.point_offset")?;
                if point_offset as usize % 0x10 != 0 {
                    return Err(PlannerContractError::new(
                        "orig.stage.rpat.point_offset",
                        "must align to an RPPN record",
                    ));
                }
                let next_raw = read_u16(record, 2, "orig.stage.rpat.next_path_index")?;
                paths.push(ExtractedPath {
                    record_index,
                    point_count: read_u16(record, 0, "orig.stage.rpat.point_count")?,
                    next_path_index: (next_raw != u16::MAX).then_some(next_raw),
                    path_argument: record[4],
                    closed: record[5] & 1 != 0,
                    closed_raw: record[5],
                    switch_no: (record[6] != u8::MAX).then_some(record[6]),
                    unknown_07: record[7],
                    point_offset,
                    first_point_index: point_offset / 0x10,
                    raw_hex: hex_bytes(record),
                });
            }
            continue;
        }

        if tag == "RPPN" {
            if saw_path_point_table {
                return Err(PlannerContractError::new(
                    "orig.stage.rppn",
                    "must contain one unique chunk",
                ));
            }
            saw_path_point_table = true;
            for record_index in 0..record_count {
                let offset = start + record_index as usize * record_size;
                let record = &input[offset..offset + record_size];
                let position = [
                    read_f32(record, 4, "orig.stage.rppn.position_x")?,
                    read_f32(record, 8, "orig.stage.rppn.position_y")?,
                    read_f32(record, 12, "orig.stage.rppn.position_z")?,
                ];
                if !position.iter().all(|coordinate| coordinate.is_finite()) {
                    return Err(PlannerContractError::new(
                        "orig.stage.rppn.position",
                        "must be finite",
                    ));
                }
                path_points.push(ExtractedPathPoint {
                    record_index,
                    arguments: [record[3], record[0], record[1], record[2]],
                    position,
                    raw_hex: hex_bytes(record),
                });
            }
            continue;
        }

        if tag == "REVT" {
            for record_index in 0..record_count {
                let offset = start + record_index as usize * record_size;
                let record = &input[offset..offset + record_size];
                let event_type = record[0];
                if event_type > 2 {
                    return Err(PlannerContractError::new(
                        "orig.stage.revt.event_type",
                        "is outside the source-audited 0..=2 dispatch",
                    ));
                }
                let event_name = if matches!(event_type, 1 | 2) {
                    Some(parse_fixed_ascii(
                        &record[0x0d..0x1a],
                        "orig.stage.revt.event_name",
                        false,
                    )?)
                } else {
                    None
                };
                map_events.push(ExtractedMapEvent {
                    record_index,
                    event_type,
                    map_tool_id: record[4],
                    priority: record[6],
                    normal_exit_id: {
                        let exit_id = if event_type == 0 {
                            record[0x17]
                        } else {
                            record[7]
                        };
                        (exit_id != u8::MAX).then_some(exit_id)
                    },
                    skip_exit_id: (record[9] != u8::MAX).then_some(record[9]),
                    event_name,
                    switch_no: (record[0x1b] != u8::MAX).then_some(record[0x1b]),
                    raw_hex: hex_bytes(record),
                });
            }
            continue;
        }

        if tag == "LBNK" {
            for layer in 0..record_count {
                let offset = start + layer as usize * record_size;
                let record = &input[offset..offset + record_size];
                let bank = (record[0] != u8::MAX).then_some(record[0]);
                let bank2 = (record[1] != u8::MAX).then_some(record[1]);
                if let Some(value) = bank
                    && (value >= 100 || bank2.is_none_or(|value| value >= 100))
                {
                    return Err(PlannerContractError::new(
                        "orig.stage.lbnk",
                        "configured demo archive bank coordinates must be below 100",
                    ));
                }
                demo_archive_banks.push(ExtractedDemoArchiveBank {
                    layer: layer.try_into().map_err(|_| {
                        PlannerContractError::new(
                            "orig.stage.lbnk.layer",
                            "must fit in one layer byte",
                        )
                    })?,
                    bank,
                    bank2,
                    archive_name: bank
                        .zip(bank2)
                        .map(|(bank, bank2)| format!("Demo{bank:02}_{bank2:02}")),
                    raw_hex: hex_bytes(record),
                });
            }
            continue;
        }

        let Some((_, scaled, layer, placement_class)) = actor_layout else {
            unreachable!("all other recognized records are actor placements")
        };

        for record_index in 0..record_count {
            let offset = start + record_index as usize * record_size;
            let record = &input[offset..offset + record_size];
            let name_end = record[..8].iter().position(|byte| *byte == 0).unwrap_or(8);
            if !record[..name_end].iter().all(u8::is_ascii_graphic) {
                return Err(PlannerContractError::new(
                    "orig.stage.actor.name",
                    "must contain printable ASCII bytes",
                ));
            }
            let name = std::str::from_utf8(&record[..name_end])
                .map_err(|_| PlannerContractError::new("orig.stage.actor.name", "must be UTF-8"))?
                .to_owned();
            let position = [
                read_f32(record, 12, "orig.stage.actor.position_x")?,
                read_f32(record, 16, "orig.stage.actor.position_y")?,
                read_f32(record, 20, "orig.stage.actor.position_z")?,
            ];
            if !position.iter().all(|coordinate| coordinate.is_finite()) {
                return Err(PlannerContractError::new(
                    "orig.stage.actor.position",
                    "must be finite",
                ));
            }
            let placement = ExtractedActorPlacement {
                chunk_tag: tag.clone(),
                record_index,
                layer,
                name,
                parameters: read_u32(record, 8, "orig.stage.actor.parameters")?,
                position,
                angle: [
                    read_i16(record, 24, "orig.stage.actor.angle_x")?,
                    read_i16(record, 26, "orig.stage.actor.angle_y")?,
                    read_i16(record, 28, "orig.stage.actor.angle_z")?,
                ],
                set_id: read_u16(record, 30, "orig.stage.actor.set_id")?,
                scale_raw: scaled.then(|| [record[32], record[33], record[34]]),
                raw_hex: hex_bytes(record),
            };
            match placement_class {
                ExtractedPlacementClass::Actor => actor_placements.push(placement),
                ExtractedPlacementClass::Treasure => treasure_placements.push(placement),
                ExtractedPlacementClass::PlayerSpawn => player_spawns.push(placement),
            }
        }
    }
    recognized_ranges.sort_by_key(|range| range.0);
    for pair in recognized_ranges.windows(2) {
        if pair[0].1 > pair[1].0 {
            return Err(PlannerContractError::new(
                "orig.stage.records",
                format!(
                    "recognized chunks {:?} and {:?} overlap",
                    pair[0].2, pair[1].2
                ),
            ));
        }
    }
    for path in &paths {
        let first = path.first_point_index as usize;
        let end = first
            .checked_add(usize::from(path.point_count))
            .ok_or_else(|| PlannerContractError::new("orig.stage.rpat", "point range overflow"))?;
        if !saw_path_point_table
            || end > path_points.len()
            || path
                .next_path_index
                .is_some_and(|next| usize::from(next) >= paths.len())
        {
            return Err(PlannerContractError::new(
                "orig.stage.rpat",
                "contains an out-of-range RPPN span or next-path index",
            ));
        }
    }
    Ok(ExtractedStageData {
        chunks,
        stage_information,
        room_transforms,
        file_lists,
        room_read_table,
        cameras,
        camera_arrows,
        paths,
        path_points,
        scene_transitions,
        map_events,
        demo_archive_banks,
        actor_placements,
        treasure_placements,
        player_spawns,
    })
}

/// Decode the engine's fixed-table `event_list.dat` format. This captures the
/// authored event/staff/cut/data graph; it does not infer actor callbacks or
/// JStudio `.stb` contents.
pub fn parse_event_list(input: &[u8]) -> Result<ExtractedEventList, PlannerContractError> {
    const HEADER_SIZE: usize = 0x40;
    const EVENT_SIZE: usize = 0xb0;
    const STAFF_SIZE: usize = 0x50;
    const CUT_SIZE: usize = 0x50;
    const DATA_SIZE: usize = 0x40;

    require_range(input, 0, HEADER_SIZE, "orig.event_list.header")?;
    let table = |offset: usize,
                 record_size: usize,
                 field: &'static str|
     -> Result<(usize, usize), PlannerContractError> {
        let start = read_u32(input, offset, field)? as usize;
        let count = read_i32(input, offset + 4, field)?;
        if count < 0 || count as usize > MAX_EVENT_RECORDS {
            return Err(PlannerContractError::new(
                field,
                format!("count must be between 0 and {MAX_EVENT_RECORDS}"),
            ));
        }
        let bytes = (count as usize)
            .checked_mul(record_size)
            .ok_or_else(|| PlannerContractError::new(field, "size overflow"))?;
        if start < HEADER_SIZE && bytes != 0 {
            return Err(PlannerContractError::new(field, "overlaps the header"));
        }
        require_range(input, start, bytes, field)?;
        Ok((start, count as usize))
    };

    let (event_top, event_count) = table(0x00, EVENT_SIZE, "orig.event_list.events")?;
    let (staff_top, staff_count) = table(0x08, STAFF_SIZE, "orig.event_list.staff")?;
    let (cut_top, cut_count) = table(0x10, CUT_SIZE, "orig.event_list.cuts")?;
    let (data_top, data_count) = table(0x18, DATA_SIZE, "orig.event_list.data")?;
    let (float_top, float_count) = table(0x20, 4, "orig.event_list.float_data")?;
    let (integer_top, integer_count) = table(0x28, 4, "orig.event_list.integer_data")?;
    let (string_top, string_count) = table(0x30, 1, "orig.event_list.string_data")?;

    let mut ranges = [
        (event_top, event_top + event_count * EVENT_SIZE, "events"),
        (staff_top, staff_top + staff_count * STAFF_SIZE, "staff"),
        (cut_top, cut_top + cut_count * CUT_SIZE, "cuts"),
        (data_top, data_top + data_count * DATA_SIZE, "data"),
        (float_top, float_top + float_count * 4, "float_data"),
        (integer_top, integer_top + integer_count * 4, "integer_data"),
        (string_top, string_top + string_count, "string_data"),
    ];
    ranges.sort_by_key(|range| range.0);
    let nonempty_ranges = ranges
        .iter()
        .filter(|range| range.0 != range.1)
        .collect::<Vec<_>>();
    for pair in nonempty_ranges.windows(2) {
        if pair[0].1 > pair[1].0 {
            return Err(PlannerContractError::new(
                "orig.event_list.tables",
                format!("tables {} and {} overlap", pair[0].2, pair[1].2),
            ));
        }
    }

    let float_data_bits = (0..float_count)
        .map(|index| read_u32(input, float_top + index * 4, "orig.event_list.float_data"))
        .collect::<Result<Vec<_>, _>>()?;
    let integer_data = (0..integer_count)
        .map(|index| {
            read_i32(
                input,
                integer_top + index * 4,
                "orig.event_list.integer_data",
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let string_data = &input[string_top..string_top + string_count];

    let mut events = Vec::with_capacity(event_count);
    for index in 0..event_count {
        let offset = event_top + index * EVENT_SIZE;
        let record = &input[offset..offset + EVENT_SIZE];
        require_dense_index(record, 0x20, index, "orig.event_list.event.index")?;
        let staff_in_event = read_i32(record, 0x7c, "orig.event_list.event.staff_count")?;
        if !(0..=20).contains(&staff_in_event) {
            return Err(PlannerContractError::new(
                "orig.event_list.event.staff_count",
                "must be between 0 and 20",
            ));
        }
        let mut staff_indices = Vec::with_capacity(staff_in_event as usize);
        for ordinal in 0..staff_in_event as usize {
            let staff_index = read_i32(
                record,
                0x2c + ordinal * 4,
                "orig.event_list.event.staff_index",
            )?;
            if staff_index < 0 || staff_index as usize >= staff_count {
                return Err(PlannerContractError::new(
                    "orig.event_list.event.staff_index",
                    "references a staff record outside the table",
                ));
            }
            staff_indices.push(staff_index as u32);
        }
        events.push(ExtractedEvent {
            index: index as u32,
            name: parse_fixed_ascii(&record[..0x20], "orig.event_list.event.name", false)?,
            priority: read_i32(record, 0x28, "orig.event_list.event.priority")?,
            staff_indices,
            finish_flags: [
                read_i32(record, 0x88, "orig.event_list.event.start_flag")?,
                read_i32(record, 0x8c, "orig.event_list.event.start_flag")?,
                read_i32(record, 0x90, "orig.event_list.event.start_flag")?,
            ],
            raw_hex: hex_bytes(record),
        });
    }

    let mut staff = Vec::with_capacity(staff_count);
    for index in 0..staff_count {
        let offset = staff_top + index * STAFF_SIZE;
        let record = &input[offset..offset + STAFF_SIZE];
        require_dense_index(record, 0x24, index, "orig.event_list.staff.index")?;
        let start_cut = read_i32(record, 0x30, "orig.event_list.staff.start_cut")?;
        if start_cut < 0 || start_cut as usize >= cut_count {
            return Err(PlannerContractError::new(
                "orig.event_list.staff.start_cut",
                "references a cut outside the table",
            ));
        }
        staff.push(ExtractedEventStaff {
            index: index as u32,
            name: parse_fixed_ascii(&record[..8], "orig.event_list.staff.name", false)?,
            tag_id: read_i32(record, 0x20, "orig.event_list.staff.tag_id")?,
            flag_id: read_u32(record, 0x28, "orig.event_list.staff.flag_id")?,
            staff_type: read_i32(record, 0x2c, "orig.event_list.staff.type")?,
            start_cut_index: start_cut as u32,
            raw_hex: hex_bytes(record),
        });
    }

    let mut cuts = Vec::with_capacity(cut_count);
    for index in 0..cut_count {
        let offset = cut_top + index * CUT_SIZE;
        let record = &input[offset..offset + CUT_SIZE];
        require_dense_index(record, 0x24, index, "orig.event_list.cut.index")?;
        cuts.push(ExtractedEventCut {
            index: index as u32,
            name: parse_fixed_ascii(&record[..0x20], "orig.event_list.cut.name", false)?,
            tag_id: read_u32(record, 0x20, "orig.event_list.cut.tag_id")?,
            start_flags: [
                read_i32(record, 0x28, "orig.event_list.cut.start_flag")?,
                read_i32(record, 0x2c, "orig.event_list.cut.start_flag")?,
                read_i32(record, 0x30, "orig.event_list.cut.start_flag")?,
            ],
            flag_id: read_u32(record, 0x34, "orig.event_list.cut.flag_id")?,
            data_index: optional_table_index(
                read_i32(record, 0x38, "orig.event_list.cut.data_index")?,
                data_count,
                "orig.event_list.cut.data_index",
            )?,
            next_cut_index: optional_table_index(
                read_i32(record, 0x3c, "orig.event_list.cut.next_cut_index")?,
                cut_count,
                "orig.event_list.cut.next_cut_index",
            )?,
            raw_hex: hex_bytes(record),
        });
    }

    let mut data = Vec::with_capacity(data_count);
    for index in 0..data_count {
        let offset = data_top + index * DATA_SIZE;
        let record = &input[offset..offset + DATA_SIZE];
        require_dense_index(record, 0x20, index, "orig.event_list.data.index")?;
        let data_type = read_i32(record, 0x24, "orig.event_list.data.type")?;
        let value_index = read_i32(record, 0x28, "orig.event_list.data.value_index")?;
        let value_count = read_i32(record, 0x2c, "orig.event_list.data.value_count")?;
        if value_index < 0 || value_count <= 0 {
            return Err(PlannerContractError::new(
                "orig.event_list.data.value",
                "must have a nonnegative index and positive count",
            ));
        }
        let value_index = value_index as usize;
        let value_count = value_count as usize;
        let value = match data_type {
            0..=2 => {
                let values = slice_values(
                    &float_data_bits,
                    value_index,
                    value_count,
                    "orig.event_list.data.float_value",
                )?
                .to_vec();
                match data_type {
                    0 => ExtractedEventDataValue::FloatBits { values },
                    1 => ExtractedEventDataValue::VectorBits { values },
                    2 => ExtractedEventDataValue::UnknownFloatBits { values },
                    _ => unreachable!(),
                }
            }
            3 => ExtractedEventDataValue::Integers {
                values: slice_values(
                    &integer_data,
                    value_index,
                    value_count,
                    "orig.event_list.data.integer_value",
                )?
                .to_vec(),
            },
            4 => {
                let bytes = slice_values(
                    string_data,
                    value_index,
                    value_count,
                    "orig.event_list.data.string_value",
                )?;
                let end = bytes
                    .iter()
                    .position(|byte| *byte == 0)
                    .unwrap_or(bytes.len());
                let ascii = (bytes[..end].iter().all(u8::is_ascii_graphic)
                    && bytes[end..].iter().all(|byte| *byte == 0))
                .then(|| std::str::from_utf8(&bytes[..end]).ok().map(str::to_owned))
                .flatten();
                ExtractedEventDataValue::StringBytes {
                    raw_hex: hex_bytes(bytes),
                    ascii,
                }
            }
            _ => {
                return Err(PlannerContractError::new(
                    "orig.event_list.data.type",
                    "is outside the source-audited 0..=4 dispatch",
                ));
            }
        };
        data.push(ExtractedEventData {
            index: index as u32,
            name: parse_fixed_ascii(&record[..0x20], "orig.event_list.data.name", false)?,
            data_type,
            value_index: value_index as u32,
            value_count: value_count as u32,
            next_data_index: optional_table_index(
                read_i32(record, 0x30, "orig.event_list.data.next_data_index")?,
                data_count,
                "orig.event_list.data.next_data_index",
            )?,
            value,
            raw_hex: hex_bytes(record),
        });
    }

    Ok(ExtractedEventList {
        resource_size: input.len().try_into().map_err(|_| {
            PlannerContractError::new("orig.event_list", "resource size exceeds u32")
        })?,
        events,
        staff,
        cuts,
        data,
        float_data_bits,
        integer_data,
        string_data_hex: hex_bytes(string_data),
    })
}

fn require_dense_index(
    record: &[u8],
    offset: usize,
    expected: usize,
    field: &'static str,
) -> Result<(), PlannerContractError> {
    if read_u32(record, offset, field)? as usize != expected {
        return Err(PlannerContractError::new(
            field,
            "must equal the record's dense table index",
        ));
    }
    Ok(())
}

fn optional_table_index(
    value: i32,
    count: usize,
    field: &'static str,
) -> Result<Option<u32>, PlannerContractError> {
    if value == -1 {
        return Ok(None);
    }
    if value < 0 || value as usize >= count {
        return Err(PlannerContractError::new(
            field,
            "references a record outside its table",
        ));
    }
    Ok(Some(value as u32))
}

fn slice_values<'a, T>(
    values: &'a [T],
    start: usize,
    count: usize,
    field: &'static str,
) -> Result<&'a [T], PlannerContractError> {
    let end = start
        .checked_add(count)
        .ok_or_else(|| PlannerContractError::new(field, "range overflow"))?;
    values
        .get(start..end)
        .ok_or_else(|| PlannerContractError::new(field, "range exceeds its backing table"))
}

fn parse_room_read_table(
    input: &[u8],
    table_offset: usize,
    room_count: usize,
    records_floor: usize,
) -> Result<Vec<ExtractedRoomRead>, PlannerContractError> {
    if room_count > 64 {
        return Err(PlannerContractError::new(
            "orig.stage.rtbl.room_count",
            "exceeds the source-audited 64-room control table",
        ));
    }
    if table_offset < records_floor {
        return Err(PlannerContractError::new(
            "orig.stage.rtbl.table_offset",
            "overlaps the chunk header table",
        ));
    }
    let table_bytes = room_count
        .checked_mul(4)
        .ok_or_else(|| PlannerContractError::new("orig.stage.rtbl", "table size overflow"))?;
    require_range(input, table_offset, table_bytes, "orig.stage.rtbl.table")?;
    let mut rooms = Vec::with_capacity(room_count);
    for room_index in 0..room_count {
        let pointer_offset = table_offset + room_index * 4;
        let record_offset = read_u32(input, pointer_offset, "orig.stage.rtbl.record_offset")?;
        let record_start = record_offset as usize;
        if record_start < records_floor {
            return Err(PlannerContractError::new(
                "orig.stage.rtbl.record_offset",
                "must follow the chunk header table",
            ));
        }
        require_range(input, record_start, 8, "orig.stage.rtbl.record")?;
        let raw_header = &input[record_start..record_start + 8];
        let load_count = usize::from(raw_header[0]);
        let room_list_offset = read_u32(raw_header, 4, "orig.stage.rtbl.room_list_offset")?;
        let room_list_start = room_list_offset as usize;
        if room_list_start < records_floor && load_count != 0 {
            return Err(PlannerContractError::new(
                "orig.stage.rtbl.room_list_offset",
                "overlaps the chunk header table",
            ));
        }
        require_range(
            input,
            room_list_start,
            load_count,
            "orig.stage.rtbl.room_list",
        )?;
        let raw_room_list = &input[room_list_start..room_list_start + load_count];
        let load_rooms = raw_room_list
            .iter()
            .map(|raw| ExtractedLoadedRoom {
                room: raw & 0x3f,
                load_background: raw & 0x80 != 0,
                unknown_bit_6: raw & 0x40 != 0,
                raw: *raw,
            })
            .collect();
        rooms.push(ExtractedRoomRead {
            room_index: room_index as u32,
            record_offset,
            room_list_offset,
            reverb: raw_header[1] & 0x7f,
            reverb_raw: raw_header[1],
            time_pass: raw_header[2] & 3,
            vrbox_enabled: raw_header[2] & 8 != 0,
            flags_raw: raw_header[2],
            padding: raw_header[3],
            load_rooms,
            raw_header_hex: hex_bytes(raw_header),
            raw_room_list_hex: hex_bytes(raw_room_list),
        });
    }
    Ok(rooms)
}

fn recognized_stage_record_size(tag: &str) -> Option<usize> {
    match tag {
        "STAG" => Some(0x3c),
        "SCLS" => Some(0x0d),
        "REVT" => Some(0x1c),
        "LBNK" => Some(0x03),
        "MULT" => Some(0x0c),
        "FILI" => Some(0x20),
        "RCAM" => Some(0x18),
        "RARO" => Some(0x14),
        "RPAT" => Some(0x0c),
        "RPPN" => Some(0x10),
        _ => None,
    }
}

fn parse_fixed_ascii(
    bytes: &[u8],
    field: &'static str,
    allow_empty: bool,
) -> Result<String, PlannerContractError> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    if (!allow_empty && end == 0) || !bytes[..end].iter().all(u8::is_ascii_graphic) {
        return Err(PlannerContractError::new(
            field,
            "must contain printable ASCII before its first NUL",
        ));
    }
    std::str::from_utf8(&bytes[..end])
        .map(str::to_owned)
        .map_err(|_| PlannerContractError::new(field, "must be UTF-8"))
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Clone, Copy)]
enum ExtractedPlacementClass {
    Actor,
    Treasure,
    PlayerSpawn,
}

fn actor_record_layout(tag: &str) -> Option<(usize, bool, Option<u8>, ExtractedPlacementClass)> {
    if tag == "PLYR" {
        return Some((0x20, false, None, ExtractedPlacementClass::PlayerSpawn));
    }
    if tag == "TRES" {
        return Some((0x20, false, None, ExtractedPlacementClass::Treasure));
    }
    if matches!(tag, "ACTR" | "TGOB") {
        return Some((0x20, false, None, ExtractedPlacementClass::Actor));
    }
    if matches!(tag, "SCOB" | "TGSC" | "TGDR" | "Door") {
        return Some((0x24, true, None, ExtractedPlacementClass::Actor));
    }
    if tag.len() != 4 {
        return None;
    }
    let (prefix, scaled, placement_class) = match &tag[..3] {
        "ACT" => ("ACT", false, ExtractedPlacementClass::Actor),
        "TRE" => ("TRE", false, ExtractedPlacementClass::Treasure),
        "SCO" | "Doo" => (&tag[..3], true, ExtractedPlacementClass::Actor),
        _ => return None,
    };
    debug_assert_eq!(prefix, &tag[..3]);
    decode_layer(tag.as_bytes()[3]).map(|layer| {
        if scaled {
            (0x24, true, Some(layer), placement_class)
        } else {
            (0x20, false, Some(layer), placement_class)
        }
    })
}

fn decode_layer(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'e' => Some(byte - b'a' + 10),
        b'A'..=b'E' => Some(byte - b'A' + 10),
        _ => None,
    }
}

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
