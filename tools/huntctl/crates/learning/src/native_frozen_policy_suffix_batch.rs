//! Content-bound v5 envelope for native per-tick frozen policy inference.

use crate::artifact::Digest;
use crate::factorized_pad_action::{
    FACTORIZED_PAD_POLICY_HEAD_SCHEMA_V1, FACTORIZED_PAD_POLICY_HEAD_WIDTH, FactorizedPadPolicyHead,
};
use crate::factorized_policy_suffix_batch::{
    MAX_NATIVE_CHECKPOINT_VALIDATION_TICKS, MAX_NATIVE_FACTORIZED_TICKS,
    NATIVE_RECORDED_REPLAY_WINDOW_KIND, NativeCheckpointValidation,
    NativeFactorizedPolicyBatchConfig, NativeFactorizedPolicyHead,
};
use crate::frozen_inference::{FrozenActivation, FrozenDenseLayer, FrozenInferenceModel};
use crate::native_policy_features::{
    NATIVE_POLICY_FEATURE_SCHEMA_SHA256, NATIVE_POLICY_FEATURE_WIDTH,
};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;

pub const NATIVE_FROZEN_POLICY_SCHEMA_V1: &str = "dusklight-native-frozen-policy/v1";
pub const NATIVE_FROZEN_POLICY_SUFFIX_BATCH_SCHEMA_V5: &str = "dusklight-suffix-batch/v5";

