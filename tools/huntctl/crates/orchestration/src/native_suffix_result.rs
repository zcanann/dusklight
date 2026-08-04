//! Fail-closed validation of native checkpoint suffix-batch results.

use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::RawPadState;
use dusklight_learning::frozen_inference::FrozenInferenceModel;
use dusklight_learning::native_frozen_policy_suffix_batch::{
    NATIVE_FROZEN_POLICY_SCHEMA_V1, NATIVE_FROZEN_POLICY_SCHEMA_V2,
    NATIVE_FROZEN_POLICY_SUFFIX_BATCH_SCHEMA_V7, NativeFrozenPolicySuffixBatch,
    NativePolicyActionAuthority,
};
use dusklight_search::suffix_batch::{
    NATIVE_CACHED_SUFFIX_BATCH_SCHEMA, NATIVE_REACTIVE_SUFFIX_BATCH_SCHEMA,
    NATIVE_SUFFIX_BATCH_SCHEMA, NATIVE_VARIABLE_CACHED_SUFFIX_BATCH_SCHEMA, NativeSuffixBatch,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const NATIVE_SUFFIX_BATCH_RESULT_SCHEMA_V6: &str = "dusklight-suffix-batch-result/v6";
pub const NATIVE_SUFFIX_BATCH_RESULT_SCHEMA_V7: &str = "dusklight-suffix-batch-result/v7";
pub const NATIVE_SUFFIX_BATCH_RESULT_SCHEMA_V8: &str = "dusklight-suffix-batch-result/v8";
pub const NATIVE_SUFFIX_BATCH_RESULT_SCHEMA_V9: &str = "dusklight-suffix-batch-result/v9";
pub const NATIVE_EPISODE_SHARD_SCHEMA_V2: &str = "dusklight-native-episode-shard/v2";
pub const NATIVE_EPISODE_SHARD_SCHEMA_V3: &str = "dusklight-native-episode-shard/v3";
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_cache: Option<NativeCheckpointCacheResult>,
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCheckpointCacheResult {
    pub source_kind: String,
    pub source_identity: Option<String>,
    pub source_route_ticks: u64,
    pub capacity_bytes: u64,
    pub capacity_entries: u64,
    pub resident_bytes: u64,
    pub resident_checkpoint_bytes: u64,
    pub resident_host_snapshot_bytes: u64,
    pub resident_entries: u64,
    pub insertions: u64,
    pub replacements: u64,
    pub evictions: u64,
    pub hits: u64,
    pub misses: u64,
    pub source_pinned: bool,
    pub batch_capture_attempts: u64,
    pub batch_capture_successes: u64,
    pub batch_capture_micros: u64,
    #[serde(default)]
    pub checkpoint_image_reuse_enabled: bool,
    #[serde(default)]
    pub batch_image_reuse_attempts: u64,
    #[serde(default)]
    pub batch_image_reuse_successes: u64,
    pub live_endpoint_capacity_entries: u64,
    pub live_endpoint_resident_entries: u64,
    pub live_endpoint_resident_host_snapshot_bytes: u64,
    pub batch_live_retention_attempts: u64,
    pub batch_live_retention_successes: u64,
    pub batch_live_retention_nanos: u64,
    pub batch_live_consumptions: u64,
    pub batch_live_invalidations: u64,
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
    #[serde(default)]
    pub headless_audit: Value,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_state_entry_digests: Option<Vec<NativeStateCheckpointEntryDigestResult>>,
    pub terminal_boundary_fingerprint: String,
    pub predicate_evidence: NativePredicateEvidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consumed_pad_states: Option<Vec<RawPadState>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retained_checkpoint: Option<NativeRetainedCheckpointResult>,
    pub terminal_observation: Value,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeStateCheckpointEntryDigestResult {
    pub name: String,
    pub kind: String,
    pub bytes: u64,
    pub digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeRetainedCheckpointResult {
    pub storage_kind: String,
    pub restore_identity: String,
    pub image_digest: Option<String>,
    pub semantic_digest: Option<String>,
    pub checkpoint_bytes: u64,
    pub host_snapshot_bytes: u64,
    #[serde(default)]
    pub machine_capture_micros: u64,
    #[serde(default)]
    pub host_snapshot_capture_nanos: u64,
    pub capture_micros: u64,
    pub route_ticks: u64,
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
    pub terminal_boundary_fingerprint: String,
    pub behavior_sha256: Digest,
    pub retained_checkpoint: Option<NativeRetainedCheckpointResult>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedNativeSuffixBatch {
    pub restore_identity: String,
    pub checkpoint_bytes: u64,
    pub simulated_ticks: u64,
    pub restore_micros: Vec<u64>,
    pub timing: ValidatedNativeSuffixTiming,
    pub checkpoint_cache: Option<NativeCheckpointCacheResult>,
    pub episode_shard_path: String,
    pub candidates: Vec<ValidatedNativeSuffixCandidate>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValidatedNativeSuffixTiming {
    pub batch_wall_micros: u64,
    pub simulation_micros: u64,
    pub observation_capture_micros: u64,
    pub corpus_encoding_micros: u64,
}

impl NativeSuffixBatchResult {
    pub fn validate_against(
        &self,
        request: &NativeSuffixBatch,
        terminal: &NativeTerminalBinding,
    ) -> Result<ValidatedNativeSuffixBatch, NativeSuffixResultError> {
        if !matches!(
            request.schema.as_str(),
            NATIVE_SUFFIX_BATCH_SCHEMA
                | NATIVE_REACTIVE_SUFFIX_BATCH_SCHEMA
                | NATIVE_CACHED_SUFFIX_BATCH_SCHEMA
                | NATIVE_VARIABLE_CACHED_SUFFIX_BATCH_SCHEMA
        ) {
            return Err(result_error(
                "unsupported residual suffix-batch request schema",
            ));
        }
        let cached = matches!(
            request.schema.as_str(),
            NATIVE_CACHED_SUFFIX_BATCH_SCHEMA | NATIVE_VARIABLE_CACHED_SUFFIX_BATCH_SCHEMA
        );
        let expected_result_schema = if cached {
            NATIVE_SUFFIX_BATCH_RESULT_SCHEMA_V9
        } else {
            NATIVE_SUFFIX_BATCH_RESULT_SCHEMA_V6
        };
        let candidate_count = request.candidates.len() as u64;
        if self.schema != expected_result_schema
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
        if request
            .checkpoint_cache
            .as_ref()
            .and_then(|cache| cache.source_identity.as_deref())
            .is_some_and(|expected| expected != restore_identity)
        {
            return Err(result_error(
                "native suffix result restored a different process-local checkpoint",
            ));
        }
        validate_checkpoint_cache(self.checkpoint_cache.as_ref(), request)?;
        validate_source_boundary(&self.source_boundary, request)?;
        validate_checkpoint(&self.checkpoint_validation, request)?;
        validate_episode_shard(
            &self.episode_shard,
            candidate_count,
            NATIVE_EPISODE_SHARD_SCHEMA_V2,
        )?;

        let mut ids = BTreeSet::new();
        let mut simulated_ticks = 0_u64;
        let mut candidates = Vec::with_capacity(self.candidates.len());
        for (candidate_index, (expected, actual)) in
            request.candidates.iter().zip(&self.candidates).enumerate()
        {
            if expected.id != actual.id || !ids.insert(actual.id.as_str()) {
                return Err(result_error(
                    "native suffix result candidates are reordered, duplicated, or detached",
                ));
            }
            let validated = actual.validate_common(
                expected.maximum_ticks.unwrap_or(request.maximum_ticks),
                request.verify_state_hashes,
                expected.controller_program_hex.is_some() || expected.cancellation_guard.is_some(),
                terminal,
            )?;
            validate_retained_checkpoint(actual, request, candidate_index, self.checkpoint_bytes)?;
            candidates.push(validated);
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
            // The native runner retains the first candidate at the best
            // first-hit tick. Preserve request order for ties instead of
            // imposing a second, detached lexical ordering on candidate IDs.
            .min_by_key(|candidate| candidate.first_hit_tick)
            .map(|candidate| candidate.id.as_str());
        if self.winner_id.as_deref() != winner {
            return Err(result_error(
                "native suffix winner does not match the exact successful candidates",
            ));
        }
        let timing = validate_timing(&self.timing, &self.restore_micros)?;
        Ok(ValidatedNativeSuffixBatch {
            restore_identity: restore_identity.into(),
            checkpoint_bytes: self.checkpoint_bytes,
            simulated_ticks,
            restore_micros: self.restore_micros.clone(),
            timing,
            checkpoint_cache: self.checkpoint_cache.clone(),
            episode_shard_path: self.episode_shard.path.clone(),
            candidates,
        })
    }

    pub fn validate_frozen_against(
        &self,
        request: &NativeFrozenPolicySuffixBatch,
        model_bytes: &[u8],
        terminal: &NativeTerminalBinding,
    ) -> Result<ValidatedNativeSuffixBatch, NativeSuffixResultError> {
        request
            .validate(model_bytes)
            .map_err(|error| result_error(error.to_string()))?;
        let model = FrozenInferenceModel::from_bytes(model_bytes)
            .map_err(|error| result_error(error.to_string()))?;
        if model.objective_sha256 != terminal.definition_sha256 {
            return Err(result_error(
                "frozen policy objective differs from the authored terminal definition",
            ));
        }
        let candidate_count = request.candidates.len() as u64;
        let parameter_count = model.layers.iter().try_fold(0_u64, |count, layer| {
            count
                .checked_add(layer.weights.len() as u64)
                .and_then(|value| value.checked_add(layer.biases.len() as u64))
        });
        let policy = self
            .policy_model
            .as_ref()
            .ok_or_else(|| result_error("native frozen policy result lacks its model identity"))?;
        let exploratory = request.schema == NATIVE_FROZEN_POLICY_SUFFIX_BATCH_SCHEMA_V7;
        let expected_result_schema = if exploratory {
            NATIVE_SUFFIX_BATCH_RESULT_SCHEMA_V7
        } else {
            NATIVE_SUFFIX_BATCH_RESULT_SCHEMA_V6
        };
        let expected_policy_schema = if exploratory {
            NATIVE_FROZEN_POLICY_SCHEMA_V2
        } else {
            NATIVE_FROZEN_POLICY_SCHEMA_V1
        };
        let expected_exploration = request
            .frozen_policy
            .rollout_exploration
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|error| result_error(error.to_string()))?
            .unwrap_or(Value::Null);
        if self.schema != expected_result_schema
            || self.status != "passed"
            || self.error.is_some()
            || self.source_frame != request.source_frame as u64
            || self.maximum_ticks != request.maximum_ticks as u64
            || self.candidate_count != candidate_count
            || self.completed_candidates != candidate_count
            || self.candidates.len() != request.candidates.len()
            || self.verify_state_hashes != request.verify_state_hashes
            || self.checkpoint_bytes == 0
            || self.capture_micros == 0
            || self.restore_micros.is_empty()
            || !self.audio_callback_quiesced
            || !self.timing.verified
            || self.timing.schema != "dusklight-suffix-batch-timing/v2"
            || policy.get("schema").and_then(Value::as_str) != Some(expected_policy_schema)
            || request.action_authority != NativePolicyActionAuthority::EpisodePolicy
            || policy.get("action_authority").and_then(Value::as_str) != Some("episode_policy")
            || policy.get("fallback_ticks").and_then(Value::as_u64) != Some(0)
            || policy.get("model_xxh3_128").and_then(Value::as_str)
                != Some(request.frozen_policy.model_xxh3_128.as_str())
            || policy.get("feature_schema_sha256").and_then(Value::as_str)
                != Some(model.feature_schema_sha256.to_string().as_str())
            || policy.get("action_schema_sha256").and_then(Value::as_str)
                != Some(model.action_schema_sha256.to_string().as_str())
            || policy.get("objective_sha256").and_then(Value::as_str)
                != Some(model.objective_sha256.to_string().as_str())
            || policy.get("parameter_count").and_then(Value::as_u64) != parameter_count
            || if exploratory {
                policy.get("rollout_exploration") != Some(&expected_exploration)
            } else {
                policy
                    .get("rollout_exploration")
                    .is_some_and(|value| !value.is_null())
            }
        {
            return Err(result_error(
                "native frozen policy result is incomplete or detached from its request and model",
            ));
        }
        let restore_identity = self
            .restore_identity
            .as_deref()
            .filter(|value| lower_hex(value, 32))
            .ok_or_else(|| {
                result_error("native frozen policy result lacks a checkpoint identity")
            })?;
        validate_source_boundary_values(
            &self.source_boundary,
            &request.source_boundary_fingerprint,
        )?;
        validate_checkpoint_values(
            &self.checkpoint_validation,
            &request.checkpoint_validation.kind,
            request.checkpoint_validation.ticks as u64,
        )?;
        validate_episode_shard(
            &self.episode_shard,
            candidate_count,
            NATIVE_EPISODE_SHARD_SCHEMA_V3,
        )?;

        let mut ids = BTreeSet::new();
        let mut simulated_ticks = 0_u64;
        let mut candidates = Vec::with_capacity(self.candidates.len());
        for (expected, actual) in request.candidates.iter().zip(&self.candidates) {
            if expected.id != actual.id || !ids.insert(actual.id.as_str()) {
                return Err(result_error(
                    "native frozen policy result candidates are reordered, duplicated, or detached",
                ));
            }
            candidates.push(actual.validate_common(
                request.maximum_ticks,
                request.verify_state_hashes,
                false,
                terminal,
            )?);
            simulated_ticks = simulated_ticks
                .checked_add(actual.ticks_executed)
                .ok_or_else(|| result_error("native frozen policy tick total overflowed"))?;
        }
        if self.timing.candidate_ticks != simulated_ticks {
            return Err(result_error(
                "native frozen policy timing does not charge every simulated tick",
            ));
        }
        if policy
            .get("policy_controlled_ticks")
            .and_then(Value::as_u64)
            != Some(simulated_ticks)
        {
            return Err(result_error(
                "native frozen policy did not control every executed episode tick",
            ));
        }
        let winner = self
            .candidates
            .iter()
            .filter(|candidate| candidate.success)
            .min_by_key(|candidate| candidate.first_hit_tick)
            .map(|candidate| candidate.id.as_str());
        if self.winner_id.as_deref() != winner {
            return Err(result_error(
                "native frozen policy winner differs from the exact terminal results",
            ));
        }
        let timing = validate_timing(&self.timing, &self.restore_micros)?;
        Ok(ValidatedNativeSuffixBatch {
            restore_identity: restore_identity.into(),
            checkpoint_bytes: self.checkpoint_bytes,
            simulated_ticks,
            restore_micros: self.restore_micros.clone(),
            timing,
            checkpoint_cache: None,
            episode_shard_path: self.episode_shard.path.clone(),
            candidates,
        })
    }
}

fn validate_timing(
    timing: &NativeSuffixTimingResult,
    restore_micros: &[u64],
) -> Result<ValidatedNativeSuffixTiming, NativeSuffixResultError> {
    let phase_micros = |phase: &'static str| {
        timing
            .phases
            .get(phase)
            .filter(|value| value.get("status").and_then(Value::as_str) == Some("measured"))
            .and_then(|value| value.get("micros"))
            .and_then(Value::as_u64)
            .ok_or_else(|| result_error(format!("native suffix timing phase {phase} is absent")))
    };
    let checkpoint_restore_micros = phase_micros("checkpoint_restore")?;
    let expected_restore_micros = restore_micros
        .iter()
        .try_fold(0_u64, |total, value| total.checked_add(*value))
        .ok_or_else(|| result_error("native suffix restore timing overflows"))?;
    let validated = ValidatedNativeSuffixTiming {
        batch_wall_micros: timing.batch_wall_micros,
        simulation_micros: phase_micros("simulation")?,
        observation_capture_micros: phase_micros("observation_capture")?,
        corpus_encoding_micros: phase_micros("corpus_encoding")?,
    };
    if validated.batch_wall_micros == 0
        || checkpoint_restore_micros != expected_restore_micros
        || validated.simulation_micros > validated.batch_wall_micros
        || validated.observation_capture_micros > validated.batch_wall_micros
        || validated.corpus_encoding_micros > validated.batch_wall_micros
    {
        return Err(result_error(
            "native suffix timing phases are internally detached",
        ));
    }
    Ok(validated)
}

impl NativeSuffixCandidateResult {
    fn validate_common(
        &self,
        maximum_ticks: usize,
        verify_state_hashes: bool,
        early_unsuccessful_terminal_allowed: bool,
        terminal: &NativeTerminalBinding,
    ) -> Result<ValidatedNativeSuffixCandidate, NativeSuffixResultError> {
        let exact_verdict = match (self.success, self.first_hit_tick) {
            (true, Some(tick))
                if tick.checked_add(1) == Some(self.ticks_executed)
                    && self.ticks_executed <= maximum_ticks as u64 =>
            {
                true
            }
            (false, None)
                if self.ticks_executed == maximum_ticks as u64
                    || early_unsuccessful_terminal_allowed
                        && (1..maximum_ticks as u64).contains(&self.ticks_executed) =>
            {
                false
            }
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
            verify_state_hashes,
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
        if let Some(entries) = &self.terminal_state_entry_digests {
            let mut names = BTreeSet::new();
            if !verify_state_hashes
                || entries.is_empty()
                || entries.len() > 1_024
                || entries.iter().any(|entry| {
                    entry.name.is_empty()
                        || !names.insert(entry.name.as_str())
                        || !matches!(entry.kind.as_str(), "memory_region" | "component")
                        || entry.bytes == 0
                        || !lower_hex(&entry.digest, 32)
                })
            {
                return Err(result_error(
                    "native suffix terminal checkpoint-entry digests are invalid",
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
        if !lower_hex(&self.terminal_boundary_fingerprint, 32) {
            return Err(result_error(
                "native suffix candidate terminal boundary fingerprint is invalid",
            ));
        }
        Ok(ValidatedNativeSuffixCandidate {
            id: self.id.clone(),
            simulated_ticks: self.ticks_executed,
            // Route scores and the native wire format both use the zero-based
            // terminal boundary index. `simulated_ticks` separately counts the
            // sampled source-adjacent boundary, so a hit at tick N executes
            // N + 1 samples.
            first_hit_tick: self.first_hit_tick,
            state_sequence_digest: state_sequence_digest.map(str::to_owned),
            terminal_boundary_fingerprint: self.terminal_boundary_fingerprint.clone(),
            behavior_sha256: behavior_digest(self)?,
            retained_checkpoint: self.retained_checkpoint.clone(),
        })
    }
}

fn validate_checkpoint_cache(
    actual: Option<&NativeCheckpointCacheResult>,
    request: &NativeSuffixBatch,
) -> Result<(), NativeSuffixResultError> {
    let Some(expected) = request.checkpoint_cache.as_ref() else {
        if actual.is_some() {
            return Err(result_error(
                "uncached suffix request returned checkpoint-cache authority",
            ));
        }
        return Ok(());
    };
    let actual =
        actual.ok_or_else(|| result_error("cached suffix result lacks cache accounting"))?;
    let expected_source_kind = if expected.source_identity.is_some() {
        if actual.source_kind == "direct_process_local_continuation" {
            "direct_process_local_continuation"
        } else {
            "direct_process_local_restore"
        }
    } else {
        "authenticated_root_restore"
    };
    if actual.source_kind != expected_source_kind
        || actual.source_identity.as_deref() != expected.source_identity.as_deref()
        || actual.source_route_ticks != expected.source_route_ticks as u64
        || actual.capacity_bytes != expected.capacity_bytes as u64
        || actual.capacity_entries != expected.capacity_entries as u64
        || actual.source_pinned
            != (expected.source_identity.is_some()
                && actual.source_kind == "direct_process_local_restore")
        || actual.batch_capture_attempts
            != if expected.retain_candidate_checkpoints {
                request.candidates.len() as u64
            } else if expected.retain_candidate_index.is_some() {
                1
            } else {
                0
            }
        || actual.batch_live_retention_attempts
            != if expected.retain_live_endpoint {
                request.candidates.len() as u64
            } else {
                0
            }
        || actual.batch_capture_successes > actual.batch_capture_attempts
        || actual.batch_image_reuse_successes > actual.batch_image_reuse_attempts
        || (!actual.checkpoint_image_reuse_enabled
            && (actual.batch_image_reuse_attempts != 0 || actual.batch_image_reuse_successes != 0))
        || actual.batch_live_retention_successes > actual.batch_live_retention_attempts
        || (actual.source_kind == "direct_process_local_continuation")
            != (actual.batch_live_consumptions == 1)
        || actual.batch_live_consumptions > 1
        || actual.live_endpoint_capacity_entries != 1
        || actual.live_endpoint_resident_entries > actual.live_endpoint_capacity_entries
        || (actual.live_endpoint_resident_entries == 0)
            != (actual.live_endpoint_resident_host_snapshot_bytes == 0)
        || (expected.retain_live_endpoint
            && (actual.batch_live_retention_successes != actual.batch_live_retention_attempts
                || actual.live_endpoint_resident_entries != 1))
        || actual.resident_entries > actual.capacity_entries
        || actual.resident_bytes > actual.capacity_bytes
        || actual.resident_bytes
            != actual
                .resident_checkpoint_bytes
                .checked_add(actual.resident_host_snapshot_bytes)
                .ok_or_else(|| result_error("native suffix cache resident bytes overflowed"))?
    {
        return Err(result_error(
            "native suffix checkpoint-cache report is incomplete or detached",
        ));
    }
    Ok(())
}

fn validate_retained_checkpoint(
    candidate: &NativeSuffixCandidateResult,
    request: &NativeSuffixBatch,
    candidate_index: usize,
    checkpoint_bytes: u64,
) -> Result<(), NativeSuffixResultError> {
    let Some(cache) = request.checkpoint_cache.as_ref() else {
        if candidate.retained_checkpoint.is_some() {
            return Err(result_error(
                "uncached suffix candidate returned process-local checkpoint authority",
            ));
        }
        return Ok(());
    };
    let portable_retention =
        cache.retain_candidate_checkpoints || cache.retain_candidate_index == Some(candidate_index);
    let retention_expected = portable_retention || cache.retain_live_endpoint;
    if !retention_expected {
        if candidate.retained_checkpoint.is_some() {
            return Err(result_error(
                "suffix candidate retained a checkpoint contrary to its cache request",
            ));
        }
        return Ok(());
    }
    let Some(retained) = candidate.retained_checkpoint.as_ref() else {
        // A bounded cache may legitimately reject a captured image that does
        // not fit while preserving the candidate result.
        return Ok(());
    };
    if !lower_hex(&retained.restore_identity, 32)
        || retained.host_snapshot_bytes == 0
        || retained.route_ticks
            != (cache.source_route_ticks as u64).saturating_add(candidate.ticks_executed)
    {
        return Err(result_error(
            "native retained checkpoint is incomplete or detached from its candidate",
        ));
    }
    if portable_retention {
        if retained.storage_kind != "portable_image"
            || !retained
                .image_digest
                .as_deref()
                .is_some_and(|digest| lower_hex(digest, 32))
            || !retained
                .semantic_digest
                .as_deref()
                .is_some_and(|digest| lower_hex(digest, 32))
            || retained.checkpoint_bytes != checkpoint_bytes
            || retained.capture_micros == 0
        {
            return Err(result_error(
                "native portable checkpoint is incomplete or detached from its candidate",
            ));
        }
    } else if retained.storage_kind != "live_endpoint"
        || retained.image_digest.is_some()
        || retained.semantic_digest.is_some()
        || retained.checkpoint_bytes != 0
        || retained.machine_capture_micros != 0
        || retained.host_snapshot_capture_nanos == 0
    {
        return Err(result_error(
            "native live endpoint is incomplete or detached from its candidate",
        ));
    }
    Ok(())
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
        terminal_boundary_fingerprint: &'a str,
        terminal_observation: &'a Value,
    }
    let value = Behavior {
        success: candidate.success,
        first_hit_tick: candidate.first_hit_tick,
        ticks_executed: candidate.ticks_executed,
        state_sequence_digest: candidate.state_sequence_digest.as_deref(),
        terminal_boundary_fingerprint: &candidate.terminal_boundary_fingerprint,
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
    validate_source_boundary_values(actual, &request.source_boundary_fingerprint)
}

fn validate_source_boundary_values(
    actual: &NativeSourceBoundaryResult,
    expected: &str,
) -> Result<(), NativeSuffixResultError> {
    if actual.expected_fingerprint != expected
        || actual.actual_fingerprint.as_deref() != Some(expected)
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
    validate_checkpoint_values(
        actual,
        &request.checkpoint_validation.kind,
        request.checkpoint_validation.ticks as u64,
    )
}

fn validate_checkpoint_values(
    actual: &NativeCheckpointValidationResult,
    expected_kind: &str,
    expected_ticks: u64,
) -> Result<(), NativeSuffixResultError> {
    if actual.kind != expected_kind
        || actual.ticks != expected_ticks
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
    expected_schema: &str,
) -> Result<(), NativeSuffixResultError> {
    if shard.schema != expected_schema
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
#[path = "native_suffix_result/tests.rs"]
mod tests;
