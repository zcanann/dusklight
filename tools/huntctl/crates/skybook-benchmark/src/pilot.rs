//! One-page, repository-bound Skybook benchmark pilot contracts.

use crate::manifest::SkybookManifest;
use dusklight_automation_contracts::artifact::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Component, Path};

pub const SKYBOOK_PILOT_SCHEMA: &str = "dusklight-skybook-pilot/v1";
const MAX_TEXT_BYTES: usize = 4_096;
const MAX_ARTIFACT_BYTES: u64 = 16 * 1_024 * 1_024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SkybookPilot {
    pub schema: String,
    pub content_sha256: Digest,
    pub source_manifest_content_sha256: Digest,
    pub source_repository_url: String,
    pub source_git_revision: String,
    pub approved_by: String,
    pub approval_reference: String,
    pub page: PilotPageIdentity,
    pub native_port: NativePortClaim,
    pub positive_case: PilotPositiveCase,
    pub negative_control: PilotNegativeControl,
    pub implementation_artifacts: Vec<PilotArtifact>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PilotPageIdentity {
    pub slug: String,
    pub source_path: String,
    pub source_sha256: Digest,
    pub body_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativePortClaim {
    pub claim: String,
    pub required_setup: Vec<String>,
    pub fidelity_limitations: Vec<String>,
    pub oracle_schema_name: String,
    pub oracle_schema_version: u32,
    pub fidelity_profile: String,
    pub retail_profile: String,
    pub expected_character_index: u16,
    pub expected_original_offset: u16,
    pub expected_gc_cached_address: u32,
    pub expected_bytes: [u8; 8],
    pub expected_xf_channels: u32,
    pub expected_bp_channels: u32,
    pub expected_tape_completion_frame: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PilotPositiveCase {
    pub tape_path: String,
    pub tape_sha256: Digest,
    pub expected_oracle_status: String,
    pub required_stages: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PilotNegativeControl {
    pub id: String,
    pub derivation: String,
    pub expected_oracle_status: String,
    pub must_not_report: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PilotArtifactRole {
    AuroraCommandProcessor,
    AuroraGfxInterface,
    AuroraRendererDiagnostic,
    NameEntryObserverImplementation,
    NameEntryObserverInterface,
    NameEntryTraceImplementation,
    NameEntryTraceInterface,
    NativeAutomationIntegration,
    OriginalLayoutShadowImplementation,
    OriginalNameLayoutInterface,
    SemanticOracleImplementation,
    SemanticOracleInterface,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PilotArtifact {
    pub role: PilotArtifactRole,
    pub path: String,
    pub sha256: Digest,
}

#[derive(Debug)]
pub struct SkybookPilotError(String);

impl fmt::Display for SkybookPilotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for SkybookPilotError {}

impl SkybookPilot {
    pub fn validate_against(
        &self,
        manifest: &SkybookManifest,
        repository_root: &Path,
    ) -> Result<(), SkybookPilotError> {
        manifest
            .validate()
            .map_err(|error| pilot_error(format!("invalid Skybook manifest: {error}")))?;
        if self.schema != SKYBOOK_PILOT_SCHEMA
            || self.source_manifest_content_sha256 != manifest.content_sha256
            || self.source_repository_url != manifest.source.repository_url
            || self.source_git_revision != manifest.source.git_revision
        {
            return Err(pilot_error("pilot is detached from its Skybook manifest"));
        }
        canonical_text("approved_by", &self.approved_by)?;
        canonical_text("approval_reference", &self.approval_reference)?;

        let page = manifest
            .pages
            .iter()
            .find(|candidate| candidate.slug == self.page.slug)
            .ok_or_else(|| pilot_error("pilot references an unknown Skybook page"))?;
        if self.page.source_path != page.source_path
            || self.page.source_sha256 != page.source_sha256
            || self.page.body_sha256 != page.body_sha256
        {
            return Err(pilot_error("pilot Skybook page identity is stale"));
        }
        self.validate_claim()?;
        self.validate_cases(repository_root)?;
        self.validate_artifacts(repository_root)?;
        if self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.compute_content_sha256()?
        {
            return Err(pilot_error("pilot content identity is invalid"));
        }
        Ok(())
    }

    pub fn refresh_content_sha256(&mut self) -> Result<(), SkybookPilotError> {
        self.content_sha256 = self.compute_content_sha256()?;
        Ok(())
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, SkybookPilotError> {
        let mut bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| pilot_error(format!("cannot encode pilot: {error}")))?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    fn validate_claim(&self) -> Result<(), SkybookPilotError> {
        canonical_text("native_port.claim", &self.native_port.claim)?;
        canonical_text_list("native_port.required_setup", &self.native_port.required_setup)?;
        canonical_text_list(
            "native_port.fidelity_limitations",
            &self.native_port.fidelity_limitations,
        )?;
        let claim = &self.native_port;
        if claim.oracle_schema_name != "dusklight.eye_shredder_oracle"
            || claim.oracle_schema_version != 4
            || claim.fidelity_profile != "cursor_breakout_shadow"
            || claim.retail_profile != "fresh_gcn_ntsc_u"
            || claim.expected_character_index != 113
            || claim.expected_original_offset != 0x654
            || claim.expected_gc_cached_address != 0x8145_7688
            || claim.expected_bytes != [0x0c, 0x00, 0x02, 0x01, 0x00, 0x00, 0x00, 0x4d]
            || claim.expected_xf_channels != 12
            || claim.expected_bp_channels != 4
            || claim.expected_tape_completion_frame != 650
        {
            return Err(pilot_error("native-port Eye Shredder contract is invalid"));
        }
        Ok(())
    }

    fn validate_cases(&self, repository_root: &Path) -> Result<(), SkybookPilotError> {
        if self.positive_case.expected_oracle_status != "passed"
            || self.positive_case.required_stages
                != ["memory", "renderer", "gameplay", "tape"]
        {
            return Err(pilot_error("positive case acceptance contract is invalid"));
        }
        validate_relative_path(&self.positive_case.tape_path)?;
        if sha256_file(repository_root, &self.positive_case.tape_path)?
            != self.positive_case.tape_sha256
        {
            return Err(pilot_error("positive tape identity is stale"));
        }
        canonical_text("negative_control.id", &self.negative_control.id)?;
        canonical_text("negative_control.derivation", &self.negative_control.derivation)?;
        canonical_text_list(
            "negative_control.must_not_report",
            &self.negative_control.must_not_report,
        )?;
        if self.negative_control.expected_oracle_status != "failed"
            || self.negative_control.must_not_report != ["memory", "renderer"]
        {
            return Err(pilot_error("negative-control acceptance contract is invalid"));
        }
        Ok(())
    }

    fn validate_artifacts(&self, repository_root: &Path) -> Result<(), SkybookPilotError> {
        let expected_roles = [
            PilotArtifactRole::AuroraCommandProcessor,
            PilotArtifactRole::AuroraGfxInterface,
            PilotArtifactRole::AuroraRendererDiagnostic,
            PilotArtifactRole::NameEntryObserverImplementation,
            PilotArtifactRole::NameEntryObserverInterface,
            PilotArtifactRole::NameEntryTraceImplementation,
            PilotArtifactRole::NameEntryTraceInterface,
            PilotArtifactRole::NativeAutomationIntegration,
            PilotArtifactRole::OriginalLayoutShadowImplementation,
            PilotArtifactRole::OriginalNameLayoutInterface,
            PilotArtifactRole::SemanticOracleImplementation,
            PilotArtifactRole::SemanticOracleInterface,
        ];
        let roles = self
            .implementation_artifacts
            .iter()
            .map(|artifact| artifact.role)
            .collect::<Vec<_>>();
        if roles != expected_roles {
            return Err(pilot_error(
                "implementation artifacts must contain the exact role inventory in order",
            ));
        }
        for artifact in &self.implementation_artifacts {
            validate_relative_path(&artifact.path)?;
            if artifact.sha256 == Digest::ZERO
                || sha256_file(repository_root, &artifact.path)? != artifact.sha256
            {
                return Err(pilot_error(format!(
                    "implementation artifact identity is stale: {}",
                    artifact.path
                )));
            }
        }
        Ok(())
    }

    fn compute_content_sha256(&self) -> Result<Digest, SkybookPilotError> {
        let encoded = serde_json::to_vec(&(
            &self.schema,
            self.source_manifest_content_sha256,
            &self.source_repository_url,
            &self.source_git_revision,
            &self.approved_by,
            &self.approval_reference,
            &self.page,
            &self.native_port,
            &self.positive_case,
            &self.negative_control,
            &self.implementation_artifacts,
        ))
        .map_err(|error| pilot_error(format!("cannot encode pilot: {error}")))?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.skybook-pilot/v1\0");
        hasher.update((encoded.len() as u64).to_le_bytes());
        hasher.update(encoded);
        Ok(Digest(hasher.finalize().into()))
    }
}

fn canonical_text(field: &str, value: &str) -> Result<(), SkybookPilotError> {
    if value.is_empty() || value != value.trim() || value.len() > MAX_TEXT_BYTES {
        return Err(pilot_error(format!("{field} is not canonical bounded text")));
    }
    Ok(())
}

fn canonical_text_list(field: &str, values: &[String]) -> Result<(), SkybookPilotError> {
    if values.is_empty() || values.len() > 16 {
        return Err(pilot_error(format!("{field} has an invalid item count")));
    }
    for value in values {
        canonical_text(field, value)?;
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> Result<(), SkybookPilotError> {
    if path.is_empty()
        || Path::new(path).is_absolute()
        || Path::new(path)
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(pilot_error(format!("pilot path is invalid: {path}")));
    }
    Ok(())
}

fn sha256_file(repository_root: &Path, relative: &str) -> Result<Digest, SkybookPilotError> {
    let path = repository_root.join(relative);
    let metadata = fs::metadata(&path)
        .map_err(|error| pilot_error(format!("cannot inspect {}: {error}", path.display())))?;
    if !metadata.is_file() || metadata.len() > MAX_ARTIFACT_BYTES {
        return Err(pilot_error(format!(
            "pilot artifact is not a bounded file: {}",
            path.display()
        )));
    }
    let bytes = fs::read(&path)
        .map_err(|error| pilot_error(format!("cannot read {}: {error}", path.display())))?;
    Ok(Digest(Sha256::digest(bytes).into()))
}

fn pilot_error(message: impl Into<String>) -> SkybookPilotError {
    SkybookPilotError(message.into())
}
