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

fn new_optimizer(
    optimization: &OptimizationRequest,
    parent_bytes: &[u8],
) -> Result<ResidualCampaignOptimizer, ResidualCampaignRunnerError> {
    match optimization.proposal.optimizer {
        ResidualOptimizerConfig::Random { .. } => Ok(ResidualCampaignOptimizer::Random(
            dusklight_search::residual_optimizer::ResidualRandomSampler::new(
                optimization.proposal.search_space.clone(),
                parent_bytes,
                optimization.execution.deterministic_seeds[0],
            )
            .map_err(runner_error)?,
        )),
        ResidualOptimizerConfig::Cem {
            population,
            elites,
            smoothing_millionths,
            ..
        } => Ok(ResidualCampaignOptimizer::Cem(
            ResidualCemOptimizer::new(
                optimization.proposal.search_space.clone(),
                parent_bytes,
                ResidualCemConfig {
                    population: population as usize,
                    elites: elites as usize,
                    smoothing_millionths,
                    seed: optimization.execution.deterministic_seeds[0],
                },
            )
            .map_err(runner_error)?,
        )),
    }
}

fn campaign_root(
    root: &Path,
    optimization: &OptimizationRequest,
) -> Result<PathBuf, ResidualCampaignRunnerError> {
    let relative = Path::new(&optimization.resume.state_path)
        .parent()
        .ok_or_else(|| runner_message("residual resume state has no campaign directory"))?;
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(runner_message(
            "residual campaign directory is not repository relative",
        ));
    }
    Ok(root.join(relative))
}

fn artifact_reference(
    root: &Path,
    path: &Path,
) -> Result<ArtifactReference, ResidualCampaignRunnerError> {
    let bytes = fs::read(path).map_err(runner_error)?;
    Ok(ArtifactReference {
        path: repository_relative(root, path)?,
        sha256: sha256(&bytes),
    })
}

fn read_artifact(
    root: &Path,
    reference: &ArtifactReference,
) -> Result<Vec<u8>, ResidualCampaignRunnerError> {
    let bytes = fs::read(root.join(&reference.path)).map_err(runner_error)?;
    if sha256(&bytes) != reference.sha256 {
        return Err(runner_message("residual campaign artifact digest differs"));
    }
    Ok(bytes)
}

fn repository_relative(
    root: &Path,
    path: &Path,
) -> Result<String, ResidualCampaignRunnerError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| runner_message("residual campaign path is outside the repository"))?;
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(runner_message("residual campaign path is not canonical"));
    }
    relative
        .to_str()
        .map(|value| value.replace(std::path::MAIN_SEPARATOR, "/"))
        .ok_or_else(|| runner_message("residual campaign path is not UTF-8"))
}

fn write_exact_or_new(
    path: &Path,
    bytes: &[u8],
) -> Result<(), ResidualCampaignRunnerError> {
    if path.exists() {
        if fs::read(path).map_err(runner_error)? != bytes {
            return Err(runner_message(format!(
                "existing residual artifact differs: {}",
                path.display()
            )));
        }
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(runner_error)?;
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(runner_error)?;
    output.write_all(bytes).map_err(runner_error)?;
    output.sync_all().map_err(runner_error)?;
    Ok(())
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
