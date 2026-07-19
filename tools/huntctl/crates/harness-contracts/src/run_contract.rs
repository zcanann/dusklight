//! Canonical request/result boundary shared by every core-harness executor.

use super::objective_suite::{
    ArtifactReference, ExpectedTerminalClass, ObjectiveBoot, ObjectiveCaseRole,
    ObjectiveProgramReference, ObjectiveSeed, ObjectiveSuiteCase, ObservationViewReference,
    SchemaIdentity,
};
use super::native_evidence::{HarnessNativeEvidenceArtifacts, HarnessNativeEvidenceRequest};
use super::observation_contract::{
    ObjectiveObservationRequirements, ObservationAdmission, ObservationAdmissionIssue,
    ObservationInventory,
};
use crate::artifact::{ArtifactIdentity, BuildIdentity, Digest};
use crate::compatibility::{CompatibilityMode, ensure_compatible};
use dusklight_automation_contracts::engine_session::POST_AUTHENTICATED_RUN_BOUNDARY;
pub use dusklight_automation_contracts::engine_session::{
    ENGINE_SESSION_REUSE_AUDIT_SCHEMA_V1, SessionReuseAudit, SessionReuseBlocker,
};
pub use dusklight_automation_contracts::run_terminal::HarnessTerminalReason;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::Read as _;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

