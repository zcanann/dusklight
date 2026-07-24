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
pub const EXTRACTED_STAGE_DATA_SCHEMA: &str = "dusklight.route-planner.extracted-stage-data/v5";
pub const EXTRACTED_EVENT_LIST_SCHEMA: &str = "dusklight.route-planner.extracted-event-list/v1";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedStageData {
    pub chunks: Vec<ExtractedStageChunk>,
    pub stage_information: Option<ExtractedStageInformation>,
    pub room_transforms: Vec<ExtractedRoomTransform>,
    pub file_lists: Vec<ExtractedFileList>,
    pub scene_transitions: Vec<ExtractedSceneTransition>,
    pub map_events: Vec<ExtractedMapEvent>,
    pub demo_archive_banks: Vec<ExtractedDemoArchiveBank>,
    pub actor_placements: Vec<ExtractedActorPlacement>,
    pub treasure_placements: Vec<ExtractedActorPlacement>,
    pub player_spawns: Vec<ExtractedActorPlacement>,
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
                if !translation_xz.iter().all(|coordinate| coordinate.is_finite()) {
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
    Ok(ExtractedStageData {
        chunks,
        stage_information,
        room_transforms,
        file_lists,
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

fn recognized_stage_record_size(tag: &str) -> Option<usize> {
    match tag {
        "STAG" => Some(0x3c),
        "SCLS" => Some(0x0d),
        "REVT" => Some(0x1c),
        "LBNK" => Some(0x03),
        "MULT" => Some(0x0c),
        "FILI" => Some(0x20),
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
mod tests {
    use super::*;

    fn event_list_fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x314];
        for (header, top, count) in [
            (0x00, 0x40_u32, 1_i32),
            (0x08, 0xf0, 2),
            (0x10, 0x190, 3),
            (0x18, 0x280, 2),
            (0x20, 0x300, 0),
            (0x28, 0x300, 1),
            (0x30, 0x304, 16),
        ] {
            bytes[header..header + 4].copy_from_slice(&top.to_be_bytes());
            bytes[header + 4..header + 8].copy_from_slice(&count.to_be_bytes());
        }

        let event = &mut bytes[0x40..0xf0];
        event[..10].copy_from_slice(b"demo07_02\0");
        event[0x20..0x24].copy_from_slice(&0_u32.to_be_bytes());
        event[0x28..0x2c].copy_from_slice(&100_i32.to_be_bytes());
        event[0x2c..0x30].copy_from_slice(&0_i32.to_be_bytes());
        event[0x30..0x34].copy_from_slice(&1_i32.to_be_bytes());
        event[0x7c..0x80].copy_from_slice(&2_i32.to_be_bytes());
        for offset in [0x88, 0x8c, 0x90] {
            event[offset..offset + 4].copy_from_slice(&(-1_i32).to_be_bytes());
        }

        for (index, name, staff_type, start_cut) in [
            (0_usize, b"PACKAGE".as_slice(), 11_i32, 0_i32),
            (1, b"DIRECTOR", 6, 2),
        ] {
            let start = 0xf0 + index * 0x50;
            let record = &mut bytes[start..start + 0x50];
            record[..name.len()].copy_from_slice(name);
            record[0x24..0x28].copy_from_slice(&(index as u32).to_be_bytes());
            record[0x2c..0x30].copy_from_slice(&staff_type.to_be_bytes());
            record[0x30..0x34].copy_from_slice(&start_cut.to_be_bytes());
        }

        for (index, name, data_index, next_cut) in [
            (0_usize, "PLAY", 0_i32, 1_i32),
            (1, "WAIT", -1, -1),
            (2, "MAPTOOL", 1, -1),
        ] {
            let start = 0x190 + index * 0x50;
            let record = &mut bytes[start..start + 0x50];
            record[..name.len()].copy_from_slice(name.as_bytes());
            record[0x24..0x28].copy_from_slice(&(index as u32).to_be_bytes());
            for offset in [0x28, 0x2c, 0x30] {
                record[offset..offset + 4].copy_from_slice(&(-1_i32).to_be_bytes());
            }
            record[0x34..0x38].copy_from_slice(&(3_u32 + index as u32).to_be_bytes());
            record[0x38..0x3c].copy_from_slice(&data_index.to_be_bytes());
            record[0x3c..0x40].copy_from_slice(&next_cut.to_be_bytes());
        }

        for (index, name, data_type, value_index, value_count) in [
            (0_usize, "FileName", 4_i32, 0_i32, 16_i32),
            (1, "ID", 3, 0, 1),
        ] {
            let start = 0x280 + index * 0x40;
            let record = &mut bytes[start..start + 0x40];
            record[..name.len()].copy_from_slice(name.as_bytes());
            record[0x20..0x24].copy_from_slice(&(index as u32).to_be_bytes());
            record[0x24..0x28].copy_from_slice(&data_type.to_be_bytes());
            record[0x28..0x2c].copy_from_slice(&value_index.to_be_bytes());
            record[0x2c..0x30].copy_from_slice(&value_count.to_be_bytes());
            record[0x30..0x34].copy_from_slice(&(-1_i32).to_be_bytes());
        }
        bytes[0x300..0x304].copy_from_slice(&4_i32.to_be_bytes());
        bytes[0x304..0x312].copy_from_slice(b"demo07_02.stb\0");
        bytes
    }

    fn bmg_fixture() -> Vec<u8> {
        let mut bmg = vec![0; 0x20];
        bmg[0..8].copy_from_slice(b"MESGbmg1");
        bmg[12..16].copy_from_slice(&2_u32.to_be_bytes());

        let mut flw1 = vec![0; 0x50];
        flw1[0..4].copy_from_slice(b"FLW1");
        let flw1_size = flw1.len() as u32;
        flw1[4..8].copy_from_slice(&flw1_size.to_be_bytes());
        flw1[8..10].copy_from_slice(&6_u16.to_be_bytes());
        flw1[10..12].copy_from_slice(&2_u16.to_be_bytes());
        flw1[0x10..0x18].copy_from_slice(&[3, 10, 0, 0, 0, 10, 0, 51]);
        flw1[0x18..0x20].copy_from_slice(&[2, 0, 0, 10, 0, 11, 0, 0]);
        flw1[0x20..0x28].copy_from_slice(&[1, 0, 0, 7, 0xff, 0xff, 0, 0]);
        flw1[0x28..0x30].copy_from_slice(&[3, 0, 0, 0, 0, 62, 0, 0]);
        flw1[0x30..0x38].copy_from_slice(&[2, 0, 0, 1, 0, 62, 0, 0]);
        flw1[0x38..0x40].copy_from_slice(&[3, 14, 0, 0, 0, 3, 0, 10]);
        flw1[0x40..0x42].copy_from_slice(&2_u16.to_be_bytes());
        flw1[0x42..0x44].copy_from_slice(&u16::MAX.to_be_bytes());
        bmg.extend(flw1);

        let mut fli1 = vec![0; 0x20];
        fli1[0..4].copy_from_slice(b"FLI1");
        let fli1_size = fli1.len() as u32;
        fli1[4..8].copy_from_slice(&fli1_size.to_be_bytes());
        fli1[8..10].copy_from_slice(&1_u16.to_be_bytes());
        fli1[0x10..0x14].copy_from_slice(&(42_u32 << 16).to_be_bytes());
        fli1[0x14..0x16].copy_from_slice(&0_u16.to_be_bytes());
        bmg.extend(fli1);
        let size = bmg.len() as u32;
        bmg[8..12].copy_from_slice(&size.to_be_bytes());
        bmg
    }

    #[test]
    fn parses_actor_treasure_and_player_placements_without_world_tool_dependencies() {
        let mut stage = vec![0; 0xa0];
        stage[0..4].copy_from_slice(&3_u32.to_be_bytes());
        for (index, (tag, offset)) in [
            (b"ACT5", 0x40_u32),
            (b"PLYR", 0x60_u32),
            (b"TREa", 0x80_u32),
        ]
        .into_iter()
        .enumerate()
        {
            let header = 4 + index * 12;
            stage[header..header + 4].copy_from_slice(tag);
            stage[header + 4..header + 8].copy_from_slice(&1_u32.to_be_bytes());
            stage[header + 8..header + 12].copy_from_slice(&offset.to_be_bytes());
        }
        stage[0x40..0x44].copy_from_slice(b"grD1");
        stage[0x48..0x4c].copy_from_slice(&0x12345678_u32.to_be_bytes());
        stage[0x4c..0x50].copy_from_slice(&1.5_f32.to_bits().to_be_bytes());
        stage[0x50..0x54].copy_from_slice(&(-2.0_f32).to_bits().to_be_bytes());
        stage[0x54..0x58].copy_from_slice(&3.25_f32.to_bits().to_be_bytes());
        stage[0x58..0x5a].copy_from_slice(&42_i16.to_be_bytes());
        stage[0x5a..0x5c].copy_from_slice(&(-1_i16).to_be_bytes());
        stage[0x5c..0x5e].copy_from_slice(&9_i16.to_be_bytes());
        stage[0x5e..0x60].copy_from_slice(&7_u16.to_be_bytes());
        stage[0x60..0x65].copy_from_slice(b"start");
        stage[0x80..0x85].copy_from_slice(b"Tbox0");
        stage[0x88..0x8c].copy_from_slice(&0xfeed_beef_u32.to_be_bytes());

        let parsed = parse_stage_data(&stage).unwrap();
        assert_eq!(parsed.chunks[0].recognized_record_size, Some(0x20));
        let actor = &parsed.actor_placements[0];
        assert_eq!(actor.name, "grD1");
        assert_eq!(actor.layer, Some(5));
        assert_eq!(actor.parameters, 0x12345678);
        assert_eq!(actor.position, [1.5, -2.0, 3.25]);
        assert_eq!(actor.angle, [42, -1, 9]);
        assert_eq!(actor.set_id, 7);
        assert_eq!(parsed.player_spawns.len(), 1);
        assert_eq!(parsed.player_spawns[0].name, "start");
        assert_eq!(parsed.player_spawns[0].layer, None);
        assert_eq!(parsed.treasure_placements.len(), 1);
        assert_eq!(parsed.treasure_placements[0].name, "Tbox0");
        assert_eq!(parsed.treasure_placements[0].parameters, 0xfeed_beef);
        assert_eq!(parsed.treasure_placements[0].layer, Some(10));
    }

    #[test]
    fn real_rsp116_room6_placement_counts_match_the_compatible_inventory_when_present() {
        use sha2::{Digest as _, Sha256};
        use std::path::Path;

        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(4)
            .unwrap();
        let archive_path = repository_root.join("orig/GZ2E01/files/res/Stage/R_SP116/R06_00.arc");
        if !archive_path.is_file() {
            return;
        }
        let archive = std::fs::read(archive_path).unwrap();
        let resource = extract_unique_rarc_resource(&archive, "room.dzr").unwrap();
        assert_eq!(
            hex_bytes(&Sha256::digest(&resource)),
            "10487ef6754fec1f454c93aa33f605ee9781b4db4b91eed8e864721d76304d40"
        );
        let parsed = parse_stage_data(&resource).unwrap();
        assert_eq!(parsed.actor_placements.len(), 95);
        assert_eq!(parsed.player_spawns.len(), 5);
        assert_eq!(parsed.treasure_placements.len(), 0);
        assert_eq!(
            parsed
                .chunks
                .iter()
                .find(|chunk| chunk.tag == "PLYR")
                .map(|chunk| (chunk.record_count, chunk.recognized_record_size)),
            Some((5, Some(0x20)))
        );
    }

    #[test]
    fn parses_stage_message_group_and_indexed_scene_transitions() {
        let mut stage = vec![0; 0xa9];
        stage[0..4].copy_from_slice(&2_u32.to_be_bytes());
        stage[4..8].copy_from_slice(b"STAG");
        stage[8..12].copy_from_slice(&1_u32.to_be_bytes());
        stage[12..16].copy_from_slice(&0x20_u32.to_be_bytes());
        stage[16..20].copy_from_slice(b"SCLS");
        stage[20..24].copy_from_slice(&2_u32.to_be_bytes());
        stage[24..28].copy_from_slice(&0x8f_u32.to_be_bytes());
        stage[0x48] = 3;

        let first = &mut stage[0x8f..0x9c];
        first[..8].copy_from_slice(b"D_MN04\0\0");
        first[8..13].copy_from_slice(&[1, 1, 0xf0, 0x1f, 0]);
        let second = &mut stage[0x9c..0xa9];
        second[..8].copy_from_slice(b"F_SP110\0");
        second[8..13].copy_from_slice(&[3, 0, 0x70, 0x42, 13]);

        let parsed = parse_stage_data(&stage).unwrap();
        assert_eq!(parsed.stage_information.unwrap().message_group, 3);
        assert_eq!(parsed.scene_transitions.len(), 2);
        assert_eq!(
            parsed.scene_transitions[0],
            ExtractedSceneTransition {
                exit_id: 0,
                destination_stage: "D_MN04".into(),
                destination_spawn: 1,
                destination_room: 1,
                scene_layer: None,
                time_hour: None,
                wipe: 0,
                wipe_time: 0,
                raw_hex: "445f4d4e303400000101f01f00".into(),
            }
        );
        assert_eq!(parsed.scene_transitions[1].exit_id, 1);
        assert_eq!(parsed.scene_transitions[1].scene_layer, Some(2));
        assert_eq!(parsed.scene_transitions[1].time_hour, Some(7));
        assert_eq!(parsed.scene_transitions[1].wipe_time, 2);
    }

    #[test]
    fn parses_room_background_transforms_and_normal_file_lists() {
        let mut stage = vec![0; 0x60];
        stage[..4].copy_from_slice(&2_u32.to_be_bytes());
        stage[4..8].copy_from_slice(b"MULT");
        stage[8..12].copy_from_slice(&1_u32.to_be_bytes());
        stage[12..16].copy_from_slice(&0x20_u32.to_be_bytes());
        stage[16..20].copy_from_slice(b"FILI");
        stage[20..24].copy_from_slice(&1_u32.to_be_bytes());
        stage[24..28].copy_from_slice(&0x40_u32.to_be_bytes());

        let transform = &mut stage[0x20..0x2c];
        transform[0..4].copy_from_slice(&125.5_f32.to_bits().to_be_bytes());
        transform[4..8].copy_from_slice(&(-42.25_f32).to_bits().to_be_bytes());
        transform[8..10].copy_from_slice(&0x4000_i16.to_be_bytes());
        transform[10] = 7;
        transform[11] = 0xaa;

        let file_list = &mut stage[0x40..0x60];
        let parameters = 0x2000_0000_u32 | (2 << 18) | (5 << 15) | (0x34 << 7) | (3 << 3);
        file_list[0..4].copy_from_slice(&parameters.to_be_bytes());
        file_list[4..8].copy_from_slice(&(-100.0_f32).to_bits().to_be_bytes());
        file_list[8..12].copy_from_slice(&1.25_f32.to_bits().to_be_bytes());
        file_list[12..16].copy_from_slice(&2.5_f32.to_bits().to_be_bytes());
        file_list[16..26].copy_from_slice(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        file_list[0x1a] = 4;
        file_list[0x1b] = 0xff;
        file_list[0x1c..0x1e].copy_from_slice(&123_u16.to_be_bytes());

        let parsed = parse_stage_data(&stage).unwrap();
        assert_eq!(parsed.chunks[0].recognized_record_size, Some(0x0c));
        assert_eq!(parsed.chunks[1].recognized_record_size, Some(0x20));
        assert_eq!(parsed.room_transforms[0].room, 7);
        assert_eq!(parsed.room_transforms[0].translation_xz, [125.5, -42.25]);
        assert_eq!(parsed.room_transforms[0].angle_y, 0x4000);
        assert_eq!(parsed.room_transforms[0].trailing_byte, 0xaa);
        let fili = &parsed.file_lists[0];
        assert_eq!(fili.sea_level, -100.0);
        assert_eq!(fili.minimap_style, 3);
        assert!(fili.enemy_appear_flag);
        assert_eq!(fili.global_wind_level, 2);
        assert_eq!(fili.global_wind_direction, 5);
        assert_eq!(fili.grass_light, 0x34);
        assert_eq!(fili.default_camera, 4);
        assert_eq!(fili.bit_switch, 0xff);
        assert_eq!(fili.message_id, 123);
    }

    #[test]
    fn parses_demo_banks_and_map_event_exit_coordinates() {
        let mut stage = vec![0; 0x70];
        stage[..4].copy_from_slice(&2_u32.to_be_bytes());
        stage[4..8].copy_from_slice(b"REVT");
        stage[8..12].copy_from_slice(&1_u32.to_be_bytes());
        stage[12..16].copy_from_slice(&0x20_u32.to_be_bytes());
        stage[16..20].copy_from_slice(b"LBNK");
        stage[20..24].copy_from_slice(&15_u32.to_be_bytes());
        stage[24..28].copy_from_slice(&0x40_u32.to_be_bytes());
        let map_event = &mut stage[0x20..0x3c];
        map_event[..13].copy_from_slice(&[2, 2, 3, 3, 4, 0xff, 100, 1, 3, 2, 0xff, 0xff, 0]);
        map_event[13..23].copy_from_slice(b"demo07_02\0");
        map_event[0x1a] = 0xff;
        map_event[0x1b] = 0xff;
        stage[0x40..0x40 + 45].fill(0xff);
        stage[0x40 + 8 * 3..0x40 + 8 * 3 + 3].copy_from_slice(&[7, 2, 0xff]);

        let parsed = parse_stage_data(&stage).unwrap();
        assert_eq!(parsed.chunks[0].recognized_record_size, Some(0x1c));
        assert_eq!(parsed.map_events[0].map_tool_id, 4);
        assert_eq!(parsed.map_events[0].normal_exit_id, Some(1));
        assert_eq!(parsed.map_events[0].skip_exit_id, Some(2));
        assert_eq!(
            parsed.map_events[0].event_name.as_deref(),
            Some("demo07_02")
        );
        assert_eq!(
            parsed.demo_archive_banks[8].archive_name.as_deref(),
            Some("Demo07_02")
        );
    }

    #[test]
    fn parses_event_staff_cut_and_typed_data_tables() {
        let parsed = parse_event_list(&event_list_fixture()).unwrap();
        assert_eq!(parsed.events[0].name, "demo07_02");
        assert_eq!(parsed.events[0].staff_indices, [0, 1]);
        assert_eq!(parsed.staff[0].name, "PACKAGE");
        assert_eq!(parsed.staff[0].start_cut_index, 0);
        assert_eq!(parsed.cuts[0].name, "PLAY");
        assert_eq!(parsed.cuts[0].data_index, Some(0));
        assert_eq!(parsed.cuts[0].next_cut_index, Some(1));
        assert_eq!(
            parsed.data[0].value,
            ExtractedEventDataValue::StringBytes {
                raw_hex: "64656d6f30375f30322e737462000000".into(),
                ascii: Some("demo07_02.stb".into()),
            }
        );
        assert_eq!(
            parsed.data[1].value,
            ExtractedEventDataValue::Integers { values: vec![4] }
        );

        let mut invalid = event_list_fixture();
        invalid[0xf0 + 0x30..0xf0 + 0x34].copy_from_slice(&99_i32.to_be_bytes());
        assert_eq!(
            parse_event_list(&invalid).unwrap_err().field(),
            "orig.event_list.staff.start_cut"
        );
    }

    #[test]
    fn parses_flow_labels_generic_temp_writers_and_reader() {
        let parsed = parse_message_flow(&bmg_fixture()).unwrap();
        assert_eq!(parsed.node_count, 6);
        assert_eq!(parsed.labels[0].flow_id, 42);
        assert_eq!(parsed.branch_targets, vec![2, u16::MAX]);
        assert_eq!(
            parsed.nodes[0],
            MessageFlowNode::Event {
                index: 0,
                event_index: 10,
                next_target_index: 0,
                parameter_0: 10,
                parameter_1: 51,
                raw_parameter_u32: 0x000a0033,
                raw_parameters: [0, 10, 0, 51],
            }
        );
        assert_eq!(
            parsed.nodes[1],
            MessageFlowNode::Branch {
                index: 1,
                flags: 0,
                raw_query_index: 10,
                query_handler_index: Some(11),
                parameter: 11,
                next_target_index: 0,
            }
        );
        assert_eq!(
            parsed.temporary_flag_accesses,
            vec![
                MessageFlowTemporaryFlagAccess {
                    node_index: 0,
                    operation: MessageFlowTemporaryFlagOperation::Set,
                    parameter_ordinal: 0,
                    label_index: 10,
                    packed_backing_coordinate: None,
                    friendly_name: None,
                },
                MessageFlowTemporaryFlagAccess {
                    node_index: 0,
                    operation: MessageFlowTemporaryFlagOperation::Set,
                    parameter_ordinal: 1,
                    label_index: 51,
                    packed_backing_coordinate: Some(0x0508),
                    friendly_name: Some("message_flow_control_f".to_owned()),
                },
                MessageFlowTemporaryFlagAccess {
                    node_index: 1,
                    operation: MessageFlowTemporaryFlagOperation::BranchTrueWhenClear,
                    parameter_ordinal: 0,
                    label_index: 11,
                    packed_backing_coordinate: Some(0x0004),
                    friendly_name: Some("message_flow_control_a".to_owned()),
                },
            ]
        );
        assert_eq!(
            parsed.persistent_flag_accesses,
            vec![
                MessageFlowPersistentFlagAccess {
                    node_index: 3,
                    operation: MessageFlowPersistentFlagOperation::Set,
                    parameter_ordinal: 0,
                    label_index: 62,
                    packed_backing_coordinate: Some(0x0704),
                    friendly_name: Some("won_gor_coron_match".to_owned()),
                },
                MessageFlowPersistentFlagAccess {
                    node_index: 4,
                    operation: MessageFlowPersistentFlagOperation::BranchTrueWhenClear,
                    parameter_ordinal: 0,
                    label_index: 62,
                    packed_backing_coordinate: Some(0x0704),
                    friendly_name: Some("won_gor_coron_match".to_owned()),
                },
            ]
        );
        assert_eq!(
            parsed.switch_accesses,
            vec![MessageFlowSwitchAccess {
                node_index: 5,
                operation: MessageFlowSwitchOperation::Set,
                store: MessageFlowSwitchStore::OneZone,
                switch_index: 10,
            }]
        );
    }

    #[test]
    fn resolves_audited_lanayru_persistent_message_labels_to_raw_backing() {
        assert_eq!(
            persistent_message_flag(615),
            Some((0x4b04, "received_lanayru_vessel"))
        );
        assert_eq!(
            persistent_message_flag(66),
            Some((0x0840, "start_carriage_guarding_game"))
        );
    }

    #[test]
    fn accepts_retail_header_and_final_alignment_quirks_without_ignoring_payload() {
        let mut fixture = bmg_fixture();
        let fli_offset = 0x20 + 0x50;
        fixture[8..12].copy_from_slice(&(fli_offset as u32).to_be_bytes());
        fixture[fli_offset + 4..fli_offset + 8].copy_from_slice(&(0x28_u32).to_be_bytes());

        let parsed = parse_message_flow(&fixture).unwrap();
        assert_eq!(parsed.header_declared_size, fli_offset as u32);
        assert_eq!(parsed.resource_size, fixture.len() as u32);
        assert_eq!(parsed.labels[0].flow_id, 42);
    }

    #[test]
    fn maps_raw_query_dispatch_indices_without_conflating_handler_numbers() {
        assert_eq!(message_query_handler_index(0), Some(5));
        assert_eq!(message_query_handler_index(6), Some(4));
        assert_eq!(message_query_handler_index(10), Some(11));
        assert_eq!(message_query_handler_index(52), Some(53));
        assert_eq!(message_query_handler_index(53), None);
    }

    #[test]
    fn malformed_or_oversized_inputs_fail_closed() {
        assert!(parse_message_flow(b"MESGbmg1").is_err());
        assert!(extract_unique_rarc_resource(b"RARC", "zel_04.bmg").is_err());
        let mut yaz0 = vec![0; 16];
        yaz0[0..4].copy_from_slice(b"Yaz0");
        yaz0[4..8].copy_from_slice(&((MAX_DECODED_ARCHIVE_BYTES as u32) + 1).to_be_bytes());
        assert!(extract_unique_rarc_resource(&yaz0, "zel_04.bmg").is_err());
    }
}
