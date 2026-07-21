//! Minimal, planner-owned extraction for immutable retail message archives.
//!
//! This module deliberately accepts bytes rather than filesystem paths. The
//! runtime CLI owns discovery and I/O; the engine owns bounded deterministic
//! decoding and portable extracted records.

use crate::PlannerContractError;
use serde::{Deserialize, Serialize};

const MAX_DECODED_ARCHIVE_BYTES: usize = 64 * 1024 * 1024;
const RARC_FILE_ENTRY_SIZE: usize = 0x14;
const MAX_STAGE_CHUNKS: usize = 4096;
const MAX_STAGE_RECORDS: usize = 1_000_000;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedStageData {
    pub chunks: Vec<ExtractedStageChunk>,
    pub actor_placements: Vec<ExtractedActorPlacement>,
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
    Save,
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
        115 => (0x0e20, "spoke_with_spring_goron_a"),
        152 => (0x1201, "lost_gor_coron_match_with_iron_boots"),
        154 => (0x1340, "lost_gor_coron_match_again"),
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
    let decoded = if input.starts_with(b"Yaz0") {
        decode_yaz0(input)?
    } else {
        input.to_vec()
    };
    extract_uncompressed_rarc_resource(&decoded, resource_name)
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
    let mut actor_placements = Vec::new();
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
        let record_size = actor_record_layout(&tag).map(|layout| layout.0);
        chunks.push(ExtractedStageChunk {
            tag: tag.clone(),
            record_count,
            data_offset,
            recognized_record_size: record_size.map(|size| size as u8),
        });
        let Some((record_size, scaled, layer)) = actor_record_layout(&tag) else {
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
            actor_placements.push(ExtractedActorPlacement {
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
                raw_hex: record.iter().map(|byte| format!("{byte:02x}")).collect(),
            });
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
        actor_placements,
    })
}

fn actor_record_layout(tag: &str) -> Option<(usize, bool, Option<u8>)> {
    if matches!(tag, "ACTR" | "TGOB") {
        return Some((0x20, false, None));
    }
    if matches!(tag, "SCOB" | "TGSC" | "TGDR" | "Door") {
        return Some((0x24, true, None));
    }
    if tag.len() != 4 {
        return None;
    }
    let (prefix, scaled) = match &tag[..3] {
        "ACT" => ("ACT", false),
        "SCO" | "Doo" => (&tag[..3], true),
        _ => return None,
    };
    debug_assert_eq!(prefix, &tag[..3]);
    decode_layer(tag.as_bytes()[3]).map(|layer| {
        if scaled {
            (0x24, true, Some(layer))
        } else {
            (0x20, false, Some(layer))
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
                    13 => MessageFlowSwitchStore::Save,
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
        0 => MessageFlowSwitchStore::Save,
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

fn extract_uncompressed_rarc_resource(
    archive: &[u8],
    resource_name: &str,
) -> Result<Vec<u8>, PlannerContractError> {
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

    let mut matched = None;
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
        if name != resource_name.as_bytes() {
            continue;
        }
        let offset = relative_offset(
            data_base,
            read_u32(archive, entry + 8, "orig.rarc.entry.offset")?,
            "orig.rarc.entry.offset",
        )?;
        let size = read_u32(archive, entry + 12, "orig.rarc.entry.size")? as usize;
        require_range(archive, offset, size, "orig.rarc.entry.data")?;
        if matched.replace((offset, size)).is_some() {
            return Err(PlannerContractError::new(
                "orig.rarc.resource",
                format!("contains multiple files named {resource_name:?}"),
            ));
        }
    }
    let (offset, size) = matched.ok_or_else(|| {
        PlannerContractError::new(
            "orig.rarc.resource",
            format!("{resource_name:?} was not found"),
        )
    })?;
    Ok(archive[offset..offset + size].to_vec())
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
    fn parses_layered_stage_actor_placement_without_world_tool_dependencies() {
        let mut stage = vec![0; 0x40];
        stage[0..4].copy_from_slice(&1_u32.to_be_bytes());
        stage[4..8].copy_from_slice(b"ACT5");
        stage[8..12].copy_from_slice(&1_u32.to_be_bytes());
        stage[12..16].copy_from_slice(&0x20_u32.to_be_bytes());
        stage[0x20..0x24].copy_from_slice(b"grD1");
        stage[0x28..0x2c].copy_from_slice(&0x12345678_u32.to_be_bytes());
        stage[0x2c..0x30].copy_from_slice(&1.5_f32.to_bits().to_be_bytes());
        stage[0x30..0x34].copy_from_slice(&(-2.0_f32).to_bits().to_be_bytes());
        stage[0x34..0x38].copy_from_slice(&3.25_f32.to_bits().to_be_bytes());
        stage[0x38..0x3a].copy_from_slice(&42_i16.to_be_bytes());
        stage[0x3a..0x3c].copy_from_slice(&(-1_i16).to_be_bytes());
        stage[0x3c..0x3e].copy_from_slice(&9_i16.to_be_bytes());
        stage[0x3e..0x40].copy_from_slice(&7_u16.to_be_bytes());

        let parsed = parse_stage_data(&stage).unwrap();
        assert_eq!(parsed.chunks[0].recognized_record_size, Some(0x20));
        let actor = &parsed.actor_placements[0];
        assert_eq!(actor.name, "grD1");
        assert_eq!(actor.layer, Some(5));
        assert_eq!(actor.parameters, 0x12345678);
        assert_eq!(actor.position, [1.5, -2.0, 3.25]);
        assert_eq!(actor.angle, [42, -1, 9]);
        assert_eq!(actor.set_id, 7);
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
