//! Bounded structural extraction for JStudio STB programs.
//!
//! This module decodes container, object, sequence-command, and paragraph
//! boundaries. Object-specific paragraph semantics remain unresolved until an
//! adaptor audit binds them to planner operations.

use crate::artifact::Digest;
use crate::{PlannerContractError, canonical_json, validate_label};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;

pub const JSTUDIO_STB_PROGRAM_SCHEMA: &str = "dusklight.route-planner.jstudio-stb-program/v1";
const STB_HEADER_SIZE: usize = 0x20;
const FVB_HEADER_SIZE: usize = 0x10;
const BLOCK_JFVB: u32 = u32::from_be_bytes(*b"JFVB");
const BLOCK_JCTB: u32 = u32::from_be_bytes(*b"JCTB");

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JstudioStbSourceIdentity {
    pub archive_sha256: Digest,
    pub resource_sha256: Digest,
    pub resource_name: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JstudioCoverageStatus {
    Indexed,
    DecodedStructure,
    Unresolved,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JstudioStbCoverage {
    pub outer_blocks: JstudioCoverageStatus,
    pub fvb_functions: JstudioCoverageStatus,
    pub object_sequences: JstudioCoverageStatus,
    pub paragraph_headers: JstudioCoverageStatus,
    pub paragraph_semantics: JstudioCoverageStatus,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JstudioStbProgram {
    pub schema: String,
    pub source: JstudioStbSourceIdentity,
    pub resource_size: u32,
    pub format_version: u16,
    pub declared_size: u32,
    pub target_name_hex: String,
    pub target_name_ascii: String,
    pub target_metadata: [u16; 3],
    pub target_version: u16,
    pub blocks: Vec<JstudioStbBlock>,
    pub coverage: JstudioStbCoverage,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JstudioStbBlock {
    pub index: u32,
    pub offset: u32,
    pub size: u32,
    pub type_code: u32,
    pub type_ascii: Option<String>,
    pub block_sha256: Digest,
    pub body: JstudioStbBlockBody,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JstudioStbBlockBody {
    EmbeddedFvb {
        format_version: u16,
        declared_size: u32,
        blocks: Vec<JstudioFvbBlock>,
    },
    EmbeddedCtbUnresolved {
        content_size: u32,
        content_sha256: Digest,
    },
    Object {
        flag: u16,
        id_size: u16,
        id_hex: String,
        id_ascii: Option<String>,
        sequence_offset: u32,
        commands: Vec<JstudioSequenceCommand>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JstudioFvbBlock {
    pub index: u32,
    pub offset: u32,
    pub size: u32,
    pub function_type: u16,
    pub id_size: u16,
    pub id_hex: String,
    pub id_ascii: Option<String>,
    pub content_size: u32,
    pub content_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JstudioSequenceCommand {
    pub index: u32,
    pub offset: u32,
    pub encoded_size: u32,
    pub type_code: u8,
    pub parameter: u32,
    pub payload_sha256: Option<Digest>,
    pub operation: JstudioSequenceOperation,
    pub paragraphs: Vec<JstudioParagraph>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JstudioSequenceOperation {
    End,
    FlagOperation { operation: u8, value: u16 },
    Wait { frames: u32 },
    RelativeJump { delta: i32, target_offset: u32 },
    Suspend { delta: i32 },
    Paragraphs,
    UnknownNoPayload,
    UnknownPayload,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JstudioParagraph {
    pub index: u32,
    pub offset: u32,
    pub header_size: u8,
    pub type_code: u32,
    pub content_size: u32,
    pub content_sha256: Option<Digest>,
    pub interpretation: JstudioParagraphInterpretation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JstudioParagraphInterpretation {
    ReservedFlagOperation {
        operation: u8,
        value: u16,
    },
    ReservedWait {
        frames: u32,
    },
    ReservedRelativeJump {
        delta: i32,
        target_offset: u32,
    },
    ReservedRawData,
    ReservedIdentifiedData {
        flags: u16,
        id_size: u16,
        id_hex: String,
        id_ascii: Option<String>,
        payload_size: u32,
        payload_sha256: Digest,
    },
    ReservedObjectReference,
    UnknownReserved,
    ObjectSpecific,
}

impl JstudioStbProgram {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != JSTUDIO_STB_PROGRAM_SCHEMA {
            return Err(PlannerContractError::new(
                "jstudio_stb.schema",
                "is unsupported",
            ));
        }
        if self.source.archive_sha256 == Digest::ZERO || self.source.resource_sha256 == Digest::ZERO
        {
            return Err(PlannerContractError::new(
                "jstudio_stb.source",
                "must contain nonzero exact source identities",
            ));
        }
        validate_label(
            "jstudio_stb.source.resource_name",
            &self.source.resource_name,
        )?;
        if self.resource_size < STB_HEADER_SIZE as u32
            || self.declared_size != self.resource_size
            || !(1..=3).contains(&self.format_version)
            || self.target_name_ascii != "jstudio"
            || !(2..=6).contains(&self.target_version)
        {
            return Err(PlannerContractError::new(
                "jstudio_stb.header",
                "does not match the supported JStudio STB contract",
            ));
        }
        let target_name = decode_hex(&self.target_name_hex, "jstudio_stb.target_name_hex")?;
        if target_name.len() != 8
            || ascii_nul_terminated(&target_name).as_deref() != Some(&self.target_name_ascii)
        {
            return Err(PlannerContractError::new(
                "jstudio_stb.target_name_hex",
                "must contain exactly eight bytes",
            ));
        }
        if self.coverage
            != (JstudioStbCoverage {
                outer_blocks: JstudioCoverageStatus::DecodedStructure,
                fvb_functions: JstudioCoverageStatus::Indexed,
                object_sequences: JstudioCoverageStatus::DecodedStructure,
                paragraph_headers: JstudioCoverageStatus::DecodedStructure,
                paragraph_semantics: JstudioCoverageStatus::Unresolved,
            })
        {
            return Err(PlannerContractError::new(
                "jstudio_stb.coverage",
                "must not overstate the v1 structural decoder boundary",
            ));
        }

        let mut next_offset = STB_HEADER_SIZE as u32;
        for (index, block) in self.blocks.iter().enumerate() {
            if block.index != index as u32
                || block.offset != next_offset
                || block.size < 8
                || !block.size.is_multiple_of(4)
                || block.block_sha256 == Digest::ZERO
                || block.type_ascii != ascii_fourcc(block.type_code)
            {
                return Err(PlannerContractError::new(
                    "jstudio_stb.blocks",
                    "are not canonical contiguous bounded records",
                ));
            }
            validate_block_body(block)?;
            next_offset = next_offset.checked_add(block.size).ok_or_else(|| {
                PlannerContractError::new("jstudio_stb.blocks", "offset range overflows")
            })?;
        }
        if next_offset != self.resource_size {
            return Err(PlannerContractError::new(
                "jstudio_stb.blocks",
                "do not exactly cover the declared resource",
            ));
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let program: Self = serde_json::from_slice(bytes)?;
        program.validate()?;
        if program.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "jstudio_stb",
                "is not canonical JSON",
            ));
        }
        Ok(program)
    }
}

pub fn parse_jstudio_stb(
    archive_sha256: Digest,
    resource_name: &str,
    bytes: &[u8],
) -> Result<JstudioStbProgram, PlannerContractError> {
    if archive_sha256 == Digest::ZERO {
        return Err(PlannerContractError::new(
            "jstudio_stb.source.archive_sha256",
            "must be nonzero",
        ));
    }
    validate_label("jstudio_stb.source.resource_name", resource_name)?;
    if bytes.len() > u32::MAX as usize {
        return Err(PlannerContractError::new(
            "jstudio_stb.resource_size",
            "exceeds the representable STB address space",
        ));
    }
    if bytes.len() < STB_HEADER_SIZE || bytes.get(0..4) != Some(b"STB\0") {
        return Err(PlannerContractError::new(
            "jstudio_stb.header.signature",
            "is missing or unsupported",
        ));
    }
    if read_u16(bytes, 4, "jstudio_stb.header.byte_order")? != 0xfeff {
        return Err(PlannerContractError::new(
            "jstudio_stb.header.byte_order",
            "is not big endian",
        ));
    }
    let format_version = read_u16(bytes, 6, "jstudio_stb.header.version")?;
    if !(1..=3).contains(&format_version) {
        return Err(PlannerContractError::new(
            "jstudio_stb.header.version",
            "is unsupported by the retail parser",
        ));
    }
    let declared_size = read_u32(bytes, 8, "jstudio_stb.header.declared_size")?;
    if declared_size as usize != bytes.len() {
        return Err(PlannerContractError::new(
            "jstudio_stb.header.declared_size",
            "does not match the resource length",
        ));
    }
    let block_count = read_u32(bytes, 12, "jstudio_stb.header.block_count")?;
    if block_count as usize > (bytes.len() - STB_HEADER_SIZE) / 8 {
        return Err(PlannerContractError::new(
            "jstudio_stb.header.block_count",
            "cannot fit in the declared resource",
        ));
    }
    let target_name = slice(bytes, 16, 8, "jstudio_stb.header.target_name")?;
    let target_name_ascii = ascii_nul_terminated(target_name).ok_or_else(|| {
        PlannerContractError::new("jstudio_stb.header.target_name", "is not bounded ASCII")
    })?;
    if target_name_ascii != "jstudio" {
        return Err(PlannerContractError::new(
            "jstudio_stb.header.target_name",
            "is not the JStudio target",
        ));
    }
    let target_metadata = [
        read_u16(bytes, 24, "jstudio_stb.header.target_metadata")?,
        read_u16(bytes, 26, "jstudio_stb.header.target_metadata")?,
        read_u16(bytes, 28, "jstudio_stb.header.target_metadata")?,
    ];
    let target_version = read_u16(bytes, 30, "jstudio_stb.header.target_version")?;
    if !(2..=6).contains(&target_version) {
        return Err(PlannerContractError::new(
            "jstudio_stb.header.target_version",
            "is unsupported by the retail JStudio parser",
        ));
    }

    let mut offset = STB_HEADER_SIZE;
    let mut blocks = Vec::with_capacity(block_count as usize);
    for index in 0..block_count {
        let size = read_u32(bytes, offset, "jstudio_stb.block.size")? as usize;
        let type_code = read_u32(bytes, offset + 4, "jstudio_stb.block.type")?;
        if size < 8 || !size.is_multiple_of(4) {
            return Err(PlannerContractError::new(
                "jstudio_stb.block.size",
                "must be aligned and include its header",
            ));
        }
        let end = offset
            .checked_add(size)
            .ok_or_else(|| PlannerContractError::new("jstudio_stb.block", "range overflows"))?;
        let block_bytes = bytes
            .get(offset..end)
            .ok_or_else(|| PlannerContractError::new("jstudio_stb.block", "is truncated"))?;
        let body = match type_code {
            BLOCK_JFVB => parse_fvb(bytes, offset + 8, end)?,
            BLOCK_JCTB => {
                let content = &bytes[offset + 8..end];
                JstudioStbBlockBody::EmbeddedCtbUnresolved {
                    content_size: content.len() as u32,
                    content_sha256: Digest(Sha256::digest(content).into()),
                }
            }
            _ => parse_object(bytes, offset, end)?,
        };
        blocks.push(JstudioStbBlock {
            index,
            offset: offset as u32,
            size: size as u32,
            type_code,
            type_ascii: ascii_fourcc(type_code),
            block_sha256: Digest(Sha256::digest(block_bytes).into()),
            body,
        });
        offset = end;
    }
    if offset != bytes.len() {
        return Err(PlannerContractError::new(
            "jstudio_stb.blocks",
            "do not exactly consume the resource",
        ));
    }

    let program = JstudioStbProgram {
        schema: JSTUDIO_STB_PROGRAM_SCHEMA.into(),
        source: JstudioStbSourceIdentity {
            archive_sha256,
            resource_sha256: Digest(Sha256::digest(bytes).into()),
            resource_name: resource_name.into(),
        },
        resource_size: bytes.len() as u32,
        format_version,
        declared_size,
        target_name_hex: encode_hex(target_name),
        target_name_ascii,
        target_metadata,
        target_version,
        blocks,
        coverage: JstudioStbCoverage {
            outer_blocks: JstudioCoverageStatus::DecodedStructure,
            fvb_functions: JstudioCoverageStatus::Indexed,
            object_sequences: JstudioCoverageStatus::DecodedStructure,
            paragraph_headers: JstudioCoverageStatus::DecodedStructure,
            paragraph_semantics: JstudioCoverageStatus::Unresolved,
        },
    };
    program.validate()?;
    Ok(program)
}

fn parse_object(
    bytes: &[u8],
    block_offset: usize,
    block_end: usize,
) -> Result<JstudioStbBlockBody, PlannerContractError> {
    if block_end < block_offset + 12 {
        return Err(PlannerContractError::new(
            "jstudio_stb.object",
            "is shorter than its object header",
        ));
    }
    let flag = read_u16(bytes, block_offset + 8, "jstudio_stb.object.flag")?;
    let id_size = read_u16(bytes, block_offset + 10, "jstudio_stb.object.id_size")?;
    let id = slice(
        bytes,
        block_offset + 12,
        usize::from(id_size),
        "jstudio_stb.object.id",
    )?;
    let sequence_offset = block_offset
        .checked_add(12)
        .and_then(|offset| offset.checked_add(align4(usize::from(id_size))?))
        .ok_or_else(|| PlannerContractError::new("jstudio_stb.object", "range overflows"))?;
    if sequence_offset > block_end {
        return Err(PlannerContractError::new(
            "jstudio_stb.object.id",
            "extends beyond the block",
        ));
    }
    let commands = parse_commands(bytes, sequence_offset, block_end)?;
    Ok(JstudioStbBlockBody::Object {
        flag,
        id_size,
        id_hex: encode_hex(id),
        id_ascii: ascii_nul_terminated(id),
        sequence_offset: sequence_offset as u32,
        commands,
    })
}

fn parse_commands(
    bytes: &[u8],
    start: usize,
    end: usize,
) -> Result<Vec<JstudioSequenceCommand>, PlannerContractError> {
    let mut offset = start;
    let mut commands = Vec::new();
    while offset < end {
        let head = read_u32(bytes, offset, "jstudio_stb.sequence.head")?;
        let type_code = (head >> 24) as u8;
        let parameter = head & 0x00ff_ffff;
        let payload_size = if type_code >= 0x80 {
            parameter as usize
        } else {
            0
        };
        let encoded_size = 4usize.checked_add(payload_size).ok_or_else(|| {
            PlannerContractError::new("jstudio_stb.sequence", "command size overflows")
        })?;
        let command_end = offset.checked_add(encoded_size).ok_or_else(|| {
            PlannerContractError::new("jstudio_stb.sequence", "command range overflows")
        })?;
        let payload = bytes.get(offset + 4..command_end).ok_or_else(|| {
            PlannerContractError::new("jstudio_stb.sequence", "command payload is truncated")
        })?;
        if command_end > end {
            return Err(PlannerContractError::new(
                "jstudio_stb.sequence",
                "command extends beyond its object block",
            ));
        }
        let operation = match type_code {
            0 if parameter == 0 => JstudioSequenceOperation::End,
            0 => {
                return Err(PlannerContractError::new(
                    "jstudio_stb.sequence.end",
                    "has a nonzero parameter",
                ));
            }
            1 => JstudioSequenceOperation::FlagOperation {
                operation: (parameter >> 16) as u8,
                value: parameter as u16,
            },
            2 => JstudioSequenceOperation::Wait { frames: parameter },
            3 => {
                let delta = sign_extend_u24(parameter);
                JstudioSequenceOperation::RelativeJump {
                    delta,
                    target_offset: relative_target(offset, delta, "jstudio_stb.sequence.jump")?,
                }
            }
            4 => JstudioSequenceOperation::Suspend {
                delta: sign_extend_u24(parameter),
            },
            0x80 => JstudioSequenceOperation::Paragraphs,
            0x81..=0xff => JstudioSequenceOperation::UnknownPayload,
            _ => JstudioSequenceOperation::UnknownNoPayload,
        };
        let paragraphs = if type_code == 0x80 {
            parse_paragraphs(bytes, offset, offset + 4, command_end)?
        } else {
            Vec::new()
        };
        commands.push(JstudioSequenceCommand {
            index: commands.len() as u32,
            offset: offset as u32,
            encoded_size: encoded_size as u32,
            type_code,
            parameter,
            payload_sha256: (!payload.is_empty()).then(|| Digest(Sha256::digest(payload).into())),
            operation,
            paragraphs,
        });
        offset = command_end;
    }
    if commands.is_empty()
        || !matches!(
            commands.last().map(|command| &command.operation),
            Some(JstudioSequenceOperation::End)
        )
    {
        return Err(PlannerContractError::new(
            "jstudio_stb.sequence",
            "must end with an explicit end command",
        ));
    }
    validate_jump_targets(&commands)?;
    Ok(commands)
}

fn parse_paragraphs(
    bytes: &[u8],
    command_offset: usize,
    start: usize,
    end: usize,
) -> Result<Vec<JstudioParagraph>, PlannerContractError> {
    let mut offset = start;
    let mut paragraphs = Vec::new();
    while offset < end {
        let first = read_u16(bytes, offset, "jstudio_stb.paragraph.header")?;
        let (header_size, content_size, type_code) = if first & 0x8000 == 0 {
            (
                4usize,
                u32::from(first),
                u32::from(read_u16(bytes, offset + 2, "jstudio_stb.paragraph.type")?),
            )
        } else {
            (
                8usize,
                (u32::from(first & 0x7fff) << 16)
                    | u32::from(read_u16(bytes, offset + 2, "jstudio_stb.paragraph.size")?),
                read_u32(bytes, offset + 4, "jstudio_stb.paragraph.type")?,
            )
        };
        let content_offset = offset
            .checked_add(header_size)
            .ok_or_else(|| PlannerContractError::new("jstudio_stb.paragraph", "range overflows"))?;
        let padded_size = align4(content_size as usize).ok_or_else(|| {
            PlannerContractError::new("jstudio_stb.paragraph", "content size overflows")
        })?;
        let next = content_offset
            .checked_add(padded_size)
            .ok_or_else(|| PlannerContractError::new("jstudio_stb.paragraph", "range overflows"))?;
        if next > end {
            return Err(PlannerContractError::new(
                "jstudio_stb.paragraph",
                "extends beyond its sequence payload",
            ));
        }
        let content = slice(
            bytes,
            content_offset,
            content_size as usize,
            "jstudio_stb.paragraph.content",
        )?;
        let interpretation = interpret_paragraph(type_code, content, command_offset)?;
        paragraphs.push(JstudioParagraph {
            index: paragraphs.len() as u32,
            offset: offset as u32,
            header_size: header_size as u8,
            type_code,
            content_size,
            content_sha256: (!content.is_empty()).then(|| Digest(Sha256::digest(content).into())),
            interpretation,
        });
        offset = next;
    }
    if offset != end {
        return Err(PlannerContractError::new(
            "jstudio_stb.paragraphs",
            "do not exactly cover the sequence payload",
        ));
    }
    Ok(paragraphs)
}

fn interpret_paragraph(
    type_code: u32,
    content: &[u8],
    command_offset: usize,
) -> Result<JstudioParagraphInterpretation, PlannerContractError> {
    Ok(match (type_code, content.len()) {
        (1, 4) => {
            let value = read_u32(content, 0, "jstudio_stb.paragraph.flag")?;
            JstudioParagraphInterpretation::ReservedFlagOperation {
                operation: (value >> 16) as u8,
                value: value as u16,
            }
        }
        (2, 4) => JstudioParagraphInterpretation::ReservedWait {
            frames: read_u32(content, 0, "jstudio_stb.paragraph.wait")?,
        },
        (3, 4) => {
            let delta = read_u32(content, 0, "jstudio_stb.paragraph.jump")? as i32;
            JstudioParagraphInterpretation::ReservedRelativeJump {
                delta,
                target_offset: relative_target(
                    command_offset,
                    delta,
                    "jstudio_stb.paragraph.jump",
                )?,
            }
        }
        (0x80, _) => JstudioParagraphInterpretation::ReservedRawData,
        (0x81, _) => parse_identified_data(content)?,
        (0x82, _) => JstudioParagraphInterpretation::ReservedObjectReference,
        (0..=0xff, _) => JstudioParagraphInterpretation::UnknownReserved,
        _ => JstudioParagraphInterpretation::ObjectSpecific,
    })
}

fn parse_identified_data(
    content: &[u8],
) -> Result<JstudioParagraphInterpretation, PlannerContractError> {
    if content.len() < 4 {
        return Err(PlannerContractError::new(
            "jstudio_stb.paragraph.identified_data",
            "is shorter than its ID header",
        ));
    }
    let flags = read_u16(content, 0, "jstudio_stb.paragraph.identified_data.flags")?;
    let id_size = read_u16(content, 2, "jstudio_stb.paragraph.identified_data.id_size")?;
    let id = slice(
        content,
        4,
        usize::from(id_size),
        "jstudio_stb.paragraph.identified_data.id",
    )?;
    let payload_offset = 4usize
        .checked_add(align4(usize::from(id_size)).ok_or_else(|| {
            PlannerContractError::new("jstudio_stb.paragraph.identified_data", "ID size overflows")
        })?)
        .ok_or_else(|| {
            PlannerContractError::new("jstudio_stb.paragraph.identified_data", "range overflows")
        })?;
    let payload = content.get(payload_offset..).ok_or_else(|| {
        PlannerContractError::new(
            "jstudio_stb.paragraph.identified_data",
            "ID padding extends beyond the paragraph",
        )
    })?;
    Ok(JstudioParagraphInterpretation::ReservedIdentifiedData {
        flags,
        id_size,
        id_hex: encode_hex(id),
        id_ascii: ascii_nul_terminated(id),
        payload_size: payload.len() as u32,
        payload_sha256: Digest(Sha256::digest(payload).into()),
    })
}

fn parse_fvb(
    bytes: &[u8],
    start: usize,
    end: usize,
) -> Result<JstudioStbBlockBody, PlannerContractError> {
    if end < start + FVB_HEADER_SIZE || bytes.get(start..start + 4) != Some(b"FVB\0") {
        return Err(PlannerContractError::new(
            "jstudio_stb.fvb.header",
            "is missing or unsupported",
        ));
    }
    if read_u16(bytes, start + 4, "jstudio_stb.fvb.byte_order")? != 0xfeff {
        return Err(PlannerContractError::new(
            "jstudio_stb.fvb.byte_order",
            "is not big endian",
        ));
    }
    let format_version = read_u16(bytes, start + 6, "jstudio_stb.fvb.version")?;
    if !(2..=0x100).contains(&format_version) {
        return Err(PlannerContractError::new(
            "jstudio_stb.fvb.version",
            "is unsupported by the retail parser",
        ));
    }
    let declared_size = read_u32(bytes, start + 8, "jstudio_stb.fvb.declared_size")?;
    if start.checked_add(declared_size as usize) != Some(end) {
        return Err(PlannerContractError::new(
            "jstudio_stb.fvb.declared_size",
            "does not match the embedded resource length",
        ));
    }
    let block_count = read_u32(bytes, start + 12, "jstudio_stb.fvb.block_count")?;
    if block_count as usize > (end - start - FVB_HEADER_SIZE) / 8 {
        return Err(PlannerContractError::new(
            "jstudio_stb.fvb.block_count",
            "cannot fit in the embedded resource",
        ));
    }
    let mut offset = start + FVB_HEADER_SIZE;
    let mut blocks = Vec::with_capacity(block_count as usize);
    for index in 0..block_count {
        let size = read_u32(bytes, offset, "jstudio_stb.fvb.block.size")? as usize;
        let function_type = read_u16(bytes, offset + 4, "jstudio_stb.fvb.block.type")?;
        let id_size = read_u16(bytes, offset + 6, "jstudio_stb.fvb.block.id_size")?;
        if size < 8 || !size.is_multiple_of(4) {
            return Err(PlannerContractError::new(
                "jstudio_stb.fvb.block.size",
                "must be aligned and include its header",
            ));
        }
        let block_end = offset
            .checked_add(size)
            .ok_or_else(|| PlannerContractError::new("jstudio_stb.fvb.block", "range overflows"))?;
        if block_end > end {
            return Err(PlannerContractError::new(
                "jstudio_stb.fvb.block",
                "extends beyond the embedded resource",
            ));
        }
        let id = slice(
            bytes,
            offset + 8,
            usize::from(id_size),
            "jstudio_stb.fvb.block.id",
        )?;
        let content_offset = (offset + 8)
            .checked_add(align4(usize::from(id_size)).ok_or_else(|| {
                PlannerContractError::new("jstudio_stb.fvb.block.id", "size overflows")
            })?)
            .ok_or_else(|| PlannerContractError::new("jstudio_stb.fvb.block", "range overflows"))?;
        let content = bytes.get(content_offset..block_end).ok_or_else(|| {
            PlannerContractError::new("jstudio_stb.fvb.block", "ID extends beyond the block")
        })?;
        blocks.push(JstudioFvbBlock {
            index,
            offset: offset as u32,
            size: size as u32,
            function_type,
            id_size,
            id_hex: encode_hex(id),
            id_ascii: ascii_nul_terminated(id),
            content_size: content.len() as u32,
            content_sha256: Digest(Sha256::digest(content).into()),
        });
        offset = block_end;
    }
    if offset != end {
        return Err(PlannerContractError::new(
            "jstudio_stb.fvb.blocks",
            "do not exactly consume the embedded resource",
        ));
    }
    Ok(JstudioStbBlockBody::EmbeddedFvb {
        format_version,
        declared_size,
        blocks,
    })
}

fn validate_block_body(block: &JstudioStbBlock) -> Result<(), PlannerContractError> {
    match &block.body {
        JstudioStbBlockBody::EmbeddedFvb {
            declared_size,
            blocks,
            ..
        } => {
            if block.type_code != BLOCK_JFVB || declared_size.checked_add(8) != Some(block.size) {
                return Err(PlannerContractError::new(
                    "jstudio_stb.fvb",
                    "does not match its outer block",
                ));
            }
            let mut next = block.offset + 8 + FVB_HEADER_SIZE as u32;
            for (index, nested) in blocks.iter().enumerate() {
                let id = decode_hex(&nested.id_hex, "jstudio_stb.fvb.block.id_hex")?;
                let id_padded = align4(usize::from(nested.id_size)).ok_or_else(|| {
                    PlannerContractError::new("jstudio_stb.fvb.block.id", "size overflows")
                })? as u32;
                if nested.index != index as u32
                    || nested.offset != next
                    || nested.size < 8
                    || nested.content_sha256 == Digest::ZERO
                    || id.len() != usize::from(nested.id_size)
                    || nested.id_ascii != ascii_nul_terminated(&id)
                    || nested.size.checked_sub(8 + id_padded) != Some(nested.content_size)
                {
                    return Err(PlannerContractError::new(
                        "jstudio_stb.fvb.blocks",
                        "are not canonical indexed records",
                    ));
                }
                next = next.checked_add(nested.size).ok_or_else(|| {
                    PlannerContractError::new("jstudio_stb.fvb.blocks", "range overflows")
                })?;
            }
            if next != block.offset + block.size {
                return Err(PlannerContractError::new(
                    "jstudio_stb.fvb.blocks",
                    "do not exactly cover the embedded resource",
                ));
            }
        }
        JstudioStbBlockBody::EmbeddedCtbUnresolved {
            content_size,
            content_sha256,
        } => {
            if block.type_code != BLOCK_JCTB
                || content_size.checked_add(8) != Some(block.size)
                || *content_sha256 == Digest::ZERO
            {
                return Err(PlannerContractError::new(
                    "jstudio_stb.ctb",
                    "does not match its outer block",
                ));
            }
        }
        JstudioStbBlockBody::Object {
            id_size,
            id_hex,
            id_ascii,
            sequence_offset,
            commands,
            ..
        } => {
            let id = decode_hex(id_hex, "jstudio_stb.object.id_hex")?;
            let id_padded = align4(usize::from(*id_size)).ok_or_else(|| {
                PlannerContractError::new("jstudio_stb.object.id", "size overflows")
            })? as u32;
            if block.type_code == BLOCK_JFVB
                || block.type_code == BLOCK_JCTB
                || id.len() != usize::from(*id_size)
                || id_ascii != &ascii_nul_terminated(&id)
                || block.offset.checked_add(12 + id_padded) != Some(*sequence_offset)
                || commands.is_empty()
            {
                return Err(PlannerContractError::new(
                    "jstudio_stb.object",
                    "does not match its outer block",
                ));
            }
            let mut next = *sequence_offset;
            for (index, command) in commands.iter().enumerate() {
                if command.index != index as u32
                    || command.offset != next
                    || command.encoded_size < 4
                    || command.encoded_size
                        != 4 + if command.type_code >= 0x80 {
                            command.parameter
                        } else {
                            0
                        }
                {
                    return Err(PlannerContractError::new(
                        "jstudio_stb.sequence",
                        "is not a canonical contiguous command stream",
                    ));
                }
                validate_command(command)?;
                next = next.checked_add(command.encoded_size).ok_or_else(|| {
                    PlannerContractError::new("jstudio_stb.sequence", "range overflows")
                })?;
            }
            if next != block.offset + block.size {
                return Err(PlannerContractError::new(
                    "jstudio_stb.sequence",
                    "does not exactly consume its object block",
                ));
            }
            if !matches!(
                commands.last().map(|command| &command.operation),
                Some(JstudioSequenceOperation::End)
            ) {
                return Err(PlannerContractError::new(
                    "jstudio_stb.sequence",
                    "must end with an explicit end command",
                ));
            }
            validate_jump_targets(commands)?;
        }
    }
    Ok(())
}

fn validate_command(command: &JstudioSequenceCommand) -> Result<(), PlannerContractError> {
    let operation_matches = match command.operation {
        JstudioSequenceOperation::End => command.type_code == 0 && command.parameter == 0,
        JstudioSequenceOperation::FlagOperation { operation, value } => {
            command.type_code == 1
                && operation == (command.parameter >> 16) as u8
                && value == command.parameter as u16
        }
        JstudioSequenceOperation::Wait { frames } => {
            command.type_code == 2 && frames == command.parameter
        }
        JstudioSequenceOperation::RelativeJump {
            delta,
            target_offset,
        } => {
            command.type_code == 3
                && delta == sign_extend_u24(command.parameter)
                && relative_target(command.offset as usize, delta, "jstudio_stb.sequence.jump")?
                    == target_offset
        }
        JstudioSequenceOperation::Suspend { delta } => {
            command.type_code == 4 && delta == sign_extend_u24(command.parameter)
        }
        JstudioSequenceOperation::Paragraphs => command.type_code == 0x80,
        JstudioSequenceOperation::UnknownNoPayload => (5..0x80).contains(&command.type_code),
        JstudioSequenceOperation::UnknownPayload => command.type_code > 0x80,
    };
    let expects_payload_digest = command.type_code >= 0x80 && command.parameter != 0;
    if !operation_matches
        || command.payload_sha256.is_some() != expects_payload_digest
        || (command.type_code != 0x80 && !command.paragraphs.is_empty())
    {
        return Err(PlannerContractError::new(
            "jstudio_stb.sequence.operation",
            "does not match its encoded type and parameter",
        ));
    }
    if command.type_code == 0x80 {
        let mut next = command.offset + 4;
        for (index, paragraph) in command.paragraphs.iter().enumerate() {
            if paragraph.index != index as u32
                || paragraph.offset != next
                || !matches!(paragraph.header_size, 4 | 8)
                || paragraph.content_sha256.is_some() != (paragraph.content_size != 0)
            {
                return Err(PlannerContractError::new(
                    "jstudio_stb.paragraphs",
                    "are not canonical contiguous records",
                ));
            }
            let padded_size = align4(paragraph.content_size as usize).ok_or_else(|| {
                PlannerContractError::new("jstudio_stb.paragraphs", "size overflows")
            })? as u32;
            next = next
                .checked_add(u32::from(paragraph.header_size))
                .and_then(|offset| offset.checked_add(padded_size))
                .ok_or_else(|| {
                    PlannerContractError::new("jstudio_stb.paragraphs", "range overflows")
                })?;
        }
        if next != command.offset + command.encoded_size {
            return Err(PlannerContractError::new(
                "jstudio_stb.paragraphs",
                "do not exactly cover the command payload",
            ));
        }
    }
    Ok(())
}

fn validate_jump_targets(commands: &[JstudioSequenceCommand]) -> Result<(), PlannerContractError> {
    let offsets = commands
        .iter()
        .map(|command| command.offset)
        .collect::<BTreeSet<_>>();
    for command in commands {
        let command_target = match command.operation {
            JstudioSequenceOperation::RelativeJump { target_offset, .. } => Some(target_offset),
            _ => None,
        };
        if command_target.is_some_and(|target| !offsets.contains(&target)) {
            return Err(PlannerContractError::new(
                "jstudio_stb.sequence.jump",
                "does not target a command boundary in the same object",
            ));
        }
        for paragraph in &command.paragraphs {
            if let JstudioParagraphInterpretation::ReservedRelativeJump { target_offset, .. } =
                paragraph.interpretation
                && !offsets.contains(&target_offset)
            {
                return Err(PlannerContractError::new(
                    "jstudio_stb.paragraph.jump",
                    "does not target a command boundary in the same object",
                ));
            }
        }
    }
    Ok(())
}

fn sign_extend_u24(value: u32) -> i32 {
    if value & 0x0080_0000 != 0 {
        (value | 0xff00_0000) as i32
    } else {
        value as i32
    }
}

fn relative_target(
    base: usize,
    delta: i32,
    field: &'static str,
) -> Result<u32, PlannerContractError> {
    let target = (base as i64)
        .checked_add(i64::from(delta))
        .ok_or_else(|| PlannerContractError::new(field, "target overflows"))?;
    u32::try_from(target).map_err(|_| PlannerContractError::new(field, "target is out of range"))
}

fn ascii_fourcc(value: u32) -> Option<String> {
    let bytes = value.to_be_bytes();
    bytes
        .iter()
        .all(|byte| byte.is_ascii_graphic())
        .then(|| String::from_utf8(bytes.to_vec()).expect("ASCII is UTF-8"))
}

fn ascii_nul_terminated(bytes: &[u8]) -> Option<String> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    if bytes[end..].iter().any(|byte| *byte != 0)
        || !bytes[..end]
            .iter()
            .all(|byte| byte.is_ascii_graphic() || *byte == b' ')
    {
        return None;
    }
    Some(String::from_utf8(bytes[..end].to_vec()).expect("ASCII is UTF-8"))
}

fn align4(value: usize) -> Option<usize> {
    value.checked_add(3).map(|value| value & !3)
}

fn slice<'a>(
    bytes: &'a [u8],
    offset: usize,
    size: usize,
    field: &'static str,
) -> Result<&'a [u8], PlannerContractError> {
    let end = offset
        .checked_add(size)
        .ok_or_else(|| PlannerContractError::new(field, "range overflows"))?;
    bytes
        .get(offset..end)
        .ok_or_else(|| PlannerContractError::new(field, "is truncated"))
}

fn read_u16(bytes: &[u8], offset: usize, field: &'static str) -> Result<u16, PlannerContractError> {
    Ok(u16::from_be_bytes(
        slice(bytes, offset, 2, field)?
            .try_into()
            .expect("two-byte slice"),
    ))
}

fn read_u32(bytes: &[u8], offset: usize, field: &'static str) -> Result<u32, PlannerContractError> {
    Ok(u32::from_be_bytes(
        slice(bytes, offset, 4, field)?
            .try_into()
            .expect("four-byte slice"),
    ))
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

fn decode_hex(value: &str, field: &'static str) -> Result<Vec<u8>, PlannerContractError> {
    if !value.len().is_multiple_of(2) {
        return Err(PlannerContractError::new(
            field,
            "must contain complete bytes",
        ));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("two input bytes");
            u8::from_str_radix(pair, 16)
                .map_err(|_| PlannerContractError::new(field, "contains a non-hex byte"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object_fixture() -> Vec<u8> {
        let mut bytes = vec![0; STB_HEADER_SIZE];
        bytes[0..4].copy_from_slice(b"STB\0");
        bytes[4..6].copy_from_slice(&0xfeffu16.to_be_bytes());
        bytes[6..8].copy_from_slice(&3u16.to_be_bytes());
        bytes[12..16].copy_from_slice(&1u32.to_be_bytes());
        bytes[16..24].copy_from_slice(b"jstudio\0");
        bytes[30..32].copy_from_slice(&6u16.to_be_bytes());

        let block_offset = bytes.len();
        bytes.extend_from_slice(&[0; 12]);
        bytes[block_offset + 4..block_offset + 8].copy_from_slice(b"JACT");
        bytes[block_offset + 10..block_offset + 12].copy_from_slice(&6u16.to_be_bytes());
        bytes.extend_from_slice(b"actor\0\0\0");
        bytes.extend_from_slice(&0x0200_0003u32.to_be_bytes());
        bytes.extend_from_slice(&0x8000_0008u32.to_be_bytes());
        bytes.extend_from_slice(&4u16.to_be_bytes());
        bytes.extend_from_slice(&2u16.to_be_bytes());
        bytes.extend_from_slice(&5u32.to_be_bytes());
        bytes.extend_from_slice(&0u32.to_be_bytes());
        let block_size = (bytes.len() - block_offset) as u32;
        bytes[block_offset..block_offset + 4].copy_from_slice(&block_size.to_be_bytes());
        let size = bytes.len() as u32;
        bytes[8..12].copy_from_slice(&size.to_be_bytes());
        bytes
    }

    fn fvb_fixture() -> Vec<u8> {
        let mut bytes = vec![0; STB_HEADER_SIZE];
        bytes[0..4].copy_from_slice(b"STB\0");
        bytes[4..6].copy_from_slice(&0xfeffu16.to_be_bytes());
        bytes[6..8].copy_from_slice(&3u16.to_be_bytes());
        bytes[12..16].copy_from_slice(&1u32.to_be_bytes());
        bytes[16..24].copy_from_slice(b"jstudio\0");
        bytes[30..32].copy_from_slice(&6u16.to_be_bytes());

        let outer_offset = bytes.len();
        bytes.extend_from_slice(&36u32.to_be_bytes());
        bytes.extend_from_slice(b"JFVB");
        bytes.extend_from_slice(b"FVB\0");
        bytes.extend_from_slice(&0xfeffu16.to_be_bytes());
        bytes.extend_from_slice(&0x100u16.to_be_bytes());
        bytes.extend_from_slice(&28u32.to_be_bytes());
        bytes.extend_from_slice(&1u32.to_be_bytes());
        bytes.extend_from_slice(&12u32.to_be_bytes());
        bytes.extend_from_slice(&5u16.to_be_bytes());
        bytes.extend_from_slice(&3u16.to_be_bytes());
        bytes.extend_from_slice(b"f0\0\0");
        assert_eq!(bytes.len() - outer_offset, 36);
        let size = bytes.len() as u32;
        bytes[8..12].copy_from_slice(&size.to_be_bytes());
        bytes
    }

    #[test]
    fn extracts_object_commands_and_raw_paragraph_structure() {
        let bytes = object_fixture();
        let program = parse_jstudio_stb(Digest([1; 32]), "fixture.stb", &bytes).unwrap();
        assert_eq!(program.blocks.len(), 1);
        let JstudioStbBlockBody::Object {
            id_ascii, commands, ..
        } = &program.blocks[0].body
        else {
            panic!("expected object block");
        };
        assert_eq!(id_ascii.as_deref(), Some("actor"));
        assert_eq!(commands.len(), 3);
        assert_eq!(
            commands[0].operation,
            JstudioSequenceOperation::Wait { frames: 3 }
        );
        assert_eq!(
            commands[1].paragraphs[0].interpretation,
            JstudioParagraphInterpretation::ReservedWait { frames: 5 }
        );
        assert_eq!(
            JstudioStbProgram::decode_canonical(&program.canonical_bytes().unwrap()).unwrap(),
            program
        );
    }

    #[test]
    fn rejects_truncation_bad_sizes_and_non_boundary_jumps() {
        let bytes = object_fixture();
        assert_eq!(
            parse_jstudio_stb(Digest([1; 32]), "fixture.stb", &bytes[..bytes.len() - 1])
                .unwrap_err()
                .field(),
            "jstudio_stb.header.declared_size"
        );

        let mut bad_size = bytes.clone();
        bad_size[STB_HEADER_SIZE..STB_HEADER_SIZE + 4].copy_from_slice(&10u32.to_be_bytes());
        assert_eq!(
            parse_jstudio_stb(Digest([1; 32]), "fixture.stb", &bad_size)
                .unwrap_err()
                .field(),
            "jstudio_stb.block.size"
        );

        let mut bad_jump = bytes;
        let command_offset = STB_HEADER_SIZE + 12 + 8;
        bad_jump[command_offset..command_offset + 4].copy_from_slice(&0x0300_0002u32.to_be_bytes());
        assert_eq!(
            parse_jstudio_stb(Digest([1; 32]), "fixture.stb", &bad_jump)
                .unwrap_err()
                .field(),
            "jstudio_stb.sequence.jump"
        );
    }

    #[test]
    fn indexes_embedded_fvb_blocks_without_copying_payloads() {
        let bytes = fvb_fixture();
        let program = parse_jstudio_stb(Digest([1; 32]), "fixture.stb", &bytes).unwrap();
        let JstudioStbBlockBody::EmbeddedFvb { blocks, .. } = &program.blocks[0].body else {
            panic!("expected FVB block");
        };
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].id_ascii.as_deref(), Some("f0"));
        assert_eq!(blocks[0].content_size, 0);
    }

    #[test]
    fn canonical_validation_rejects_a_forged_command_operation() {
        let bytes = object_fixture();
        let mut program = parse_jstudio_stb(Digest([1; 32]), "fixture.stb", &bytes).unwrap();
        let JstudioStbBlockBody::Object { commands, .. } = &mut program.blocks[0].body else {
            panic!("expected object block");
        };
        commands[0].operation = JstudioSequenceOperation::End;
        assert_eq!(
            program.canonical_bytes().unwrap_err().field(),
            "jstudio_stb.sequence.operation"
        );
    }
}
