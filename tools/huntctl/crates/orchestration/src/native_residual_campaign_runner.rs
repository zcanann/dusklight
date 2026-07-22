//! Resumable residual optimization over a campaign-owned native checkpoint pool.

use crate::native_residual_campaign::{
    NativeResidualAttempt, NativeResidualCampaignEvaluation, NativeResidualExecutionBinding,
};
use crate::native_suffix_result::{
    NativeTerminalBinding, ValidatedNativeSuffixBatch, ValidatedNativeSuffixCandidate,
};
use crate::native_suffix_worker::{
    NativeSuffixWorkerLaunch, NativeSuffixWorkerSession, validate_native_suffix_artifacts,
};
use crate::optimization_request::{OptimizationRequest, ResidualOptimizerConfig};
use crate::optimization_resume::{
    OptimizationResumeEvent, OptimizationResumeState, append_optimization_resume_events,
    initialize_optimization_resume, load_optimization_resume,
};
use crate::residual_campaign::{ResidualCampaignCheckpoint, ResidualCampaignOptimizer};
use crate::residual_campaign_runner::{
    PreparedCandidate, ResidualCampaignRunSummary, append_checkpoint, artifact_reference,
    campaign_root, load_candidate, load_checkpoint, load_generation, new_optimizer, prepare_batch,
    read_artifact, seal_candidate_batch, write_exact_or_new,
};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::InputTape;
use dusklight_search::residual_retention::{
    ResidualGenerationEvaluation, ResidualOutcomeArchive, rank_residual_generation,
};
use dusklight_search::search::Candidate;
use dusklight_search::suffix_batch::{
    NATIVE_SUFFIX_BATCH_SCHEMA, NativeCheckpointValidation, NativeSuffixBatch,
    NativeSuffixCandidate,
};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
pub struct NativeResidualCampaignRunConfig<'a> {
    pub repository_root: &'a Path,
    pub optimization: &'a OptimizationRequest,
    pub execution: &'a NativeResidualExecutionBinding,
}

struct WorkerLane {
    index: usize,
    seed: u64,
    session: Option<NativeSuffixWorkerSession>,
    state_root: PathBuf,
}

struct WorkerPool<'a> {
    root: &'a Path,
    campaign: &'a Path,
    optimization: &'a OptimizationRequest,
    execution: &'a NativeResidualExecutionBinding,
    terminal: NativeTerminalBinding,
    lanes: Vec<WorkerLane>,
}

struct BatchJob {
    lane: usize,
    request_path: PathBuf,
    result_path: PathBuf,
    batch: NativeSuffixBatch,
}

struct BatchOutput {
    lane: usize,
    request_path: PathBuf,
    result_path: PathBuf,
    validated: ValidatedNativeSuffixBatch,
}

