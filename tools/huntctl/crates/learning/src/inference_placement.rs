//! Measured placement gate for Rust-control-plane versus native-worker inference.

use crate::artifact::Digest;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const INFERENCE_PLACEMENT_REPORT_SCHEMA_V1: &str = "dusklight-inference-placement-report/v1";
const MAX_TIMING_SAMPLES: usize = 100_000;
const MAX_BATCH_ROWS: usize = 8192;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InferencePlacement {
    RustControlPlane,
    NativeWorker,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct InferenceTimingSample {
    pub sample_sha256: Digest,
    pub placement: InferencePlacement,
    pub batch_rows: usize,
    pub inference_ns: u64,
    pub serialization_ns: u64,
    pub ipc_round_trip_ns: u64,
}

impl InferenceTimingSample {
    fn total_ns(self) -> Option<u64> {
        self.inference_ns
            .checked_add(self.serialization_ns)?
            .checked_add(self.ipc_round_trip_ns)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct InferencePlacementConfig {
    pub minimum_repetitions_per_batch: usize,
    pub minimum_distinct_batch_sizes: usize,
    pub minimum_native_relative_improvement: f64,
}

impl Default for InferencePlacementConfig {
    fn default() -> Self {
        Self {
            minimum_repetitions_per_batch: 20,
            minimum_distinct_batch_sizes: 3,
            minimum_native_relative_improvement: 0.1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlacementTimingSummary {
    pub placement: InferencePlacement,
    pub batch_rows: usize,
    pub samples: usize,
    pub median_inference_ns: u64,
    pub median_serialization_ns: u64,
    pub median_ipc_round_trip_ns: u64,
    pub median_end_to_end_ns: u64,
    pub p95_end_to_end_ns: u64,
    pub p95_end_to_end_ns_per_row: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InferencePlacementDecision {
    RetainRustBatchInference,
    NativeWorkerBatchCandidate,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InferencePlacementReport {
    pub schema: &'static str,
    pub frozen_model_sha256: Digest,
    pub benchmark_corpus_sha256: Digest,
    pub config: InferencePlacementConfig,
    pub timings: Vec<PlacementTimingSummary>,
    pub equal_repetition_budget: bool,
    pub measured_serialization_cost: bool,
    pub measured_ipc_cost: bool,
    pub decision: InferencePlacementDecision,
    pub per_tick_inference_authorized: bool,
    pub promotion_authority: bool,
    pub limitation: &'static str,
    pub report_sha256: Digest,
}

impl InferencePlacementReport {
    pub fn compare(
        frozen_model_sha256: Digest,
        benchmark_corpus_sha256: Digest,
        samples: &[InferenceTimingSample],
        config: InferencePlacementConfig,
    ) -> Result<Self, InferencePlacementError> {
        validate_inputs(
            frozen_model_sha256,
            benchmark_corpus_sha256,
            samples,
            config,
        )?;
        let mut groups = BTreeMap::<(InferencePlacement, usize), Vec<InferenceTimingSample>>::new();
        for sample in samples {
            groups
                .entry((sample.placement, sample.batch_rows))
                .or_default()
                .push(*sample);
        }
        let batch_sizes = groups
            .keys()
            .map(|(_, batch_rows)| *batch_rows)
            .collect::<BTreeSet<_>>();
        let mut timings = Vec::with_capacity(batch_sizes.len() * 2);
        let mut equal_repetition_budget = true;
        let mut native_wins_every_batch = true;
        for batch_rows in batch_sizes {
            let rust = groups
                .get(&(InferencePlacement::RustControlPlane, batch_rows))
                .ok_or_else(|| InferencePlacementError::new("Rust timing group is missing"))?;
            let native = groups
                .get(&(InferencePlacement::NativeWorker, batch_rows))
                .ok_or_else(|| InferencePlacementError::new("native timing group is missing"))?;
            if rust.len() != native.len() {
                equal_repetition_budget = false;
            }
            let rust_summary = summarize(InferencePlacement::RustControlPlane, batch_rows, rust)?;
            let native_summary = summarize(InferencePlacement::NativeWorker, batch_rows, native)?;
            let required = rust_summary.p95_end_to_end_ns_per_row
                * (1.0 - config.minimum_native_relative_improvement);
            native_wins_every_batch &= native_summary.p95_end_to_end_ns_per_row <= required;
            timings.extend([rust_summary, native_summary]);
        }
        if !equal_repetition_budget {
            return Err(InferencePlacementError::new(
                "placement comparison requires equal repetition budgets",
            ));
        }
        let decision = if native_wins_every_batch {
            InferencePlacementDecision::NativeWorkerBatchCandidate
        } else {
            InferencePlacementDecision::RetainRustBatchInference
        };
        let mut report = Self {
            schema: INFERENCE_PLACEMENT_REPORT_SCHEMA_V1,
            frozen_model_sha256,
            benchmark_corpus_sha256,
            config,
            timings,
            equal_repetition_budget,
            measured_serialization_cost: samples.iter().any(|sample| sample.serialization_ns > 0),
            measured_ipc_cost: samples.iter().any(|sample| sample.ipc_round_trip_ns > 0),
            decision,
            per_tick_inference_authorized: false,
            promotion_authority: false,
            limitation: "placement timings do not establish native objective success",
            report_sha256: Digest::ZERO,
        };
        report.report_sha256 = report.digest()?;
        Ok(report)
    }

    fn digest(&self) -> Result<Digest, InferencePlacementError> {
        let bytes = serde_json::to_vec(&(
            self.schema,
            self.frozen_model_sha256,
            self.benchmark_corpus_sha256,
            self.config,
            &self.timings,
            self.equal_repetition_budget,
            self.measured_serialization_cost,
            self.measured_ipc_cost,
            self.decision,
            self.per_tick_inference_authorized,
            self.promotion_authority,
            self.limitation,
        ))
        .map_err(|error| InferencePlacementError::new(error.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.inference-placement-report/v1\0");
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }
}

fn summarize(
    placement: InferencePlacement,
    batch_rows: usize,
    samples: &[InferenceTimingSample],
) -> Result<PlacementTimingSummary, InferencePlacementError> {
    let mut inference = samples
        .iter()
        .map(|sample| sample.inference_ns)
        .collect::<Vec<_>>();
    let mut serialization = samples
        .iter()
        .map(|sample| sample.serialization_ns)
        .collect::<Vec<_>>();
    let mut ipc = samples
        .iter()
        .map(|sample| sample.ipc_round_trip_ns)
        .collect::<Vec<_>>();
    let mut total = samples
        .iter()
        .map(|sample| {
            sample
                .total_ns()
                .ok_or_else(|| InferencePlacementError::new("timing total overflowed"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    for values in [&mut inference, &mut serialization, &mut ipc, &mut total] {
        values.sort_unstable();
    }
    let median_index = samples.len() / 2;
    let p95_index = samples.len().saturating_mul(95).div_ceil(100) - 1;
    Ok(PlacementTimingSummary {
        placement,
        batch_rows,
        samples: samples.len(),
        median_inference_ns: inference[median_index],
        median_serialization_ns: serialization[median_index],
        median_ipc_round_trip_ns: ipc[median_index],
        median_end_to_end_ns: total[median_index],
        p95_end_to_end_ns: total[p95_index],
        p95_end_to_end_ns_per_row: total[p95_index] as f64 / batch_rows as f64,
    })
}

fn validate_inputs(
    frozen_model_sha256: Digest,
    benchmark_corpus_sha256: Digest,
    samples: &[InferenceTimingSample],
    config: InferencePlacementConfig,
) -> Result<(), InferencePlacementError> {
    if frozen_model_sha256 == Digest::ZERO
        || benchmark_corpus_sha256 == Digest::ZERO
        || samples.is_empty()
        || samples.len() > MAX_TIMING_SAMPLES
        || config.minimum_repetitions_per_batch == 0
        || config.minimum_distinct_batch_sizes < 2
        || !config.minimum_native_relative_improvement.is_finite()
        || !(0.0..=1.0).contains(&config.minimum_native_relative_improvement)
    {
        return Err(InferencePlacementError::new(
            "inference placement configuration is invalid",
        ));
    }
    let mut identities = BTreeSet::new();
    let mut counts = BTreeMap::<(InferencePlacement, usize), usize>::new();
    for sample in samples {
        if sample.sample_sha256 == Digest::ZERO
            || !identities.insert(sample.sample_sha256)
            || sample.batch_rows == 0
            || sample.batch_rows > MAX_BATCH_ROWS
            || sample.inference_ns == 0
            || sample.total_ns().is_none()
            || (sample.placement == InferencePlacement::RustControlPlane
                && sample.ipc_round_trip_ns != 0)
            || (sample.placement == InferencePlacement::NativeWorker
                && (sample.serialization_ns == 0 || sample.ipc_round_trip_ns == 0))
        {
            return Err(InferencePlacementError::new(
                "inference timing sample is invalid or duplicated",
            ));
        }
        *counts
            .entry((sample.placement, sample.batch_rows))
            .or_default() += 1;
    }
    let batch_sizes = counts
        .keys()
        .map(|(_, batch_rows)| *batch_rows)
        .collect::<BTreeSet<_>>();
    if batch_sizes.len() < config.minimum_distinct_batch_sizes
        || batch_sizes.iter().any(|batch_rows| {
            [
                InferencePlacement::RustControlPlane,
                InferencePlacement::NativeWorker,
            ]
            .iter()
            .any(|placement| {
                counts.get(&(*placement, *batch_rows)).copied().unwrap_or(0)
                    < config.minimum_repetitions_per_batch
            })
        })
    {
        return Err(InferencePlacementError::new(
            "IPC and batching measurements are incomplete",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InferencePlacementError(String);

impl InferencePlacementError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for InferencePlacementError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for InferencePlacementError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples(native_fast: bool) -> Vec<InferenceTimingSample> {
        let mut samples = Vec::new();
        let mut identity = 1_u8;
        for batch_rows in [1, 16, 128] {
            for placement in [
                InferencePlacement::RustControlPlane,
                InferencePlacement::NativeWorker,
            ] {
                for _ in 0..3 {
                    let native = placement == InferencePlacement::NativeWorker;
                    samples.push(InferenceTimingSample {
                        sample_sha256: Digest([identity; 32]),
                        placement,
                        batch_rows,
                        inference_ns: if native && native_fast { 40 } else { 100 },
                        serialization_ns: if native { 5 } else { 1 },
                        ipc_round_trip_ns: if native { 5 } else { 0 },
                    });
                    identity += 1;
                }
            }
        }
        samples
    }

    fn config() -> InferencePlacementConfig {
        InferencePlacementConfig {
            minimum_repetitions_per_batch: 3,
            minimum_distinct_batch_sizes: 3,
            minimum_native_relative_improvement: 0.1,
        }
    }

    #[test]
    fn measured_ipc_and_batches_gate_native_placement_without_tick_authority() {
        let report = InferencePlacementReport::compare(
            Digest([1; 32]),
            Digest([2; 32]),
            &samples(true),
            config(),
        )
        .unwrap();
        assert_eq!(
            report.decision,
            InferencePlacementDecision::NativeWorkerBatchCandidate
        );
        assert!(report.equal_repetition_budget);
        assert!(report.measured_ipc_cost);
        assert!(!report.per_tick_inference_authorized);
        assert!(!report.promotion_authority);
        assert_ne!(report.report_sha256, Digest::ZERO);
    }

    #[test]
    fn incomplete_measurements_fail_and_ipc_regression_retains_rust() {
        let mut incomplete = samples(true);
        incomplete.retain(|sample| {
            !(sample.placement == InferencePlacement::NativeWorker && sample.batch_rows == 16)
        });
        assert!(
            InferencePlacementReport::compare(
                Digest([1; 32]),
                Digest([2; 32]),
                &incomplete,
                config()
            )
            .is_err()
        );

        let mut missing_serialization = samples(true);
        for sample in &mut missing_serialization {
            if sample.placement == InferencePlacement::NativeWorker {
                sample.serialization_ns = 0;
            }
        }
        assert!(
            InferencePlacementReport::compare(
                Digest([1; 32]),
                Digest([2; 32]),
                &missing_serialization,
                config()
            )
            .is_err()
        );

        let report = InferencePlacementReport::compare(
            Digest([1; 32]),
            Digest([2; 32]),
            &samples(false),
            config(),
        )
        .unwrap();
        assert_eq!(
            report.decision,
            InferencePlacementDecision::RetainRustBatchInference
        );
    }
}
