//! Deterministic CPU and tolerance-declared accelerator inference evidence.

use super::frozen_inference::{FrozenInferenceError, FrozenInferenceModel};
use crate::artifact::Digest;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const INFERENCE_CONFORMANCE_SCHEMA_V1: &str = "dusklight-inference-conformance/v1";
const MAX_CPU_REPETITIONS: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct AcceleratorTolerance {
    pub absolute: f64,
    pub relative: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InferenceConformanceReport {
    pub schema: &'static str,
    pub frozen_model_sha256: Digest,
    pub input_corpus_sha256: Digest,
    pub input_batch_sha256: Digest,
    pub accelerator_backend: String,
    pub accelerator_build_sha256: Digest,
    pub tolerance: AcceleratorTolerance,
    pub rows: usize,
    pub values_per_row: usize,
    pub cpu_repetitions: usize,
    pub cpu_bitwise_deterministic: bool,
    pub cpu_output_sha256: Digest,
    pub accelerator_output_sha256: Digest,
    pub accelerator_maximum_absolute_error: f64,
    pub accelerator_maximum_relative_error: f64,
    pub accelerator_within_declared_tolerance: bool,
    pub promotion_authority: bool,
    pub report_sha256: Digest,
}

impl InferenceConformanceReport {
    #[allow(clippy::too_many_arguments)]
    pub fn evaluate(
        model: &FrozenInferenceModel,
        input_corpus_sha256: Digest,
        inputs: &[Vec<f32>],
        cpu_repetitions: usize,
        accelerator_backend: impl Into<String>,
        accelerator_build_sha256: Digest,
        accelerator_outputs: &[Vec<f32>],
        tolerance: AcceleratorTolerance,
    ) -> Result<Self, InferenceConformanceError> {
        if input_corpus_sha256 == Digest::ZERO
            || accelerator_build_sha256 == Digest::ZERO
            || !(2..=MAX_CPU_REPETITIONS).contains(&cpu_repetitions)
            || !tolerance.absolute.is_finite()
            || tolerance.absolute < 0.0
            || !tolerance.relative.is_finite()
            || tolerance.relative < 0.0
        {
            return Err(InferenceConformanceError::new(
                "inference conformance configuration is invalid",
            ));
        }
        let accelerator_backend = accelerator_backend.into();
        if accelerator_backend.trim().is_empty() || accelerator_backend.len() > 128 {
            return Err(InferenceConformanceError::new(
                "accelerator backend identity is invalid",
            ));
        }
        let reference = model.infer_batch(inputs)?;
        let cpu_bitwise_deterministic = (1..cpu_repetitions).all(|_| {
            model
                .infer_batch(inputs)
                .is_ok_and(|output| bitwise_equal(&reference, &output))
        });
        if !cpu_bitwise_deterministic {
            return Err(InferenceConformanceError::new(
                "CPU inference is not bitwise deterministic",
            ));
        }
        if accelerator_outputs.len() != reference.len()
            || accelerator_outputs
                .iter()
                .zip(&reference)
                .any(|(actual, expected)| {
                    actual.len() != expected.len() || actual.iter().any(|value| !value.is_finite())
                })
        {
            return Err(InferenceConformanceError::new(
                "accelerator output shape or values are invalid",
            ));
        }
        let mut maximum_absolute_error = 0.0_f64;
        let mut maximum_relative_error = 0.0_f64;
        let mut within_tolerance = true;
        for (actual_row, expected_row) in accelerator_outputs.iter().zip(&reference) {
            for (actual, expected) in actual_row.iter().zip(expected_row) {
                let actual = f64::from(*actual);
                let expected = f64::from(*expected);
                let absolute = (actual - expected).abs();
                let relative = absolute / expected.abs().max(1.0e-12);
                maximum_absolute_error = maximum_absolute_error.max(absolute);
                maximum_relative_error = maximum_relative_error.max(relative);
                within_tolerance &=
                    absolute <= tolerance.absolute + tolerance.relative * expected.abs();
            }
        }
        let frozen_model_sha256 = model.artifact_sha256()?;
        let input_batch_sha256 = tensor_digest(b"dusklight.inference-input-batch/v1\0", inputs);
        let cpu_output_sha256 = tensor_digest(b"dusklight.cpu-inference-output/v1\0", &reference);
        let accelerator_output_sha256 = tensor_digest(
            b"dusklight.accelerator-inference-output/v1\0",
            accelerator_outputs,
        );
        let values_per_row = reference.first().map(Vec::len).unwrap_or(0);
        let mut report = Self {
            schema: INFERENCE_CONFORMANCE_SCHEMA_V1,
            frozen_model_sha256,
            input_corpus_sha256,
            input_batch_sha256,
            accelerator_backend,
            accelerator_build_sha256,
            tolerance,
            rows: reference.len(),
            values_per_row,
            cpu_repetitions,
            cpu_bitwise_deterministic,
            cpu_output_sha256,
            accelerator_output_sha256,
            accelerator_maximum_absolute_error: maximum_absolute_error,
            accelerator_maximum_relative_error: maximum_relative_error,
            accelerator_within_declared_tolerance: within_tolerance,
            promotion_authority: false,
            report_sha256: Digest::ZERO,
        };
        report.report_sha256 = report.digest()?;
        Ok(report)
    }

    fn digest(&self) -> Result<Digest, InferenceConformanceError> {
        let bytes = serde_json::to_vec(&(
            (
                self.schema,
                self.frozen_model_sha256,
                self.input_corpus_sha256,
                self.input_batch_sha256,
                &self.accelerator_backend,
                self.accelerator_build_sha256,
                self.tolerance,
                self.rows,
                self.values_per_row,
            ),
            (
                self.cpu_repetitions,
                self.cpu_bitwise_deterministic,
                self.cpu_output_sha256,
                self.accelerator_output_sha256,
                self.accelerator_maximum_absolute_error,
                self.accelerator_maximum_relative_error,
                self.accelerator_within_declared_tolerance,
                self.promotion_authority,
            ),
        ))
        .map_err(|error| InferenceConformanceError::new(error.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.inference-conformance/v1\0");
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }
}

fn bitwise_equal(left: &[Vec<f32>], right: &[Vec<f32>]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| left.to_bits() == right.to_bits())
        })
}