impl<'a> WorkerPool<'a> {
    fn new(
        root: &'a Path,
        campaign: &'a Path,
        optimization: &'a OptimizationRequest,
        execution: &'a NativeResidualExecutionBinding,
    ) -> Result<Self, NativeResidualCampaignRunnerError> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(native_error)?
            .as_nanos();
        let session_root = campaign
            .join("native-sessions")
            .join(format!("run-{}-{nonce}", std::process::id()));
        let lanes = optimization
            .execution
            .deterministic_seeds
            .iter()
            .enumerate()
            .map(|(index, seed)| WorkerLane {
                index,
                seed: *seed,
                session: None,
                state_root: session_root.join(format!("worker-{index:03}")),
            })
            .collect();
        Ok(Self {
            root,
            campaign,
            optimization,
            execution,
            terminal: NativeTerminalBinding {
                goal: optimization.terminal_predicate.goal.clone(),
                program_sha256: optimization.terminal_predicate.program_sha256,
                definition_sha256: optimization.terminal_predicate.definition_sha256,
            },
            lanes,
        })
    }

    fn run_jobs(
        &mut self,
        jobs: Vec<BatchJob>,
    ) -> Result<Vec<BatchOutput>, NativeResidualCampaignRunnerError> {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }
        let mut by_lane = jobs
            .into_iter()
            .map(|job| (job.lane, job))
            .collect::<BTreeMap<_, _>>();
        let root = self.root;
        let campaign = self.campaign;
        let optimization = self.optimization;
        let execution = self.execution;
        let terminal = &self.terminal;
        let outputs = std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for lane in &mut self.lanes {
                let Some(job) = by_lane.remove(&lane.index) else {
                    continue;
                };
                handles.push(scope.spawn(move || {
                    run_lane_job(root, campaign, optimization, execution, terminal, lane, job)
                }));
            }
            handles
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .map_err(|_| native_message("native residual worker thread panicked"))?
                })
                .collect::<Result<Vec<_>, _>>()
        })?;
        if !by_lane.is_empty() {
            return Err(native_message(
                "native batch targeted an unknown worker lane",
            ));
        }
        let mut hello: Option<&dusklight_worker_protocol::client::HelloResponse> = None;
        for lane in &self.lanes {
            let Some(session) = &lane.session else {
                continue;
            };
            if let Some(expected) = hello {
                let differences = expected.identity_differences(session.hello());
                if !differences.is_empty() {
                    return Err(native_message(format!(
                        "native residual worker pool build identity differs: {}",
                        differences
                            .iter()
                            .map(|difference| difference.message())
                            .collect::<Vec<_>>()
                            .join("; ")
                    )));
                }
            } else {
                hello = Some(session.hello());
            }
        }
        Ok(outputs)
    }

    fn shutdown(&mut self) -> Result<(), NativeResidualCampaignRunnerError> {
        let mut failures = Vec::new();
        for lane in &mut self.lanes {
            if let Some(session) = lane.session.take()
                && let Err(error) = session.shutdown()
            {
                failures.push(format!("worker {}: {error}", lane.index));
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(native_message(format!(
                "native residual worker shutdown failed: {}",
                failures.join("; ")
            )))
        }
    }
}

fn run_lane_job(
    root: &Path,
    _campaign: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    terminal: &NativeTerminalBinding,
    lane: &mut WorkerLane,
    job: BatchJob,
) -> Result<BatchOutput, NativeResidualCampaignRunnerError> {
    write_exact_or_new(
        &job.request_path,
        &pretty_json(&job.batch).map_err(native_error)?,
    )
    .map_err(native_error)?;
    let validated = if let Some(session) = &mut lane.session {
        session
            .run_batch(&job.request_path, &job.result_path, None)
            .map_err(native_error)?
    } else {
        let launch = NativeSuffixWorkerLaunch {
            executable: root.join(&execution.executable.path),
            game_data: root.join(&execution.game_data.path),
            input_tape: root.join(&execution.process_boot_tape.path),
            milestone_program: root.join(&execution.milestone_program.path),
            working_directory: root.to_path_buf(),
            state_root: lane.state_root.clone(),
            world_context_sha256: execution.world_context.sha256,
            terminal: terminal.clone(),
            initial_batch: job.request_path.clone(),
            initial_result: job.result_path.clone(),
            initial_winner_tape: None,
        };
        let (session, validated) =
            NativeSuffixWorkerSession::launch(&launch).map_err(native_error)?;
        let identity = session.identity();
        if identity.source_frame != optimization.route.source_boundary_index
            || identity.source_boundary_fingerprint
                != optimization.route.source_boundary_fingerprint
            || identity.maximum_ticks != optimization.budgets.exploration_horizon_ticks
            || identity.checkpoint_validation_ticks != execution.checkpoint_validation_ticks
            || identity.world_context_sha256 != execution.world_context.sha256
            || identity.terminal != *terminal
        {
            return Err(native_message(
                "native residual worker identity differs from its sealed execution",
            ));
        }
        lane.session = Some(session);
        validated
    };
    Ok(BatchOutput {
        lane: lane.index,
        request_path: job.request_path,
        result_path: job.result_path,
        validated,
    })
}