pub const RUN_REQUEST_SCHEMA_V2: &str = "dusklight-harness-run-request/v2";
pub const RUN_RESULT_SCHEMA_V2: &str = "dusklight-harness-run-result/v2";
pub const NATIVE_LIFECYCLE_TIMING_SCHEMA_V1: &str = "dusklight-native-lifecycle-timing/v1";
pub const NATIVE_LIFECYCLE_TIMING_SCHEMA_V2: &str = "dusklight-native-lifecycle-timing/v2";
const MAX_LOGICAL_TICKS: u64 = 10_000_000;
const MAX_HOST_TIMEOUT_SECONDS: u32 = 86_400;
const MAX_FACTS: usize = 128;
const MAX_CAPABILITIES: usize = 128;
const MAX_MESSAGE_BYTES: usize = 8_192;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessRunRequest {
    pub schema: String,
    pub content_sha256: Digest,
    pub id: String,
    pub executable: ArtifactReference,
    pub game_data: ArtifactReference,
    pub build: BuildIdentity,
    pub identity: ArtifactIdentity,
    pub protocol: HarnessProtocolIdentity,
    pub boot: ObjectiveBoot,
    pub scenario: ArtifactReference,
    pub objective: ObjectiveProgramReference,
    pub observation_view: ObservationViewReference,
    pub action_schema: SchemaIdentity,
    pub observation_requirements: ObjectiveObservationRequirements,
    pub input: ObjectiveSeed,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_evidence: Option<HarnessNativeEvidenceRequest>,
    pub rng_seed: u64,
    pub logical_tick_budget: u64,
    pub host_timeout_seconds: u32,
    pub fidelity: HarnessFidelityMode,
    pub artifact_destination: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessProtocolIdentity {
    pub name: String,
    pub version: u16,
    pub capabilities_sha256: Digest,
    pub capabilities: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessFidelityMode {
    Headless,
    UnpacedHeadful,
    RealtimeHeadful,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessRunResult {
    pub schema: String,
    pub content_sha256: Digest,
    pub request_id: String,
    pub request_sha256: Digest,
    pub identity: ArtifactIdentity,
    pub attempt: u32,
    pub worker: HarnessWorkerIdentity,
    pub terminal: HarnessTerminalReason,
    pub detail: HarnessTerminalDetail,
    pub objective: HarnessObjectiveResult,
    pub artifacts: HarnessRunArtifacts,
    pub timing: HarnessRunTiming,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessWorkerIdentity {
    pub id: String,
    pub build: BuildIdentity,
    pub protocol: HarnessProtocolIdentity,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessTerminalDetail {
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_query_facts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observation_issues: Vec<ObservationAdmissionIssue>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessObjectiveResult {
    pub reached: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_hit_tick: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<ArtifactReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boundary_fingerprint: Option<HarnessBoundaryFingerprint>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessBoundaryFingerprint {
    pub schema: String,
    pub algorithm: String,
    pub canonical_encoding: String,
    pub digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessRunArtifacts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realized_input: Option<ArtifactReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gameplay_trace: Option<ArtifactReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective_result: Option<ArtifactReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout: Option<ArtifactReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr: Option<ArtifactReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_phase_timing: Option<ArtifactReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_evidence: Option<HarnessNativeEvidenceArtifacts>,
    pub complete: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessRunTiming {
    pub logical_ticks: u64,
    pub consumed_input_ticks: u64,
    pub host_elapsed_millis: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_phases: Option<HarnessNativePhaseTiming>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessNativePhaseTiming {
    pub schema: String,
    pub clock: String,
    pub process_entry_micros: u64,
    pub cli_configured_micros: u64,
    pub aurora_initialized_micros: u64,
    pub engine_ready_micros: u64,
    pub stage_ready_micros: u64,
    pub first_simulation_tick_micros: u64,
    pub last_simulation_tick_micros: u64,
    pub proof_artifacts_written_micros: u64,
    pub engine_shutdown_micros: u64,
    pub exit_ready_micros: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_reuse_audit: Option<SessionReuseAudit>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessRunRequestValidationReport {
    pub schema: &'static str,
    pub request_id: String,
    pub request_sha256: Digest,
    pub objective_id: String,
    pub logical_tick_budget: u64,
    pub artifact_destination: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessRunResultValidationReport {
    pub schema: &'static str,
    pub request_id: String,
    pub request_sha256: Digest,
    pub result_sha256: Digest,
    pub terminal: HarnessTerminalReason,
    pub artifacts_complete: bool,
}

impl HarnessRunRequest {
    pub fn validate(&self) -> Result<(), HarnessRunContractError> {
        if self.schema != RUN_REQUEST_SCHEMA_V2 {
            return Err(contract_error("unsupported harness run-request schema"));
        }
        validate_id("request id", &self.id)?;
        validate_artifact("executable", &self.executable)?;
        validate_artifact("game data", &self.game_data)?;
        self.build
            .validate()
            .map_err(|error| contract_error(format!("invalid run build: {error}")))?;
        if self.build.game_digest != self.game_data.sha256 {
            return Err(contract_error(
                "run build game digest does not match game-data bytes",
            ));
        }
        self.protocol.validate()?;
        self.identity
            .validate()
            .map_err(|error| contract_error(format!("invalid complete run identity: {error}")))?;
        if self.identity.build != self.build
            || self.identity.protocol_name != self.protocol.name
            || self.identity.protocol_version != self.protocol.version
            || self.identity.protocol_capabilities_digest != self.protocol.capabilities_sha256
            || self.identity.scenario_digest != self.scenario.sha256
            || self.identity.predicate_program_digest != self.objective.program_sha256
            || self.identity.observation_schema_digest != self.observation_view.schema_sha256
            || self.identity.action_schema_digest != self.action_schema.sha256
        {
            return Err(contract_error(
                "complete run identity disagrees with an explicit request binding",
            ));
        }
        self.as_bound_case()
            .validate_bound_structure()
            .map_err(|error| contract_error(error.to_string()))?;
        if let Some(native_evidence) = self.native_evidence {
            native_evidence
                .validate_for(&self.boot, &self.input)
                .map_err(|error| contract_error(error.to_string()))?;
        }
        if !(1..=MAX_LOGICAL_TICKS).contains(&self.logical_tick_budget)
            || !(1..=MAX_HOST_TIMEOUT_SECONDS).contains(&self.host_timeout_seconds)
        {
            return Err(contract_error(
                "run request budgets are outside supported bounds",
            ));
        }
        validate_relative_path("artifact destination", &self.artifact_destination)?;
        if self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.compute_content_sha256()?
        {
            return Err(contract_error(
                "harness run-request content identity is invalid",
            ));
        }
        Ok(())
    }

    pub fn validate_files(
        &self,
        repository_root: &Path,
    ) -> Result<HarnessRunRequestValidationReport, HarnessRunContractError> {
        self.validate()?;
        let root = canonical_root(repository_root)?;
        read_artifact(&root, &self.executable, "executable", false)?;
        // Local game data is commonly an ignored repository-relative symlink
        // to a mounted disc image. Its exact bytes are authenticated, but the
        // canonical target is intentionally allowed outside the repository.
        read_artifact(&root, &self.game_data, "game data", true)?;
        let bound = self.as_bound_case();
        bound
            .validate_bound_files(&root)
            .map_err(|error| contract_error(error.to_string()))?;
        Ok(HarnessRunRequestValidationReport {
            schema: "dusklight-harness-run-request-validation/v1",
            request_id: self.id.clone(),
            request_sha256: self.content_sha256,
            objective_id: self.objective.goal.clone(),
            logical_tick_budget: self.logical_tick_budget,
            artifact_destination: self.artifact_destination.clone(),
        })
    }

    pub fn refresh_content_sha256(&mut self) -> Result<(), HarnessRunContractError> {
        self.content_sha256 = self.compute_content_sha256()?;
        Ok(())
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, HarnessRunContractError> {
        pretty_json(self)
    }

    pub fn assess_observations(
        &self,
        inventory: &ObservationInventory,
    ) -> Result<ObservationAdmission, HarnessRunContractError> {
        self.observation_requirements
            .assess(inventory)
            .map_err(|error| contract_error(error.to_string()))
    }

    pub fn unsupported_observation_detail(
        &self,
        inventory: &ObservationInventory,
    ) -> Result<Option<HarnessTerminalDetail>, HarnessRunContractError> {
        let admission = self.assess_observations(inventory)?;
        if admission.supported {
            return Ok(None);
        }
        Ok(Some(HarnessTerminalDetail {
            message: "required observation families are unsupported".into(),
            missing_query_facts: self
                .observation_requirements
                .facts_for_issues(&admission.issues),
            missing_capabilities: Vec::new(),
            observation_issues: admission.issues,
        }))
    }

    fn compute_content_sha256(&self) -> Result<Digest, HarnessRunContractError> {
        let mut identity = self.clone();
        identity.content_sha256 = Digest::ZERO;
        canonical_digest(b"dusklight.harness-run-request/v2\0", &identity)
    }

    fn as_bound_case(&self) -> ObjectiveSuiteCase {
        ObjectiveSuiteCase {
            id: self.id.clone(),
            description: "materialized harness run request".into(),
            role: ObjectiveCaseRole::Positive,
            control_for: None,
            boot: self.boot.clone(),
            scenario: self.scenario.clone(),
            objective: self.objective.clone(),
            observation_view: self.observation_view.clone(),
            action_schema: self.action_schema.clone(),
            observation_requirements: self.observation_requirements.clone(),
            seed: self.input.clone(),
            logical_tick_budget: self.logical_tick_budget,
            host_timeout_seconds: self.host_timeout_seconds,
            repetitions: 2,
            expected_terminal: ExpectedTerminalClass::Reached,
        }
    }
}

impl HarnessProtocolIdentity {
    fn validate(&self) -> Result<(), HarnessRunContractError> {
        validate_id("protocol name", &self.name)?;
        if self.version == 0
            || self.capabilities_sha256 == Digest::ZERO
            || self.capabilities.is_empty()
        {
            return Err(contract_error("protocol identity is incomplete"));
        }
        validate_sorted_names(
            "protocol capabilities",
            &self.capabilities,
            MAX_CAPABILITIES,
        )?;
        let expected =
            canonical_digest(b"dusklight.harness-capabilities/v1\0", &self.capabilities)?;
        if self.capabilities_sha256 != expected {
            return Err(contract_error("protocol capability identity is stale"));
        }
        Ok(())
    }

    pub fn refresh_capabilities_sha256(&mut self) -> Result<(), HarnessRunContractError> {
        self.capabilities_sha256 =
            canonical_digest(b"dusklight.harness-capabilities/v1\0", &self.capabilities)?;
        Ok(())
    }
}

impl HarnessRunResult {
    pub fn validate_against(
        &self,
        request: &HarnessRunRequest,
    ) -> Result<(), HarnessRunContractError> {
        request.validate()?;
        if self.schema != RUN_RESULT_SCHEMA_V2
            || self.request_id != request.id
            || self.request_sha256 != request.content_sha256
            || self.attempt == 0
        {
            return Err(contract_error(
                "run result does not bind the declared request",
            ));
        }
        ensure_compatible(CompatibilityMode::Replay, &request.identity, &self.identity)
            .map_err(|error| contract_error(error.to_string()))?;
        if self.identity != request.identity {
            return Err(contract_error(
                "run result changed the request payload identity",
            ));
        }
        self.worker.validate()?;
        let build_matches = self.worker.build == request.build;
        let protocol_family_matches = self.worker.protocol.name == request.protocol.name
            && self.worker.protocol.version == request.protocol.version;
        let capabilities_match = self.worker.protocol.capabilities_sha256
            == request.protocol.capabilities_sha256
            && self.worker.protocol.capabilities == request.protocol.capabilities;
        match self.terminal {
            HarnessTerminalReason::IdentityMismatch
                if !build_matches || !protocol_family_matches => {}
            HarnessTerminalReason::CapabilityMismatch
                if build_matches && protocol_family_matches && !capabilities_match => {}
            HarnessTerminalReason::IdentityMismatch | HarnessTerminalReason::CapabilityMismatch => {
                return Err(contract_error(
                    "run-result mismatch terminal contradicts worker identity",
                ));
            }
            _ if build_matches && protocol_family_matches && capabilities_match => {}
            _ => {
                return Err(contract_error(
                    "run-result worker identity differs without a mismatch terminal",
                ));
            }
        }
        self.detail.validate(self.terminal, request)?;
        self.objective.validate(self.terminal, request)?;
        self.artifacts
            .validate(self.terminal, request.native_evidence.is_some())?;
        if self.artifacts.native_evidence.is_some() && request.native_evidence.is_none() {
            return Err(contract_error(
                "run result contains native evidence not requested by the run",
            ));
        }
        if self.timing.logical_ticks > request.logical_tick_budget
            || self.timing.consumed_input_ticks > self.timing.logical_ticks
        {
            return Err(contract_error(
                "run-result timing counters exceed request bounds",
            ));
        }
        if self.artifacts.native_phase_timing.is_some() != self.timing.native_phases.is_some() {
            return Err(contract_error(
                "native phase artifact and decoded timing must be present together",
            ));
        }
        if let Some(native_phases) = &self.timing.native_phases {
            native_phases.validate(self.timing.host_elapsed_millis)?;
        }
        if self
            .objective
            .first_hit_tick
            .is_some_and(|tick| tick >= self.timing.logical_ticks)
        {
            return Err(contract_error(
                "run-result first-hit tick is outside realized timing",
            ));
        }
        if self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.compute_content_sha256()?
        {
            return Err(contract_error(
                "harness run-result content identity is invalid",
            ));
        }
        Ok(())
    }

    pub fn validate_files(
        &self,
        request: &HarnessRunRequest,
        artifact_root: &Path,
    ) -> Result<HarnessRunResultValidationReport, HarnessRunContractError> {
        self.validate_against(request)?;
        let root = canonical_root(artifact_root)?;
        for (label, reference) in self.artifact_references() {
            read_artifact(&root, reference, label, false)?;
        }
        Ok(HarnessRunResultValidationReport {
            schema: "dusklight-harness-run-result-validation/v1",
            request_id: self.request_id.clone(),
            request_sha256: self.request_sha256,
            result_sha256: self.content_sha256,
            terminal: self.terminal,
            artifacts_complete: self.artifacts.complete,
        })
    }

    pub fn refresh_content_sha256(&mut self) -> Result<(), HarnessRunContractError> {
        self.content_sha256 = self.compute_content_sha256()?;
        Ok(())
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, HarnessRunContractError> {
        pretty_json(self)
    }

    fn compute_content_sha256(&self) -> Result<Digest, HarnessRunContractError> {
        let mut identity = self.clone();
        identity.content_sha256 = Digest::ZERO;
        canonical_digest(b"dusklight.harness-run-result/v2\0", &identity)
    }

    fn artifact_references(&self) -> Vec<(&'static str, &ArtifactReference)> {
        let mut output = Vec::new();
        for (label, reference) in [
            ("objective evidence", self.objective.evidence.as_ref()),
            ("realized input", self.artifacts.realized_input.as_ref()),
            ("gameplay trace", self.artifacts.gameplay_trace.as_ref()),
            ("objective result", self.artifacts.objective_result.as_ref()),
            ("stdout", self.artifacts.stdout.as_ref()),
            ("stderr", self.artifacts.stderr.as_ref()),
            (
                "native phase timing",
                self.artifacts.native_phase_timing.as_ref(),
            ),
            (
                "native evidence oracle result",
                self.artifacts
                    .native_evidence
                    .as_ref()
                    .map(|evidence| &evidence.oracle_result),
            ),
            (
                "native evidence semantic trace",
                self.artifacts
                    .native_evidence
                    .as_ref()
                    .map(|evidence| &evidence.semantic_trace),
            ),
        ] {
            if let Some(reference) = reference {
                output.push((label, reference));
            }
        }
        output
    }
}

impl HarnessWorkerIdentity {
    fn validate(&self) -> Result<(), HarnessRunContractError> {
        validate_id("worker id", &self.id)?;
        self.build
            .validate()
            .map_err(|error| contract_error(format!("invalid worker build: {error}")))?;
        self.protocol.validate()
    }
}

impl HarnessTerminalDetail {
    fn validate(
        &self,
        terminal: HarnessTerminalReason,
        request: &HarnessRunRequest,
    ) -> Result<(), HarnessRunContractError> {
        validate_text("terminal message", &self.message)?;
        validate_sorted_facts("missing query facts", &self.missing_query_facts, MAX_FACTS)?;
        validate_sorted_names(
            "missing capabilities",
            &self.missing_capabilities,
            MAX_CAPABILITIES,
        )?;
        if self.missing_query_facts.iter().any(|fact| {
            request
                .observation_requirements
                .facts
                .binary_search(fact)
                .is_err()
        }) {
            return Err(contract_error(
                "run-result reports a missing fact not required by the objective",
            ));
        }
        if !self
            .observation_issues
            .windows(2)
            .all(|pair| pair[0].family < pair[1].family)
        {
            return Err(contract_error(
                "observation issues must be unique and family-sorted",
            ));
        }
        for issue in &self.observation_issues {
            issue
                .validate()
                .map_err(|error| contract_error(error.to_string()))?;
            let requirement = request
                .observation_requirements
                .families
                .binary_search_by_key(&issue.family.as_str(), |family| family.id.as_str())
                .ok()
                .map(|index| &request.observation_requirements.families[index]);
            if requirement
                .is_none_or(|requirement| requirement.minimum_version != issue.minimum_version)
            {
                return Err(contract_error(
                    "run-result observation issue is not a request requirement",
                ));
            }
        }
        let has_unsupported_observation =
            !self.missing_query_facts.is_empty() || !self.observation_issues.is_empty();
        if (terminal == HarnessTerminalReason::Unsupported) != has_unsupported_observation
            || (terminal == HarnessTerminalReason::CapabilityMismatch)
                == self.missing_capabilities.is_empty()
        {
            return Err(contract_error(
                "run-result missing-fact/capability details contradict the terminal reason",
            ));
        }
        Ok(())
    }
}

impl HarnessObjectiveResult {
    fn validate(
        &self,
        terminal: HarnessTerminalReason,
        request: &HarnessRunRequest,
    ) -> Result<(), HarnessRunContractError> {
        let reached = terminal == HarnessTerminalReason::Reached;
        let complete_proof = self.first_hit_tick.is_some()
            && self.evidence.is_some()
            && self.boundary_fingerprint.is_some();
        let any_proof = self.first_hit_tick.is_some()
            || self.evidence.is_some()
            || self.boundary_fingerprint.is_some();
        if self.reached != reached
            || (reached && !complete_proof)
            || (!reached && any_proof)
            || self
                .first_hit_tick
                .is_some_and(|tick| tick >= request.logical_tick_budget)
        {
            return Err(contract_error(
                "run-result objective evidence contradicts the terminal reason",
            ));
        }
        if let Some(evidence) = &self.evidence {
            validate_artifact("objective evidence", evidence)?;
        }
        if let Some(fingerprint) = &self.boundary_fingerprint {
            fingerprint.validate()?;
        }
        Ok(())
    }
}

impl HarnessBoundaryFingerprint {
    fn validate(&self) -> Result<(), HarnessRunContractError> {
        validate_id("boundary-fingerprint schema", &self.schema)?;
        validate_id("boundary-fingerprint algorithm", &self.algorithm)?;
        validate_id(
            "boundary-fingerprint canonical encoding",
            &self.canonical_encoding,
        )?;
        if self.digest.len() < 32
            || self.digest.len() > 128
            || !self.digest.len().is_multiple_of(2)
            || !self
                .digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(contract_error(
                "boundary fingerprint digest is not canonical hex",
            ));
        }
        Ok(())
    }
}

impl HarnessRunArtifacts {
    fn validate(
        &self,
        terminal: HarnessTerminalReason,
        native_evidence_requested: bool,
    ) -> Result<(), HarnessRunContractError> {
        for (label, reference) in [
            ("realized input", self.realized_input.as_ref()),
            ("gameplay trace", self.gameplay_trace.as_ref()),
            ("objective result", self.objective_result.as_ref()),
            ("stdout", self.stdout.as_ref()),
            ("stderr", self.stderr.as_ref()),
            ("native phase timing", self.native_phase_timing.as_ref()),
            (
                "native evidence oracle result",
                self.native_evidence
                    .as_ref()
                    .map(|evidence| &evidence.oracle_result),
            ),
            (
                "native evidence semantic trace",
                self.native_evidence
                    .as_ref()
                    .map(|evidence| &evidence.semantic_trace),
            ),
        ] {
            if let Some(reference) = reference {
                validate_artifact(label, reference)?;
            }
        }
        let complete_artifacts = self.realized_input.is_some()
            && self.gameplay_trace.is_some()
            && self.objective_result.is_some()
            && (!native_evidence_requested || self.native_evidence.is_some());
        if self.complete && !complete_artifacts {
            return Err(contract_error(
                "complete run result is missing replay or objective artifacts",
            ));
        }
        if terminal == HarnessTerminalReason::Reached && !self.complete {
            return Err(contract_error(
                "reached run result requires complete replay and proof artifacts",
            ));
        }
        Ok(())
    }
}

impl HarnessNativePhaseTiming {
    pub fn validate(&self, host_elapsed_millis: u64) -> Result<(), HarnessRunContractError> {
        if !matches!(
            self.schema.as_str(),
            NATIVE_LIFECYCLE_TIMING_SCHEMA_V1 | NATIVE_LIFECYCLE_TIMING_SCHEMA_V2
        ) || self.clock != "steady_clock"
        {
            return Err(contract_error(
                "unsupported native lifecycle timing identity",
            ));
        }
        match (self.schema.as_str(), &self.session_reuse_audit) {
            (NATIVE_LIFECYCLE_TIMING_SCHEMA_V1, None) => {}
            (NATIVE_LIFECYCLE_TIMING_SCHEMA_V2, Some(audit)) => {
                audit.validate().map_err(|error| {
                    contract_error(format!("invalid native session reuse audit: {error}"))
                })?;
                if audit.evaluated_boundary != POST_AUTHENTICATED_RUN_BOUNDARY {
                    return Err(contract_error(
                        "native session reuse audit was not evaluated after the authenticated run",
                    ));
                }
            }
            _ => {
                return Err(contract_error(
                    "native lifecycle timing schema contradicts session reuse evidence",
                ));
            }
        }
        let phases = [
            self.process_entry_micros,
            self.cli_configured_micros,
            self.aurora_initialized_micros,
            self.engine_ready_micros,
            self.stage_ready_micros,
            self.first_simulation_tick_micros,
            self.last_simulation_tick_micros,
            self.proof_artifacts_written_micros,
            self.engine_shutdown_micros,
            self.exit_ready_micros,
        ];
        if self.process_entry_micros != 0 || !phases.windows(2).all(|pair| pair[0] <= pair[1]) {
            return Err(contract_error(
                "native lifecycle timing phases are nonmonotonic",
            ));
        }
        let host_upper_bound = host_elapsed_millis
            .saturating_mul(1_000)
            .saturating_add(1_000);
        if self.exit_ready_micros > host_upper_bound {
            return Err(contract_error(
                "native lifecycle timing exceeds host process duration",
            ));
        }
        Ok(())
    }
}

fn validate_artifact(
    label: &str,
    reference: &ArtifactReference,
) -> Result<(), HarnessRunContractError> {
    validate_relative_path(label, &reference.path)?;
    if reference.sha256 == Digest::ZERO {
        return Err(contract_error(format!("{label} digest must be nonzero")));
    }
    Ok(())
}

fn canonical_root(root: &Path) -> Result<PathBuf, HarnessRunContractError> {
    root.canonicalize().map_err(|error| {
        contract_error(format!(
            "cannot resolve validation root {}: {error}",
            root.display()
        ))
    })
}

fn read_artifact(
    root: &Path,
    reference: &ArtifactReference,
    label: &str,
    allow_canonical_escape: bool,
) -> Result<(), HarnessRunContractError> {
    let path = root.join(&reference.path);
    let canonical = path.canonicalize().map_err(|error| {
        contract_error(format!(
            "cannot resolve {label} {}: {error}",
            path.display()
        ))
    })?;
    if (!allow_canonical_escape && !canonical.starts_with(root)) || !canonical.is_file() {
        return Err(contract_error(format!(
            "{label} escapes the validation root or is not a file"
        )));
    }
    if sha256_artifact_file(&canonical)? != reference.sha256 {
        return Err(contract_error(format!("{label} digest is stale")));
    }
    Ok(())
}

/// Stream an artifact SHA-256 and retain a bounded metadata-keyed digest for
/// large immutable inputs such as disc images. Repeated requests in one
/// process avoid rereading gigabytes while size or mtime changes invalidate the
/// cached value.
pub fn sha256_artifact_file(path: &Path) -> Result<Digest, HarnessRunContractError> {
    let canonical = path.canonicalize().map_err(|error| {
        contract_error(format!(
            "cannot resolve artifact {}: {error}",
            path.display()
        ))
    })?;
    let metadata = canonical.metadata().map_err(|error| {
        contract_error(format!(
            "cannot inspect artifact {}: {error}",
            canonical.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(contract_error("artifact is not a file"));
    }
    let cache_key = (metadata.len() >= LARGE_ARTIFACT_CACHE_THRESHOLD_BYTES)
        .then(|| ArtifactDigestCacheKey::new(&canonical, &metadata))
        .transpose()?;
    if let Some(digest) = cache_key.as_ref().and_then(cached_artifact_digest) {
        return Ok(digest);
    }
    let mut file = fs::File::open(&canonical).map_err(|error| {
        contract_error(format!(
            "cannot read artifact {}: {error}",
            canonical.display()
        ))
    })?;
    let mut hasher = Sha256::new();
    // The Windows CLI thread has a 1 MiB stack. Keeping the whole read buffer
    // there made a successful cold replay crash while hashing its disc artifact.
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|error| {
            contract_error(format!(
                "cannot hash artifact {}: {error}",
                canonical.display()
            ))
        })?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let digest = Digest(hasher.finalize().into());
    if let Some(cache_key) = cache_key {
        remember_artifact_digest(cache_key, digest);
    }
    Ok(digest)
}

const LARGE_ARTIFACT_CACHE_THRESHOLD_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ARTIFACT_DIGEST_CACHE_ENTRIES: usize = 32;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ArtifactDigestCacheKey {
    path: PathBuf,
    length: u64,
    modified_nanos: u128,
}

impl ArtifactDigestCacheKey {
    fn new(path: &Path, metadata: &fs::Metadata) -> Result<Self, HarnessRunContractError> {
        let modified_nanos = metadata
            .modified()
            .map_err(|error| contract_error(format!("cannot inspect artifact mtime: {error}")))?
            .duration_since(UNIX_EPOCH)
            .map_err(|_| contract_error("artifact mtime predates the Unix epoch"))?
            .as_nanos();
        Ok(Self {
            path: path.to_path_buf(),
            length: metadata.len(),
            modified_nanos,
        })
    }
}

fn artifact_digest_cache() -> &'static Mutex<BTreeMap<ArtifactDigestCacheKey, Digest>> {
    static CACHE: OnceLock<Mutex<BTreeMap<ArtifactDigestCacheKey, Digest>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn cached_artifact_digest(key: &ArtifactDigestCacheKey) -> Option<Digest> {
    artifact_digest_cache().lock().unwrap().get(key).copied()
}

fn remember_artifact_digest(key: ArtifactDigestCacheKey, digest: Digest) {
    let mut cache = artifact_digest_cache().lock().unwrap();
    cache.insert(key, digest);
    while cache.len() > MAX_ARTIFACT_DIGEST_CACHE_ENTRIES {
        cache.pop_first();
    }
}

fn validate_relative_path(label: &str, value: &str) -> Result<(), HarnessRunContractError> {
    let path = PathBuf::from(value);
    if value.is_empty()
        || value.contains('\\')
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::CurDir
                    | Component::ParentDir
                    | Component::RootDir
                    | Component::Prefix(_)
            )
        })
    {
        return Err(contract_error(format!(
            "{label} must be a canonical relative path"
        )));
    }
    Ok(())
}

fn validate_id(label: &str, value: &str) -> Result<(), HarnessRunContractError> {
    if value.is_empty()
        || value.len() > 192
        || value != value.trim()
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'-' | b'_' | b'.' | b'/' | b':')
        })
    {
        return Err(contract_error(format!(
            "{label} is not a canonical identifier"
        )));
    }
    Ok(())
}

fn validate_sorted_names(
    label: &str,
    values: &[String],
    maximum: usize,
) -> Result<(), HarnessRunContractError> {
    if values.len() > maximum
        || !values.windows(2).all(|pair| pair[0] < pair[1])
        || values
            .iter()
            .any(|value| validate_id(label, value).is_err())
    {
        return Err(contract_error(format!(
            "{label} must be bounded, unique, canonical, and sorted"
        )));
    }
    Ok(())
}

fn validate_sorted_facts(
    label: &str,
    values: &[String],
    maximum: usize,
) -> Result<(), HarnessRunContractError> {
    if values.len() > maximum
        || !values.windows(2).all(|pair| pair[0] < pair[1])
        || values.iter().any(|value| {
            value.is_empty()
                || value.len() > 192
                || value.starts_with('.')
                || value.ends_with('.')
                || value.bytes().any(|byte| {
                    !(byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'_' | b'.' | b'[' | b']' | b'-'))
                })
        })
    {
        return Err(contract_error(format!(
            "{label} must be bounded, unique, canonical, and sorted"
        )));
    }
    Ok(())
}

fn validate_text(label: &str, value: &str) -> Result<(), HarnessRunContractError> {
    if value.is_empty()
        || value.len() > MAX_MESSAGE_BYTES
        || value != value.trim()
        || value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
    {
        return Err(contract_error(format!("{label} is empty or invalid")));
    }
    Ok(())
}

fn canonical_digest<T: Serialize>(
    domain: &[u8],
    value: &T,
) -> Result<Digest, HarnessRunContractError> {
    let encoded = serde_json::to_vec(value)
        .map_err(|error| contract_error(format!("cannot encode harness identity: {error}")))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((encoded.len() as u64).to_le_bytes());
    hasher.update(encoded);
    Ok(Digest(hasher.finalize().into()))
}

fn pretty_json<T: Serialize>(value: &T) -> Result<Vec<u8>, HarnessRunContractError> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| contract_error(format!("cannot encode harness contract: {error}")))?;
    bytes.push(b'\n');
    Ok(bytes)
}

#[derive(Debug)]
pub struct HarnessRunContractError(String);

impl fmt::Display for HarnessRunContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for HarnessRunContractError {}

fn contract_error(message: impl Into<String>) -> HarnessRunContractError {
    HarnessRunContractError(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::ARTIFACT_SCHEMA_VERSION;
    use crate::milestone_dsl;
    use crate::observation_view::movement_state_v2_spec;
    use crate::scenario_fixture::{SCENARIO_FIXTURE_SCHEMA, ScenarioFixture};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

    fn digest(bytes: &[u8]) -> Digest {
        Digest(Sha256::digest(bytes).into())
    }

    fn artifact(path: &str, bytes: &[u8]) -> ArtifactReference {
        ArtifactReference {
            path: path.into(),
            sha256: digest(bytes),
        }
    }

    fn root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "huntctl-run-contract-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_ROOT.fetch_add(1, Ordering::Relaxed),
        ))
    }

    fn build(game_digest: Digest) -> BuildIdentity {
        BuildIdentity {
            dusklight_commit: "1".repeat(40),
            aurora_commit: "2".repeat(40),
            compiler: "apple-clang-20".into(),
            target: "arm64-apple-darwin".into(),
            profile: "debug-observers".into(),
            feature_digest: Digest([3; 32]),
            game_digest,
            dirty_digest: None,
            fidelity_profile: "native-read-only".into(),
        }
    }

    fn protocol(capabilities: &[&str]) -> HarnessProtocolIdentity {
        let mut value = HarnessProtocolIdentity {
            name: "dusklight-automation".into(),
            version: 2,
            capabilities_sha256: Digest::ZERO,
            capabilities: capabilities.iter().map(|value| (*value).into()).collect(),
        };
        value.refresh_capabilities_sha256().unwrap();
        value
    }

    fn request(root: &Path) -> HarnessRunRequest {
        fs::create_dir_all(root.join("inputs")).unwrap();
        let executable = b"test executable";
        let game = b"test game data";
        fs::write(root.join("inputs/dusklight"), executable).unwrap();
        fs::write(root.join("inputs/game.iso"), game).unwrap();

        let scenario = ScenarioFixture {
            schema: SCENARIO_FIXTURE_SCHEMA.into(),
            name: "stage-ready".into(),
            form: None,
            health: None,
            rng: Vec::new(),
            video_mode: None,
            inventory: Vec::new(),
            equipment: Vec::new(),
            flags: Vec::new(),
            settings: Vec::new(),
        };
        let scenario_bytes = serde_json::to_vec_pretty(&scenario).unwrap();
        fs::write(root.join("inputs/scenario.json"), &scenario_bytes).unwrap();

        let objective_bytes = b"milestones 1.0\n\nmilestone stage_ready {\n  phase post_sim\n  when stage.name == \"F_SP103\" && player.exists\n}\n";
        fs::write(root.join("inputs/objective.milestones"), objective_bytes).unwrap();
        let objective =
            milestone_dsl::parse(std::str::from_utf8(objective_bytes).unwrap()).unwrap();
        let compiled = milestone_dsl::compile(&objective).unwrap();

        let mut observation = movement_state_v2_spec();
        observation.objective.id = "stage_ready".into();
        let observation_bytes = serde_json::to_vec_pretty(&observation).unwrap();
        fs::write(root.join("inputs/observation.json"), &observation_bytes).unwrap();

        let build = build(digest(game));
        let protocol = protocol(&[
            "gameplay-trace-v5",
            "input-tape-v3",
            "milestone-program-v1.5",
            "stage-boot",
        ]);
        let scenario = artifact("inputs/scenario.json", &scenario_bytes);
        let objective = ObjectiveProgramReference {
            source: artifact("inputs/objective.milestones", objective_bytes),
            program_sha256: Digest(compiled.program_sha256),
            goal: "stage_ready".into(),
        };
        let observation_view = ObservationViewReference {
            source: artifact("inputs/observation.json", &observation_bytes),
            schema_sha256: observation.digest().unwrap(),
        };
        let action_schema = SchemaIdentity {
            id: "movement-pad-frame/v2".into(),
            sha256: Digest([4; 32]),
        };
        let identity = ArtifactIdentity {
            schema_version: ARTIFACT_SCHEMA_VERSION,
            content_digest: Digest([5; 32]),
            build: build.clone(),
            protocol_name: protocol.name.clone(),
            protocol_version: protocol.version,
            protocol_capabilities_digest: protocol.capabilities_sha256,
            scenario_id: "stage-ready-scenario".into(),
            region_digest: Digest([6; 32]),
            language_assets_digest: Digest([7; 32]),
            scenario_digest: scenario.sha256,
            predicate_program_digest: objective.program_sha256,
            action_schema_digest: action_schema.sha256,
            observation_schema_digest: observation_view.schema_sha256,
            settings_digest: Digest([8; 32]),
        };
        let mut request = HarnessRunRequest {
            schema: RUN_REQUEST_SCHEMA_V2.into(),
            content_sha256: Digest::ZERO,
            id: "stage-ready-attempt".into(),
            executable: artifact("inputs/dusklight", executable),
            game_data: artifact("inputs/game.iso", game),
            build,
            identity,
            protocol,
            boot: ObjectiveBoot::Stage {
                stage: "F_SP103".into(),
                room: 1,
                point: 1,
                layer: 3,
                save_slot: None,
            },
            scenario,
            objective,
            observation_view,
            action_schema,
            observation_requirements: ObjectiveObservationRequirements {
                schema: crate::observation_contract::OBJECTIVE_OBSERVATION_REQUIREMENTS_SCHEMA_V1
                    .into(),
                families: vec![
                    crate::observation_contract::ObservationFamilyRequirement {
                        id: "player_motion".into(),
                        minimum_version: 1,
                    },
                    crate::observation_contract::ObservationFamilyRequirement {
                        id: "stage".into(),
                        minimum_version: 1,
                    },
                ],
                facts: vec!["player.exists".into(), "stage.name".into()],
            },
            input: ObjectiveSeed::Neutral,
            native_evidence: None,
            rng_seed: 42,
            logical_tick_budget: 300,
            host_timeout_seconds: 30,
            fidelity: HarnessFidelityMode::Headless,
            artifact_destination: "build/harness/stage-ready-attempt".into(),
        };
        request.refresh_content_sha256().unwrap();
        request
    }

    fn reached_result(request: &HarnessRunRequest, root: &Path) -> HarnessRunResult {
        fs::create_dir_all(root).unwrap();
        let tape = b"realized input";
        let trace = b"gameplay trace";
        let evidence = b"objective evidence";
        fs::write(root.join("realized.tape"), tape).unwrap();
        fs::write(root.join("gameplay.trace"), trace).unwrap();
        fs::write(root.join("objective.json"), evidence).unwrap();
        let objective = artifact("objective.json", evidence);
        let mut result = HarnessRunResult {
            schema: RUN_RESULT_SCHEMA_V2.into(),
            content_sha256: Digest::ZERO,
            request_id: request.id.clone(),
            request_sha256: request.content_sha256,
            identity: request.identity.clone(),
            attempt: 1,
            worker: HarnessWorkerIdentity {
                id: "local-worker-0".into(),
                build: request.build.clone(),
                protocol: request.protocol.clone(),
            },
            terminal: HarnessTerminalReason::Reached,
            detail: HarnessTerminalDetail {
                message: "objective reached".into(),
                missing_query_facts: Vec::new(),
                missing_capabilities: Vec::new(),
                observation_issues: Vec::new(),
            },
            objective: HarnessObjectiveResult {
                reached: true,
                first_hit_tick: Some(5),
                evidence: Some(objective.clone()),
                boundary_fingerprint: Some(HarnessBoundaryFingerprint {
                    schema: "dusklight.milestone-boundary/v4".into(),
                    algorithm: "xxh3-128".into(),
                    canonical_encoding: "little-endian-fixed-v4".into(),
                    digest: "12".repeat(16),
                }),
            },
            artifacts: HarnessRunArtifacts {
                realized_input: Some(artifact("realized.tape", tape)),
                gameplay_trace: Some(artifact("gameplay.trace", trace)),
                objective_result: Some(objective),
                stdout: None,
                stderr: None,
                native_phase_timing: None,
                native_evidence: None,
                complete: true,
            },
            timing: HarnessRunTiming {
                logical_ticks: 6,
                consumed_input_ticks: 6,
                host_elapsed_millis: 50,
                native_phases: None,
            },
        };
        result.refresh_content_sha256().unwrap();
        result
    }

    #[test]
    fn request_binds_all_inputs_and_validates_their_bytes() {
        let root = root();
        let mut request = request(&root);
        let report = request.validate_files(&root).unwrap();
        assert_eq!(report.request_sha256, request.content_sha256);
        assert_eq!(report.objective_id, "stage_ready");
        let decoded: HarnessRunRequest =
            serde_json::from_slice(&request.to_pretty_json().unwrap()).unwrap();
        assert_eq!(decoded, request);

        fs::write(root.join("inputs/game.iso"), b"changed").unwrap();
        assert!(request.validate_files(&root).is_err());
        fs::write(root.join("inputs/game.iso"), b"test game data").unwrap();
        let mut detached = request.clone();
        detached.identity.action_schema_digest = Digest([99; 32]);
        detached.refresh_content_sha256().unwrap();
        assert!(detached.validate().is_err());

        request.observation_requirements.facts.reverse();
        request.refresh_content_sha256().unwrap();
        assert!(request.validate().is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn request_authenticates_a_repo_relative_game_data_symlink() {
        let repository_root = root();
        let external_root = root();
        let request = request(&repository_root);
        fs::create_dir_all(&external_root).unwrap();
        let external_game = external_root.join("game.iso");
        fs::write(&external_game, b"test game data").unwrap();
        fs::remove_file(repository_root.join("inputs/game.iso")).unwrap();
        std::os::unix::fs::symlink(&external_game, repository_root.join("inputs/game.iso"))
            .unwrap();

        request.validate_files(&repository_root).unwrap();
        fs::write(&external_game, b"changed").unwrap();
        assert!(request.validate_files(&repository_root).is_err());

        fs::remove_dir_all(repository_root).unwrap();
        fs::remove_dir_all(external_root).unwrap();
    }

    #[test]
    fn reached_result_requires_and_authenticates_replay_proof() {
        let repository_root = root();
        let request = request(&repository_root);
        let artifact_root = repository_root.join("result");
        let mut result = reached_result(&request, &artifact_root);
        let report = result.validate_files(&request, &artifact_root).unwrap();
        assert_eq!(report.terminal, HarnessTerminalReason::Reached);
        assert!(report.artifacts_complete);

        result.artifacts.realized_input = None;
        result.refresh_content_sha256().unwrap();
        assert!(result.validate_against(&request).is_err());
        fs::remove_dir_all(repository_root).unwrap();
    }

    #[test]
    fn mismatch_and_unsupported_terminals_are_not_ambiguous_successes() {
        let root = root();
        let request = request(&root);
        let mut result = reached_result(&request, &root.join("result"));
        result.terminal = HarnessTerminalReason::Unsupported;
        result.detail.message = "required observation is unavailable".into();
        result.detail.missing_query_facts = vec!["player.exists".into()];
        result.objective = HarnessObjectiveResult {
            reached: false,
            first_hit_tick: None,
            evidence: None,
            boundary_fingerprint: None,
        };
        result.artifacts.complete = false;
        result.refresh_content_sha256().unwrap();
        assert!(result.validate_against(&request).is_ok());

        result.terminal = HarnessTerminalReason::CapabilityMismatch;
        result.detail.missing_query_facts.clear();
        result.detail.missing_capabilities = vec!["stage-boot".into()];
        result.worker.protocol = protocol(&["gameplay-trace-v5"]);
        result.refresh_content_sha256().unwrap();
        assert!(result.validate_against(&request).is_ok());

        result.worker.protocol = request.protocol.clone();
        result.refresh_content_sha256().unwrap();
        assert!(result.validate_against(&request).is_err());

        result.terminal = HarnessTerminalReason::IdentityMismatch;
        result.detail.missing_capabilities.clear();
        result.worker.protocol.name = "different-automation-protocol".into();
        result.refresh_content_sha256().unwrap();
        assert!(result.validate_against(&request).is_ok());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unavailable_or_truncated_objective_families_admit_only_unsupported() {
        use crate::observation_contract::{
            OBSERVATION_INVENTORY_SCHEMA_V1, ObservationFamilyAvailability, ObservationFamilyStatus,
        };

        let root = root();
        let request = request(&root);
        let inventory = ObservationInventory {
            schema: OBSERVATION_INVENTORY_SCHEMA_V1.into(),
            families: vec![
                ObservationFamilyAvailability {
                    id: "player_motion".into(),
                    version: Some(1),
                    status: ObservationFamilyStatus::Truncated,
                },
                ObservationFamilyAvailability {
                    id: "stage".into(),
                    version: Some(1),
                    status: ObservationFamilyStatus::Present,
                },
            ],
        };
        let detail = request
            .unsupported_observation_detail(&inventory)
            .unwrap()
            .unwrap();
        assert_eq!(detail.missing_query_facts, ["player.exists"]);
        assert_eq!(detail.observation_issues.len(), 1);
        assert_eq!(detail.observation_issues[0].family, "player_motion");

        let mut result = reached_result(&request, &root.join("result"));
        result.terminal = HarnessTerminalReason::Unsupported;
        result.detail = detail;
        result.objective = HarnessObjectiveResult {
            reached: false,
            first_hit_tick: None,
            evidence: None,
            boundary_fingerprint: None,
        };
        result.artifacts.complete = false;
        result.refresh_content_sha256().unwrap();
        assert!(result.validate_against(&request).is_ok());

        result.terminal = HarnessTerminalReason::Exhausted;
        result.refresh_content_sha256().unwrap();
        assert!(result.validate_against(&request).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn crash_can_retain_authenticated_partial_artifacts_without_success() {
        let root = root();
        let request = request(&root);
        let mut result = reached_result(&request, &root.join("result"));
        result.terminal = HarnessTerminalReason::WorkerCrashed;
        result.detail.message = "worker exited after signal 11".into();
        result.objective = HarnessObjectiveResult {
            reached: false,
            first_hit_tick: None,
            evidence: None,
            boundary_fingerprint: None,
        };
        result.artifacts.objective_result = None;
        result.artifacts.complete = false;
        result.refresh_content_sha256().unwrap();
        assert!(
            result
                .validate_files(&request, &root.join("result"))
                .is_ok()
        );

        result.identity.settings_digest = Digest([98; 32]);
        result.refresh_content_sha256().unwrap();
        let error = result.validate_against(&request).unwrap_err();
        assert!(error.to_string().contains("settings_digest"));
        fs::remove_dir_all(root).unwrap();
    }

    fn native_phase_timing_v2() -> HarnessNativePhaseTiming {
        HarnessNativePhaseTiming {
            schema: NATIVE_LIFECYCLE_TIMING_SCHEMA_V2.into(),
            clock: "steady_clock".into(),
            process_entry_micros: 0,
            cli_configured_micros: 1,
            aurora_initialized_micros: 2,
            engine_ready_micros: 3,
            stage_ready_micros: 4,
            first_simulation_tick_micros: 5,
            last_simulation_tick_micros: 6,
            proof_artifacts_written_micros: 7,
            engine_shutdown_micros: 8,
            exit_ready_micros: 9,
            session_reuse_audit: Some(SessionReuseAudit {
                schema: ENGINE_SESSION_REUSE_AUDIT_SCHEMA_V1.into(),
                reusable: false,
                evaluated_boundary: POST_AUTHENTICATED_RUN_BOUNDARY.into(),
                target_boundary: POST_AUTHENTICATED_RUN_BOUNDARY.into(),
                blockers: vec![
                    SessionReuseBlocker {
                        code: "game_global_reconstruction".into(),
                        subsystem: "game_state".into(),
                        required_guarantee: "game state reconstructs from a clean origin".into(),
                    },
                ],
            }),
        }
    }

    #[test]
    fn native_phase_v2_authenticates_post_run_reuse_refusal() {
        native_phase_timing_v2().validate(1).unwrap();
    }

    #[test]
    fn native_phase_v2_rejects_a_preboot_only_audit() {
        let mut timing = native_phase_timing_v2();
        timing
            .session_reuse_audit
            .as_mut()
            .unwrap()
            .evaluated_boundary = "pre_engine_boot".into();
        let error = timing.validate(1).unwrap_err().to_string();
        assert!(error.contains("was not evaluated after the authenticated run"));
    }

    #[test]
    fn terminal_reason_names_are_stable_and_round_trip() {
        for terminal in HarnessTerminalReason::ALL {
            let encoded = serde_json::to_string(&terminal).unwrap();
            assert_eq!(encoded, format!("\"{}\"", terminal.name()));
            assert_eq!(
                serde_json::from_str::<HarnessTerminalReason>(&encoded).unwrap(),
                terminal
            );
        }
        assert_eq!(HarnessTerminalReason::ALL.len(), 15);
    }
}
