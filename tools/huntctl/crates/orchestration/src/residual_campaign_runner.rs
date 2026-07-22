//! Native proposal, dispatch, retention, and resume loop for residual campaigns.

use crate::optimization_request::{OptimizationRequest, ResidualOptimizerConfig};
use crate::optimization_resume::{
    OptimizationResumeCandidate, OptimizationResumeEvent, OptimizationResumeState,
    append_optimization_resume_event, append_optimization_resume_events,
    initialize_optimization_resume, load_optimization_resume,
};
use crate::residual_campaign::{
    ResidualCampaignCandidate, ResidualCampaignCheckpoint, ResidualCampaignError,
    ResidualCampaignEvaluation, ResidualCampaignOptimizer, ResidualNativeAttempt,
};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::InputTape;
use dusklight_evaluation::derive_candidate_request;
use dusklight_harness_contracts::objective_suite::ArtifactReference;
use dusklight_harness_contracts::run_contract::{HarnessRunRequest, HarnessRunResult};
use dusklight_harness_runtime::execution::execute_request;
use dusklight_search::residual_action::{
    CompiledResidualCandidate, compile_residual_candidate_to_horizon,
};
use dusklight_search::residual_optimizer::{ResidualCemConfig, ResidualCemOptimizer};
use dusklight_search::residual_retention::{
    ResidualGenerationEvaluation, ResidualOutcomeArchive, rank_residual_generation,
};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug)]
pub struct ResidualCampaignRunConfig<'a> {
    pub repository_root: &'a Path,
    pub optimization: &'a OptimizationRequest,
    pub harness_template: &'a HarnessRunRequest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualCampaignRunSummary {
    pub schema: &'static str,
    pub optimization_request_sha256: Digest,
    pub harness_template_sha256: Digest,
    pub completed: bool,
    pub generation: u64,
    pub sealed_candidates: u64,
    pub completed_candidates: u64,
    pub charged_simulated_ticks: u64,
    pub retained_successes: u64,
    pub retained_failures: u64,
    pub best_first_hit_tick: Option<u64>,
    pub resume_state: String,
}

#[derive(Debug)]
struct PreparedCandidate {
    envelope: ResidualCampaignCandidate,
    compiled: CompiledResidualCandidate,
}

fn sha256(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

#[derive(Debug)]
pub struct ResidualCampaignRunnerError(String);

fn runner_message(message: impl Into<String>) -> ResidualCampaignRunnerError {
    ResidualCampaignRunnerError(message.into())
}

fn runner_error(error: impl fmt::Display) -> ResidualCampaignRunnerError {
    runner_message(error.to_string())
}

impl fmt::Display for ResidualCampaignRunnerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ResidualCampaignRunnerError {}

impl From<ResidualCampaignError> for ResidualCampaignRunnerError {
    fn from(error: ResidualCampaignError) -> Self {
        runner_error(error)
    }
}