fn native_batch(
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    segment: dusklight_search::search::SegmentProfile,
    candidates: &[&PreparedCandidate],
    repetition: u16,
) -> Result<NativeSuffixBatch, NativeResidualCampaignRunnerError> {
    let native_candidates = candidates
        .iter()
        .map(|candidate| {
            let imported = Candidate::from_absolute_tape(segment, &candidate.compiled.tape)
                .map_err(native_error)?;
            Ok(NativeSuffixCandidate {
                id: wire_candidate_id(&candidate.envelope.id, repetition),
                actions: imported.actions,
            })
        })
        .collect::<Result<Vec<_>, NativeResidualCampaignRunnerError>>()?;
    Ok(NativeSuffixBatch {
        schema: NATIVE_SUFFIX_BATCH_SCHEMA.into(),
        source_frame: usize::try_from(optimization.route.source_boundary_index)
            .map_err(native_error)?,
        source_boundary_fingerprint: optimization.route.source_boundary_fingerprint.clone(),
        checkpoint_validation: NativeCheckpointValidation {
            kind: "recorded_replay_window".into(),
            ticks: usize::try_from(execution.checkpoint_validation_ticks).map_err(native_error)?,
        },
        maximum_ticks: usize::try_from(optimization.budgets.exploration_horizon_ticks)
            .map_err(native_error)?,
        verify_state_hashes: execution.verify_state_hashes,
        candidates: native_candidates,
    })
}

fn wire_candidate_id(candidate_id: &str, repetition: u16) -> String {
    format!("{candidate_id}-r{repetition:03}")
}

fn segment_profile(
    root: &Path,
    optimization: &OptimizationRequest,
) -> Result<dusklight_search::search::SegmentProfile, NativeResidualCampaignRunnerError> {
    let timeline = dusklight_routes::timeline::Timeline::parse(
        &fs::read_to_string(root.join(&optimization.route.timeline.path)).map_err(native_error)?,
    )
    .map_err(native_error)?;
    timeline
        .segments
        .get(&optimization.route.segment)
        .map(|segment| segment.profile)
        .ok_or_else(|| native_message("native residual segment is absent from its timeline"))
}

fn load_native_evaluation(
    root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    row: &crate::optimization_resume::OptimizationResumeCandidate,
    candidate: &PreparedCandidate,
) -> Result<NativeResidualCampaignEvaluation, NativeResidualCampaignRunnerError> {
    let reference = row
        .result
        .as_ref()
        .ok_or_else(|| native_message("native residual evaluation is not journaled"))?;
    let evaluation: NativeResidualCampaignEvaluation =
        serde_json::from_slice(&read_artifact(root, reference).map_err(native_error)?)
            .map_err(native_error)?;
    evaluation
        .validate(optimization, execution, &candidate.envelope)
        .map_err(native_error)?;
    validate_evaluation_artifacts(root, optimization, &evaluation)?;
    Ok(evaluation)
}

fn validate_evaluation_artifacts(
    root: &Path,
    optimization: &OptimizationRequest,
    evaluation: &NativeResidualCampaignEvaluation,
) -> Result<(), NativeResidualCampaignRunnerError> {
    let terminal = NativeTerminalBinding {
        goal: optimization.terminal_predicate.goal.clone(),
        program_sha256: optimization.terminal_predicate.program_sha256,
        definition_sha256: optimization.terminal_predicate.definition_sha256,
    };
    for attempt in &evaluation.attempts {
        let batch: NativeSuffixBatch = serde_json::from_slice(
            &read_artifact(root, &attempt.batch_request).map_err(native_error)?,
        )
        .map_err(native_error)?;
        let result_path = root.join(&attempt.batch_result.path);
        if artifact_reference(root, &result_path).map_err(native_error)? != attempt.batch_result {
            return Err(native_message(
                "native residual batch result artifact digest differs",
            ));
        }
        let validated = validate_native_suffix_artifacts(&batch, &result_path, &terminal)
            .map_err(native_error)?;
        let candidate = validated
            .candidates
            .iter()
            .find(|candidate| candidate.id == attempt.wire_candidate_id)
            .ok_or_else(|| native_message("native residual attempt is absent from its batch"))?;
        let episode = artifact_reference(root, Path::new(&validated.episode_shard_path))
            .map_err(native_error)?;
        if episode != attempt.episode_shard
            || validated.restore_identity != attempt.restore_identity
            || validated.checkpoint_bytes != attempt.checkpoint_bytes
            || candidate.simulated_ticks != attempt.simulated_ticks
            || candidate.first_hit_tick != attempt.first_hit_tick
            || candidate.behavior_sha256 != attempt.behavior_sha256
        {
            return Err(native_message(
                "native residual attempt differs from its validated batch artifacts",
            ));
        }
    }
    Ok(())
}

