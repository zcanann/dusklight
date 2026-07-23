//! Exact-content decoding of raw JStudio data consumed by generic `d_actN` actors.

use crate::artifact::Digest;
use crate::identity::ContentIdentity;
use crate::jstudio_import::{
    JstudioParagraphInterpretation, JstudioStbBlockBody, parse_jstudio_stb,
};
use crate::logic::{ComparisonOperator, PredicateExpression, ValueReference};
use crate::return_place::GZ2E01_CONTENT_SHA256;
use crate::state::{ComponentBindingReference, ComponentKind, StateValue};
use crate::transition::StateOperation;
use crate::{PlannerContractError, canonical_json, validate_label, validate_stable_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;

pub const DEMO_ACTOR_PROGRAM_SCHEMA: &str = "dusklight.route-planner.demo-actor-program/v1";

const DEMO_SOURCE_SHA256: Digest = Digest([
    0x5f, 0xa6, 0x37, 0xbe, 0x37, 0x7c, 0xef, 0xbb, 0xbf, 0x81, 0x8c, 0xa0, 0x65, 0xe7, 0x22, 0x4d,
    0xdb, 0x1f, 0x88, 0x6b, 0x3a, 0xd0, 0x6e, 0xb5, 0x9f, 0x99, 0x6d, 0x0a, 0x42, 0xd0, 0xff, 0x5a,
]);
const JSTUDIO_SOURCE_SHA256: Digest = Digest([
    0x4a, 0x35, 0xb7, 0x03, 0xeb, 0x94, 0xb6, 0xde, 0x67, 0x44, 0x06, 0x90, 0x3d, 0x10, 0x6d, 0xb5,
    0x2c, 0xd0, 0x6e, 0x4b, 0x67, 0x2f, 0xfd, 0x5c, 0x91, 0x1a, 0x0d, 0xdd, 0x8c, 0xf3, 0xde, 0x2c,
]);
const ACTOR_NAME_SOURCE_SHA256: Digest = Digest([
    0x5c, 0x46, 0xff, 0xc7, 0x9e, 0x89, 0x1b, 0x59, 0xb0, 0x24, 0x55, 0xb8, 0x37, 0xd9, 0x96, 0x6d,
    0x05, 0xc1, 0x47, 0xd8, 0xd9, 0x5c, 0x91, 0xc6, 0x5c, 0xc8, 0x45, 0xdd, 0x84, 0x8d, 0x32, 0xad,
]);
const GZ2E01_EXECUTABLE_SHA256: Digest = Digest([
    0xe7, 0xf1, 0x97, 0x43, 0x68, 0x15, 0xe6, 0x6c, 0x4a, 0x11, 0xdf, 0x3d, 0x7b, 0xd5, 0x57, 0xd6,
    0x60, 0x83, 0xb6, 0x41, 0xff, 0x8a, 0x8e, 0x76, 0x43, 0x9f, 0x3c, 0xab, 0xa7, 0xae, 0x60, 0xe8,
]);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DemoActorProgram {
    pub schema: String,
    pub content_sha256: Digest,
    pub executable_sha256: Digest,
    pub source_program_sha256: Digest,
    pub source_resource_sha256: Digest,
    pub evidence: Vec<DemoActorEvidence>,
    pub streams: Vec<DemoActorStream>,
    pub coverage: DemoActorCoverage,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DemoActorEvidence {
    pub source_sha256: Digest,
    pub note: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DemoActorStream {
    pub object_block_index: u32,
    pub object_id: String,
    pub actor_variant: Option<u8>,
    pub writes: Vec<DemoActorWrite>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DemoActorWrite {
    pub command_index: u32,
    pub paragraph_index: u32,
    pub paragraph_offset: u32,
    pub content_sha256: Digest,
    pub commands: Vec<DemoActorCommand>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DemoActorCommand {
    pub raw_word: u32,
    pub argument_class: u8,
    pub argument_variant: u8,
    pub opcode: u8,
    pub option_flag: bool,
    pub value: u16,
    pub effect: DemoActorEffect,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DemoActorEffect {
    SetPersistentEventBit { label_index: u16 },
    SetTemporaryEventBit { label_index: u16 },
    Other,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DemoActorCoverage {
    pub generic_actor_objects: u32,
    pub decoded_raw_writes: u32,
    pub decoded_commands: u32,
    pub persistent_event_bit_writes: u32,
    pub temporary_event_bit_writes: u32,
    pub non_generic_reserved_raw_paragraphs: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DemoActorLabelBinding {
    pub label_index: u16,
    pub component_kind: ComponentKind,
    pub binding: ComponentBindingReference,
    pub byte_offset: u32,
    pub mask: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DemoActorStreamRuntimeBinding {
    pub object_block_index: u32,
    pub actor_instance_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DemoActorOrderedOperation {
    pub object_block_index: u32,
    pub command_index: u32,
    pub paragraph_index: u32,
    pub command_ordinal: u32,
    pub actor_execution_required: PredicateExpression,
    pub operation: StateOperation,
}

/// Resolves decoded generic-actor event commands into ordered potential state
/// operations. The result is deliberately not an executable transition: every
/// operation retains a predicate requiring the separately observed live actor
/// instance to be executing.
pub fn compile_ordered_demo_actor_operations(
    program: &DemoActorProgram,
    persistent_bindings: &[DemoActorLabelBinding],
    temporary_bindings: &[DemoActorLabelBinding],
    stream_bindings: &[DemoActorStreamRuntimeBinding],
) -> Result<Vec<DemoActorOrderedOperation>, PlannerContractError> {
    program.validate()?;
    validate_label_bindings("demo_actor.persistent_bindings", persistent_bindings)?;
    validate_label_bindings("demo_actor.temporary_bindings", temporary_bindings)?;
    let mut prior_stream = None;
    for binding in stream_bindings {
        validate_stable_id(
            "demo_actor.stream_bindings.actor_instance_id",
            &binding.actor_instance_id,
        )?;
        if prior_stream.is_some_and(|prior| prior >= binding.object_block_index) {
            return Err(PlannerContractError::new(
                "demo_actor.stream_bindings",
                "must be unique and sorted by object block index",
            ));
        }
        if !program
            .streams
            .iter()
            .any(|stream| stream.object_block_index == binding.object_block_index)
        {
            return Err(PlannerContractError::new(
                "demo_actor.stream_bindings",
                format!(
                    "references unknown object block {}",
                    binding.object_block_index
                ),
            ));
        }
        prior_stream = Some(binding.object_block_index);
    }

    let mut output = Vec::new();
    for stream in &program.streams {
        let mut ordinal = 0u32;
        for write in &stream.writes {
            for command in &write.commands {
                let binding = match command.effect {
                    DemoActorEffect::SetPersistentEventBit { label_index } => Some((
                        label_index,
                        persistent_bindings,
                        "demo_actor.persistent_bindings",
                    )),
                    DemoActorEffect::SetTemporaryEventBit { label_index } => Some((
                        label_index,
                        temporary_bindings,
                        "demo_actor.temporary_bindings",
                    )),
                    DemoActorEffect::Other => None,
                };
                let Some((label_index, bindings, field)) = binding else {
                    ordinal = ordinal.checked_add(1).ok_or_else(|| {
                        PlannerContractError::new("demo_actor.command_ordinal", "overflowed")
                    })?;
                    continue;
                };
                let raw = bindings
                    .iter()
                    .find(|binding| binding.label_index == label_index)
                    .ok_or_else(|| {
                        PlannerContractError::new(
                            field,
                            format!("has no raw binding for label {label_index}"),
                        )
                    })?;
                let actor = stream_bindings
                    .iter()
                    .find(|binding| binding.object_block_index == stream.object_block_index)
                    .ok_or_else(|| {
                        PlannerContractError::new(
                            "demo_actor.stream_bindings",
                            format!(
                                "has no live actor binding for object block {}",
                                stream.object_block_index
                            ),
                        )
                    })?;
                let operation = StateOperation::WriteBoundRaw {
                    component_kind: raw.component_kind.clone(),
                    binding: raw.binding.clone(),
                    byte_offset: raw.byte_offset,
                    mask: vec![raw.mask],
                    value: vec![raw.mask],
                };
                operation.validate()?;
                output.push(DemoActorOrderedOperation {
                    object_block_index: stream.object_block_index,
                    command_index: write.command_index,
                    paragraph_index: write.paragraph_index,
                    command_ordinal: ordinal,
                    actor_execution_required: PredicateExpression::Compare {
                        left: ValueReference::ActorField {
                            instance_id: actor.actor_instance_id.clone(),
                            field: "executing".into(),
                        },
                        operator: ComparisonOperator::Equal,
                        right: ValueReference::Literal {
                            value: StateValue::Boolean(true),
                        },
                    },
                    operation,
                });
                ordinal = ordinal.checked_add(1).ok_or_else(|| {
                    PlannerContractError::new("demo_actor.command_ordinal", "overflowed")
                })?;
            }
        }
    }
    Ok(output)
}

fn validate_label_bindings(
    field: &str,
    bindings: &[DemoActorLabelBinding],
) -> Result<(), PlannerContractError> {
    let mut prior = None;
    for binding in bindings {
        if binding.mask == 0 {
            return Err(PlannerContractError::new(field, "contains a zero mask"));
        }
        if prior.is_some_and(|prior| prior >= binding.label_index) {
            return Err(PlannerContractError::new(
                field,
                "must be unique and sorted by label index",
            ));
        }
        StateOperation::WriteBoundRaw {
            component_kind: binding.component_kind.clone(),
            binding: binding.binding.clone(),
            byte_offset: binding.byte_offset,
            mask: vec![binding.mask],
            value: vec![binding.mask],
        }
        .validate()?;
        prior = Some(binding.label_index);
    }
    Ok(())
}

impl DemoActorProgram {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != DEMO_ACTOR_PROGRAM_SCHEMA {
            return Err(PlannerContractError::new(
                "demo_actor_program.schema",
                "is unsupported",
            ));
        }
        if [
            self.content_sha256,
            self.executable_sha256,
            self.source_program_sha256,
            self.source_resource_sha256,
        ]
        .contains(&Digest::ZERO)
        {
            return Err(PlannerContractError::new(
                "demo_actor_program.identity",
                "must contain nonzero provenance digests",
            ));
        }
        if self.evidence.is_empty() {
            return Err(PlannerContractError::new(
                "demo_actor_program.evidence",
                "must not be empty",
            ));
        }
        let mut prior_evidence = None;
        for evidence in &self.evidence {
            if evidence.source_sha256 == Digest::ZERO {
                return Err(PlannerContractError::new(
                    "demo_actor_program.evidence.source_sha256",
                    "must be nonzero",
                ));
            }
            validate_label("demo_actor_program.evidence.note", &evidence.note)?;
            let key = (evidence.source_sha256, evidence.note.as_str());
            if prior_evidence.is_some_and(|prior| prior >= key) {
                return Err(PlannerContractError::new(
                    "demo_actor_program.evidence",
                    "must be unique and sorted",
                ));
            }
            prior_evidence = Some(key);
        }

        let mut prior_stream = None;
        let mut coordinates = BTreeSet::new();
        let mut writes = 0u32;
        let mut commands = 0u32;
        let mut persistent = 0u32;
        let mut temporary = 0u32;
        for stream in &self.streams {
            validate_generic_actor_id(&stream.object_id, stream.actor_variant)?;
            if prior_stream.is_some_and(|prior| prior >= stream.object_block_index) {
                return Err(PlannerContractError::new(
                    "demo_actor_program.streams",
                    "must be ordered by unique object block index",
                ));
            }
            prior_stream = Some(stream.object_block_index);
            let mut prior_write = None;
            for write in &stream.writes {
                if write.content_sha256 == Digest::ZERO || write.commands.is_empty() {
                    return Err(PlannerContractError::new(
                        "demo_actor_program.streams.writes",
                        "must contain a digest and at least one command",
                    ));
                }
                if !coordinates.insert((
                    stream.object_block_index,
                    write.command_index,
                    write.paragraph_index,
                )) {
                    return Err(PlannerContractError::new(
                        "demo_actor_program.streams.writes",
                        "contain duplicate coordinates",
                    ));
                }
                let write_order = (write.command_index, write.paragraph_index);
                if prior_write.is_some_and(|prior| prior >= write_order) {
                    return Err(PlannerContractError::new(
                        "demo_actor_program.streams.writes",
                        "must preserve unique command and paragraph order",
                    ));
                }
                prior_write = Some(write_order);
                writes = checked_count(writes, 1)?;
                commands = checked_count(commands, write.commands.len() as u32)?;
                for command in &write.commands {
                    let decoded = decode_word(command.raw_word);
                    if &decoded != command {
                        return Err(PlannerContractError::new(
                            "demo_actor_program.streams.writes.commands",
                            "do not match their raw words",
                        ));
                    }
                    match command.effect {
                        DemoActorEffect::SetPersistentEventBit { .. } => {
                            persistent = checked_count(persistent, 1)?;
                        }
                        DemoActorEffect::SetTemporaryEventBit { .. } => {
                            temporary = checked_count(temporary, 1)?;
                        }
                        DemoActorEffect::Other => {}
                    }
                }
            }
        }
        if self.coverage.generic_actor_objects != self.streams.len() as u32
            || self.coverage.decoded_raw_writes != writes
            || self.coverage.decoded_commands != commands
            || self.coverage.persistent_event_bit_writes != persistent
            || self.coverage.temporary_event_bit_writes != temporary
        {
            return Err(PlannerContractError::new(
                "demo_actor_program.coverage",
                "does not match decoded streams",
            ));
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let program: Self = serde_json::from_slice(bytes)?;
        program.validate()?;
        if program.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "demo_actor_program",
                "is not canonical JSON",
            ));
        }
        Ok(program)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

pub fn extract_gz2e01_demo_actor_program(
    content: &ContentIdentity,
    archive_sha256: Digest,
    resource_name: &str,
    bytes: &[u8],
) -> Result<DemoActorProgram, PlannerContractError> {
    content.validate()?;
    if content.digest()? != GZ2E01_CONTENT_SHA256
        || content.fingerprint.executable_sha256 != GZ2E01_EXECUTABLE_SHA256
    {
        return Err(PlannerContractError::new(
            "demo_actor_program.identity",
            "is not the audited GZ2E01 context",
        ));
    }
    let structural = parse_jstudio_stb(archive_sha256, resource_name, bytes)?;
    let mut streams = Vec::new();
    let mut non_generic_reserved_raw = 0u32;
    for block in &structural.blocks {
        let JstudioStbBlockBody::Object {
            id_ascii, commands, ..
        } = &block.body
        else {
            continue;
        };
        let Some(object_id) = id_ascii else {
            non_generic_reserved_raw =
                checked_count(non_generic_reserved_raw, count_reserved_raw(commands)?)?;
            continue;
        };
        let actor_variant = parse_generic_actor_id(object_id);
        if actor_variant.is_none() && object_id != "d_act" {
            non_generic_reserved_raw =
                checked_count(non_generic_reserved_raw, count_reserved_raw(commands)?)?;
            continue;
        }
        let mut writes = Vec::new();
        for command in commands {
            for paragraph in &command.paragraphs {
                if !matches!(
                    paragraph.interpretation,
                    JstudioParagraphInterpretation::ReservedRawData
                ) {
                    continue;
                }
                let start = usize::try_from(paragraph.offset)
                    .ok()
                    .and_then(|offset| offset.checked_add(paragraph.header_size.into()))
                    .ok_or_else(|| {
                        PlannerContractError::new(
                            "demo_actor_program.paragraph",
                            "content offset overflows",
                        )
                    })?;
                let end = start
                    .checked_add(paragraph.content_size as usize)
                    .ok_or_else(|| {
                        PlannerContractError::new(
                            "demo_actor_program.paragraph",
                            "content range overflows",
                        )
                    })?;
                let raw = bytes.get(start..end).ok_or_else(|| {
                    PlannerContractError::new(
                        "demo_actor_program.paragraph",
                        "content is outside the resource",
                    )
                })?;
                let digest = Digest(Sha256::digest(raw).into());
                if paragraph.content_sha256 != Some(digest) {
                    return Err(PlannerContractError::new(
                        "demo_actor_program.paragraph",
                        "digest disagrees with structural extraction",
                    ));
                }
                writes.push(DemoActorWrite {
                    command_index: command.index,
                    paragraph_index: paragraph.index,
                    paragraph_offset: paragraph.offset,
                    content_sha256: digest,
                    commands: decode_raw_write(raw)?,
                });
            }
        }
        streams.push(DemoActorStream {
            object_block_index: block.index,
            object_id: object_id.clone(),
            actor_variant,
            writes,
        });
    }
    let decoded_raw_writes = streams
        .iter()
        .map(|stream| stream.writes.len() as u32)
        .sum();
    let decoded_commands = streams
        .iter()
        .flat_map(|stream| &stream.writes)
        .map(|write| write.commands.len() as u32)
        .sum();
    let persistent_event_bit_writes = streams
        .iter()
        .flat_map(|stream| &stream.writes)
        .flat_map(|write| &write.commands)
        .filter(|command| {
            matches!(
                command.effect,
                DemoActorEffect::SetPersistentEventBit { .. }
            )
        })
        .count() as u32;
    let temporary_event_bit_writes = streams
        .iter()
        .flat_map(|stream| &stream.writes)
        .flat_map(|write| &write.commands)
        .filter(|command| matches!(command.effect, DemoActorEffect::SetTemporaryEventBit { .. }))
        .count() as u32;
    let program = DemoActorProgram {
        schema: DEMO_ACTOR_PROGRAM_SCHEMA.into(),
        content_sha256: content.digest()?,
        executable_sha256: content.fingerprint.executable_sha256,
        source_program_sha256: structural.digest()?,
        source_resource_sha256: structural.source.resource_sha256,
        evidence: vec![
            DemoActorEvidence {
                source_sha256: JSTUDIO_SOURCE_SHA256,
                note: "dDemo_actor_c stores raw JStudio data and decodes status 51 as packed big-endian command words.".into(),
            },
            DemoActorEvidence {
                source_sha256: ACTOR_NAME_SOURCE_SHA256,
                note: "The GZ2E01 stage actor-name table maps d_act and d_act0 through d_act31 to the generic Demo00 actor.".into(),
            },
            DemoActorEvidence {
                source_sha256: DEMO_SOURCE_SHA256,
                note: "daDemo00_c dispatches packed opcode 1 to a persistent event-bit set and opcode 3 to a temporary event-bit set.".into(),
            },
        ],
        coverage: DemoActorCoverage {
            generic_actor_objects: streams.len() as u32,
            decoded_raw_writes,
            decoded_commands,
            persistent_event_bit_writes,
            temporary_event_bit_writes,
            non_generic_reserved_raw_paragraphs: non_generic_reserved_raw,
        },
        streams,
    };
    program.validate()?;
    Ok(program)
}

fn count_reserved_raw(
    commands: &[crate::jstudio_import::JstudioSequenceCommand],
) -> Result<u32, PlannerContractError> {
    let count = commands
        .iter()
        .flat_map(|command| &command.paragraphs)
        .filter(|paragraph| {
            matches!(
                paragraph.interpretation,
                JstudioParagraphInterpretation::ReservedRawData
            )
        })
        .count();
    u32::try_from(count)
        .map_err(|_| PlannerContractError::new("demo_actor_program.coverage", "count overflows"))
}

fn decode_raw_write(raw: &[u8]) -> Result<Vec<DemoActorCommand>, PlannerContractError> {
    let Some(&header) = raw.first() else {
        return Err(PlannerContractError::new(
            "demo_actor_program.raw_write",
            "is empty",
        ));
    };
    if header & !0x08 != 51 || header & 0x07 != 3 {
        return Err(PlannerContractError::new(
            "demo_actor_program.raw_write",
            "is not status-51 packed u32 data",
        ));
    }
    let (count, start) = if header & 0x08 != 0 {
        let count = *raw.get(1).ok_or_else(|| {
            PlannerContractError::new("demo_actor_program.raw_write", "is missing its count")
        })?;
        (usize::from(count), 2usize)
    } else {
        (1usize, 1usize)
    };
    let end = start
        .checked_add(count.checked_mul(4).ok_or_else(|| {
            PlannerContractError::new("demo_actor_program.raw_write", "size overflows")
        })?)
        .ok_or_else(|| {
            PlannerContractError::new("demo_actor_program.raw_write", "size overflows")
        })?;
    let payload = raw
        .get(start..end)
        .ok_or_else(|| PlannerContractError::new("demo_actor_program.raw_write", "is truncated"))?;
    let padding = raw.get(end..).ok_or_else(|| {
        PlannerContractError::new("demo_actor_program.raw_write", "range is invalid")
    })?;
    if padding.len() > 3 || padding.iter().any(|byte| *byte != 0) {
        return Err(PlannerContractError::new(
            "demo_actor_program.raw_write",
            "has noncanonical alignment padding",
        ));
    }
    Ok(payload
        .chunks_exact(4)
        .map(|word| {
            decode_word(u32::from_be_bytes(
                word.try_into().expect("four-byte chunk"),
            ))
        })
        .collect())
}

fn decode_word(raw_word: u32) -> DemoActorCommand {
    let argument_class = (raw_word >> 30) as u8;
    let argument_variant = ((raw_word >> 24) & 0x0f) as u8;
    let opcode = ((raw_word >> 16) & 0x0f) as u8;
    let option_flag = raw_word & 0x0080_0000 != 0;
    let value = raw_word as u16;
    let effect = match (argument_class, opcode) {
        (0, 1) => DemoActorEffect::SetPersistentEventBit { label_index: value },
        (0, 3) => DemoActorEffect::SetTemporaryEventBit { label_index: value },
        _ => DemoActorEffect::Other,
    };
    DemoActorCommand {
        raw_word,
        argument_class,
        argument_variant,
        opcode,
        option_flag,
        value,
        effect,
    }
}

fn parse_generic_actor_id(id: &str) -> Option<u8> {
    id.strip_prefix("d_act")
        .filter(|suffix| !suffix.is_empty())
        .and_then(|suffix| suffix.parse::<u8>().ok())
        .filter(|variant| *variant <= 31)
}

fn validate_generic_actor_id(
    id: &str,
    actor_variant: Option<u8>,
) -> Result<(), PlannerContractError> {
    if id == "d_act" && actor_variant.is_none()
        || parse_generic_actor_id(id).is_some_and(|variant| actor_variant == Some(variant))
    {
        Ok(())
    } else {
        Err(PlannerContractError::new(
            "demo_actor_program.streams.object_id",
            "does not match its generic actor variant",
        ))
    }
}

fn checked_count(value: u32, increment: u32) -> Result<u32, PlannerContractError> {
    value
        .checked_add(increment)
        .ok_or_else(|| PlannerContractError::new("demo_actor_program.coverage", "count overflows"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event_program() -> DemoActorProgram {
        DemoActorProgram {
            schema: DEMO_ACTOR_PROGRAM_SCHEMA.into(),
            content_sha256: Digest([1; 32]),
            executable_sha256: Digest([2; 32]),
            source_program_sha256: Digest([3; 32]),
            source_resource_sha256: Digest([4; 32]),
            evidence: vec![DemoActorEvidence {
                source_sha256: Digest([5; 32]),
                note: "Synthetic ordered generic-actor command fixture.".into(),
            }],
            streams: vec![DemoActorStream {
                object_block_index: 7,
                object_id: "d_act0".into(),
                actor_variant: Some(0),
                writes: vec![DemoActorWrite {
                    command_index: 2,
                    paragraph_index: 3,
                    paragraph_offset: 4,
                    content_sha256: Digest([6; 32]),
                    commands: vec![decode_word(0x0001_002d), decode_word(0x0003_0007)],
                }],
            }],
            coverage: DemoActorCoverage {
                generic_actor_objects: 1,
                decoded_raw_writes: 1,
                decoded_commands: 2,
                persistent_event_bit_writes: 1,
                temporary_event_bit_writes: 1,
                non_generic_reserved_raw_paragraphs: 0,
            },
        }
    }

    #[test]
    fn decodes_persistent_and_temporary_event_writes_from_backing_words() {
        let commands = decode_raw_write(&[
            0x3b, 0x02, 0x00, 0x01, 0x00, 0x2d, 0x00, 0x03, 0x00, 0x07, 0x00, 0x00,
        ])
        .unwrap();
        assert_eq!(
            commands[0].effect,
            DemoActorEffect::SetPersistentEventBit { label_index: 45 }
        );
        assert_eq!(
            commands[1].effect,
            DemoActorEffect::SetTemporaryEventBit { label_index: 7 }
        );
    }

    #[test]
    fn exact_demo07_generic_actor_payloads_have_no_event_bit_write() {
        for raw in [
            &[0x33, 0x00, 0x00, 0x0b, 0x00, 0x00, 0x00, 0x00][..],
            &[0x33, 0x00, 0x00, 0x05, 0x1e, 0x00, 0x00, 0x00][..],
        ] {
            let commands = decode_raw_write(raw).unwrap();
            assert!(
                commands
                    .iter()
                    .all(|command| command.effect == DemoActorEffect::Other)
            );
        }

        let mut program = event_program();
        program.streams[0].writes[0].commands = vec![decode_word(0x0000_000b)];
        program.coverage.decoded_commands = 1;
        program.coverage.persistent_event_bit_writes = 0;
        program.coverage.temporary_event_bit_writes = 0;
        assert!(
            compile_ordered_demo_actor_operations(&program, &[], &[], &[])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn rejects_truncation_wrong_status_and_nonzero_padding() {
        assert!(decode_raw_write(&[0x33, 0, 0]).is_err());
        assert!(decode_raw_write(&[0x32, 0, 0, 0, 0]).is_err());
        assert!(decode_raw_write(&[0x33, 0, 0, 0, 0, 1]).is_err());
    }

    #[test]
    fn compiles_ordered_raw_operations_but_requires_live_actor_execution() {
        let persistent = [DemoActorLabelBinding {
            label_index: 45,
            component_kind: ComponentKind::PersistentSave,
            binding: ComponentBindingReference::ActiveRuntimeFile,
            byte_offset: 6,
            mask: 0x20,
        }];
        let temporary = [DemoActorLabelBinding {
            label_index: 7,
            component_kind: ComponentKind::TemporaryFlags,
            binding: ComponentBindingReference::ActiveRuntimeFile,
            byte_offset: 1,
            mask: 0x04,
        }];
        let streams = [DemoActorStreamRuntimeBinding {
            object_block_index: 7,
            actor_instance_id: "actor.demo-0".into(),
        }];
        let operations = compile_ordered_demo_actor_operations(
            &event_program(),
            &persistent,
            &temporary,
            &streams,
        )
        .unwrap();
        assert_eq!(operations.len(), 2);
        assert_eq!(operations[0].command_ordinal, 0);
        assert_eq!(operations[1].command_ordinal, 1);
        assert!(matches!(
            operations[0].operation,
            StateOperation::WriteBoundRaw {
                byte_offset: 6,
                ref mask,
                ref value,
                ..
            } if mask == &[0x20] && value == &[0x20]
        ));
        assert!(matches!(
            operations[1].operation,
            StateOperation::WriteBoundRaw {
                byte_offset: 1,
                ref mask,
                ref value,
                ..
            } if mask == &[0x04] && value == &[0x04]
        ));
        assert_eq!(
            operations[0].actor_execution_required,
            PredicateExpression::Compare {
                left: ValueReference::ActorField {
                    instance_id: "actor.demo-0".into(),
                    field: "executing".into(),
                },
                operator: ComparisonOperator::Equal,
                right: ValueReference::Literal {
                    value: StateValue::Boolean(true),
                },
            }
        );
        assert!(
            compile_ordered_demo_actor_operations(&event_program(), &persistent, &temporary, &[],)
                .is_err()
        );
    }

    #[test]
    fn rejects_reordered_authored_writes_and_unknown_stream_bindings() {
        let mut program = event_program();
        program.streams[0].writes.push(DemoActorWrite {
            command_index: 1,
            paragraph_index: 4,
            paragraph_offset: 8,
            content_sha256: Digest([7; 32]),
            commands: vec![decode_word(0x0000_000b)],
        });
        program.coverage.decoded_raw_writes = 2;
        program.coverage.decoded_commands = 3;
        assert!(program.validate().is_err());

        let streams = [DemoActorStreamRuntimeBinding {
            object_block_index: 8,
            actor_instance_id: "actor.demo-1".into(),
        }];
        assert!(
            compile_ordered_demo_actor_operations(&event_program(), &[], &[], &streams).is_err()
        );
    }
}
