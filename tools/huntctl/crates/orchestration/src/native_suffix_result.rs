//! Fail-closed validation of native checkpoint suffix-batch results.

use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::RawPadState;
use dusklight_search::suffix_batch::{NATIVE_SUFFIX_BATCH_SCHEMA, NativeSuffixBatch};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const NATIVE_SUFFIX_BATCH_RESULT_SCHEMA_V4: &str = "dusklight-suffix-batch-result/v4";
pub const NATIVE_EPISODE_SHARD_SCHEMA_V2: &str = "dusklight-native-episode-shard/v2";
pub const RAW_PAD_ACTION_SCHEMA_V2: &str = "dusklight-raw-pad-action/v2";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTerminalBinding {
    pub goal: String,
    pub program_sha256: Digest,
    pub definition_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSuffixBatchResult {
    pub schema: String,
    pub status: String,
    pub source_frame: u64,
    pub source_boundary: NativeSourceBoundaryResult,
    pub checkpoint_validation: NativeCheckpointValidationResult,
    pub maximum_ticks: u64,
    pub candidate_count: u64,
    pub completed_candidates: u64,
    pub verify_state_hashes: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_model: Option<Value>,
    pub checkpoint_bytes: u64,
    pub restore_identity: Option<String>,
    pub capture_micros: u64,
    pub restore_micros: Vec<u64>,
    pub timing: NativeSuffixTimingResult,
    pub audio_callback_quiesced: bool,
    pub episode_shard: NativeEpisodeShardResult,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub winner_id: Option<String>,
    pub candidates: Vec<NativeSuffixCandidateResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSourceBoundaryResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestone: Option<String>,
    pub expected_fingerprint: String,
    pub actual_fingerprint: Option<String>,
    pub fingerprint_verified: bool,
    pub verified: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCheckpointValidationResult {
    pub kind: String,
    pub ticks: u64,
    pub verified: bool,
    pub source_semantic_digest: Option<String>,
    pub fresh_sequence_digest: Option<String>,
    pub restored_sequence_digest: Option<String>,
    pub first_divergence_tick: Option<u64>,
    pub expected_tick_digest: Option<String>,
    pub actual_tick_digest: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSuffixTimingResult {
    pub schema: String,
    pub batch_wall_micros: u64,
    pub candidate_ticks: u64,
    pub verified: bool,
    pub accounting: Value,
    pub phases: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeEpisodeShardResult {
    pub schema: String,
    pub path: String,
    pub observation_schema: String,
    pub action_schema: String,
    pub episode_count: u64,
    pub uncompressed_bytes: u64,
    pub compressed_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSuffixCandidateResult {
    pub id: String,
    pub success: bool,
    pub ticks_executed: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_hit_tick: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_sequence_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_tick_digests: Option<Vec<String>>,
    pub predicate_evidence: NativePredicateEvidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consumed_pad_states: Option<Vec<RawPadState>>,
    pub terminal_observation: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativePredicateEvidence {
    pub schema: NativeMilestoneSchema,
    pub boot: NativeBootEvidence,
    pub boot_origin_established: bool,
    pub goal: String,
    pub goal_reached: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_digest: Option<String>,
    pub milestones: Vec<NativeMilestoneEvidence>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeMilestoneSchema {
    pub name: String,
    pub version: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeBootEvidence {
    pub kind: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeMilestoneEvidence {
    pub id: String,
    pub hit: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sim_tick: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tape_frame: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boundary_index: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stable_ticks: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projections: Option<Value>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedNativeSuffixCandidate {
    pub id: String,
    pub simulated_ticks: u64,
    pub first_hit_tick: Option<u64>,
    pub state_sequence_digest: Option<String>,
    pub behavior_sha256: Digest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedNativeSuffixBatch {
    pub restore_identity: String,
    pub checkpoint_bytes: u64,
    pub simulated_ticks: u64,
    pub episode_shard_path: String,
    pub candidates: Vec<ValidatedNativeSuffixCandidate>,
}

impl NativeSuffixBatchResult {
    pub fn validate_against(
        &self,
        request: &NativeSuffixBatch,
        terminal: &NativeTerminalBinding,
    ) -> Result<ValidatedNativeSuffixBatch, NativeSuffixResultError> {
        if request.schema != NATIVE_SUFFIX_BATCH_SCHEMA {
            return Err(result_error(
                "unsupported residual suffix-batch request schema",
            ));
        }
        let candidate_count = request.candidates.len() as u64;
        if self.schema != NATIVE_SUFFIX_BATCH_RESULT_SCHEMA_V4
            || self.status != "passed"
            || self.error.is_some()
            || self.source_frame != request.source_frame as u64
            || self.maximum_ticks != request.maximum_ticks as u64
            || self.candidate_count != candidate_count
            || self.completed_candidates != candidate_count
            || self.candidates.len() != request.candidates.len()
            || self.verify_state_hashes != request.verify_state_hashes
            || self.policy_model.is_some()
            || self.checkpoint_bytes == 0
            || self.capture_micros == 0
            || self.restore_micros.is_empty()
            || !self.audio_callback_quiesced
            || !self.timing.verified
            || self.timing.schema != "dusklight-suffix-batch-timing/v1"
        {
            return Err(result_error(
                "native suffix result is incomplete or detached from its request",
            ));
        }
        let restore_identity = self
            .restore_identity
            .as_deref()
            .filter(|value| lower_hex(value, 32))
            .ok_or_else(|| result_error("native suffix result lacks a checkpoint identity"))?;
        validate_source_boundary(&self.source_boundary, request)?;
        validate_checkpoint(&self.checkpoint_validation, request)?;
        validate_episode_shard(&self.episode_shard, candidate_count)?;

        let mut ids = BTreeSet::new();
        let mut simulated_ticks = 0_u64;
        let mut candidates = Vec::with_capacity(self.candidates.len());
        for (expected, actual) in request.candidates.iter().zip(&self.candidates) {
            if expected.id != actual.id || !ids.insert(actual.id.as_str()) {
                return Err(result_error(
                    "native suffix result candidates are reordered, duplicated, or detached",
                ));
            }
            candidates.push(actual.validate_against(request, terminal)?);
            simulated_ticks = simulated_ticks
                .checked_add(actual.ticks_executed)
                .ok_or_else(|| result_error("native suffix simulated tick total overflowed"))?;
        }
        if self.timing.candidate_ticks != simulated_ticks {
            return Err(result_error(
                "native suffix timing does not charge every simulated candidate tick",
            ));
        }
        let winner = self
            .candidates
            .iter()
            .filter(|candidate| candidate.success)
            .min_by_key(|candidate| (candidate.first_hit_tick, candidate.id.as_str()))
            .map(|candidate| candidate.id.as_str());
        if self.winner_id.as_deref() != winner {
            return Err(result_error(
                "native suffix winner does not match the exact successful candidates",
            ));
        }
        Ok(ValidatedNativeSuffixBatch {
            restore_identity: restore_identity.into(),
            checkpoint_bytes: self.checkpoint_bytes,
            simulated_ticks,
            episode_shard_path: self.episode_shard.path.clone(),
            candidates,
        })
    }
}

impl NativeSuffixCandidateResult {
    fn validate_against(
        &self,
        request: &NativeSuffixBatch,
        terminal: &NativeTerminalBinding,
    ) -> Result<ValidatedNativeSuffixCandidate, NativeSuffixResultError> {
        let exact_verdict = match (self.success, self.first_hit_tick) {
            (true, Some(tick))
                if tick.checked_add(1) == Some(self.ticks_executed)
                    && self.ticks_executed <= request.maximum_ticks as u64 =>
            {
                true
            }
            (false, None) if self.ticks_executed == request.maximum_ticks as u64 => false,
            _ => {
                return Err(result_error(
                    "native suffix candidate has an invalid exact terminal verdict",
                ));
            }
        };
        let state_sequence_digest = self.state_sequence_digest.as_deref();
        match (
            state_sequence_digest,
            &self.state_tick_digests,
            request.verify_state_hashes,
        ) {
            (Some(sequence), Some(digests), true)
                if lower_hex(sequence, 32)
                    && digests.len() == self.ticks_executed as usize
                    && digests.iter().all(|digest| lower_hex(digest, 32)) => {}
            (None, None, false) => {}
            _ => {
                return Err(result_error(
                    "native suffix candidate state-hash evidence differs from the request",
                ));
            }
        }
        match (&self.consumed_pad_states, exact_verdict) {
            (Some(pads), true) if pads.len() == self.ticks_executed as usize => {}
            (None, false) => {}
            _ => {
                return Err(result_error(
                    "native suffix candidate consumed PAD evidence is not success-exact",
                ));
            }
        }
        validate_predicate(&self.predicate_evidence, terminal, exact_verdict)?;
        Ok(ValidatedNativeSuffixCandidate {
            id: self.id.clone(),
            simulated_ticks: self.ticks_executed,
            // The native suffix wire format records a zero-based candidate
            // index; route scores and retention use one-based elapsed ticks.
            first_hit_tick: self.first_hit_tick.map(|tick| tick + 1),
            state_sequence_digest: state_sequence_digest.map(str::to_owned),
            behavior_sha256: behavior_digest(self)?,
        })
    }
}

fn behavior_digest(
    candidate: &NativeSuffixCandidateResult,
) -> Result<Digest, NativeSuffixResultError> {
    #[derive(Serialize)]
    struct Behavior<'a> {
        success: bool,
        first_hit_tick: Option<u64>,
        ticks_executed: u64,
        state_sequence_digest: Option<&'a str>,
        terminal_observation: &'a Value,
    }
    let value = Behavior {
        success: candidate.success,
        first_hit_tick: candidate.first_hit_tick,
        ticks_executed: candidate.ticks_executed,
        state_sequence_digest: candidate.state_sequence_digest.as_deref(),
        terminal_observation: &candidate.terminal_observation,
    };
    let bytes = serde_json::to_vec(&value).map_err(|error| result_error(error.to_string()))?;
    let mut hasher = sha2::Sha256::new();
    use sha2::Digest as _;
    hasher.update(b"dusklight.native-suffix-behavior/v1\0");
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

fn validate_source_boundary(
    actual: &NativeSourceBoundaryResult,
    request: &NativeSuffixBatch,
) -> Result<(), NativeSuffixResultError> {
    if actual.expected_fingerprint != request.source_boundary_fingerprint
        || actual.actual_fingerprint.as_deref()
            != Some(request.source_boundary_fingerprint.as_str())
        || !actual.fingerprint_verified
        || !actual.verified
    {
        return Err(result_error(
            "native suffix source boundary is unverified or detached",
        ));
    }
    Ok(())
}

fn validate_checkpoint(
    actual: &NativeCheckpointValidationResult,
    request: &NativeSuffixBatch,
) -> Result<(), NativeSuffixResultError> {
    if actual.kind != request.checkpoint_validation.kind
        || actual.ticks != request.checkpoint_validation.ticks as u64
        || !actual.verified
        || actual.first_divergence_tick.is_some()
        || actual.fresh_sequence_digest.as_deref() != actual.restored_sequence_digest.as_deref()
        || !actual
            .fresh_sequence_digest
            .as_deref()
            .is_some_and(|digest| lower_hex(digest, 32))
        || !actual
            .source_semantic_digest
            .as_deref()
            .is_some_and(|digest| lower_hex(digest, 32))
    {
        return Err(result_error(
            "native suffix checkpoint replay validation is incomplete or divergent",
        ));
    }
    Ok(())
}

fn validate_episode_shard(
    shard: &NativeEpisodeShardResult,
    candidate_count: u64,
) -> Result<(), NativeSuffixResultError> {
    if shard.schema != NATIVE_EPISODE_SHARD_SCHEMA_V2
        || shard.path.is_empty()
        || shard.observation_schema.is_empty()
        || shard.action_schema != RAW_PAD_ACTION_SCHEMA_V2
        || shard.episode_count != candidate_count
        || shard.uncompressed_bytes == 0
        || shard.compressed_bytes == 0
    {
        return Err(result_error(
            "native suffix episode shard is incomplete or misaligned",
        ));
    }
    Ok(())
}

fn validate_predicate(
    evidence: &NativePredicateEvidence,
    terminal: &NativeTerminalBinding,
    reached: bool,
) -> Result<(), NativeSuffixResultError> {
    let program = terminal.program_sha256.to_string();
    let definition = terminal.definition_sha256.to_string();
    let matches = evidence
        .milestones
        .iter()
        .filter(|milestone| milestone.id == terminal.goal)
        .collect::<Vec<_>>();
    if evidence.schema.name != "dusklight.automation.milestones"
        || evidence.schema.version != 5
        || evidence.boot.kind != "process"
        || !evidence.boot_origin_established
        || evidence.goal != terminal.goal
        || evidence.goal_reached != reached
        || evidence.program_digest.as_deref() != Some(program.as_str())
        || matches.len() != 1
        || matches[0].hit != reached
        || matches[0].phase.as_deref() != Some("post_sim")
        || matches[0].definition_digest.as_deref() != Some(definition.as_str())
        || matches[0].program_digest.as_deref() != Some(program.as_str())
    {
        return Err(result_error(
            "native suffix authored terminal evidence is incomplete or detached",
        ));
    }
    Ok(())
}

fn lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeSuffixResultError(String);

fn result_error(message: impl Into<String>) -> NativeSuffixResultError {
    NativeSuffixResultError(message.into())
}

impl fmt::Display for NativeSuffixResultError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeSuffixResultError {}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_search::search::MacroAction;
    use dusklight_search::suffix_batch::{NativeCheckpointValidation, NativeSuffixCandidate};

    fn request(verify_state_hashes: bool) -> NativeSuffixBatch {
        NativeSuffixBatch {
            schema: NATIVE_SUFFIX_BATCH_SCHEMA.into(),
            source_frame: 500,
            source_boundary_fingerprint: "1".repeat(32),
            checkpoint_validation: NativeCheckpointValidation {
                kind: "recorded_replay_window".into(),
                ticks: 2,
            },
            maximum_ticks: 2,
            verify_state_hashes,
            candidates: vec![NativeSuffixCandidate {
                id: "candidate-0".into(),
                actions: vec![MacroAction::Neutral { frames: 2 }],
            }],
        }
    }

    fn terminal() -> NativeTerminalBinding {
        NativeTerminalBinding {
            goal: "goal".into(),
            program_sha256: Digest([2; 32]),
            definition_sha256: Digest([3; 32]),
        }
    }

    fn result(success: bool, verify_state_hashes: bool) -> NativeSuffixBatchResult {
        let terminal = terminal();
        let first_hit_tick = success.then_some(0);
        let ticks = first_hit_tick.map_or(2, |tick| tick + 1);
        NativeSuffixBatchResult {
            schema: NATIVE_SUFFIX_BATCH_RESULT_SCHEMA_V4.into(),
            status: "passed".into(),
            source_frame: 500,
            source_boundary: NativeSourceBoundaryResult {
                milestone: None,
                expected_fingerprint: "1".repeat(32),
                actual_fingerprint: Some("1".repeat(32)),
                fingerprint_verified: true,
                verified: true,
            },
            checkpoint_validation: NativeCheckpointValidationResult {
                kind: "recorded_replay_window".into(),
                ticks: 2,
                verified: true,
                source_semantic_digest: Some("4".repeat(32)),
                fresh_sequence_digest: Some("5".repeat(32)),
                restored_sequence_digest: Some("5".repeat(32)),
                first_divergence_tick: None,
                expected_tick_digest: None,
                actual_tick_digest: None,
            },
            maximum_ticks: 2,
            candidate_count: 1,
            completed_candidates: 1,
            verify_state_hashes,
            policy_model: None,
            checkpoint_bytes: 128,
            restore_identity: Some("6".repeat(32)),
            capture_micros: 1,
            restore_micros: vec![1],
            timing: NativeSuffixTimingResult {
                schema: "dusklight-suffix-batch-timing/v1".into(),
                batch_wall_micros: 1,
                candidate_ticks: ticks,
                verified: true,
                accounting: Value::Object(Default::default()),
                phases: Value::Object(Default::default()),
            },
            audio_callback_quiesced: true,
            episode_shard: NativeEpisodeShardResult {
                schema: NATIVE_EPISODE_SHARD_SCHEMA_V2.into(),
                path: "result.json.episodes.dseps".into(),
                observation_schema: "dusklight-learning-observation/v27".into(),
                action_schema: RAW_PAD_ACTION_SCHEMA_V2.into(),
                episode_count: 1,
                uncompressed_bytes: 10,
                compressed_bytes: 5,
            },
            winner_id: success.then(|| "candidate-0".into()),
            candidates: vec![NativeSuffixCandidateResult {
                id: "candidate-0".into(),
                success,
                ticks_executed: ticks,
                first_hit_tick,
                state_sequence_digest: verify_state_hashes.then(|| "7".repeat(32)),
                state_tick_digests: verify_state_hashes
                    .then(|| vec!["8".repeat(32); ticks as usize]),
                predicate_evidence: NativePredicateEvidence {
                    schema: NativeMilestoneSchema {
                        name: "dusklight.automation.milestones".into(),
                        version: 5,
                    },
                    boot: NativeBootEvidence {
                        kind: "process".into(),
                    },
                    boot_origin_established: true,
                    goal: terminal.goal.clone(),
                    goal_reached: success,
                    program_digest: Some(terminal.program_sha256.to_string()),
                    milestones: vec![NativeMilestoneEvidence {
                        id: terminal.goal,
                        hit: success,
                        sim_tick: success.then_some(501),
                        tape_frame: success.then_some(500),
                        boundary_index: success.then_some(501),
                        phase: Some("post_sim".into()),
                        stable_ticks: Some(1),
                        definition_digest: Some(terminal.definition_sha256.to_string()),
                        program_digest: Some(terminal.program_sha256.to_string()),
                        evidence: None,
                        projections: None,
                    }],
                },
                consumed_pad_states: success.then(|| vec![RawPadState::default(); ticks as usize]),
                terminal_observation: Value::Object(Default::default()),
            }],
            error: None,
        }
    }

    #[test]
    fn accepts_exact_miss_and_success_evidence() {
        let miss = result(false, true)
            .validate_against(&request(true), &terminal())
            .unwrap();
        assert_eq!(miss.simulated_ticks, 2);
        assert_eq!(miss.candidates[0].first_hit_tick, None);

        let success = result(true, false)
            .validate_against(&request(false), &terminal())
            .unwrap();
        assert_eq!(success.simulated_ticks, 1);
        assert_eq!(success.candidates[0].first_hit_tick, Some(1));
    }

    #[test]
    fn rejects_boundary_checkpoint_terminal_and_tick_tampering() {
        let batch = request(true);
        let authority = terminal();

        let mut tampered = result(false, true);
        tampered.source_boundary.actual_fingerprint = Some("9".repeat(32));
        assert!(tampered.validate_against(&batch, &authority).is_err());

        let mut tampered = result(false, true);
        tampered.checkpoint_validation.restored_sequence_digest = Some("9".repeat(32));
        assert!(tampered.validate_against(&batch, &authority).is_err());

        let mut tampered = result(false, true);
        tampered.candidates[0].predicate_evidence.milestones[0].definition_digest =
            Some("9".repeat(64));
        assert!(tampered.validate_against(&batch, &authority).is_err());

        let mut tampered = result(false, true);
        tampered.timing.candidate_ticks = 1;
        assert!(tampered.validate_against(&batch, &authority).is_err());
    }

    #[test]
    fn serde_contract_rejects_unknown_native_fields() {
        let mut value = serde_json::to_value(result(false, false)).unwrap();
        value["unreviewed_authority"] = Value::Bool(true);
        assert!(serde_json::from_value::<NativeSuffixBatchResult>(value).is_err());
    }
}