fn replay_completed(
    root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    parent: &InputTape,
    parent_bytes: &[u8],
    resume: &OptimizationResumeState,
    archive: &mut ResidualOutcomeArchive,
) -> Result<(), NativeResidualCampaignRunnerError> {
    for row in resume.candidates.iter().filter(|row| row.result.is_some()) {
        let candidate =
            load_candidate(root, optimization, parent, parent_bytes, row).map_err(native_error)?;
        let evaluation = load_native_evaluation(root, optimization, execution, row, &candidate)?;
        archive
            .record(&candidate.compiled, evaluation.evidence)
            .map_err(native_error)?;
    }
    Ok(())
}

fn existing_evaluation(
    root: &Path,
    path: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    candidate: &PreparedCandidate,
) -> Result<Option<NativeResidualCampaignEvaluation>, NativeResidualCampaignRunnerError> {
    if !path.exists() {
        return Ok(None);
    }
    let evaluation: NativeResidualCampaignEvaluation =
        serde_json::from_slice(&fs::read(path).map_err(native_error)?).map_err(native_error)?;
    evaluation
        .validate(optimization, execution, &candidate.envelope)
        .map_err(native_error)?;
    validate_evaluation_artifacts(root, optimization, &evaluation)?;
    Ok(Some(evaluation))
}

fn batch_group_id(batch: &NativeSuffixBatch) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.native-residual-batch-group/v1\0");
    for candidate in &batch.candidates {
        hasher.update((candidate.id.len() as u64).to_le_bytes());
        hasher.update(candidate.id.as_bytes());
    }
    let digest: [u8; 32] = hasher.finalize().into();
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn select_result_path(
    batch_root: &Path,
    batch: &NativeSuffixBatch,
    terminal: &NativeTerminalBinding,
) -> Result<(PathBuf, Option<ValidatedNativeSuffixBatch>), NativeResidualCampaignRunnerError> {
    for trial in 1..=100_u32 {
        let result = batch_root.join(format!("result-try{trial:03}.json"));
        if result.is_file() {
            match validate_native_suffix_artifacts(batch, &result, terminal) {
                Ok(validated) => return Ok((result, Some(validated))),
                Err(_) => continue,
            }
        }
        let mut episode = result.as_os_str().to_os_string();
        episode.push(".episodes.dseps");
        if !result.exists() && !Path::new(&episode).exists() {
            return Ok((result, None));
        }
    }
    Err(native_message(
        "native residual batch exhausted crash-recovery result paths",
    ))
}

