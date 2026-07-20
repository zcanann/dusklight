use crate::{
    COLD_PROCESS_BENCHMARK_SCHEMA, COLD_PROCESS_MODE, ColdProcessBenchmarkError, MAX_REPETITIONS,
    benchmark_error,
};
use dusklight_harness_contracts::artifact::Digest;
use dusklight_harness_contracts::run_contract::{
    HarnessBoundaryFingerprint, HarnessNativePhaseTiming, HarnessTerminalReason,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

mod comparison;
mod native_phases;
pub(crate) use comparison::comparison_issue;
pub use native_phases::ColdProcessNativePhaseBreakdown;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ColdProcessBenchmarkAttempt {
    pub attempt: u32,
    pub request: String,
    pub request_sha256: Digest,
    pub artifact_destination: String,
    pub result: String,
    pub result_sha256: Digest,
    pub terminal: HarnessTerminalReason,
    pub objective_reached: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_hit_tick: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boundary_fingerprint: Option<HarnessBoundaryFingerprint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realized_input_sha256: Option<Digest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gameplay_trace_sha256: Option<Digest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective_evidence_sha256: Option<Digest>,
    pub artifacts_complete: bool,
    pub logical_ticks: u64,
    pub consumed_input_ticks: u64,
    pub native_process_millis: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_process_cpu_micros: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_file_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix_ticks: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_ticks: Option<u64>,
    pub end_to_end_micros: u128,
    pub harness_outside_process_micros: u128,
    pub native_phases: HarnessNativePhaseTiming,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ColdProcessBenchmarkSummary {
    pub total_logical_ticks: u64,
    pub total_consumed_input_ticks: u64,
    pub total_native_process_millis: u64,
    pub total_end_to_end_micros: u128,
    pub total_harness_outside_process_micros: u128,
    pub median_end_to_end_micros: u128,
    pub p95_end_to_end_micros: u128,
    pub candidates_per_second_millionths: u64,
    pub logical_ticks_per_second_millionths: u64,
    pub consumed_input_ticks_per_second_millionths: u64,
    pub native_process_time_share_millionths: u32,
    pub total_native_lifecycle_micros: u64,
    pub native_phase_totals_micros: ColdProcessNativePhaseBreakdown,
    pub native_phase_shares_millionths: ColdProcessNativePhaseBreakdown,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_launches: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_prefix_ticks: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_candidate_ticks: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_ticks_per_second_millionths: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_native_process_cpu_micros: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_cpu_utilization_millionths: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_artifact_file_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_artifact_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub simulator_idle_micros: Option<u128>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ColdProcessBenchmarkHost {
    pub operating_system: String,
    pub architecture: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operating_system_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hardware_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_model: Option<String>,
    pub logical_cpu_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_bytes: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ColdProcessBenchmarkReport {
    pub schema: String,
    pub content_sha256: Digest,
    pub mode: String,
    pub recorded_unix_millis: u128,
    pub host: ColdProcessBenchmarkHost,
    pub template_request_sha256: Digest,
    pub artifact_destination_root: String,
    pub repetitions: u32,
    pub comparable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comparison_issue: Option<String>,
    pub attempts: Vec<ColdProcessBenchmarkAttempt>,
    pub summary: ColdProcessBenchmarkSummary,
}

impl ColdProcessBenchmarkReport {
    pub fn validate(&self) -> Result<(), ColdProcessBenchmarkError> {
        if self.schema != COLD_PROCESS_BENCHMARK_SCHEMA || self.mode != COLD_PROCESS_MODE {
            return Err(benchmark_error("unsupported cold-process benchmark report"));
        }
        if self.recorded_unix_millis == 0
            || self.host.operating_system.is_empty()
            || self.host.architecture.is_empty()
            || self.host.logical_cpu_count == 0
            || self.template_request_sha256 == Digest::ZERO
            || self.repetitions < 2
            || self.repetitions > MAX_REPETITIONS
            || self.attempts.len() != self.repetitions as usize
        {
            return Err(benchmark_error(
                "cold-process benchmark report has invalid identity or repetition bounds",
            ));
        }
        for (index, attempt) in self.attempts.iter().enumerate() {
            let expected = u32::try_from(index + 1).map_err(|_| {
                benchmark_error("cold-process attempt index does not fit its contract")
            })?;
            if attempt.attempt != expected
                || attempt.request_sha256 == Digest::ZERO
                || attempt.result_sha256 == Digest::ZERO
                || attempt.end_to_end_micros == 0
                || attempt.artifact_file_count.is_some() != attempt.artifact_bytes.is_some()
                || attempt.prefix_ticks.is_some() != attempt.candidate_ticks.is_some()
                || attempt.prefix_ticks.is_some_and(|ticks| {
                    ticks > attempt.logical_ticks
                        || attempt.candidate_ticks != Some(attempt.logical_ticks - ticks)
                })
            {
                return Err(benchmark_error(
                    "cold-process benchmark attempt identity or timing is invalid",
                ));
            }
            attempt
                .native_phases
                .validate(attempt.native_process_millis)
                .map_err(|error| {
                    benchmark_error(format!(
                        "cold-process attempt {} has invalid native phase timing: {error}",
                        attempt.attempt
                    ))
                })?;
        }
        let issue = comparison_issue(&self.attempts);
        if self.comparable != issue.is_none() || self.comparison_issue != issue {
            return Err(benchmark_error(
                "cold-process benchmark comparability does not match attempt evidence",
            ));
        }
        let expected_summary = summarize(&self.attempts)?;
        if self.summary != expected_summary {
            return Err(benchmark_error(format!(
                "cold-process benchmark summary does not match its attempts: expected {expected_summary:?}, found {:?}",
                self.summary
            )));
        }
        if self.content_sha256 != self.compute_content_sha256()? {
            return Err(benchmark_error(
                "cold-process benchmark report content digest is stale",
            ));
        }
        Ok(())
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, ColdProcessBenchmarkError> {
        self.validate()?;
        let mut bytes = serde_json::to_vec_pretty(self).map_err(|error| {
            benchmark_error(format!(
                "cannot encode cold-process benchmark report: {error}"
            ))
        })?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    pub(crate) fn refresh_content_sha256(&mut self) -> Result<(), ColdProcessBenchmarkError> {
        self.content_sha256 = self.compute_content_sha256()?;
        Ok(())
    }

    fn compute_content_sha256(&self) -> Result<Digest, ColdProcessBenchmarkError> {
        let mut identity = self.clone();
        identity.content_sha256 = Digest::ZERO;
        let encoded = serde_json::to_vec(&identity).map_err(|error| {
            benchmark_error(format!(
                "cannot encode cold-process report identity: {error}"
            ))
        })?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.cold-process-throughput/v2\0");
        hasher.update((encoded.len() as u64).to_le_bytes());
        hasher.update(encoded);
        Ok(Digest(hasher.finalize().into()))
    }
}

pub(crate) fn summarize(
    attempts: &[ColdProcessBenchmarkAttempt],
) -> Result<ColdProcessBenchmarkSummary, ColdProcessBenchmarkError> {
    if attempts.is_empty() {
        return Err(benchmark_error(
            "cannot summarize an empty cold-process benchmark",
        ));
    }
    let total_logical_ticks = attempts
        .iter()
        .try_fold(0_u64, |total, attempt| {
            total.checked_add(attempt.logical_ticks)
        })
        .ok_or_else(|| benchmark_error("logical tick total overflowed"))?;
    let total_consumed_input_ticks = attempts
        .iter()
        .try_fold(0_u64, |total, attempt| {
            total.checked_add(attempt.consumed_input_ticks)
        })
        .ok_or_else(|| benchmark_error("consumed input tick total overflowed"))?;
    let total_native_process_millis = attempts
        .iter()
        .try_fold(0_u64, |total, attempt| {
            total.checked_add(attempt.native_process_millis)
        })
        .ok_or_else(|| benchmark_error("native process time total overflowed"))?;
    let total_end_to_end_micros = attempts
        .iter()
        .try_fold(0_u128, |total, attempt| {
            total.checked_add(attempt.end_to_end_micros)
        })
        .ok_or_else(|| benchmark_error("end-to-end time total overflowed"))?;
    let total_harness_outside_process_micros = attempts
        .iter()
        .try_fold(0_u128, |total, attempt| {
            total.checked_add(attempt.harness_outside_process_micros)
        })
        .ok_or_else(|| benchmark_error("outer harness time total overflowed"))?;
    let mut durations = attempts
        .iter()
        .map(|attempt| attempt.end_to_end_micros)
        .collect::<Vec<_>>();
    durations.sort_unstable();
    let candidate_count = u64::try_from(attempts.len())
        .map_err(|_| benchmark_error("candidate count does not fit throughput summary"))?;
    let native_process_micros = u128::from(total_native_process_millis) * 1_000;
    let mut native_phase_totals_micros = ColdProcessNativePhaseBreakdown::default();
    for attempt in attempts {
        native_phase_totals_micros
            .checked_add(&ColdProcessNativePhaseBreakdown::from_attempt(attempt))
            .ok_or_else(|| benchmark_error("native lifecycle phase totals overflowed"))?;
    }
    let total_native_lifecycle_micros = attempts
        .iter()
        .try_fold(0_u64, |total, attempt| {
            total.checked_add(attempt.native_phases.exit_ready_micros)
        })
        .ok_or_else(|| benchmark_error("native lifecycle total overflowed"))?;
    let native_phase_shares_millionths =
        native_phase_totals_micros.shares(total_native_process_millis.saturating_mul(1_000))?;
    let cpu_measured = attempts
        .iter()
        .all(|attempt| attempt.native_process_cpu_micros.is_some());
    let artifacts_measured = attempts
        .iter()
        .all(|attempt| attempt.artifact_file_count.is_some());
    let route_ticks = attempts
        .iter()
        .all(|attempt| attempt.prefix_ticks.is_some());
    let process_launches = Some(
        u64::try_from(attempts.len())
            .map_err(|_| benchmark_error("process launch count does not fit u64"))?,
    );
    let total_native_process_cpu_micros = cpu_measured
        .then(|| sum_optional_u64(attempts, |attempt| attempt.native_process_cpu_micros))
        .transpose()?;
    let total_artifact_file_count = artifacts_measured
        .then(|| sum_optional_u64(attempts, |attempt| attempt.artifact_file_count))
        .transpose()?;
    let total_artifact_bytes = artifacts_measured
        .then(|| sum_optional_u64(attempts, |attempt| attempt.artifact_bytes))
        .transpose()?;
    let total_prefix_ticks = route_ticks
        .then(|| sum_optional_u64(attempts, |attempt| attempt.prefix_ticks))
        .transpose()?;
    let total_candidate_ticks = route_ticks
        .then(|| sum_optional_u64(attempts, |attempt| attempt.candidate_ticks))
        .transpose()?;
    let candidate_ticks_per_second_millionths = total_candidate_ticks
        .map(|ticks| per_second_millionths(ticks, total_end_to_end_micros))
        .transpose()?;
    let native_cpu_utilization_millionths = total_native_process_cpu_micros
        .map(|cpu| fixed_share_millionths(u128::from(cpu), total_end_to_end_micros))
        .transpose()?;
    let simulator_idle_micros = Some(
        total_end_to_end_micros.saturating_sub(u128::from(native_phase_totals_micros.simulation)),
    );
    Ok(ColdProcessBenchmarkSummary {
        total_logical_ticks,
        total_consumed_input_ticks,
        total_native_process_millis,
        total_end_to_end_micros,
        total_harness_outside_process_micros,
        median_end_to_end_micros: percentile(&durations, 50),
        p95_end_to_end_micros: percentile(&durations, 95),
        candidates_per_second_millionths: per_second_millionths(
            candidate_count,
            total_end_to_end_micros,
        )?,
        logical_ticks_per_second_millionths: per_second_millionths(
            total_logical_ticks,
            total_end_to_end_micros,
        )?,
        consumed_input_ticks_per_second_millionths: per_second_millionths(
            total_consumed_input_ticks,
            total_end_to_end_micros,
        )?,
        native_process_time_share_millionths: u32::try_from(
            native_process_micros
                .checked_mul(1_000_000)
                .ok_or_else(|| benchmark_error("native process share overflowed"))?
                / total_end_to_end_micros,
        )
        .map_err(|_| benchmark_error("native process share exceeds its fixed-point range"))?,
        total_native_lifecycle_micros,
        native_phase_totals_micros,
        native_phase_shares_millionths,
        process_launches,
        total_prefix_ticks,
        total_candidate_ticks,
        candidate_ticks_per_second_millionths,
        total_native_process_cpu_micros,
        native_cpu_utilization_millionths,
        total_artifact_file_count,
        total_artifact_bytes,
        simulator_idle_micros,
    })
}

fn sum_optional_u64(
    attempts: &[ColdProcessBenchmarkAttempt],
    value: impl Fn(&ColdProcessBenchmarkAttempt) -> Option<u64>,
) -> Result<u64, ColdProcessBenchmarkError> {
    attempts.iter().try_fold(0_u64, |total, attempt| {
        total
            .checked_add(value(attempt).ok_or_else(|| {
                benchmark_error("benchmark measurement set is internally incomplete")
            })?)
            .ok_or_else(|| benchmark_error("benchmark measurement total overflowed"))
    })
}

fn fixed_share_millionths(
    numerator: u128,
    denominator: u128,
) -> Result<u64, ColdProcessBenchmarkError> {
    u64::try_from(
        numerator
            .checked_mul(1_000_000)
            .ok_or_else(|| benchmark_error("fixed-point share overflowed"))?
            / denominator,
    )
    .map_err(|_| benchmark_error("fixed-point share exceeds report range"))
}

fn per_second_millionths(
    units: u64,
    total_end_to_end_micros: u128,
) -> Result<u64, ColdProcessBenchmarkError> {
    u64::try_from(
        u128::from(units)
            .checked_mul(1_000_000_000_000)
            .ok_or_else(|| benchmark_error("fixed-point throughput overflowed"))?
            / total_end_to_end_micros,
    )
    .map_err(|_| benchmark_error("fixed-point throughput exceeds its report range"))
}

fn percentile(sorted: &[u128], percentage: usize) -> u128 {
    let index = (sorted.len() * percentage).div_ceil(100).saturating_sub(1);
    sorted[index]
}
