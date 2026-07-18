//! Rust-owned generation boundaries for native and external offline trainers.

use crate::artifact::Digest;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const MODEL_GENERATION_REQUEST_SCHEMA_V1: &str = "dusklight-model-generation-request/v1";
pub const OFFLINE_TRAINING_RESULT_SCHEMA_V1: &str = "dusklight-offline-training-result/v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OfflineTrainerKind {
    RustNative,
    PythonPytorch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FrozenModelFormat {
    Onnx,
    DusklightFrozenV1,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ModelGenerationRequest {
    pub schema: &'static str,
    pub generation: u32,
    pub dataset_generation_sha256: Digest,
    pub feature_schema_sha256: Digest,
    pub action_schema_sha256: Digest,
    pub objective_sha256: Digest,
    pub trainer: OfflineTrainerKind,
    pub requested_format: FrozenModelFormat,
    pub control_plane: &'static str,
    pub trainer_role: &'static str,
    pub immutable_inputs: bool,
    pub may_launch_workers: bool,
    pub may_mutate_corpora: bool,
    pub may_schedule_evaluations: bool,
    pub may_promote_models: bool,
    pub per_frame_dependency: bool,
    pub request_sha256: Digest,
}

impl ModelGenerationRequest {
    pub fn issue(
        generation: u32,
        dataset_generation_sha256: Digest,
        feature_schema_sha256: Digest,
        action_schema_sha256: Digest,
        objective_sha256: Digest,
        trainer: OfflineTrainerKind,
        requested_format: FrozenModelFormat,
    ) -> Result<Self, ModelOwnershipError> {
        if generation == 0
            || [
                dataset_generation_sha256,
                feature_schema_sha256,
                action_schema_sha256,
                objective_sha256,
            ]
            .contains(&Digest::ZERO)
        {
            return Err(ModelOwnershipError::new(
                "model generation identity is invalid",
            ));
        }
        let mut request = Self {
            schema: MODEL_GENERATION_REQUEST_SCHEMA_V1,
            generation,
            dataset_generation_sha256,
            feature_schema_sha256,
            action_schema_sha256,
            objective_sha256,
            trainer,
            requested_format,
            control_plane: "huntctl-rust",
            trainer_role: "offline_immutable_batch_transform",
            immutable_inputs: true,
            may_launch_workers: false,
            may_mutate_corpora: false,
            may_schedule_evaluations: false,
            may_promote_models: false,
            per_frame_dependency: false,
            request_sha256: Digest::ZERO,
        };
        request.request_sha256 = request.digest()?;
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<(), ModelOwnershipError> {
        if self.schema != MODEL_GENERATION_REQUEST_SCHEMA_V1
            || self.generation == 0
            || [
                self.dataset_generation_sha256,
                self.feature_schema_sha256,
                self.action_schema_sha256,
                self.objective_sha256,
            ]
            .contains(&Digest::ZERO)
            || self.control_plane != "huntctl-rust"
            || self.trainer_role != "offline_immutable_batch_transform"
            || !self.immutable_inputs
            || self.may_launch_workers
            || self.may_mutate_corpora
            || self.may_schedule_evaluations
            || self.may_promote_models
            || self.per_frame_dependency
            || self.request_sha256 != self.digest()?
        {
            return Err(ModelOwnershipError::new(
                "model generation request violates the Rust control-plane boundary",
            ));
        }
        Ok(())
    }

    fn digest(&self) -> Result<Digest, ModelOwnershipError> {
        canonical_digest(
            b"dusklight.model-generation-request/v1\0",
            &(
                self.schema,
                self.generation,
                self.dataset_generation_sha256,
                self.feature_schema_sha256,
                self.action_schema_sha256,
                self.objective_sha256,
                self.trainer,
                self.requested_format,
                self.control_plane,
                self.trainer_role,
                self.immutable_inputs,
                self.may_launch_workers,
                self.may_mutate_corpora,
                self.may_schedule_evaluations,
                self.may_promote_models,
                self.per_frame_dependency,
            ),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OfflineTrainingResult {
    pub schema: &'static str,
    pub request_sha256: Digest,
    pub trainer: OfflineTrainerKind,
    pub model_format: FrozenModelFormat,
    pub model_bytes_sha256: Digest,
    pub producer_sha256: Digest,
    pub import_as_candidate_only: bool,
    pub promotion_authority: bool,
    pub result_sha256: Digest,
}

impl OfflineTrainingResult {
    pub fn seal(
        request: &ModelGenerationRequest,
        producer_sha256: Digest,
        model_bytes: &[u8],
    ) -> Result<Self, ModelOwnershipError> {
        request.validate()?;
        if producer_sha256 == Digest::ZERO || model_bytes.is_empty() {
            return Err(ModelOwnershipError::new(
                "offline training result identity is invalid",
            ));
        }
        let mut result = Self {
            schema: OFFLINE_TRAINING_RESULT_SCHEMA_V1,
            request_sha256: request.request_sha256,
            trainer: request.trainer,
            model_format: request.requested_format,
            model_bytes_sha256: bytes_digest(b"dusklight.frozen-model-bytes/v1\0", model_bytes),
            producer_sha256,
            import_as_candidate_only: true,
            promotion_authority: false,
            result_sha256: Digest::ZERO,
        };
        result.result_sha256 = result.digest()?;
        result.verify(request, model_bytes)?;
        Ok(result)
    }

    pub fn verify(
        &self,
        request: &ModelGenerationRequest,
        model_bytes: &[u8],
    ) -> Result<(), ModelOwnershipError> {
        request.validate()?;
        if self.schema != OFFLINE_TRAINING_RESULT_SCHEMA_V1
            || self.request_sha256 != request.request_sha256
            || self.trainer != request.trainer
            || self.model_format != request.requested_format
            || self.model_bytes_sha256
                != bytes_digest(b"dusklight.frozen-model-bytes/v1\0", model_bytes)
            || self.producer_sha256 == Digest::ZERO
            || !self.import_as_candidate_only
            || self.promotion_authority
            || self.result_sha256 != self.digest()?
        {
            return Err(ModelOwnershipError::new(
                "offline training result is detached or exceeds candidate authority",
            ));
        }
        Ok(())
    }

    fn digest(&self) -> Result<Digest, ModelOwnershipError> {
        canonical_digest(
            b"dusklight.offline-training-result/v1\0",
            &(
                self.schema,
                self.request_sha256,
                self.trainer,
                self.model_format,
                self.model_bytes_sha256,
                self.producer_sha256,
                self.import_as_candidate_only,
                self.promotion_authority,
            ),
        )
    }
}

fn canonical_digest<T: Serialize>(domain: &[u8], value: &T) -> Result<Digest, ModelOwnershipError> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| ModelOwnershipError::new(error.to_string()))?;
    Ok(bytes_digest(domain, &bytes))
}

fn bytes_digest(domain: &[u8], bytes: &[u8]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    Digest(hasher.finalize().into())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelOwnershipError(String);

impl ModelOwnershipError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ModelOwnershipError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ModelOwnershipError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn generation_request(trainer: OfflineTrainerKind) -> ModelGenerationRequest {
        ModelGenerationRequest::issue(
            3,
            Digest([1; 32]),
            Digest([2; 32]),
            Digest([3; 32]),
            Digest([4; 32]),
            trainer,
            FrozenModelFormat::Onnx,
        )
        .unwrap()
    }

    #[test]
    fn python_is_an_offline_transform_without_control_plane_authority() {
        let request = generation_request(OfflineTrainerKind::PythonPytorch);
        assert_eq!(request.control_plane, "huntctl-rust");
        assert!(request.immutable_inputs);
        assert!(!request.may_launch_workers);
        assert!(!request.may_mutate_corpora);
        assert!(!request.may_schedule_evaluations);
        assert!(!request.may_promote_models);
        assert!(!request.per_frame_dependency);
    }

    #[test]
    fn external_result_is_bound_to_request_and_candidate_only() {
        let request = generation_request(OfflineTrainerKind::PythonPytorch);
        let bytes = b"frozen model";
        let result = OfflineTrainingResult::seal(&request, Digest([8; 32]), bytes).unwrap();
        assert!(result.import_as_candidate_only);
        assert!(!result.promotion_authority);
        result.verify(&request, bytes).unwrap();
        assert!(result.verify(&request, b"different model").is_err());

        let other = generation_request(OfflineTrainerKind::RustNative);
        assert!(result.verify(&other, bytes).is_err());
    }
}