#[allow(clippy::too_many_arguments)]
fn evaluate_generation(
    root: &Path,
    campaign: &Path,
    config: &NativeResidualCampaignRunConfig<'_>,
    parent: &InputTape,
    parent_bytes: &[u8],
    resume: &mut OptimizationResumeState,
    archive: &mut ResidualOutcomeArchive,
    pool: &mut WorkerPool<'_>,
    generation: u64,
) -> Result<(), NativeResidualCampaignRunnerError> {
    let candidates = load_generation(
        root,
        config.optimization,
        parent,
        parent_bytes,
        resume,
        generation,
    )
    .map_err(native_error)?;
    let pending = candidates
        .iter()
        .filter(|candidate| {
            resume
                .candidates
                .iter()
                .find(|row| row.id == candidate.envelope.id)
                .is_some_and(|row| row.result.is_none())
        })
        .collect::<Vec<_>>();
    if pending.is_empty() {
        return Ok(());
    }

    let mut evaluations = BTreeMap::new();
    let mut dispatch = Vec::new();
    for candidate in &pending {
        let path = campaign
            .join("evaluations")
            .join(format!("{}.json", candidate.envelope.id));
        if let Some(evaluation) = existing_evaluation(
            root,
            &path,
            config.optimization,
            config.execution,
            candidate,
        )? {
            evaluations.insert(candidate.envelope.id.clone(), evaluation);
        } else {
            dispatch.push(*candidate);
        }
    }

    if !dispatch.is_empty() {
        let profile = segment_profile(root, config.optimization)?;
        let lane_count = pool.lanes.len();
        let mut attempts = dispatch
            .iter()
            .map(|candidate| (candidate.envelope.id.clone(), Vec::new()))
            .collect::<BTreeMap<_, _>>();
        for repetition in 1..=config.optimization.execution.repetitions {
            let mut groups = vec![Vec::new(); lane_count];
            for (index, candidate) in dispatch.iter().enumerate() {
                groups[index % lane_count].push(*candidate);
            }
            let mut jobs = Vec::new();
            let mut adopted = Vec::new();
            for (lane, group) in groups.iter().enumerate() {
                if group.is_empty() {
                    continue;
                }
                let batch = native_batch(
                    config.optimization,
                    config.execution,
                    profile,
                    group,
                    repetition,
                )?;
                let batch_root = campaign
                    .join("native-batches")
                    .join(format!("generation-{generation:06}"))
                    .join(format!("repetition-{repetition:03}"))
                    .join(format!("worker-{lane:03}"))
                    .join(format!("batch-{}", batch_group_id(&batch)));
                fs::create_dir_all(&batch_root).map_err(native_error)?;
                let request_path = batch_root.join("request.json");
                write_exact_or_new(&request_path, &pretty_json(&batch).map_err(native_error)?)
                    .map_err(native_error)?;
                let (result_path, validated) =
                    select_result_path(&batch_root, &batch, &pool.terminal)?;
                if let Some(validated) = validated {
                    adopted.push(BatchOutput {
                        lane,
                        request_path,
                        result_path,
                        validated,
                    });
                } else {
                    jobs.push(BatchJob {
                        lane,
                        request_path,
                        result_path,
                        batch,
                    });
                }
            }
            let mut outputs = pool.run_jobs(jobs)?;
            outputs.extend(adopted);
            outputs.sort_by_key(|output| output.lane);
            for output in outputs {
                let request =
                    artifact_reference(root, &output.request_path).map_err(native_error)?;
                let result = artifact_reference(root, &output.result_path).map_err(native_error)?;
                let episode =
                    artifact_reference(root, Path::new(&output.validated.episode_shard_path))
                        .map_err(native_error)?;
                for actual in &output.validated.candidates {
                    let candidate_id = actual
                        .id
                        .strip_suffix(&format!("-r{repetition:03}"))
                        .ok_or_else(|| native_message("native wire candidate ID is malformed"))?;
                    let rows = attempts.get_mut(candidate_id).ok_or_else(|| {
                        native_message("native result names an unsealed residual candidate")
                    })?;
                    rows.push(native_attempt(
                        repetition,
                        pool.lanes[output.lane].seed,
                        actual,
                        request.clone(),
                        result.clone(),
                        episode.clone(),
                        &output.validated,
                    ));
                }
            }
        }
        for candidate in dispatch {
            let evaluation = NativeResidualCampaignEvaluation::seal(
                config.optimization,
                config.execution,
                &candidate.envelope,
                attempts.remove(&candidate.envelope.id).ok_or_else(|| {
                    native_message("native candidate lacks completed repetitions")
                })?,
            )
            .map_err(native_error)?;
            let path = campaign
                .join("evaluations")
                .join(format!("{}.json", candidate.envelope.id));
            write_exact_or_new(&path, &evaluation.to_pretty_json().map_err(native_error)?)
                .map_err(native_error)?;
            evaluations.insert(candidate.envelope.id.clone(), evaluation);
        }
    }

    let mut events = Vec::new();
    for candidate in pending {
        let row = resume
            .candidates
            .iter()
            .find(|row| row.id == candidate.envelope.id)
            .ok_or_else(|| native_message("native residual candidate is absent from journal"))?;
        let evaluation = evaluations
            .remove(&candidate.envelope.id)
            .ok_or_else(|| native_message("native residual evaluation is absent"))?;
        let path = campaign
            .join("evaluations")
            .join(format!("{}.json", candidate.envelope.id));
        events.push(OptimizationResumeEvent::EvaluationCompleted {
            candidate_id: candidate.envelope.id.clone(),
            candidate_sha256: row.candidate_sha256,
            result: artifact_reference(root, &path).map_err(native_error)?,
            simulated_ticks: evaluation.simulated_ticks,
        });
    }
    *resume = append_optimization_resume_events(config.optimization, root, events)
        .map_err(native_error)?;
    for candidate in &candidates {
        if let Some(row) = resume
            .candidates
            .iter()
            .find(|row| row.id == candidate.envelope.id)
            && row.result.is_some()
        {
            let evaluation = load_native_evaluation(
                root,
                config.optimization,
                config.execution,
                row,
                candidate,
            )?;
            archive
                .record(&candidate.compiled, evaluation.evidence)
                .map_err(native_error)?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn native_attempt(
    repetition: u16,
    worker_seed: u64,
    candidate: &ValidatedNativeSuffixCandidate,
    batch_request: dusklight_harness_contracts::objective_suite::ArtifactReference,
    batch_result: dusklight_harness_contracts::objective_suite::ArtifactReference,
    episode_shard: dusklight_harness_contracts::objective_suite::ArtifactReference,
    batch: &ValidatedNativeSuffixBatch,
) -> NativeResidualAttempt {
    NativeResidualAttempt {
        repetition,
        worker_seed,
        wire_candidate_id: candidate.id.clone(),
        batch_request,
        batch_result,
        episode_shard,
        restore_identity: batch.restore_identity.clone(),
        checkpoint_bytes: batch.checkpoint_bytes,
        simulated_ticks: candidate.simulated_ticks,
        first_hit_tick: candidate.first_hit_tick,
        behavior_sha256: candidate.behavior_sha256,
    }
}

fn generation_rank(
    root: &Path,
    config: &NativeResidualCampaignRunConfig<'_>,
    parent: &InputTape,
    parent_bytes: &[u8],
    resume: &OptimizationResumeState,
    generation: u64,
) -> Result<Vec<Digest>, NativeResidualCampaignRunnerError> {
    let candidates = load_generation(
        root,
        config.optimization,
        parent,
        parent_bytes,
        resume,
        generation,
    )
    .map_err(native_error)?;
    let evaluations = candidates
        .iter()
        .map(|candidate| {
            let row = resume
                .candidates
                .iter()
                .find(|row| row.id == candidate.envelope.id)
                .ok_or_else(|| native_message("ranked native candidate is not journaled"))?;
            load_native_evaluation(root, config.optimization, config.execution, row, candidate)
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
            .map_err(native_error)?,
        &inputs,
    )
    .map_err(native_error)
}

pub fn run_native_residual_campaign(
    config: &NativeResidualCampaignRunConfig<'_>,
) -> Result<ResidualCampaignRunSummary, NativeResidualCampaignRunnerError> {
    let root = config
        .repository_root
        .canonicalize()
        .map_err(native_error)?;
    config
        .execution
        .validate_files(&root, config.optimization)
        .map_err(native_error)?;
    let incumbent = config
        .optimization
        .incumbent
        .as_ref()
        .ok_or_else(|| native_message("native residual campaign requires an incumbent tape"))?;
    let parent_bytes = fs::read(root.join(&incumbent.tape.path)).map_err(native_error)?;
    let parent = InputTape::decode(&parent_bytes).map_err(native_error)?.tape;
    let campaign = campaign_root(&root, config.optimization).map_err(native_error)?;
    fs::create_dir_all(&campaign).map_err(native_error)?;
    let mut pool = WorkerPool::new(&root, &campaign, config.optimization, config.execution)?;
    let result = run_campaign_loop(config, &root, &campaign, &parent, &parent_bytes, &mut pool);
    let shutdown = pool.shutdown();
    match (result, shutdown) {
        (Ok(summary), Ok(())) => Ok(summary),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

fn run_campaign_loop(
    config: &NativeResidualCampaignRunConfig<'_>,
    root: &Path,
    campaign: &Path,
    parent: &InputTape,
    parent_bytes: &[u8],
    pool: &mut WorkerPool<'_>,
) -> Result<ResidualCampaignRunSummary, NativeResidualCampaignRunnerError> {
    let mut resume = if root.join(&config.optimization.resume.journal_path).exists() {
        load_optimization_resume(config.optimization, root)
    } else {
        initialize_optimization_resume(config.optimization, root)
    }
    .map_err(native_error)?;
    if resume.latest_optimizer_checkpoint.is_none() {
        let optimizer = new_optimizer(config.optimization, parent_bytes).map_err(native_error)?;
        let archive = ResidualOutcomeArchive::new(
            config
                .optimization
                .residual_retention_config()
                .map_err(native_error)?,
        )
        .map_err(native_error)?;
        resume = append_checkpoint(
            root,
            campaign,
            config.optimization,
            config.execution.content_sha256,
            &resume,
            0,
            &optimizer,
            &archive,
        )
        .map_err(native_error)?;
    }
    loop {
        let checkpoint: ResidualCampaignCheckpoint = load_checkpoint(
            root,
            config.optimization,
            config.execution.content_sha256,
            &resume,
        )
        .map_err(native_error)?;
        let optimizer = checkpoint
            .restore_optimizer(config.optimization, parent_bytes)
            .map_err(native_error)?;
        let mut archive = checkpoint.restore_archive().map_err(native_error)?;
        replay_completed(
            root,
            config.optimization,
            config.execution,
            parent,
            parent_bytes,
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
                    random.snapshot().map_err(native_error)?.produced_candidates as usize;
                if generation_count == 0 {
                    let count = (samples - resume.candidates.len() as u64)
                        .min(config.optimization.resume.checkpoint_every_candidates)
                        .min(16_384) as usize;
                    let batch = random
                        .sample(parent, parent_bytes, count)
                        .map_err(native_error)?;
                    let prepared =
                        prepare_batch(config.optimization, parent, parent_bytes, generation, batch)
                            .map_err(native_error)?;
                    resume = seal_candidate_batch(
                        root,
                        campaign,
                        config.optimization,
                        &resume,
                        &prepared,
                    )
                    .map_err(native_error)?;
                    let optimizer = ResidualCampaignOptimizer::Random(random);
                    resume = append_checkpoint(
                        root,
                        campaign,
                        config.optimization,
                        config.execution.content_sha256,
                        &resume,
                        generation,
                        &optimizer,
                        &archive,
                    )
                    .map_err(native_error)?;
                    continue;
                }
                if produced < resume.candidates.len() {
                    let batch = random
                        .sample(parent, parent_bytes, resume.candidates.len() - produced)
                        .map_err(native_error)?;
                    let prepared =
                        prepare_batch(config.optimization, parent, parent_bytes, generation, batch)
                            .map_err(native_error)?;
                    resume = seal_candidate_batch(
                        root,
                        campaign,
                        config.optimization,
                        &resume,
                        &prepared,
                    )
                    .map_err(native_error)?;
                }
                evaluate_generation(
                    root,
                    campaign,
                    config,
                    parent,
                    parent_bytes,
                    &mut resume,
                    &mut archive,
                    pool,
                    generation,
                )?;
                let optimizer = ResidualCampaignOptimizer::Random(random);
                resume = append_checkpoint(
                    root,
                    campaign,
                    config.optimization,
                    config.execution.content_sha256,
                    &resume,
                    generation + 1,
                    &optimizer,
                    &archive,
                )
                .map_err(native_error)?;
            }
            ResidualCampaignOptimizer::Cem(mut cem) => {
                let ResidualOptimizerConfig::Cem { generations, .. } =
                    config.optimization.proposal.optimizer
                else {
                    unreachable!()
                };
                let state = cem.snapshot().map_err(native_error)?;
                let generation = u64::from(state.generation);
                if state.pending.is_empty() && generation >= u64::from(generations) {
                    return Ok(summary(config, &resume, generation, &archive));
                }
                if state.pending.is_empty() {
                    let batch = cem.ask(parent, parent_bytes).map_err(native_error)?;
                    let prepared =
                        prepare_batch(config.optimization, parent, parent_bytes, generation, batch)
                            .map_err(native_error)?;
                    resume = seal_candidate_batch(
                        root,
                        campaign,
                        config.optimization,
                        &resume,
                        &prepared,
                    )
                    .map_err(native_error)?;
                    let optimizer = ResidualCampaignOptimizer::Cem(cem);
                    resume = append_checkpoint(
                        root,
                        campaign,
                        config.optimization,
                        config.execution.content_sha256,
                        &resume,
                        generation,
                        &optimizer,
                        &archive,
                    )
                    .map_err(native_error)?;
                    continue;
                }
                let actual = resume
                    .candidates
                    .iter()
                    .filter(|row| row.generation == generation)
                    .count();
                if actual != state.pending.len() {
                    return Err(native_message(
                        "pending CEM checkpoint differs from its atomic candidate batch",
                    ));
                }
                evaluate_generation(
                    root,
                    campaign,
                    config,
                    parent,
                    parent_bytes,
                    &mut resume,
                    &mut archive,
                    pool,
                    generation,
                )?;
                let ranked =
                    generation_rank(root, config, parent, parent_bytes, &resume, generation)?;
                cem.tell(&ranked).map_err(native_error)?;
                let optimizer = ResidualCampaignOptimizer::Cem(cem);
                resume = append_checkpoint(
                    root,
                    campaign,
                    config.optimization,
                    config.execution.content_sha256,
                    &resume,
                    generation + 1,
                    &optimizer,
                    &archive,
                )
                .map_err(native_error)?;
            }
        }
    }
}

fn summary(
    config: &NativeResidualCampaignRunConfig<'_>,
    resume: &OptimizationResumeState,
    generation: u64,
    archive: &ResidualOutcomeArchive,
) -> ResidualCampaignRunSummary {
    ResidualCampaignRunSummary {
        schema: "dusklight-residual-campaign-run-summary/v2",
        optimization_request_sha256: config.optimization.content_sha256,
        execution_binding_sha256: config.execution.content_sha256,
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

fn pretty_json(value: &impl Serialize) -> Result<Vec<u8>, serde_json::Error> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

#[derive(Debug)]
pub struct NativeResidualCampaignRunnerError(String);

fn native_message(message: impl Into<String>) -> NativeResidualCampaignRunnerError {
    NativeResidualCampaignRunnerError(message.into())
}

fn native_error(error: impl fmt::Display) -> NativeResidualCampaignRunnerError {
    native_message(error.to_string())
}

impl fmt::Display for NativeResidualCampaignRunnerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeResidualCampaignRunnerError {}
