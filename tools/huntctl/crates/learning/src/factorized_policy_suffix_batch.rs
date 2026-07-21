//! Bounded host envelope for executing continuous factorized policy rows.
//!
//! A learner can emit a set of continuous policy-head rows without knowing the
//! native suffix-batch wire shape. This module validates every row with the
//! shared Rust decoder, proves that each candidate expands to the requested
//! frame horizon, and materializes the exact v4 envelope accepted by the native
//! input boundary.

use crate::factorized_pad_action::{
    FACTORIZED_PAD_POLICY_HEAD_SCHEMA_V1, FACTORIZED_PAD_POLICY_HEAD_WIDTH, FactorizedPadPolicyHead,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const FACTORIZED_POLICY_OUTPUT_SET_SCHEMA_V1: &str =
    "dusklight-factorized-policy-output-set/v1";
pub const NATIVE_FACTORIZED_SUFFIX_BATCH_SCHEMA_V4: &str = "dusklight-suffix-batch/v4";
pub const NATIVE_RECORDED_REPLAY_WINDOW_KIND: &str = "recorded_replay_window";
pub const MAX_NATIVE_FACTORIZED_CANDIDATES: usize = 16_384;
pub const MAX_NATIVE_FACTORIZED_TICKS: usize = 4_096;
pub const MAX_NATIVE_FACTORIZED_EXPANDED_TICKS: usize = 8 * 1_024 * 1_024;
pub const MAX_NATIVE_CHECKPOINT_VALIDATION_TICKS: usize = 256;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FactorizedPolicyOutputSet {
    pub schema: String,
    pub policy_head: NativeFactorizedPolicyHead,
    pub candidates: Vec<FactorizedPolicyOutputCandidate>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FactorizedPolicyOutputCandidate {
    pub id: String,
    pub policy_outputs: Vec<Vec<f32>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeFactorizedPolicyHead {
    pub schema: String,
    pub maximum_duration_ticks: u32,
    pub button_logit_threshold: f32,
}

impl NativeFactorizedPolicyHead {
    fn decoder(&self) -> Result<FactorizedPadPolicyHead, FactorizedPolicyBatchError> {
        if self.schema != FACTORIZED_PAD_POLICY_HEAD_SCHEMA_V1 {
            return Err(FactorizedPolicyBatchError::new(format!(
                "policy head schema must be {FACTORIZED_PAD_POLICY_HEAD_SCHEMA_V1}"
            )));
        }
        let decoder = FactorizedPadPolicyHead {
            maximum_duration_ticks: self.maximum_duration_ticks,
            button_logit_threshold: self.button_logit_threshold,
        };
        decoder
            .validate()
            .map_err(|_| FactorizedPolicyBatchError::new("policy head configuration is invalid"))?;
        Ok(decoder)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeFactorizedPolicyBatchConfig {
    pub source_frame: usize,
    pub source_boundary_fingerprint: String,
    pub checkpoint_validation_ticks: usize,
    pub maximum_ticks: usize,
    pub verify_state_hashes: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeFactorizedPolicySuffixBatch {
    pub schema: String,
    pub source_frame: usize,
    pub source_boundary_fingerprint: String,
    pub checkpoint_validation: NativeCheckpointValidation,
    pub maximum_ticks: usize,
    pub verify_state_hashes: bool,
    pub candidates: Vec<NativeFactorizedPolicyCandidate>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCheckpointValidation {
    pub kind: String,
    pub ticks: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeFactorizedPolicyCandidate {
    pub id: String,
    pub policy_head: NativeFactorizedPolicyHead,
    pub policy_outputs: Vec<[f32; FACTORIZED_PAD_POLICY_HEAD_WIDTH]>,
}

impl NativeFactorizedPolicySuffixBatch {
    pub fn build(
        output_set: FactorizedPolicyOutputSet,
        config: NativeFactorizedPolicyBatchConfig,
    ) -> Result<Self, FactorizedPolicyBatchError> {
        if output_set.schema != FACTORIZED_POLICY_OUTPUT_SET_SCHEMA_V1 {
            return Err(FactorizedPolicyBatchError::new(format!(
                "policy output set schema must be {FACTORIZED_POLICY_OUTPUT_SET_SCHEMA_V1}"
            )));
        }
        if !valid_boundary_fingerprint(&config.source_boundary_fingerprint) {
            return Err(FactorizedPolicyBatchError::new(
                "source boundary fingerprint must be 32 lowercase hexadecimal characters",
            ));
        }
        if config.maximum_ticks == 0 || config.maximum_ticks > MAX_NATIVE_FACTORIZED_TICKS {
            return Err(FactorizedPolicyBatchError::new(format!(
                "maximum ticks must be between 1 and {MAX_NATIVE_FACTORIZED_TICKS}"
            )));
        }
        if config.checkpoint_validation_ticks == 0
            || config.checkpoint_validation_ticks > MAX_NATIVE_CHECKPOINT_VALIDATION_TICKS
        {
            return Err(FactorizedPolicyBatchError::new(format!(
                "checkpoint validation ticks must be between 1 and {MAX_NATIVE_CHECKPOINT_VALIDATION_TICKS}"
            )));
        }
        if output_set.candidates.is_empty()
            || output_set.candidates.len() > MAX_NATIVE_FACTORIZED_CANDIDATES
            || output_set.candidates.len()
                > MAX_NATIVE_FACTORIZED_EXPANDED_TICKS / config.maximum_ticks
        {
            return Err(FactorizedPolicyBatchError::new(
                "candidate count or expanded tick count exceeds the native bound",
            ));
        }

        let decoder = output_set.policy_head.decoder()?;
        let mut ids = BTreeSet::new();
        let mut candidates = Vec::with_capacity(output_set.candidates.len());
        for (candidate_index, candidate) in output_set.candidates.into_iter().enumerate() {
            if !valid_candidate_id(&candidate.id) {
                return Err(FactorizedPolicyBatchError::new(format!(
                    "candidate {candidate_index} id must contain 1 through 128 printable non-whitespace ASCII bytes"
                )));
            }
            if !ids.insert(candidate.id.clone()) {
                return Err(FactorizedPolicyBatchError::new(format!(
                    "candidate {candidate_index} has duplicate id {:?}",
                    candidate.id
                )));
            }
            if candidate.policy_outputs.is_empty()
                || candidate.policy_outputs.len() > config.maximum_ticks
            {
                return Err(FactorizedPolicyBatchError::new(format!(
                    "candidate {candidate_index} output row count is empty or exceeds maximum ticks"
                )));
            }

            let mut expanded_ticks = 0_usize;
            let mut policy_outputs = Vec::with_capacity(candidate.policy_outputs.len());
            for (row_index, row) in candidate.policy_outputs.into_iter().enumerate() {
                let row: [f32; FACTORIZED_PAD_POLICY_HEAD_WIDTH] = row.try_into().map_err(|row: Vec<f32>| {
                    FactorizedPolicyBatchError::new(format!(
                        "candidate {candidate_index} policy output {row_index} has width {} instead of {FACTORIZED_PAD_POLICY_HEAD_WIDTH}",
                        row.len()
                    ))
                })?;
                let decision = decoder.decode(&row).map_err(|_| {
                    FactorizedPolicyBatchError::new(format!(
                        "candidate {candidate_index} policy output {row_index} is nonfinite or invalid"
                    ))
                })?;
                expanded_ticks = expanded_ticks
                    .checked_add(decision.duration_ticks as usize)
                    .filter(|ticks| *ticks <= config.maximum_ticks)
                    .ok_or_else(|| {
                        FactorizedPolicyBatchError::new(format!(
                            "candidate {candidate_index} policy output {row_index} exceeds maximum ticks"
                        ))
                    })?;
                policy_outputs.push(row);
            }
            if expanded_ticks != config.maximum_ticks {
                return Err(FactorizedPolicyBatchError::new(format!(
                    "candidate {candidate_index} policy outputs expand to {expanded_ticks} ticks instead of {}",
                    config.maximum_ticks
                )));
            }
            candidates.push(NativeFactorizedPolicyCandidate {
                id: candidate.id,
                policy_head: output_set.policy_head.clone(),
                policy_outputs,
            });
        }

        Ok(Self {
            schema: NATIVE_FACTORIZED_SUFFIX_BATCH_SCHEMA_V4.into(),
            source_frame: config.source_frame,
            source_boundary_fingerprint: config.source_boundary_fingerprint,
            checkpoint_validation: NativeCheckpointValidation {
                kind: NATIVE_RECORDED_REPLAY_WINDOW_KIND.into(),
                ticks: config.checkpoint_validation_ticks,
            },
            maximum_ticks: config.maximum_ticks,
            verify_state_hashes: config.verify_state_hashes,
            candidates,
        })
    }
}

fn valid_boundary_fingerprint(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_candidate_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FactorizedPolicyBatchError(String);

impl FactorizedPolicyBatchError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for FactorizedPolicyBatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for FactorizedPolicyBatchError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(duration: f32) -> Vec<f32> {
        let mut row = vec![0.0; FACTORIZED_PAD_POLICY_HEAD_WIDTH];
        row[24] = duration;
        row
    }

    fn output_set() -> FactorizedPolicyOutputSet {
        FactorizedPolicyOutputSet {
            schema: FACTORIZED_POLICY_OUTPUT_SET_SCHEMA_V1.into(),
            policy_head: NativeFactorizedPolicyHead {
                schema: FACTORIZED_PAD_POLICY_HEAD_SCHEMA_V1.into(),
                maximum_duration_ticks: 2,
                button_logit_threshold: 0.0,
            },
            candidates: vec![FactorizedPolicyOutputCandidate {
                id: "factorized-online".into(),
                policy_outputs: vec![row(0.0), row(1.0)],
            }],
        }
    }

    fn config() -> NativeFactorizedPolicyBatchConfig {
        NativeFactorizedPolicyBatchConfig {
            source_frame: 500,
            source_boundary_fingerprint: "1f849e432274771426236d60fbf7d72f".into(),
            checkpoint_validation_ticks: 2,
            maximum_ticks: 3,
            verify_state_hashes: false,
        }
    }

    #[test]
    fn builds_exact_native_v4_envelope_after_duration_proof() {
        let batch = NativeFactorizedPolicySuffixBatch::build(output_set(), config()).unwrap();
        assert_eq!(batch.schema, NATIVE_FACTORIZED_SUFFIX_BATCH_SCHEMA_V4);
        assert_eq!(batch.candidates.len(), 1);
        assert_eq!(batch.candidates[0].policy_outputs.len(), 2);
        let encoded = serde_json::to_value(&batch).unwrap();
        assert_eq!(encoded["candidates"][0]["policy_outputs"][1][24], 1.0);
        assert!(encoded["candidates"][0].get("actions").is_none());
        assert!(encoded["candidates"][0].get("source").is_none());
    }

    #[test]
    fn rejects_detached_width_duration_identity_and_nonfinite_rows() {
        let mut wrong_width = output_set();
        wrong_width.candidates[0].policy_outputs[0].pop();
        assert!(NativeFactorizedPolicySuffixBatch::build(wrong_width, config()).is_err());

        let mut wrong_duration = output_set();
        wrong_duration.candidates[0].policy_outputs[1][24] = 0.0;
        assert!(NativeFactorizedPolicySuffixBatch::build(wrong_duration, config()).is_err());

        let mut wrong_identity = config();
        wrong_identity
            .source_boundary_fingerprint
            .make_ascii_uppercase();
        assert!(NativeFactorizedPolicySuffixBatch::build(output_set(), wrong_identity).is_err());

        let mut nonfinite = output_set();
        nonfinite.candidates[0].policy_outputs[0][0] = f32::NAN;
        assert!(NativeFactorizedPolicySuffixBatch::build(nonfinite, config()).is_err());
    }
}
