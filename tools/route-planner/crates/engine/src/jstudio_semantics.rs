//! Exact-content JStudio adaptor dispatch profiles and semantic resolution.
//!
//! The STB container parser deliberately does not guess what an object-specific
//! paragraph means. This layer joins those paragraphs to an audited executable
//! profile and decodes only the payload contracts proven by that profile.

use crate::artifact::Digest;
use crate::identity::ContentIdentity;
use crate::jstudio_import::{
    JstudioParagraphInterpretation, JstudioStbBlockBody, parse_jstudio_stb,
};
use crate::{PlannerContractError, canonical_json, validate_label, validate_stable_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;

pub const JSTUDIO_ADAPTOR_PROFILE_SCHEMA: &str =
    "dusklight.route-planner.jstudio-adaptor-profile/v1";
pub const JSTUDIO_SEMANTIC_PROGRAM_SCHEMA: &str =
    "dusklight.route-planner.jstudio-semantic-program/v1";
const BUNDLED_GZ2E01_ADAPTOR_PROFILE: &[u8] =
    include_bytes!("../data/jstudio-adaptor-profiles/gz2e01.json");

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JstudioAdaptorProfile {
    pub schema: String,
    pub id: String,
    pub content_sha256: Digest,
    pub executable_sha256: Digest,
    pub target_version: u16,
    pub evidence: Vec<JstudioAdaptorEvidence>,
    pub rules: Vec<JstudioAdaptorRule>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JstudioAdaptorEvidence {
    pub source_sha256: Digest,
    pub note: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JstudioAdaptorRule {
    pub object_type: String,
    pub selector: u32,
    pub semantic: String,
    pub target: JstudioAdaptorTarget,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum JstudioAdaptorTarget {
    Variable {
        indices: Vec<u32>,
    },
    Adaptor {
        handler: JstudioAdaptorHandler,
    },
    VariableOrAdaptor {
        indices: Vec<u32>,
        handler: JstudioAdaptorHandler,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JstudioAdaptorHandler {
    ActorShape,
    ActorAnimation,
    ActorAnimationMode,
    Message,
    ParticleResource,
    ParticleBegin,
    ParticleBeginFadeIn,
    ParticleEnd,
    ParticleRepeat,
    SoundResource,
    SoundBegin,
    SoundEndFadeOut,
    SoundOnExitNotEnd,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JstudioSemanticProgram {
    pub schema: String,
    pub content_sha256: Digest,
    pub source_program_sha256: Digest,
    pub profile_sha256: Digest,
    pub source_resource_sha256: Digest,
    pub records: Vec<JstudioSemanticParagraph>,
    pub coverage: JstudioSemanticCoverage,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JstudioSemanticCoverage {
    pub reserved_paragraphs: u32,
    pub object_specific_paragraphs: u32,
    pub resolved_object_specific_paragraphs: u32,
    pub unresolved_object_specific_paragraphs: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JstudioSemanticParagraph {
    pub object_block_index: u32,
    pub object_type_code: u32,
    pub object_type_ascii: Option<String>,
    pub object_id_hex: String,
    pub command_index: u32,
    pub paragraph_index: u32,
    pub paragraph_offset: u32,
    pub type_code: u32,
    pub selector: u32,
    pub operation_code: u8,
    pub content_size: u32,
    pub content_sha256: Option<Digest>,
    pub resolution: JstudioSemanticResolution,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum JstudioSemanticResolution {
    Variable {
        semantic: String,
        indices: Vec<u32>,
        behavior: JstudioVariableBehavior,
        payload: JstudioSemanticPayload,
    },
    AdaptorCall {
        semantic: String,
        handler: JstudioAdaptorHandler,
        behavior: JstudioAdaptorBehavior,
        payload: JstudioSemanticPayload,
    },
    VariableOutputBindingOnly {
        semantic: String,
        indices: Vec<u32>,
        handler: JstudioAdaptorHandler,
        payload_sha256: Option<Digest>,
    },
    Unresolved {
        reason: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JstudioVariableBehavior {
    Clear,
    ImmediateFloat32,
    TimeScaledFloat32,
    FunctionValueName,
    FunctionValueIndex,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JstudioAdaptorBehavior {
    Invoke,
    ImmediateUnsigned32,
    ImmediateBoolean32,
    ImmediateFloat32,
    DirectName,
    DirectUnsigned32,
}

/// Numeric floating-point data is retained as exact IEEE-754 words. This keeps
/// canonical JSON deterministic and preserves NaNs without converting them.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum JstudioSemanticPayload {
    None,
    Float32Bits {
        words: Vec<u32>,
    },
    Unsigned32 {
        values: Vec<u32>,
    },
    Bytes {
        hex: String,
        ascii_nul_terminated: Option<String>,
    },
}

impl JstudioAdaptorProfile {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != JSTUDIO_ADAPTOR_PROFILE_SCHEMA {
            return Err(PlannerContractError::new(
                "jstudio_adaptor_profile.schema",
                "is unsupported",
            ));
        }
        validate_stable_id("jstudio_adaptor_profile.id", &self.id)?;
        if self.content_sha256 == Digest::ZERO || self.executable_sha256 == Digest::ZERO {
            return Err(PlannerContractError::new(
                "jstudio_adaptor_profile.identity",
                "must pin nonzero content and executable digests",
            ));
        }
        if !(2..=6).contains(&self.target_version) {
            return Err(PlannerContractError::new(
                "jstudio_adaptor_profile.target_version",
                "is unsupported",
            ));
        }
        if self.evidence.is_empty() || self.rules.is_empty() {
            return Err(PlannerContractError::new(
                "jstudio_adaptor_profile",
                "must contain evidence and at least one dispatch rule",
            ));
        }
        let mut prior_evidence = None;
        for evidence in &self.evidence {
            if evidence.source_sha256 == Digest::ZERO {
                return Err(PlannerContractError::new(
                    "jstudio_adaptor_profile.evidence.source_sha256",
                    "must be nonzero",
                ));
            }
            validate_label("jstudio_adaptor_profile.evidence.note", &evidence.note)?;
            let key = (evidence.source_sha256, evidence.note.as_str());
            if prior_evidence.is_some_and(|prior| prior >= key) {
                return Err(PlannerContractError::new(
                    "jstudio_adaptor_profile.evidence",
                    "must be unique and sorted by source digest then note",
                ));
            }
            prior_evidence = Some(key);
        }
        let mut keys = BTreeSet::new();
        let mut prior_rule = None;
        for rule in &self.rules {
            validate_object_type(&rule.object_type)?;
            if rule.selector < 8 || rule.selector > (u32::MAX >> 5) {
                return Err(PlannerContractError::new(
                    "jstudio_adaptor_profile.rules.selector",
                    "must encode an object-specific paragraph selector",
                ));
            }
            validate_stable_id("jstudio_adaptor_profile.rules.semantic", &rule.semantic)?;
            if !keys.insert((rule.object_type.clone(), rule.selector)) {
                return Err(PlannerContractError::new(
                    "jstudio_adaptor_profile.rules",
                    "contain a duplicate object-type/selector dispatch",
                ));
            }
            let key = (rule.object_type.as_str(), rule.selector);
            if prior_rule.is_some_and(|prior| prior >= key) {
                return Err(PlannerContractError::new(
                    "jstudio_adaptor_profile.rules",
                    "must be sorted by object type then selector",
                ));
            }
            prior_rule = Some(key);
            match &rule.target {
                JstudioAdaptorTarget::Variable { indices } => validate_indices(indices)?,
                JstudioAdaptorTarget::Adaptor { .. } => {}
                JstudioAdaptorTarget::VariableOrAdaptor { indices, .. } => {
                    validate_indices(indices)?;
                    if indices.len() != 1 {
                        return Err(PlannerContractError::new(
                            "jstudio_adaptor_profile.rules.indices",
                            "variable-or-adaptor dispatch requires one output variable",
                        ));
                    }
                }
            }
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
        let profile: Self = serde_json::from_slice(bytes)?;
        profile.validate()?;
        if profile.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "jstudio_adaptor_profile",
                "is not canonical JSON",
            ));
        }
        Ok(profile)
    }
}

pub fn bundled_gz2e01_adaptor_profile() -> Result<JstudioAdaptorProfile, PlannerContractError> {
    JstudioAdaptorProfile::decode_canonical(BUNDLED_GZ2E01_ADAPTOR_PROFILE)
}

impl JstudioSemanticProgram {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != JSTUDIO_SEMANTIC_PROGRAM_SCHEMA {
            return Err(PlannerContractError::new(
                "jstudio_semantic_program.schema",
                "is unsupported",
            ));
        }
        if [
            self.content_sha256,
            self.source_program_sha256,
            self.profile_sha256,
            self.source_resource_sha256,
        ]
        .contains(&Digest::ZERO)
        {
            return Err(PlannerContractError::new(
                "jstudio_semantic_program.identity",
                "must contain nonzero provenance digests",
            ));
        }
        if self.coverage.object_specific_paragraphs != self.records.len() as u32
            || self.coverage.resolved_object_specific_paragraphs
                + self.coverage.unresolved_object_specific_paragraphs
                != self.coverage.object_specific_paragraphs
        {
            return Err(PlannerContractError::new(
                "jstudio_semantic_program.coverage",
                "does not match its semantic records",
            ));
        }
        let mut coordinates = BTreeSet::new();
        let mut resolved = 0u32;
        for record in &self.records {
            if record.selector != record.type_code >> 5
                || u32::from(record.operation_code) != record.type_code & 0x1f
                || !coordinates.insert((
                    record.object_block_index,
                    record.command_index,
                    record.paragraph_index,
                ))
            {
                return Err(PlannerContractError::new(
                    "jstudio_semantic_program.records",
                    "contain inconsistent or duplicate paragraph coordinates",
                ));
            }
            if let Some(object_type) = &record.object_type_ascii {
                validate_object_type(object_type)?;
            }
            if record.content_size == 0 && record.content_sha256.is_some()
                || record.content_size != 0 && record.content_sha256.is_none()
            {
                return Err(PlannerContractError::new(
                    "jstudio_semantic_program.records.content_sha256",
                    "must be present exactly when content is nonempty",
                ));
            }
            validate_semantic_resolution(record)?;
            if !matches!(
                record.resolution,
                JstudioSemanticResolution::Unresolved { .. }
            ) {
                resolved = resolved.checked_add(1).ok_or_else(|| {
                    PlannerContractError::new("jstudio_semantic_program.coverage", "overflows")
                })?;
            }
        }
        if resolved != self.coverage.resolved_object_specific_paragraphs {
            return Err(PlannerContractError::new(
                "jstudio_semantic_program.coverage",
                "resolved count does not match semantic records",
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
                "jstudio_semantic_program",
                "is not canonical JSON",
            ));
        }
        Ok(program)
    }
}

pub fn resolve_jstudio_stb_semantics(
    content: &ContentIdentity,
    profile: &JstudioAdaptorProfile,
    archive_sha256: Digest,
    resource_name: &str,
    bytes: &[u8],
) -> Result<JstudioSemanticProgram, PlannerContractError> {
    content.validate()?;
    profile.validate()?;
    let content_sha256 = content.digest()?;
    if profile.content_sha256 != content_sha256
        || profile.executable_sha256 != content.fingerprint.executable_sha256
    {
        return Err(PlannerContractError::new(
            "jstudio_adaptor_profile.identity",
            "does not match the selected exact content identity",
        ));
    }
    let structural = parse_jstudio_stb(archive_sha256, resource_name, bytes)?;
    if structural.target_version != profile.target_version {
        return Err(PlannerContractError::new(
            "jstudio_adaptor_profile.target_version",
            "does not match the STB target version",
        ));
    }
    let mut records = Vec::new();
    let mut reserved_paragraphs = 0u32;
    for block in &structural.blocks {
        let JstudioStbBlockBody::Object {
            id_hex, commands, ..
        } = &block.body
        else {
            continue;
        };
        for command in commands {
            for paragraph in &command.paragraphs {
                if !matches!(
                    paragraph.interpretation,
                    JstudioParagraphInterpretation::ObjectSpecific
                ) {
                    reserved_paragraphs = reserved_paragraphs.checked_add(1).ok_or_else(|| {
                        PlannerContractError::new("jstudio_semantic_program.coverage", "overflows")
                    })?;
                    continue;
                }
                let content_start = usize::try_from(paragraph.offset)
                    .ok()
                    .and_then(|offset| offset.checked_add(usize::from(paragraph.header_size)))
                    .ok_or_else(|| {
                        PlannerContractError::new(
                            "jstudio_semantic_program.paragraph",
                            "content offset overflows",
                        )
                    })?;
                let content_end = content_start
                    .checked_add(paragraph.content_size as usize)
                    .ok_or_else(|| {
                        PlannerContractError::new(
                            "jstudio_semantic_program.paragraph",
                            "content range overflows",
                        )
                    })?;
                let paragraph_content = bytes.get(content_start..content_end).ok_or_else(|| {
                    PlannerContractError::new(
                        "jstudio_semantic_program.paragraph",
                        "content range is outside the verified resource",
                    )
                })?;
                let content_sha256 = (!paragraph_content.is_empty())
                    .then(|| Digest(Sha256::digest(paragraph_content).into()));
                if content_sha256 != paragraph.content_sha256 {
                    return Err(PlannerContractError::new(
                        "jstudio_semantic_program.paragraph",
                        "content digest disagrees with the structural program",
                    ));
                }
                let selector = paragraph.type_code >> 5;
                let operation_code = (paragraph.type_code & 0x1f) as u8;
                let rule = block.type_ascii.as_ref().and_then(|object_type| {
                    profile
                        .rules
                        .iter()
                        .find(|rule| &rule.object_type == object_type && rule.selector == selector)
                });
                let resolution = match rule {
                    Some(rule) => resolve_rule(rule, operation_code, paragraph_content),
                    None => JstudioSemanticResolution::Unresolved {
                        reason: match &block.type_ascii {
                            Some(object_type) => {
                                format!("profile has no {object_type} selector {selector} dispatch")
                            }
                            None => "object type is not an ASCII four-character code".into(),
                        },
                    },
                };
                records.push(JstudioSemanticParagraph {
                    object_block_index: block.index,
                    object_type_code: block.type_code,
                    object_type_ascii: block.type_ascii.clone(),
                    object_id_hex: id_hex.clone(),
                    command_index: command.index,
                    paragraph_index: paragraph.index,
                    paragraph_offset: paragraph.offset,
                    type_code: paragraph.type_code,
                    selector,
                    operation_code,
                    content_size: paragraph.content_size,
                    content_sha256,
                    resolution,
                });
            }
        }
    }
    let resolved = records
        .iter()
        .filter(|record| {
            !matches!(
                record.resolution,
                JstudioSemanticResolution::Unresolved { .. }
            )
        })
        .count() as u32;
    let object_specific = records.len() as u32;
    let program = JstudioSemanticProgram {
        schema: JSTUDIO_SEMANTIC_PROGRAM_SCHEMA.into(),
        content_sha256,
        source_program_sha256: structural.digest()?,
        profile_sha256: profile.digest()?,
        source_resource_sha256: structural.source.resource_sha256,
        records,
        coverage: JstudioSemanticCoverage {
            reserved_paragraphs,
            object_specific_paragraphs: object_specific,
            resolved_object_specific_paragraphs: resolved,
            unresolved_object_specific_paragraphs: object_specific - resolved,
        },
    };
    program.validate()?;
    Ok(program)
}

fn resolve_rule(
    rule: &JstudioAdaptorRule,
    operation_code: u8,
    content: &[u8],
) -> JstudioSemanticResolution {
    match &rule.target {
        JstudioAdaptorTarget::Variable { indices } => {
            resolve_variable(&rule.semantic, indices, operation_code, content)
        }
        JstudioAdaptorTarget::Adaptor { handler } => {
            resolve_adaptor(&rule.semantic, *handler, operation_code, content)
        }
        JstudioAdaptorTarget::VariableOrAdaptor { indices, handler } => {
            if matches!(operation_code, 0x10 | 0x12) {
                resolve_variable(&rule.semantic, indices, operation_code, content)
            } else if operation_code == 0x11 {
                JstudioSemanticResolution::VariableOutputBindingOnly {
                    semantic: rule.semantic.clone(),
                    indices: indices.clone(),
                    handler: *handler,
                    payload_sha256: (!content.is_empty())
                        .then(|| Digest(Sha256::digest(content).into())),
                }
            } else {
                resolve_adaptor(&rule.semantic, *handler, operation_code, content)
            }
        }
    }
}

fn resolve_variable(
    semantic: &str,
    indices: &[u32],
    operation_code: u8,
    content: &[u8],
) -> JstudioSemanticResolution {
    let result = match operation_code {
        1 if content.is_empty() => {
            Some((JstudioVariableBehavior::Clear, JstudioSemanticPayload::None))
        }
        2 if content.len() == indices.len() * 4 => Some((
            JstudioVariableBehavior::ImmediateFloat32,
            JstudioSemanticPayload::Float32Bits {
                words: words(content),
            },
        )),
        3 if content.len() == indices.len() * 4 => Some((
            JstudioVariableBehavior::TimeScaledFloat32,
            JstudioSemanticPayload::Float32Bits {
                words: words(content),
            },
        )),
        0x10 if indices.len() == 1 && !content.is_empty() => Some((
            JstudioVariableBehavior::FunctionValueName,
            byte_payload(content),
        )),
        0x12 if content.len() == indices.len() * 4 => Some((
            JstudioVariableBehavior::FunctionValueIndex,
            JstudioSemanticPayload::Unsigned32 {
                values: words(content),
            },
        )),
        _ => None,
    };
    match result {
        Some((behavior, payload)) => JstudioSemanticResolution::Variable {
            semantic: semantic.into(),
            indices: indices.to_vec(),
            behavior,
            payload,
        },
        None => JstudioSemanticResolution::Unresolved {
            reason: format!(
                "variable dispatch operation 0x{operation_code:02x} has an unsupported payload size {} for {} value(s)",
                content.len(),
                indices.len()
            ),
        },
    }
}

fn resolve_adaptor(
    semantic: &str,
    handler: JstudioAdaptorHandler,
    operation_code: u8,
    content: &[u8],
) -> JstudioSemanticResolution {
    use JstudioAdaptorBehavior as Behavior;
    use JstudioAdaptorHandler as Handler;
    let result = match handler {
        Handler::ActorShape | Handler::ActorAnimation => match operation_code {
            0x18 if !content.is_empty() => Some((Behavior::DirectName, byte_payload(content))),
            0x19 if content.len() == 4 => Some((
                Behavior::DirectUnsigned32,
                JstudioSemanticPayload::Unsigned32 {
                    values: words(content),
                },
            )),
            _ => None,
        },
        Handler::ActorAnimationMode if operation_code == 2 && content.len() == 4 => Some((
            Behavior::ImmediateUnsigned32,
            JstudioSemanticPayload::Unsigned32 {
                values: words(content),
            },
        )),
        Handler::Message | Handler::ParticleResource | Handler::SoundResource
            if operation_code == 0x19 && content.len() == 4 =>
        {
            Some((
                Behavior::DirectUnsigned32,
                JstudioSemanticPayload::Unsigned32 {
                    values: words(content),
                },
            ))
        }
        Handler::ParticleBegin | Handler::ParticleEnd | Handler::SoundBegin
            if operation_code == 1 && content.is_empty() =>
        {
            Some((Behavior::Invoke, JstudioSemanticPayload::None))
        }
        Handler::ParticleBeginFadeIn | Handler::SoundEndFadeOut
            if operation_code == 2 && content.len() == 4 =>
        {
            Some((
                Behavior::ImmediateFloat32,
                JstudioSemanticPayload::Float32Bits {
                    words: words(content),
                },
            ))
        }
        Handler::ParticleRepeat | Handler::SoundOnExitNotEnd
            if operation_code == 2 && content.len() == 4 =>
        {
            Some((
                Behavior::ImmediateBoolean32,
                JstudioSemanticPayload::Unsigned32 {
                    values: words(content),
                },
            ))
        }
        _ => None,
    };
    match result {
        Some((behavior, payload)) => JstudioSemanticResolution::AdaptorCall {
            semantic: semantic.into(),
            handler,
            behavior,
            payload,
        },
        None => JstudioSemanticResolution::Unresolved {
            reason: format!(
                "{handler:?} does not accept operation 0x{operation_code:02x} with {} payload byte(s)",
                content.len()
            ),
        },
    }
}

fn validate_semantic_resolution(
    record: &JstudioSemanticParagraph,
) -> Result<(), PlannerContractError> {
    let (semantic, target, payload) = match &record.resolution {
        JstudioSemanticResolution::Variable {
            semantic,
            indices,
            payload,
            ..
        } => (
            semantic,
            JstudioAdaptorTarget::Variable {
                indices: indices.clone(),
            },
            payload,
        ),
        JstudioSemanticResolution::AdaptorCall {
            semantic,
            handler,
            payload,
            ..
        } => (
            semantic,
            JstudioAdaptorTarget::Adaptor { handler: *handler },
            payload,
        ),
        JstudioSemanticResolution::VariableOutputBindingOnly {
            semantic,
            indices,
            handler: _,
            payload_sha256,
        } => {
            validate_stable_id("jstudio_semantic_program.records.semantic", semantic)?;
            validate_indices(indices)?;
            if record.operation_code != 0x11
                || indices.len() != 1
                || *payload_sha256 != record.content_sha256
            {
                return Err(PlannerContractError::new(
                    "jstudio_semantic_program.records.resolution",
                    "does not match its operation or source payload digest",
                ));
            }
            return Ok(());
        }
        JstudioSemanticResolution::Unresolved { reason } => {
            validate_label("jstudio_semantic_program.records.reason", reason)?;
            return Ok(());
        }
    };
    validate_stable_id("jstudio_semantic_program.records.semantic", semantic)?;
    let payload_bytes = semantic_payload_bytes(payload)?;
    let payload_digest =
        (!payload_bytes.is_empty()).then(|| Digest(Sha256::digest(&payload_bytes).into()));
    if payload_bytes.len() != record.content_size as usize
        || payload_digest != record.content_sha256
    {
        return Err(PlannerContractError::new(
            "jstudio_semantic_program.records.payload",
            "does not match the sealed source payload",
        ));
    }
    let rule = JstudioAdaptorRule {
        object_type: record
            .object_type_ascii
            .clone()
            .unwrap_or_else(|| "NONE".into()),
        selector: record.selector,
        semantic: semantic.clone(),
        target,
    };
    if resolve_rule(&rule, record.operation_code, &payload_bytes) != record.resolution {
        return Err(PlannerContractError::new(
            "jstudio_semantic_program.records.resolution",
            "does not match the adaptor operation and payload contract",
        ));
    }
    Ok(())
}

fn semantic_payload_bytes(
    payload: &JstudioSemanticPayload,
) -> Result<Vec<u8>, PlannerContractError> {
    match payload {
        JstudioSemanticPayload::None => Ok(Vec::new()),
        JstudioSemanticPayload::Float32Bits { words }
        | JstudioSemanticPayload::Unsigned32 { values: words } => {
            Ok(words.iter().flat_map(|word| word.to_be_bytes()).collect())
        }
        JstudioSemanticPayload::Bytes {
            hex,
            ascii_nul_terminated,
        } => {
            if hex.len() % 2 != 0 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(PlannerContractError::new(
                    "jstudio_semantic_program.records.payload.hex",
                    "must be an even-length hexadecimal string",
                ));
            }
            let bytes = hex
                .as_bytes()
                .chunks_exact(2)
                .map(|pair| {
                    let text = std::str::from_utf8(pair).expect("ASCII hex is UTF-8");
                    u8::from_str_radix(text, 16).expect("validated hexadecimal byte")
                })
                .collect::<Vec<_>>();
            if byte_payload(&bytes)
                != (JstudioSemanticPayload::Bytes {
                    hex: hex.clone(),
                    ascii_nul_terminated: ascii_nul_terminated.clone(),
                })
            {
                return Err(PlannerContractError::new(
                    "jstudio_semantic_program.records.payload",
                    "contains a noncanonical byte interpretation",
                ));
            }
            Ok(bytes)
        }
    }
}

fn validate_object_type(value: &str) -> Result<(), PlannerContractError> {
    if value.len() != 4
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    {
        return Err(PlannerContractError::new(
            "jstudio_adaptor_profile.rules.object_type",
            "must be an uppercase ASCII four-character code",
        ));
    }
    Ok(())
}

fn validate_indices(indices: &[u32]) -> Result<(), PlannerContractError> {
    if indices.is_empty() || indices.len() > 32 {
        return Err(PlannerContractError::new(
            "jstudio_adaptor_profile.rules.indices",
            "must contain between one and 32 variable indices",
        ));
    }
    let unique = indices.iter().copied().collect::<BTreeSet<_>>();
    if unique.len() != indices.len() {
        return Err(PlannerContractError::new(
            "jstudio_adaptor_profile.rules.indices",
            "must not contain duplicates",
        ));
    }
    Ok(())
}

fn words(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| u32::from_be_bytes(chunk.try_into().expect("four-byte chunk")))
        .collect()
}

fn byte_payload(bytes: &[u8]) -> JstudioSemanticPayload {
    let ascii_nul_terminated = bytes.iter().position(|byte| *byte == 0).and_then(|end| {
        bytes[..end]
            .iter()
            .all(u8::is_ascii_graphic)
            .then(|| String::from_utf8(bytes[..end].to_vec()).expect("ASCII is UTF-8"))
    });
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(hex, "{byte:02x}").expect("writing to String cannot fail");
    }
    JstudioSemanticPayload::Bytes {
        hex,
        ascii_nul_terminated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{ContentFingerprint, GamePlatform, GameRegion};

    fn content() -> ContentIdentity {
        ContentIdentity::new(
            "fixture",
            ContentFingerprint {
                platform: GamePlatform::GameCube,
                region: GameRegion::Usa,
                revision: "1.0".into(),
                product_id: "FIXTURE".into(),
                executable_sha256: Digest([2; 32]),
                game_data_sha256: Digest([3; 32]),
                resource_manifest_sha256: Digest([4; 32]),
            },
        )
        .unwrap()
    }

    fn profile(content: &ContentIdentity) -> JstudioAdaptorProfile {
        JstudioAdaptorProfile {
            schema: JSTUDIO_ADAPTOR_PROFILE_SCHEMA.into(),
            id: "fixture".into(),
            content_sha256: content.digest().unwrap(),
            executable_sha256: content.fingerprint.executable_sha256,
            target_version: 6,
            evidence: vec![JstudioAdaptorEvidence {
                source_sha256: Digest([5; 32]),
                note: "Fixture adaptor audit.".into(),
            }],
            rules: vec![JstudioAdaptorRule {
                object_type: "JACT".into(),
                selector: 57,
                semantic: "actor.shape".into(),
                target: JstudioAdaptorTarget::Adaptor {
                    handler: JstudioAdaptorHandler::ActorShape,
                },
            }],
        }
    }

    fn stb() -> Vec<u8> {
        let mut bytes = vec![0; 0x20];
        bytes[0..4].copy_from_slice(b"STB\0");
        bytes[4..6].copy_from_slice(&0xfeffu16.to_be_bytes());
        bytes[6..8].copy_from_slice(&3u16.to_be_bytes());
        bytes[12..16].copy_from_slice(&1u32.to_be_bytes());
        bytes[16..24].copy_from_slice(b"jstudio\0");
        bytes[30..32].copy_from_slice(&6u16.to_be_bytes());
        let block = bytes.len();
        bytes.extend_from_slice(&[0; 12]);
        bytes[block + 4..block + 8].copy_from_slice(b"JACT");
        bytes[block + 10..block + 12].copy_from_slice(&6u16.to_be_bytes());
        bytes.extend_from_slice(b"actor\0\0\0");
        bytes.extend_from_slice(&0x8000_0008u32.to_be_bytes());
        bytes.extend_from_slice(&4u16.to_be_bytes());
        bytes.extend_from_slice(&((57u16 << 5) | 0x19).to_be_bytes());
        bytes.extend_from_slice(&7u32.to_be_bytes());
        bytes.extend_from_slice(&0u32.to_be_bytes());
        let block_size = (bytes.len() - block) as u32;
        bytes[block..block + 4].copy_from_slice(&block_size.to_be_bytes());
        let size = bytes.len() as u32;
        bytes[8..12].copy_from_slice(&size.to_be_bytes());
        bytes
    }

    #[test]
    fn exact_profile_resolves_direct_actor_resource_id() {
        let content = content();
        let profile = profile(&content);
        let program = resolve_jstudio_stb_semantics(
            &content,
            &profile,
            Digest([1; 32]),
            "fixture.stb",
            &stb(),
        )
        .unwrap();
        assert_eq!(program.coverage.resolved_object_specific_paragraphs, 1);
        assert_eq!(program.coverage.unresolved_object_specific_paragraphs, 0);
        assert!(matches!(
            &program.records[0].resolution,
            JstudioSemanticResolution::AdaptorCall {
                handler: JstudioAdaptorHandler::ActorShape,
                behavior: JstudioAdaptorBehavior::DirectUnsigned32,
                payload: JstudioSemanticPayload::Unsigned32 { values },
                ..
            } if values == &[7]
        ));
        assert_eq!(
            JstudioSemanticProgram::decode_canonical(&program.canonical_bytes().unwrap()).unwrap(),
            program
        );
    }

    #[test]
    fn semantic_payload_cannot_be_changed_without_invalidating_source_digest() {
        let content = content();
        let mut program = resolve_jstudio_stb_semantics(
            &content,
            &profile(&content),
            Digest([1; 32]),
            "fixture.stb",
            &stb(),
        )
        .unwrap();
        let JstudioSemanticResolution::AdaptorCall {
            payload: JstudioSemanticPayload::Unsigned32 { values },
            ..
        } = &mut program.records[0].resolution
        else {
            panic!("fixture should resolve to a direct unsigned adaptor call");
        };
        values[0] = 8;
        assert_eq!(
            program.validate().unwrap_err().field(),
            "jstudio_semantic_program.records.payload"
        );
    }

    #[test]
    fn wrong_content_identity_and_unknown_selector_fail_closed() {
        let content = content();
        let mut wrong_profile = profile(&content);
        wrong_profile.executable_sha256 = Digest([9; 32]);
        assert_eq!(
            resolve_jstudio_stb_semantics(
                &content,
                &wrong_profile,
                Digest([1; 32]),
                "fixture.stb",
                &stb(),
            )
            .unwrap_err()
            .field(),
            "jstudio_adaptor_profile.identity"
        );

        let profile = JstudioAdaptorProfile {
            rules: vec![JstudioAdaptorRule {
                selector: 58,
                ..profile(&content).rules[0].clone()
            }],
            ..profile(&content)
        };
        let program = resolve_jstudio_stb_semantics(
            &content,
            &profile,
            Digest([1; 32]),
            "fixture.stb",
            &stb(),
        )
        .unwrap();
        assert_eq!(program.coverage.unresolved_object_specific_paragraphs, 1);
    }

    #[test]
    fn bundled_exact_profile_is_canonical_and_covers_observed_dispatches() {
        let profile = bundled_gz2e01_adaptor_profile().unwrap();
        assert_eq!(profile.id, "gz2e01-jstudio-adaptors");
        assert_eq!(profile.rules.len(), 29);
        let audited_sources = [
            include_bytes!(
                "../../../../../libs/JSystem/src/JStudio/JStudio_JAudio2/object-sound.cpp"
            )
            .as_slice(),
            include_bytes!("../../../../../src/d/d_demo.cpp").as_slice(),
            include_bytes!(
                "../../../../../libs/JSystem/src/JStudio/JStudio_JStage/object-actor.cpp"
            )
            .as_slice(),
            include_bytes!(
                "../../../../../libs/JSystem/src/JStudio/JStudio_JParticle/object-particle.cpp"
            )
            .as_slice(),
            include_bytes!("../../../../../libs/JSystem/src/JStudio/JStudio/jstudio-object.cpp")
                .as_slice(),
        ];
        assert_eq!(
            profile
                .evidence
                .iter()
                .map(|evidence| evidence.source_sha256)
                .collect::<Vec<_>>(),
            audited_sources
                .iter()
                .map(|source| Digest(Sha256::digest(source).into()))
                .collect::<Vec<_>>()
        );
        assert!(profile.rules.iter().any(|rule| {
            rule.object_type == "JMSG"
                && rule.selector == 66
                && matches!(
                    rule.target,
                    JstudioAdaptorTarget::Adaptor {
                        handler: JstudioAdaptorHandler::Message
                    }
                )
        }));
    }
}
