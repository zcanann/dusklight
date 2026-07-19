//! Authenticated process-per-run throughput measurement.
//!
//! This crate repeats one already-sealed harness request in isolated native
//! processes and summarizes their verified evidence. It cannot alter gameplay
//! authority, keep an engine alive, rank candidates, or claim that a future
//! reset/session implementation is equivalent.

mod report;
mod runner;

pub use report::{
    ColdProcessBenchmarkAttempt, ColdProcessBenchmarkHost, ColdProcessBenchmarkReport,
    ColdProcessBenchmarkSummary, ColdProcessNativePhaseBreakdown,
};
pub use runner::{ColdProcessBenchmarkConfig, run_cold_process_benchmark};

use std::error::Error;
use std::fmt;

pub const COLD_PROCESS_BENCHMARK_SCHEMA: &str = "dusklight-cold-process-throughput/v2";
pub(crate) const COLD_PROCESS_MODE: &str = "isolated_cold_process";
pub(crate) const MAX_REPETITIONS: u32 = 100;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ColdProcessBenchmarkError {
    message: String,
}

impl fmt::Display for ColdProcessBenchmarkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ColdProcessBenchmarkError {}

pub(crate) fn benchmark_error(message: impl Into<String>) -> ColdProcessBenchmarkError {
    ColdProcessBenchmarkError {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests;
