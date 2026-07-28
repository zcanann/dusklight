use super::*;

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
