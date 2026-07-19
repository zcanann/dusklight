use super::ColdProcessBenchmarkAttempt;
use crate::{ColdProcessBenchmarkError, benchmark_error};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ColdProcessNativePhaseBreakdown {
    pub process_startup: u64,
    pub stage_loading: u64,
    pub simulation: u64,
    pub artifact_flush: u64,
    pub teardown: u64,
    pub process_envelope_overhead: u64,
}

impl ColdProcessNativePhaseBreakdown {
    pub(super) fn from_attempt(attempt: &ColdProcessBenchmarkAttempt) -> Self {
        let timing = &attempt.native_phases;
        Self {
            process_startup: timing.engine_ready_micros - timing.process_entry_micros,
            stage_loading: timing.stage_ready_micros - timing.engine_ready_micros,
            simulation: timing.last_simulation_tick_micros - timing.stage_ready_micros,
            artifact_flush: timing.proof_artifacts_written_micros
                - timing.last_simulation_tick_micros,
            teardown: timing.exit_ready_micros - timing.proof_artifacts_written_micros,
            process_envelope_overhead: attempt
                .native_process_millis
                .saturating_mul(1_000)
                .saturating_sub(timing.exit_ready_micros),
        }
    }

    pub(super) fn checked_add(&mut self, other: &Self) -> Option<()> {
        self.process_startup = self.process_startup.checked_add(other.process_startup)?;
        self.stage_loading = self.stage_loading.checked_add(other.stage_loading)?;
        self.simulation = self.simulation.checked_add(other.simulation)?;
        self.artifact_flush = self.artifact_flush.checked_add(other.artifact_flush)?;
        self.teardown = self.teardown.checked_add(other.teardown)?;
        self.process_envelope_overhead = self
            .process_envelope_overhead
            .checked_add(other.process_envelope_overhead)?;
        Some(())
    }

    pub(super) fn shares(&self, total: u64) -> Result<Self, ColdProcessBenchmarkError> {
        if total == 0 {
            return Err(benchmark_error("native lifecycle timing has zero duration"));
        }
        let share = |value: u64| {
            value
                .checked_mul(1_000_000)
                .map(|scaled| scaled / total)
                .ok_or_else(|| benchmark_error("native phase share overflowed"))
        };
        Ok(Self {
            process_startup: share(self.process_startup)?,
            stage_loading: share(self.stage_loading)?,
            simulation: share(self.simulation)?,
            artifact_flush: share(self.artifact_flush)?,
            teardown: share(self.teardown)?,
            process_envelope_overhead: share(self.process_envelope_overhead)?,
        })
    }
}