/// Build a tiny deterministic policy for exercising the complete native online
/// inference boundary. It drives forward only while a player is present and
/// adds a bounded steering response to the player's current yaw. This is a
/// conformance probe, not a trained or promotion-eligible policy.
pub fn native_frozen_policy_probe_model(
    objective_sha256: Digest,
) -> Result<FrozenInferenceModel, NativeFrozenPolicyBatchError> {
    if objective_sha256 == Digest::ZERO {
        return Err(NativeFrozenPolicyBatchError::new(
            "native frozen policy probe objective identity is invalid",
        ));
    }
    let head = FactorizedPadPolicyHead::default();
    let mut weights = vec![0.0; NATIVE_POLICY_FEATURE_WIDTH * FACTORIZED_PAD_POLICY_HEAD_WIDTH];
    // Output 0 (main-stick X) reacts to normalized current yaw at feature 9.
    weights[9] = 0.25;
    // Output 1 (main-stick Y) is forward only when player-present feature 0 is set.
    weights[NATIVE_POLICY_FEATURE_WIDTH] = 0.5;
    FrozenInferenceModel::new(
        Digest(NATIVE_POLICY_FEATURE_SCHEMA_SHA256),
        Digest(
            head.schema_sha256()
                .map_err(|error| NativeFrozenPolicyBatchError::new(error.to_string()))?,
        ),
        objective_sha256,
        NATIVE_POLICY_FEATURE_WIDTH,
        (0..FACTORIZED_PAD_POLICY_HEAD_WIDTH as u32).collect(),
        vec![FrozenDenseLayer {
            output_width: FACTORIZED_PAD_POLICY_HEAD_WIDTH,
            activation: FrozenActivation::Linear,
            weights,
            biases: vec![0.0; FACTORIZED_PAD_POLICY_HEAD_WIDTH],
        }],
    )
    .map_err(|error| NativeFrozenPolicyBatchError::new(error.to_string()))
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeFrozenPolicySuffixBatch {
    pub schema: String,
    pub source_frame: usize,
    pub source_boundary_fingerprint: String,
    pub checkpoint_validation: NativeCheckpointValidation,
    pub maximum_ticks: usize,
    pub verify_state_hashes: bool,
    pub frozen_policy: NativeFrozenPolicyReference,
    pub candidates: Vec<NativeFrozenPolicyCandidate>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeFrozenPolicyReference {
    pub schema: String,
    pub model_path: String,
    pub model_xxh3_128: String,
    pub policy_head: NativeFactorizedPolicyHead,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeFrozenPolicyCandidate {
    pub id: String,
    pub source: String,
}

impl NativeFrozenPolicySuffixBatch {
    pub fn build(
        model_bytes: &[u8],
        model_path: String,
        expected_objective_sha256: Digest,
        candidate_id: String,
        config: NativeFactorizedPolicyBatchConfig,
    ) -> Result<Self, NativeFrozenPolicyBatchError> {
        if !valid_boundary_fingerprint(&config.source_boundary_fingerprint)
            || config.maximum_ticks == 0
            || config.maximum_ticks > MAX_NATIVE_FACTORIZED_TICKS
            || config.checkpoint_validation_ticks == 0
            || config.checkpoint_validation_ticks > MAX_NATIVE_CHECKPOINT_VALIDATION_TICKS
        {
            return Err(NativeFrozenPolicyBatchError::new(
                "native frozen policy batch boundary, horizon, or validation window is invalid",
            ));
        }
        if model_path.is_empty()
            || model_path.len() > 4096
            || model_path.contains('\0')
            || !valid_candidate_id(&candidate_id)
            || expected_objective_sha256 == Digest::ZERO
        {
            return Err(NativeFrozenPolicyBatchError::new(
                "native frozen policy path, candidate, or objective identity is invalid",
            ));
        }
        let model = FrozenInferenceModel::from_bytes(model_bytes)
            .map_err(|error| NativeFrozenPolicyBatchError::new(error.to_string()))?;
        let head = FactorizedPadPolicyHead::default();
        let action_schema_sha256 = head
            .schema_sha256()
            .map_err(|error| NativeFrozenPolicyBatchError::new(error.to_string()))?;
        if model.input_width != NATIVE_POLICY_FEATURE_WIDTH
            || model.feature_schema_sha256.0 != NATIVE_POLICY_FEATURE_SCHEMA_SHA256
            || model.action_schema_sha256.0 != action_schema_sha256
            || model.objective_sha256 != expected_objective_sha256
            || model.actions != (0..FACTORIZED_PAD_POLICY_HEAD_WIDTH as u32).collect::<Vec<_>>()
        {
            return Err(NativeFrozenPolicyBatchError::new(
                "frozen model is detached from the native feature, action, or objective contract",
            ));
        }
        let model_xxh3_128 = format!("{:032x}", xxhash_rust::xxh3::xxh3_128(model_bytes));
        Ok(Self {
            schema: NATIVE_FROZEN_POLICY_SUFFIX_BATCH_SCHEMA_V5.into(),
            source_frame: config.source_frame,
            source_boundary_fingerprint: config.source_boundary_fingerprint,
            checkpoint_validation: NativeCheckpointValidation {
                kind: NATIVE_RECORDED_REPLAY_WINDOW_KIND.into(),
                ticks: config.checkpoint_validation_ticks,
            },
            maximum_ticks: config.maximum_ticks,
            verify_state_hashes: config.verify_state_hashes,
            frozen_policy: NativeFrozenPolicyReference {
                schema: NATIVE_FROZEN_POLICY_SCHEMA_V1.into(),
                model_path,
                model_xxh3_128,
                policy_head: NativeFactorizedPolicyHead {
                    schema: FACTORIZED_PAD_POLICY_HEAD_SCHEMA_V1.into(),
                    maximum_duration_ticks: 1,
                    button_logit_threshold: 0.0,
                },
            },
            candidates: vec![NativeFrozenPolicyCandidate {
                id: candidate_id,
                source: "frozen_policy".into(),
            }],
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
pub struct NativeFrozenPolicyBatchError(String);

impl NativeFrozenPolicyBatchError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for NativeFrozenPolicyBatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeFrozenPolicyBatchError {}

#[cfg(test)]
mod tests {
    use super::*;
    fn model(objective: Digest) -> FrozenInferenceModel {
        FrozenInferenceModel::new(
            Digest(NATIVE_POLICY_FEATURE_SCHEMA_SHA256),
            Digest(FactorizedPadPolicyHead::default().schema_sha256().unwrap()),
            objective,
            NATIVE_POLICY_FEATURE_WIDTH,
            (0..FACTORIZED_PAD_POLICY_HEAD_WIDTH as u32).collect(),
            vec![FrozenDenseLayer {
                output_width: FACTORIZED_PAD_POLICY_HEAD_WIDTH,
                activation: FrozenActivation::Linear,
                weights: vec![0.0; NATIVE_POLICY_FEATURE_WIDTH * FACTORIZED_PAD_POLICY_HEAD_WIDTH],
                biases: vec![0.0; FACTORIZED_PAD_POLICY_HEAD_WIDTH],
            }],
        )
        .unwrap()
    }

    #[test]
    fn probe_model_is_content_bound_and_state_reactive() {
        let objective = Digest([0x43; 32]);
        let model = native_frozen_policy_probe_model(objective).unwrap();
        assert_eq!(model.objective_sha256, objective);
        assert_eq!(model.input_width, NATIVE_POLICY_FEATURE_WIDTH);
        assert_eq!(
            model.actions,
            (0..FACTORIZED_PAD_POLICY_HEAD_WIDTH as u32).collect::<Vec<_>>()
        );

        let absent = vec![0.0; NATIVE_POLICY_FEATURE_WIDTH];
        let mut present = absent.clone();
        present[0] = 1.0;
        present[9] = -0.5;
        let outputs = model.infer_batch(&[absent, present]).unwrap();
        assert_eq!(outputs[0], vec![0.0; FACTORIZED_PAD_POLICY_HEAD_WIDTH]);
        assert_eq!(outputs[1][0], -0.125);
        assert_eq!(outputs[1][1], 0.5);
        assert!(outputs[1][2..].iter().all(|value| *value == 0.0));

        let head = FactorizedPadPolicyHead::default();
        let absent_pad = head.decode(&outputs[0]).unwrap().realized_pad().unwrap();
        let present_pad = head.decode(&outputs[1]).unwrap().realized_pad().unwrap();
        assert_eq!((absent_pad.stick_x, absent_pad.stick_y), (0, 0));
        assert_eq!((present_pad.stick_x, present_pad.stick_y), (-16, 64));

        let bytes = model.to_bytes().unwrap();
        assert_eq!(FrozenInferenceModel::from_bytes(&bytes).unwrap(), model);
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
    fn builds_content_bound_v5_batch_for_exact_native_contract() {
        let objective = Digest([0x44; 32]);
        let bytes = model(objective).to_bytes().unwrap();
        let batch = NativeFrozenPolicySuffixBatch::build(
            &bytes,
            "C:/models/policy.dsfrozen".into(),
            objective,
            "native-policy".into(),
            config(),
        )
        .unwrap();
        assert_eq!(batch.schema, NATIVE_FROZEN_POLICY_SUFFIX_BATCH_SCHEMA_V5);
        assert_eq!(batch.frozen_policy.policy_head.maximum_duration_ticks, 1);
        assert_eq!(batch.candidates[0].source, "frozen_policy");
        assert_eq!(batch.frozen_policy.model_xxh3_128.len(), 32);
    }

    #[test]
    fn rejects_detached_model_objective_feature_and_action_contracts() {
        let objective = Digest([0x44; 32]);
        let bytes = model(objective).to_bytes().unwrap();
        assert!(
            NativeFrozenPolicySuffixBatch::build(
                &bytes,
                "policy.dsfrozen".into(),
                Digest([0x45; 32]),
                "native-policy".into(),
                config(),
            )
            .is_err()
        );

        let mut detached = model(objective);
        detached.feature_schema_sha256 = Digest([0x46; 32]);
        assert!(
            NativeFrozenPolicySuffixBatch::build(
                &detached.to_bytes().unwrap(),
                "policy.dsfrozen".into(),
                objective,
                "native-policy".into(),
                config(),
            )
            .is_err()
        );
        detached = model(objective);
        detached.actions[24] = 25;
        assert!(
            NativeFrozenPolicySuffixBatch::build(
                &detached.to_bytes().unwrap(),
                "policy.dsfrozen".into(),
                objective,
                "native-policy".into(),
                config(),
            )
            .is_err()
        );
    }
}