fn tensor_digest(domain: &[u8], output: &[Vec<f32>]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((output.len() as u64).to_le_bytes());
    for row in output {
        hasher.update((row.len() as u64).to_le_bytes());
        for value in row {
            hasher.update(value.to_bits().to_le_bytes());
        }
    }
    Digest(hasher.finalize().into())
}

#[derive(Debug)]
pub struct InferenceConformanceError(String);

impl InferenceConformanceError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for InferenceConformanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for InferenceConformanceError {}

impl From<FrozenInferenceError> for InferenceConformanceError {
    fn from(value: FrozenInferenceError) -> Self {
        Self::new(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frozen_inference::{FrozenActivation, FrozenDenseLayer};

    fn model() -> FrozenInferenceModel {
        FrozenInferenceModel::new(
            Digest([1; 32]),
            Digest([2; 32]),
            Digest([3; 32]),
            2,
            vec![0, 1],
            vec![FrozenDenseLayer {
                output_width: 2,
                activation: FrozenActivation::Linear,
                weights: vec![1.0, 2.0, -1.0, 0.5],
                biases: vec![0.0, 1.0],
            }],
        )
        .unwrap()
    }

    #[test]
    fn cpu_is_exact_and_accelerator_uses_declared_tolerance() {
        let inputs = vec![vec![1.0, 2.0], vec![-2.0, 4.0]];
        let accelerator = vec![vec![5.000_001, 1.000_001], vec![6.0, 5.000_001]];
        let report = InferenceConformanceReport::evaluate(
            &model(),
            Digest([4; 32]),
            &inputs,
            8,
            "test-accelerator",
            Digest([5; 32]),
            &accelerator,
            AcceleratorTolerance {
                absolute: 2.0e-6,
                relative: 1.0e-6,
            },
        )
        .unwrap();
        assert!(report.cpu_bitwise_deterministic);
        assert!(report.accelerator_within_declared_tolerance);
        assert!(!report.promotion_authority);
        assert_ne!(report.input_batch_sha256, Digest::ZERO);
        assert_ne!(report.cpu_output_sha256, Digest::ZERO);
        assert_ne!(report.accelerator_output_sha256, Digest::ZERO);
        assert_ne!(report.report_sha256, Digest::ZERO);
    }

    #[test]
    fn out_of_tolerance_accelerator_is_recorded_as_nonconformant() {
        let report = InferenceConformanceReport::evaluate(
            &model(),
            Digest([4; 32]),
            &[vec![1.0, 2.0]],
            2,
            "test-accelerator",
            Digest([5; 32]),
            &[vec![6.0, 1.0]],
            AcceleratorTolerance {
                absolute: 1.0e-6,
                relative: 1.0e-6,
            },
        )
        .unwrap();
        assert!(!report.accelerator_within_declared_tolerance);
        assert!(report.accelerator_maximum_absolute_error >= 1.0);
    }
}
