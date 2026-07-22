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

fn repository_relative(root: &Path, path: &Path) -> Result<String, ResidualCampaignRunnerError> {
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

fn write_exact_or_new(path: &Path, bytes: &[u8]) -> Result<(), ResidualCampaignRunnerError> {
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

#[allow(clippy::too_many_arguments)]
fn append_checkpoint(
    root: &Path,
    campaign: &Path,
    optimization: &OptimizationRequest,
    template: &HarnessRunRequest,
    resume: &OptimizationResumeState,
    generation: u64,
    optimizer: &ResidualCampaignOptimizer,
    archive: &ResidualOutcomeArchive,
) -> Result<OptimizationResumeState, ResidualCampaignRunnerError> {
    let checkpoint = ResidualCampaignCheckpoint::seal(
        optimization,
        template,
        generation,
        resume.completed_candidates,
        optimizer.snapshot()?,
        archive,
    )?;
    let path = campaign
        .join("checkpoints")
        .join(format!("checkpoint-{:08}.json", resume.next_sequence));
    write_exact_or_new(&path, &checkpoint.to_pretty_json()?)?;
    append_optimization_resume_event(
        optimization,
        root,
        OptimizationResumeEvent::OptimizerCheckpoint {
            generation,
            completed_candidates: resume.completed_candidates,
            state: artifact_reference(root, &path)?,
        },
    )
    .map_err(runner_error)
}

fn load_checkpoint(
    root: &Path,
    optimization: &OptimizationRequest,
    template: &HarnessRunRequest,
    resume: &OptimizationResumeState,
) -> Result<ResidualCampaignCheckpoint, ResidualCampaignRunnerError> {
    let reference = &resume
        .latest_optimizer_checkpoint
        .as_ref()
        .ok_or_else(|| runner_message("residual campaign has no checkpoint"))?
        .artifact;
    let checkpoint: ResidualCampaignCheckpoint =
        serde_json::from_slice(&read_artifact(root, reference)?).map_err(runner_error)?;
    checkpoint.validate(optimization, template)?;
    Ok(checkpoint)
}

fn prepare_candidate(
    optimization: &OptimizationRequest,
    generation: u64,
    sample_index: u32,
    genome: dusklight_search::residual_optimizer::ResidualGenome,
    candidate: dusklight_search::residual_action::ResidualCandidate,
    compiled: CompiledResidualCandidate,
) -> Result<PreparedCandidate, ResidualCampaignRunnerError> {
    let digest = candidate.content_sha256.to_string();
    let id = format!("g{generation:06}-s{sample_index:05}-{}", &digest[..12]);
    let proposer_seed = optimization.execution.deterministic_seeds
        [sample_index as usize % optimization.execution.deterministic_seeds.len()];
    let envelope = ResidualCampaignCandidate::seal(
        id,
        generation,
        sample_index,
        proposer_seed,
        genome,
        candidate,
        &compiled,
    )?;
    Ok(PreparedCandidate { envelope, compiled })
}

fn seal_candidate_batch(
    root: &Path,
    campaign: &Path,
    optimization: &OptimizationRequest,
    resume: &OptimizationResumeState,
    prepared: &[PreparedCandidate],
) -> Result<OptimizationResumeState, ResidualCampaignRunnerError> {
    let mut events = Vec::new();
    for candidate in prepared {
        if let Some(existing) = resume
            .candidates
            .iter()
            .find(|row| row.id == candidate.envelope.id)
        {
            let loaded: ResidualCampaignCandidate =
                serde_json::from_slice(&read_artifact(root, &existing.candidate)?)
                    .map_err(runner_error)?;
            loaded.validate()?;
            if loaded != candidate.envelope
                || existing.compiled_tape_sha256 != candidate.compiled.report.realized_tape_sha256
            {
                return Err(runner_message(
                    "journaled candidate differs from deterministic reproposal",
                ));
            }
            continue;
        }
        let json_path = campaign
            .join("candidates")
            .join(format!("{}.json", candidate.envelope.id));
        let tape_path = campaign
            .join("candidates")
            .join(format!("{}.tape", candidate.envelope.id));
        write_exact_or_new(&json_path, &candidate.envelope.to_pretty_json()?)?;
        write_exact_or_new(&tape_path, &candidate.compiled.bytes)?;
        events.push(OptimizationResumeEvent::CandidateSealed {
            candidate_id: candidate.envelope.id.clone(),
            candidate: artifact_reference(root, &json_path)?,
            compiled_tape: artifact_reference(root, &tape_path)?,
            parent_tape_sha256: Some(candidate.envelope.candidate.parent_tape_sha256),
            generation: candidate.envelope.generation,
            proposer_seed: candidate.envelope.proposer_seed,
        });
    }
    if events.is_empty() {
        return Ok(resume.clone());
    }
    append_optimization_resume_events(optimization, root, events).map_err(runner_error)
}

fn load_candidate(
    root: &Path,
    optimization: &OptimizationRequest,
    parent: &InputTape,
    parent_bytes: &[u8],
    row: &OptimizationResumeCandidate,
) -> Result<PreparedCandidate, ResidualCampaignRunnerError> {
    let envelope: ResidualCampaignCandidate =
        serde_json::from_slice(&read_artifact(root, &row.candidate)?).map_err(runner_error)?;
    envelope.validate()?;
    let compiled = compile_residual_candidate_to_horizon(
        parent,
        parent_bytes,
        &envelope.candidate,
        optimization.budgets.exploration_horizon_ticks,
    )
    .map_err(runner_error)?;
    if envelope.id != row.id
        || envelope.generation != row.generation
        || envelope.proposer_seed != row.proposer_seed
        || envelope.compilation != compiled.report
        || compiled.bytes != read_artifact(root, &row.compiled_tape)?
    {
        return Err(runner_message(
            "residual candidate differs from its journaled artifacts",
        ));
    }
    Ok(PreparedCandidate { envelope, compiled })
}

fn load_generation(
    root: &Path,
    optimization: &OptimizationRequest,
    parent: &InputTape,
    parent_bytes: &[u8],
    resume: &OptimizationResumeState,
    generation: u64,
) -> Result<Vec<PreparedCandidate>, ResidualCampaignRunnerError> {
    let mut values = resume
        .candidates
        .iter()
        .filter(|row| row.generation == generation)
        .map(|row| load_candidate(root, optimization, parent, parent_bytes, row))
        .collect::<Result<Vec<_>, _>>()?;
    values.sort_by_key(|candidate| candidate.envelope.sample_index);
    Ok(values)
}

fn load_evaluation(
    root: &Path,
    optimization: &OptimizationRequest,
    template: &HarnessRunRequest,
    row: &OptimizationResumeCandidate,
    candidate: &PreparedCandidate,
) -> Result<ResidualCampaignEvaluation, ResidualCampaignRunnerError> {
    let reference = row
        .result
        .as_ref()
        .ok_or_else(|| runner_message("residual evaluation is not journaled"))?;
    let evaluation: ResidualCampaignEvaluation =
        serde_json::from_slice(&read_artifact(root, reference)?).map_err(runner_error)?;
    evaluation.validate(optimization, template, &candidate.envelope)?;
    Ok(evaluation)
}

#[allow(clippy::too_many_arguments)]
fn replay_completed(
    root: &Path,
    optimization: &OptimizationRequest,
    template: &HarnessRunRequest,
    parent: &InputTape,
    parent_bytes: &[u8],
    resume: &OptimizationResumeState,
    archive: &mut ResidualOutcomeArchive,
) -> Result<(), ResidualCampaignRunnerError> {
    for row in resume.candidates.iter().filter(|row| row.result.is_some()) {
        let candidate = load_candidate(root, optimization, parent, parent_bytes, row)?;
        let evaluation = load_evaluation(root, optimization, template, row, &candidate)?;
        archive
            .record(&candidate.compiled, evaluation.evidence)
            .map_err(runner_error)?;
    }
    Ok(())
}

fn execute_native_attempt(
    root: &Path,
    campaign: &Path,
    config: &ResidualCampaignRunConfig<'_>,
    row: &OptimizationResumeCandidate,
    candidate: &PreparedCandidate,
    repetition: u16,
) -> Result<(ResidualNativeAttempt, HarnessRunRequest, HarnessRunResult), ResidualCampaignRunnerError>
{
    let seed = candidate
        .envelope
        .proposer_seed
        .checked_add(u64::from(repetition - 1))
        .ok_or_else(|| runner_message("residual repetition seed overflowed"))?;
    let tape_path = root.join(&row.compiled_tape.path);
    for trial in 1..=100_u32 {
        let destination_path = campaign
            .join("native")
            .join(&candidate.envelope.id)
            .join(format!("r{repetition:03}-try{trial:03}"));
        let destination = repository_relative(root, &destination_path)?;
        let request_path = campaign
            .join("native-requests")
            .join(&candidate.envelope.id)
            .join(format!("r{repetition:03}-try{trial:03}.json"));
        let request = derive_candidate_request(
            config.harness_template,
            root,
            &tape_path,
            &destination,
            seed,
        )
        .map_err(runner_error)?;
        write_exact_or_new(
            &request_path,
            &request.to_pretty_json().map_err(runner_error)?,
        )?;
        let result_path = destination_path.join("result.json");
        let result = if result_path.is_file() {
            serde_json::from_slice(&fs::read(&result_path).map_err(runner_error)?)
                .map_err(runner_error)?
        } else if destination_path.exists() {
            continue;
        } else {
            execute_request(&request, root, trial).map_err(runner_error)?
        };
        result
            .validate_files(&request, &destination_path)
            .map_err(runner_error)?;
        if !matches!(
            (result.terminal, result.objective.reached),
            (
                dusklight_harness_contracts::run_contract::HarnessTerminalReason::Reached,
                true
            ) | (
                dusklight_harness_contracts::run_contract::HarnessTerminalReason::Exhausted,
                false
            )
        ) {
            continue;
        }
        return Ok((
            ResidualNativeAttempt {
                repetition,
                rng_seed: seed,
                request: artifact_reference(root, &request_path)?,
                request_content_sha256: request.content_sha256,
                result: artifact_reference(root, &result_path)?,
                result_content_sha256: result.content_sha256,
            },
            request,
            result,
        ));
    }
    Err(runner_message(
        "residual candidate exhausted native recovery attempts",
    ))
}

fn execute_candidate(
    root: &Path,
    campaign: &Path,
    config: &ResidualCampaignRunConfig<'_>,
    row: &OptimizationResumeCandidate,
    candidate: &PreparedCandidate,
) -> Result<ResidualCampaignEvaluation, ResidualCampaignRunnerError> {
    let attempts = (1..=config.optimization.execution.repetitions)
        .map(|repetition| {
            execute_native_attempt(root, campaign, config, row, candidate, repetition)
        })
        .collect::<Result<Vec<_>, _>>()?;
    ResidualCampaignEvaluation::from_native(
        config.optimization,
        config.harness_template,
        &candidate.envelope,
        attempts,
    )
    .map_err(Into::into)
}

#[allow(clippy::too_many_arguments)]
fn evaluate_generation(
    root: &Path,
    campaign: &Path,
    config: &ResidualCampaignRunConfig<'_>,
    parent: &InputTape,
    parent_bytes: &[u8],
    resume: &mut OptimizationResumeState,
    archive: &mut ResidualOutcomeArchive,
    generation: u64,
) -> Result<(), ResidualCampaignRunnerError> {
    for candidate in load_generation(
        root,
        config.optimization,
        parent,
        parent_bytes,
        resume,
        generation,
    )? {
        let row = resume
            .candidates
            .iter()
            .find(|row| row.id == candidate.envelope.id)
            .cloned()
            .ok_or_else(|| runner_message("residual candidate is absent from journal"))?;
        if row.result.is_some() {
            continue;
        }
        let evaluation = execute_candidate(root, campaign, config, &row, &candidate)?;
        archive
            .record(&candidate.compiled, evaluation.evidence.clone())
            .map_err(runner_error)?;
        let path = campaign
            .join("evaluations")
            .join(format!("{}.json", candidate.envelope.id));
        write_exact_or_new(&path, &evaluation.to_pretty_json()?)?;
        *resume = append_optimization_resume_event(
            config.optimization,
            root,
            OptimizationResumeEvent::EvaluationCompleted {
                candidate_id: candidate.envelope.id,
                candidate_sha256: row.candidate_sha256,
                result: artifact_reference(root, &path)?,
                simulated_ticks: evaluation.simulated_ticks,
            },
        )
        .map_err(runner_error)?;
    }
    Ok(())
}

fn generation_rank(
    root: &Path,
    config: &ResidualCampaignRunConfig<'_>,
    parent: &InputTape,
    parent_bytes: &[u8],
    resume: &OptimizationResumeState,
    generation: u64,
) -> Result<Vec<Digest>, ResidualCampaignRunnerError> {
    let candidates = load_generation(
        root,
        config.optimization,
        parent,
        parent_bytes,
        resume,
        generation,
    )?;
    let evaluations = candidates
        .iter()
        .map(|candidate| {
            let row = resume
                .candidates
                .iter()
                .find(|row| row.id == candidate.envelope.id)
                .ok_or_else(|| runner_message("ranked residual candidate is not journaled"))?;
            load_evaluation(
                root,
                config.optimization,
                config.harness_template,
                row,
                candidate,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let inputs = candidates
        .iter()
        .zip(&evaluations)
        .map(|(candidate, evaluation)| ResidualGenerationEvaluation {
            compiled: &candidate.compiled,
            evidence: &evaluation.evidence,
        })
        .collect::<Vec<_>>();
    rank_residual_generation(
        &config
            .optimization
            .residual_retention_config()
            .map_err(runner_error)?,
        &inputs,
    )
    .map_err(runner_error)
}

pub fn run_residual_campaign(
    config: &ResidualCampaignRunConfig<'_>,
) -> Result<ResidualCampaignRunSummary, ResidualCampaignRunnerError> {
    let root = config
        .repository_root
        .canonicalize()
        .map_err(runner_error)?;
    config
        .optimization
        .validate_harness_template(&root, config.harness_template)
        .map_err(runner_error)?;
    let incumbent = config
        .optimization
        .incumbent
        .as_ref()
        .ok_or_else(|| runner_message("residual campaign requires an incumbent tape"))?;
    let parent_bytes = fs::read(root.join(&incumbent.tape.path)).map_err(runner_error)?;
    let parent = InputTape::decode(&parent_bytes).map_err(runner_error)?.tape;
    let campaign = campaign_root(&root, config.optimization)?;
    fs::create_dir_all(&campaign).map_err(runner_error)?;
    let mut resume = if root.join(&config.optimization.resume.journal_path).exists() {
        load_optimization_resume(config.optimization, &root)
    } else {
        initialize_optimization_resume(config.optimization, &root)
    }
    .map_err(runner_error)?;
    if resume.latest_optimizer_checkpoint.is_none() {
        let optimizer = new_optimizer(config.optimization, &parent_bytes)?;
        let archive = ResidualOutcomeArchive::new(
            config
                .optimization
                .residual_retention_config()
                .map_err(runner_error)?,
        )
        .map_err(runner_error)?;
        resume = append_checkpoint(
            &root,
            &campaign,
            config.optimization,
            config.harness_template,
            &resume,
            0,
            &optimizer,
            &archive,
        )?;
    }

    loop {
        let checkpoint =
            load_checkpoint(&root, config.optimization, config.harness_template, &resume)?;
        let optimizer = checkpoint.restore_optimizer(config.optimization, &parent_bytes)?;
        let mut archive = checkpoint.restore_archive()?;
        replay_completed(
            &root,
            config.optimization,
            config.harness_template,
            &parent,
            &parent_bytes,
            &resume,
            &mut archive,
        )?;
        match optimizer {
            ResidualCampaignOptimizer::Random(mut random) => {
                let ResidualOptimizerConfig::Random { samples } =
                    config.optimization.proposal.optimizer
                else {
                    unreachable!()
                };
                if resume.completed_candidates >= samples {
                    return Ok(summary(config, &resume, checkpoint.generation, &archive));
                }
                let generation = checkpoint.generation;
                let generation_count = resume
                    .candidates
                    .iter()
                    .filter(|row| row.generation == generation)
                    .count();
                let produced =
                    random.snapshot().map_err(runner_error)?.produced_candidates as usize;
                if generation_count == 0 {
                    let count = (samples - resume.candidates.len() as u64)
                        .min(config.optimization.resume.checkpoint_every_candidates)
                        .min(16_384) as usize;
                    let batch = random
                        .sample(&parent, &parent_bytes, count)
                        .map_err(runner_error)?;
                    let prepared = prepare_batch(
                        config.optimization,
                        &parent,
                        &parent_bytes,
                        generation,
                        batch,
                    )?;
                    resume = seal_candidate_batch(
                        &root,
                        &campaign,
                        config.optimization,
                        &resume,
                        &prepared,
                    )?;
                    let optimizer = ResidualCampaignOptimizer::Random(random);
                    resume = append_checkpoint(
                        &root,
                        &campaign,
                        config.optimization,
                        config.harness_template,
                        &resume,
                        generation,
                        &optimizer,
                        &archive,
                    )?;
                    continue;
                }
                if produced < resume.candidates.len() {
                    let batch = random
                        .sample(&parent, &parent_bytes, resume.candidates.len() - produced)
                        .map_err(runner_error)?;
                    let prepared = prepare_batch(
                        config.optimization,
                        &parent,
                        &parent_bytes,
                        generation,
                        batch,
                    )?;
                    resume = seal_candidate_batch(
                        &root,
                        &campaign,
                        config.optimization,
                        &resume,
                        &prepared,
                    )?;
                }
                evaluate_generation(
                    &root,
                    &campaign,
                    config,
                    &parent,
                    &parent_bytes,
                    &mut resume,
                    &mut archive,
                    generation,
                )?;
                let optimizer = ResidualCampaignOptimizer::Random(random);
                resume = append_checkpoint(
                    &root,
                    &campaign,
                    config.optimization,
                    config.harness_template,
                    &resume,
                    generation + 1,
                    &optimizer,
                    &archive,
                )?;
            }
            ResidualCampaignOptimizer::Cem(mut cem) => {
                let ResidualOptimizerConfig::Cem { generations, .. } =
                    config.optimization.proposal.optimizer
                else {
                    unreachable!()
                };
                let state = cem.snapshot().map_err(runner_error)?;
                let generation = u64::from(state.generation);
                if state.pending.is_empty() && generation >= u64::from(generations) {
                    return Ok(summary(config, &resume, generation, &archive));
                }
                if state.pending.is_empty() {
                    let batch = cem.ask(&parent, &parent_bytes).map_err(runner_error)?;
                    let prepared = prepare_batch(
                        config.optimization,
                        &parent,
                        &parent_bytes,
                        generation,
                        batch,
                    )?;
                    resume = seal_candidate_batch(
                        &root,
                        &campaign,
                        config.optimization,
                        &resume,
                        &prepared,
                    )?;
                    let optimizer = ResidualCampaignOptimizer::Cem(cem);
                    resume = append_checkpoint(
                        &root,
                        &campaign,
                        config.optimization,
                        config.harness_template,
                        &resume,
                        generation,
                        &optimizer,
                        &archive,
                    )?;
                    continue;
                }
                let actual = resume
                    .candidates
                    .iter()
                    .filter(|row| row.generation == generation)
                    .count();
                if actual != state.pending.len() {
                    return Err(runner_message(
                        "pending CEM checkpoint differs from its atomic candidate batch",
                    ));
                }
                evaluate_generation(
                    &root,
                    &campaign,
                    config,
                    &parent,
                    &parent_bytes,
                    &mut resume,
                    &mut archive,
                    generation,
                )?;
                let ranked =
                    generation_rank(&root, config, &parent, &parent_bytes, &resume, generation)?;
                cem.tell(&ranked).map_err(runner_error)?;
                let optimizer = ResidualCampaignOptimizer::Cem(cem);
                resume = append_checkpoint(
                    &root,
                    &campaign,
                    config.optimization,
                    config.harness_template,
                    &resume,
                    generation + 1,
                    &optimizer,
                    &archive,
                )?;
            }
        }
    }
}

fn prepare_batch(
    optimization: &OptimizationRequest,
    parent: &InputTape,
    parent_bytes: &[u8],
    generation: u64,
    batch: dusklight_search::residual_optimizer::ResidualProposalBatch,
) -> Result<Vec<PreparedCandidate>, ResidualCampaignRunnerError> {
    batch
        .proposals
        .into_iter()
        .map(|proposal| {
            let compiled = compile_residual_candidate_to_horizon(
                parent,
                parent_bytes,
                &proposal.candidate,
                optimization.budgets.exploration_horizon_ticks,
            )
            .map_err(runner_error)?;
            prepare_candidate(
                optimization,
                generation,
                proposal.sample_index,
                proposal.genome,
                proposal.candidate,
                compiled,
            )
        })
        .collect()
}

fn summary(
    config: &ResidualCampaignRunConfig<'_>,
    resume: &OptimizationResumeState,
    generation: u64,
    archive: &ResidualOutcomeArchive,
) -> ResidualCampaignRunSummary {
    ResidualCampaignRunSummary {
        schema: "dusklight-residual-campaign-run-summary/v1",
        optimization_request_sha256: config.optimization.content_sha256,
        harness_template_sha256: config.harness_template.content_sha256,
        completed: true,
        generation,
        sealed_candidates: resume.candidates.len() as u64,
        completed_candidates: resume.completed_candidates,
        charged_simulated_ticks: resume.charged_simulated_ticks,
        retained_successes: archive.successes().len() as u64,
        retained_failures: archive.failures().len() as u64,
        best_first_hit_tick: archive
            .successes()
            .first()
            .map(|success| success.first_hit_tick),
        resume_state: config.optimization.resume.state_path.clone(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn repository() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../..")
            .canonicalize()
            .unwrap()
    }

    #[test]
    fn crash_before_optimizer_checkpoint_reproposes_without_repeating_candidates() {
        let root = repository();
        let checked = root.join(
            "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json",
        );
        let mut optimization: OptimizationRequest =
            serde_json::from_slice(&fs::read(checked).unwrap()).unwrap();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let relative = format!(
            "build/campaigns/residual-crash-window-{}-{nonce}",
            std::process::id()
        );
        optimization.resume.state_path = format!("{relative}/state.json");
        optimization.resume.journal_path = format!("{relative}/journal.jsonl");
        optimization.content_sha256 = Digest::ZERO;
        optimization.refresh_content_sha256().unwrap();
        optimization.validate_files(&root).unwrap();
        let incumbent = optimization.incumbent.as_ref().unwrap();
        let parent_bytes = fs::read(root.join(&incumbent.tape.path)).unwrap();
        let parent = InputTape::decode(&parent_bytes).unwrap().tape;
        let campaign = root.join(&relative);
        let resume = initialize_optimization_resume(&optimization, &root).unwrap();
        let mut first = new_optimizer(&optimization, &parent_bytes).unwrap();
        let ResidualCampaignOptimizer::Cem(first) = &mut first else {
            panic!("checked campaign is not CEM");
        };
        let batch = first.ask(&parent, &parent_bytes).unwrap();
        let prepared = prepare_batch(&optimization, &parent, &parent_bytes, 0, batch).unwrap();
        let sealed =
            seal_candidate_batch(&root, &campaign, &optimization, &resume, &prepared).unwrap();
        assert_eq!(sealed.candidates.len(), 64);
        assert_eq!(sealed.record_count, 64);

        let mut recovered = new_optimizer(&optimization, &parent_bytes).unwrap();
        let ResidualCampaignOptimizer::Cem(recovered) = &mut recovered else {
            panic!("checked campaign is not CEM");
        };
        let reproposed = recovered.ask(&parent, &parent_bytes).unwrap();
        let reproposed =
            prepare_batch(&optimization, &parent, &parent_bytes, 0, reproposed).unwrap();
        let adopted =
            seal_candidate_batch(&root, &campaign, &optimization, &sealed, &reproposed).unwrap();
        assert_eq!(adopted.record_count, sealed.record_count);
        assert_eq!(adopted.candidates, sealed.candidates);
        fs::remove_dir_all(campaign).unwrap();
    }
}
