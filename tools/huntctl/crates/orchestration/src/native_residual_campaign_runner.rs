//! Resumable residual optimization over a campaign-owned native checkpoint pool.

use crate::campaign_replay::{
    append_incumbent_demonstration_replay, append_residual_replay_generation, load_corpus,
    validate_residual_corpus_scope,
};
use crate::native_residual_campaign::{
    NativeAlternateTerminalEvaluation, NativeIncumbentDemonstration, NativeResidualAttempt,
    NativeResidualCampaignEvaluation, NativeResidualExecutionBinding,
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
use crate::residual_campaign::{
    ResidualCampaignCheckpoint, ResidualCampaignOptimizer, ResidualReplayCheckpoint,
};
use crate::residual_campaign_runner::{
    PreparedCandidate, ResidualCampaignRunSummary, append_checkpoint, artifact_reference,
    campaign_root, load_candidate, load_checkpoint, load_generation, new_optimizer, prepare_batch,
    read_artifact, seal_candidate_batch, write_exact_or_new,
};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::InputTape;
use dusklight_evidence::native_episode_shard::NativeEpisodeShard;
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
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
pub struct NativeResidualCampaignRunConfig<'a> {
    pub repository_root: &'a Path,
    pub optimization: &'a OptimizationRequest,
    pub execution: &'a NativeResidualExecutionBinding,
    /// Cooperative cancellation is observed only at durable orchestration
    /// boundaries. A batch already executing is allowed to finish so its exact
    /// result can be adopted on resume; all workers are shut down before this
    /// runner returns a cancelled outcome.
    pub cancellation: Option<&'a AtomicBool>,
}

struct WorkerLane {
    index: usize,
    seed: u64,
    session: Option<NativeSuffixWorkerSession>,
    state_root: PathBuf,
}

struct WorkerPool<'a> {
    root: &'a Path,
    optimization: &'a OptimizationRequest,
    execution: &'a NativeResidualExecutionBinding,
    terminal: NativeTerminalBinding,
    milestone_program: PathBuf,
    card_fixture_root: PathBuf,
    session_root: PathBuf,
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
        Self::new_for_terminal(
            root,
            campaign,
            optimization,
            execution,
            NativeTerminalBinding {
                goal: optimization.terminal_predicate.goal.clone(),
                program_sha256: optimization.terminal_predicate.program_sha256,
                definition_sha256: optimization.terminal_predicate.definition_sha256,
            },
            root.join(&execution.milestone_program.path),
            "promotion",
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_for_terminal(
        root: &'a Path,
        campaign: &'a Path,
        optimization: &'a OptimizationRequest,
        execution: &'a NativeResidualExecutionBinding,
        terminal: NativeTerminalBinding,
        milestone_program: PathBuf,
        namespace: &str,
    ) -> Result<Self, NativeResidualCampaignRunnerError> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(native_error)?
            .as_nanos();
        let session_root = campaign
            .join("native-sessions")
            .join(namespace)
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
            optimization,
            execution,
            card_fixture_root: execution
                .card_fixture_root(root, optimization)
                .map_err(native_error)?,
            terminal,
            milestone_program,
            session_root,
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
        let optimization = self.optimization;
        let execution = self.execution;
        let terminal = &self.terminal;
        let milestone_program = &self.milestone_program;
        let card_fixture_root = &self.card_fixture_root;
        let outputs = std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for lane in &mut self.lanes {
                let Some(job) = by_lane.remove(&lane.index) else {
                    continue;
                };
                handles.push(scope.spawn(move || {
                    run_lane_job(
                        root,
                        optimization,
                        execution,
                        terminal,
                        milestone_program,
                        card_fixture_root,
                        lane,
                        job,
                    )
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
        match fs::remove_dir_all(&self.session_root) {
            Ok(()) => {
                if let Some(parent) = self.session_root.parent() {
                    let _ = fs::remove_dir(parent);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => failures.push(format!(
                "ephemeral session {}: {error}",
                self.session_root.display()
            )),
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

impl Drop for WorkerPool<'_> {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn alternate_worker_pools<'a>(
    root: &'a Path,
    campaign: &'a Path,
    optimization: &'a OptimizationRequest,
    execution: &'a NativeResidualExecutionBinding,
) -> Result<Vec<WorkerPool<'a>>, NativeResidualCampaignRunnerError> {
    optimization
        .alternate_terminal_predicates(root)
        .map_err(native_error)?
        .into_iter()
        .enumerate()
        .map(|(index, binding)| {
            let source =
                fs::read_to_string(root.join(&binding.source.path)).map_err(native_error)?;
            let program =
                dusklight_objectives::milestone_dsl::parse(&source).map_err(native_error)?;
            let program =
                dusklight_objectives::milestone_dsl::compile(&program).map_err(native_error)?;
            if Digest(program.program_sha256) != binding.program_sha256 {
                return Err(native_message(format!(
                    "alternate terminal {} compiled identity changed after request validation",
                    binding.goal
                )));
            }
            let program_path = campaign
                .join("alternate-terminals")
                .join(format!("{index:03}-{}", binding.goal))
                .join(format!("program-{}.dmsp", binding.program_sha256));
            write_exact_or_new(&program_path, &program.bytes).map_err(native_error)?;
            WorkerPool::new_for_terminal(
                root,
                campaign,
                optimization,
                execution,
                NativeTerminalBinding {
                    goal: binding.goal,
                    program_sha256: binding.program_sha256,
                    definition_sha256: binding.definition_sha256,
                },
                program_path,
                &format!("alternate-{index:03}"),
            )
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn run_lane_job(
    root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    terminal: &NativeTerminalBinding,
    milestone_program: &Path,
    card_fixture_root: &Path,
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
            milestone_program: milestone_program.to_path_buf(),
            card_fixture: card_fixture_root.to_path_buf(),
            card_fixture_sha256: execution.card_fixture_manifest.sha256,
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
                != optimization.route.native_source_boundary_fingerprint
            || identity.maximum_ticks != optimization.budgets.exploration_horizon_ticks
            || identity.checkpoint_validation_ticks != execution.checkpoint_validation_ticks
            || identity.world_context_sha256 != execution.world_context.sha256
            || identity.card_fixture_sha256 != execution.card_fixture_manifest.sha256
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
        source_boundary_fingerprint: optimization
            .route
            .native_source_boundary_fingerprint
            .clone(),
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
        validate_attempt_artifacts(root, &terminal, attempt)?;
    }
    let expected = optimization
        .alternate_terminal_predicates(root)
        .map_err(native_error)?
        .into_iter()
        .map(|binding| NativeTerminalBinding {
            goal: binding.goal,
            program_sha256: binding.program_sha256,
            definition_sha256: binding.definition_sha256,
        })
        .collect::<Vec<_>>();
    let expected = if evaluation
        .attempts
        .first()
        .is_some_and(|attempt| attempt.first_hit_tick.is_none())
    {
        expected
    } else {
        Vec::new()
    };
    if evaluation
        .alternate_terminals
        .iter()
        .map(|alternate| &alternate.terminal)
        .ne(expected.iter())
    {
        return Err(native_message(
            "native residual alternate terminals differ from the sealed optimization request",
        ));
    }
    for alternate in &evaluation.alternate_terminals {
        for attempt in &alternate.attempts {
            validate_attempt_artifacts(root, &alternate.terminal, attempt)?;
        }
    }
    Ok(())
}

fn validate_attempt_artifacts(
    root: &Path,
    terminal: &NativeTerminalBinding,
    attempt: &NativeResidualAttempt,
) -> Result<(), NativeResidualCampaignRunnerError> {
    let batch: NativeSuffixBatch =
        serde_json::from_slice(&read_artifact(root, &attempt.batch_request).map_err(native_error)?)
            .map_err(native_error)?;
    let result_path = root.join(&attempt.batch_result.path);
    if artifact_reference(root, &result_path).map_err(native_error)? != attempt.batch_result {
        return Err(native_message(
            "native residual batch result artifact digest differs",
        ));
    }
    let validated =
        validate_native_suffix_artifacts(&batch, &result_path, terminal).map_err(native_error)?;
    let candidate = validated
        .candidates
        .iter()
        .find(|candidate| candidate.id == attempt.wire_candidate_id)
        .ok_or_else(|| native_message("native residual attempt is absent from its batch"))?;
    let episode =
        artifact_reference(root, Path::new(&validated.episode_shard_path)).map_err(native_error)?;
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
    Ok(())
}

fn replay_completed(
    config: &NativeResidualCampaignRunConfig<'_>,
    root: &Path,
    parent: &InputTape,
    parent_bytes: &[u8],
    resume: &OptimizationResumeState,
    archive: &mut ResidualOutcomeArchive,
) -> Result<(), NativeResidualCampaignRunnerError> {
    for row in resume.candidates.iter().filter(|row| row.result.is_some()) {
        ensure_not_cancelled(config)?;
        let candidate = load_candidate(root, config.optimization, parent, parent_bytes, row)
            .map_err(native_error)?;
        let evaluation =
            load_native_evaluation(root, config.optimization, config.execution, row, &candidate)?;
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
fn execute_native_attempts(
    root: &Path,
    campaign: &Path,
    config: &NativeResidualCampaignRunConfig<'_>,
    profile: dusklight_search::search::SegmentProfile,
    candidates: &[&PreparedCandidate],
    pool: &mut WorkerPool<'_>,
    generation: u64,
    alternate_namespace: Option<&str>,
) -> Result<BTreeMap<String, Vec<NativeResidualAttempt>>, NativeResidualCampaignRunnerError> {
    let lane_count = pool.lanes.len();
    let mut attempts = candidates
        .iter()
        .map(|candidate| (candidate.envelope.id.clone(), Vec::new()))
        .collect::<BTreeMap<_, _>>();
    for repetition in 1..=config.optimization.execution.repetitions {
        ensure_not_cancelled(config)?;
        let mut groups = vec![Vec::new(); lane_count];
        for (index, candidate) in candidates.iter().enumerate() {
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
            let batch_root = alternate_namespace.map_or_else(
                || campaign.join("native-batches"),
                |namespace| campaign.join("alternate-native-batches").join(namespace),
            );
            let batch_root = batch_root
                .join(format!("generation-{generation:06}"))
                .join(format!("repetition-{repetition:03}"))
                .join(format!("worker-{lane:03}"))
                .join(format!("batch-{}", batch_group_id(&batch)));
            fs::create_dir_all(&batch_root).map_err(native_error)?;
            let request_path = batch_root.join("request.json");
            write_exact_or_new(&request_path, &pretty_json(&batch).map_err(native_error)?)
                .map_err(native_error)?;
            let (result_path, validated) = select_result_path(&batch_root, &batch, &pool.terminal)?;
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
        ensure_not_cancelled(config)?;
        let mut outputs = pool.run_jobs(jobs)?;
        ensure_not_cancelled(config)?;
        outputs.extend(adopted);
        outputs.sort_by_key(|output| output.lane);
        for output in outputs {
            let request = artifact_reference(root, &output.request_path).map_err(native_error)?;
            let result = artifact_reference(root, &output.result_path).map_err(native_error)?;
            let episode = artifact_reference(root, Path::new(&output.validated.episode_shard_path))
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
    Ok(attempts)
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
    alternate_pools: &mut [WorkerPool<'_>],
    generation: u64,
) -> Result<(), NativeResidualCampaignRunnerError> {
    ensure_not_cancelled(config)?;
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
        let mut attempts = execute_native_attempts(
            root, campaign, config, profile, &dispatch, pool, generation, None,
        )?;
        let missed = dispatch
            .iter()
            .copied()
            .filter(|candidate| {
                attempts
                    .get(&candidate.envelope.id)
                    .is_some_and(|rows| rows.iter().all(|attempt| attempt.first_hit_tick.is_none()))
            })
            .collect::<Vec<_>>();
        let mut alternate_attempts = dispatch
            .iter()
            .map(|candidate| (candidate.envelope.id.clone(), Vec::new()))
            .collect::<BTreeMap<_, Vec<NativeAlternateTerminalEvaluation>>>();
        if !missed.is_empty() {
            for (index, alternate_pool) in alternate_pools.iter_mut().enumerate() {
                let terminal = alternate_pool.terminal.clone();
                let mut rows = execute_native_attempts(
                    root,
                    campaign,
                    config,
                    profile,
                    &missed,
                    alternate_pool,
                    generation,
                    Some(&format!("{index:03}-{}", terminal.goal)),
                )?;
                for candidate in &missed {
                    alternate_attempts
                        .get_mut(&candidate.envelope.id)
                        .expect("dispatch establishes alternate attempt row")
                        .push(NativeAlternateTerminalEvaluation {
                            terminal: terminal.clone(),
                            attempts: rows.remove(&candidate.envelope.id).ok_or_else(|| {
                                native_message(
                                    "alternate terminal lacks completed candidate repetitions",
                                )
                            })?,
                        });
                }
            }
        }
        for candidate in dispatch {
            ensure_not_cancelled(config)?;
            let evaluation = NativeResidualCampaignEvaluation::seal_with_alternate_terminals(
                config.optimization,
                config.execution,
                &candidate.envelope,
                attempts.remove(&candidate.envelope.id).ok_or_else(|| {
                    native_message("native candidate lacks completed repetitions")
                })?,
                alternate_attempts
                    .remove(&candidate.envelope.id)
                    .expect("dispatch establishes alternate evaluation row"),
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
    ensure_not_cancelled(config)?;
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
        terminal_boundary_fingerprint: candidate.terminal_boundary_fingerprint.clone(),
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

#[allow(clippy::too_many_arguments)]
fn append_generation_replay(
    root: &Path,
    campaign: &Path,
    config: &NativeResidualCampaignRunConfig<'_>,
    parent: &InputTape,
    parent_bytes: &[u8],
    resume: &OptimizationResumeState,
    previous: Option<&ResidualReplayCheckpoint>,
    generation: u64,
) -> Result<ResidualReplayCheckpoint, NativeResidualCampaignRunnerError> {
    let generations = if previous.is_none() {
        resume
            .candidates
            .iter()
            .filter(|row| row.result.is_some())
            .map(|row| row.generation)
            .collect::<BTreeSet<_>>()
    } else {
        BTreeSet::from([generation])
    };
    let mut evaluations = Vec::new();
    for generation in generations {
        let candidates = load_generation(
            root,
            config.optimization,
            parent,
            parent_bytes,
            resume,
            generation,
        )
        .map_err(native_error)?;
        for candidate in &candidates {
            let row = resume
                .candidates
                .iter()
                .find(|row| row.id == candidate.envelope.id)
                .ok_or_else(|| native_message("replay candidate is absent from resume state"))?;
            if row.result.is_some() {
                evaluations.push(load_native_evaluation(
                    root,
                    config.optimization,
                    config.execution,
                    row,
                    candidate,
                )?);
            }
        }
    }
    append_residual_replay_generation(root, campaign, config.optimization, previous, &evaluations)
        .map_err(native_error)
}

fn validate_incumbent_demonstration_artifacts(
    root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    demonstration: &NativeIncumbentDemonstration,
) -> Result<(), NativeResidualCampaignRunnerError> {
    demonstration
        .validate(optimization, execution)
        .map_err(native_error)?;
    let batch: NativeSuffixBatch = serde_json::from_slice(
        &read_artifact(root, &demonstration.attempt.batch_request).map_err(native_error)?,
    )
    .map_err(native_error)?;
    let result_path = root.join(&demonstration.attempt.batch_result.path);
    if artifact_reference(root, &result_path).map_err(native_error)?
        != demonstration.attempt.batch_result
    {
        return Err(native_message(
            "incumbent demonstration result artifact digest differs",
        ));
    }
    let terminal = NativeTerminalBinding {
        goal: optimization.terminal_predicate.goal.clone(),
        program_sha256: optimization.terminal_predicate.program_sha256,
        definition_sha256: optimization.terminal_predicate.definition_sha256,
    };
    let validated =
        validate_native_suffix_artifacts(&batch, &result_path, &terminal).map_err(native_error)?;
    let candidate = validated
        .candidates
        .first()
        .filter(|candidate| {
            validated.candidates.len() == 1
                && candidate.id == demonstration.attempt.wire_candidate_id
        })
        .ok_or_else(|| native_message("incumbent demonstration lacks one exact candidate"))?;
    let episode_reference =
        artifact_reference(root, Path::new(&validated.episode_shard_path)).map_err(native_error)?;
    if episode_reference != demonstration.attempt.episode_shard
        || native_attempt(
            1,
            optimization.execution.deterministic_seeds[0],
            candidate,
            demonstration.attempt.batch_request.clone(),
            demonstration.attempt.batch_result.clone(),
            episode_reference,
            &validated,
        ) != demonstration.attempt
    {
        return Err(native_message(
            "incumbent demonstration differs from its validated native result",
        ));
    }
    let corpus = load_corpus(root, &demonstration.replay.artifact).map_err(native_error)?;
    demonstration
        .replay
        .validate_corpus(&corpus)
        .map_err(native_error)?;
    validate_residual_corpus_scope(optimization, &corpus).map_err(native_error)?;
    if corpus.entries.len() != 1
        || corpus.entries[0].role
            != dusklight_learning::native_replay_corpus::ReplayExperienceRole::Demonstration
        || corpus.entries[0].shard_sha256 != demonstration.attempt.episode_shard.sha256
    {
        return Err(native_message(
            "incumbent demonstration replay differs from its exact native episode",
        ));
    }
    Ok(())
}

fn load_incumbent_demonstration(
    root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    reference: &dusklight_harness_contracts::objective_suite::ArtifactReference,
) -> Result<NativeIncumbentDemonstration, NativeResidualCampaignRunnerError> {
    let demonstration: NativeIncumbentDemonstration =
        serde_json::from_slice(&read_artifact(root, reference).map_err(native_error)?)
            .map_err(native_error)?;
    validate_incumbent_demonstration_artifacts(root, optimization, execution, &demonstration)?;
    Ok(demonstration)
}

#[allow(clippy::too_many_arguments)]
fn ensure_incumbent_demonstration(
    root: &Path,
    campaign: &Path,
    config: &NativeResidualCampaignRunConfig<'_>,
    parent: &InputTape,
    resume: &OptimizationResumeState,
    pool: &mut WorkerPool<'_>,
) -> Result<
    (OptimizationResumeState, NativeIncumbentDemonstration),
    NativeResidualCampaignRunnerError,
> {
    if let Some(reference) = &resume.demonstration {
        return Ok((
            resume.clone(),
            load_incumbent_demonstration(root, config.optimization, config.execution, reference)?,
        ));
    }
    ensure_not_cancelled(config)?;
    let profile = segment_profile(root, config.optimization)?;
    let imported = Candidate::from_absolute_tape(profile, parent).map_err(native_error)?;
    let batch = NativeSuffixBatch {
        schema: NATIVE_SUFFIX_BATCH_SCHEMA.into(),
        source_frame: usize::try_from(config.optimization.route.source_boundary_index)
            .map_err(native_error)?,
        source_boundary_fingerprint: config
            .optimization
            .route
            .native_source_boundary_fingerprint
            .clone(),
        checkpoint_validation: NativeCheckpointValidation {
            kind: "recorded_replay_window".into(),
            ticks: usize::try_from(config.execution.checkpoint_validation_ticks)
                .map_err(native_error)?,
        },
        maximum_ticks: usize::try_from(config.optimization.budgets.exploration_horizon_ticks)
            .map_err(native_error)?,
        verify_state_hashes: config.execution.verify_state_hashes,
        candidates: vec![NativeSuffixCandidate {
            id: "incumbent-demonstration".into(),
            actions: imported.actions,
        }],
    };
    let batch_root = campaign.join("demonstration").join("native");
    fs::create_dir_all(&batch_root).map_err(native_error)?;
    let request_path = batch_root.join("request.json");
    write_exact_or_new(&request_path, &pretty_json(&batch).map_err(native_error)?)
        .map_err(native_error)?;
    let (result_path, adopted) = select_result_path(&batch_root, &batch, &pool.terminal)?;
    let output = if let Some(validated) = adopted {
        BatchOutput {
            lane: 0,
            request_path,
            result_path,
            validated,
        }
    } else {
        ensure_not_cancelled(config)?;
        pool.run_jobs(vec![BatchJob {
            lane: 0,
            request_path,
            result_path,
            batch,
        }])?
        .pop()
        .ok_or_else(|| native_message("incumbent demonstration produced no native result"))?
    };
    let candidate = output
        .validated
        .candidates
        .first()
        .filter(|candidate| {
            output.validated.candidates.len() == 1 && candidate.id == "incumbent-demonstration"
        })
        .ok_or_else(|| native_message("incumbent demonstration lacks one exact result"))?;
    let incumbent = config
        .optimization
        .incumbent
        .as_ref()
        .ok_or_else(|| native_message("native residual campaign requires an incumbent"))?;
    if candidate.first_hit_tick != Some(incumbent.first_hit_tick)
        || candidate.simulated_ticks != incumbent.first_hit_tick
    {
        return Err(native_message(
            "incumbent demonstration did not reproduce its exact terminal proof",
        ));
    }
    let request = artifact_reference(root, &output.request_path).map_err(native_error)?;
    let result = artifact_reference(root, &output.result_path).map_err(native_error)?;
    let episode = artifact_reference(root, Path::new(&output.validated.episode_shard_path))
        .map_err(native_error)?;
    let shard = NativeEpisodeShard::read(root.join(&episode.path)).map_err(native_error)?;
    if shard.content_sha256 != episode.sha256 {
        return Err(native_message(
            "incumbent demonstration shard differs from its artifact identity",
        ));
    }
    let replay = append_incumbent_demonstration_replay(
        root,
        campaign,
        config.optimization,
        &shard,
        &candidate.id,
    )
    .map_err(native_error)?;
    let attempt = native_attempt(
        1,
        pool.lanes[0].seed,
        candidate,
        request,
        result,
        episode,
        &output.validated,
    );
    let demonstration =
        NativeIncumbentDemonstration::seal(config.optimization, config.execution, attempt, replay)
            .map_err(native_error)?;
    let path = campaign.join("demonstration").join("manifest.json");
    write_exact_or_new(
        &path,
        &demonstration.to_pretty_json().map_err(native_error)?,
    )
    .map_err(native_error)?;
    let reference = artifact_reference(root, &path).map_err(native_error)?;
    let resume = crate::optimization_resume::append_optimization_resume_event(
        config.optimization,
        root,
        OptimizationResumeEvent::DemonstrationSeeded {
            demonstration: reference,
            simulated_ticks: demonstration.attempt.simulated_ticks,
        },
    )
    .map_err(native_error)?;
    Ok((resume, demonstration))
}

fn validate_checkpoint_replay(
    root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    parent: &InputTape,
    parent_bytes: &[u8],
    resume: &OptimizationResumeState,
    checkpoint: &ResidualCampaignCheckpoint,
) -> Result<(), NativeResidualCampaignRunnerError> {
    let Some(replay) = &checkpoint.replay_corpus else {
        // Checkpoints written before automatic ingestion remain migratable. The
        // next completed generation backfills all authenticated evaluations.
        if resume.demonstration.is_some() {
            return Err(native_message(
                "seeded optimization checkpoint omits its incumbent demonstration replay",
            ));
        }
        return Ok(());
    };
    let corpus = load_corpus(root, &replay.artifact).map_err(native_error)?;
    replay.validate_corpus(&corpus).map_err(native_error)?;
    validate_residual_corpus_scope(optimization, &corpus).map_err(native_error)?;
    let expected_randomized = checkpoint
        .completed_candidates
        .checked_mul(u64::from(optimization.execution.repetitions))
        .ok_or_else(|| native_message("residual replay checkpoint entry count overflowed"))?;
    let demonstration_entries = corpus
        .entries
        .iter()
        .filter(|entry| {
            entry.role
                == dusklight_learning::native_replay_corpus::ReplayExperienceRole::Demonstration
        })
        .count() as u64;
    let randomized_entries = corpus
        .entries
        .iter()
        .filter(|entry| {
            entry.role
                == dusklight_learning::native_replay_corpus::ReplayExperienceRole::RandomizedCoverage
        })
        .count() as u64;
    let alternate_entries = corpus
        .entries
        .iter()
        .filter(|entry| {
            entry.role
                == dusklight_learning::native_replay_corpus::ReplayExperienceRole::AlternateTerminal
        })
        .map(|entry| {
            (
                entry.shard_sha256,
                entry.episode_id.clone(),
                entry.objective.clone(),
            )
        })
        .collect::<BTreeSet<_>>();
    let mut authenticated_alternates = BTreeSet::new();
    for row in resume.candidates.iter().filter(|row| row.result.is_some()) {
        let candidate =
            load_candidate(root, optimization, parent, parent_bytes, row).map_err(native_error)?;
        let evaluation = load_native_evaluation(root, optimization, execution, row, &candidate)?;
        for alternate in &evaluation.alternate_terminals {
            for attempt in alternate
                .attempts
                .iter()
                .filter(|attempt| attempt.first_hit_tick.is_some())
            {
                authenticated_alternates.insert((
                    attempt.episode_shard.sha256,
                    attempt.wire_candidate_id.clone(),
                    alternate.terminal.goal.clone(),
                ));
            }
        }
    }
    if randomized_entries != expected_randomized
        || demonstration_entries != u64::from(resume.demonstration.is_some())
        || !alternate_entries.is_subset(&authenticated_alternates)
        || (checkpoint.completed_candidates == resume.completed_candidates
            && alternate_entries != authenticated_alternates)
    {
        return Err(native_message(
            "residual replay corpus does not cover every checkpointed native attempt",
        ));
    }
    Ok(())
}

pub fn run_native_residual_campaign(
    config: &NativeResidualCampaignRunConfig<'_>,
) -> Result<ResidualCampaignRunSummary, NativeResidualCampaignRunnerError> {
    ensure_not_cancelled(config)?;
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
    let mut alternate_pools =
        alternate_worker_pools(&root, &campaign, config.optimization, config.execution)?;
    let result = run_campaign_loop(
        config,
        &root,
        &campaign,
        &parent,
        &parent_bytes,
        &mut pool,
        &mut alternate_pools,
    );
    let mut shutdown_failures = Vec::new();
    if let Err(error) = pool.shutdown() {
        shutdown_failures.push(error.to_string());
    }
    for alternate in &mut alternate_pools {
        if let Err(error) = alternate.shutdown() {
            shutdown_failures.push(error.to_string());
        }
    }
    let shutdown = if shutdown_failures.is_empty() {
        Ok(())
    } else {
        Err(native_message(shutdown_failures.join("; ")))
    };
    match (result, shutdown) {
        (Ok(summary), Ok(())) => Ok(summary),
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(shutdown)) => Err(native_message(format!(
            "{error}; worker teardown reported: {shutdown}"
        ))),
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
    alternate_pools: &mut [WorkerPool<'_>],
) -> Result<ResidualCampaignRunSummary, NativeResidualCampaignRunnerError> {
    ensure_not_cancelled(config)?;
    let mut resume = if root.join(&config.optimization.resume.journal_path).exists() {
        load_optimization_resume(config.optimization, root)
    } else {
        initialize_optimization_resume(config.optimization, root)
    }
    .map_err(native_error)?;
    if resume.latest_optimizer_checkpoint.is_none() {
        let (seeded_resume, demonstration) =
            ensure_incumbent_demonstration(root, campaign, config, parent, &resume, pool)?;
        resume = seeded_resume;
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
            Some(demonstration.replay),
        )
        .map_err(native_error)?;
    } else if let Some(reference) = &resume.demonstration {
        load_incumbent_demonstration(root, config.optimization, config.execution, reference)?;
    }
    loop {
        ensure_not_cancelled(config)?;
        let checkpoint: ResidualCampaignCheckpoint = load_checkpoint(
            root,
            config.optimization,
            config.execution.content_sha256,
            &resume,
        )
        .map_err(native_error)?;
        validate_checkpoint_replay(
            root,
            config.optimization,
            config.execution,
            parent,
            parent_bytes,
            &resume,
            &checkpoint,
        )?;
        let optimizer = checkpoint
            .restore_optimizer(config.optimization, parent_bytes)
            .map_err(native_error)?;
        let mut archive = checkpoint.restore_archive().map_err(native_error)?;
        replay_completed(config, root, parent, parent_bytes, &resume, &mut archive)?;
        match optimizer {
            ResidualCampaignOptimizer::Random(mut random) => {
                let ResidualOptimizerConfig::Random { samples } =
                    config.optimization.proposal.optimizer
                else {
                    unreachable!()
                };
                if resume.completed_candidates >= samples {
                    if checkpoint.completed_candidates != resume.completed_candidates
                        || (checkpoint.replay_corpus.is_none() && resume.completed_candidates > 0)
                    {
                        let replay_corpus = append_generation_replay(
                            root,
                            campaign,
                            config,
                            parent,
                            parent_bytes,
                            &resume,
                            checkpoint.replay_corpus.as_ref(),
                            checkpoint.generation,
                        )?;
                        let optimizer = ResidualCampaignOptimizer::Random(random);
                        resume = append_checkpoint(
                            root,
                            campaign,
                            config.optimization,
                            config.execution.content_sha256,
                            &resume,
                            checkpoint.generation
                                + u64::from(
                                    checkpoint.completed_candidates != resume.completed_candidates,
                                ),
                            &optimizer,
                            &archive,
                            Some(replay_corpus),
                        )
                        .map_err(native_error)?;
                        continue;
                    }
                    return Ok(summary(
                        config,
                        &resume,
                        checkpoint.generation,
                        &archive,
                        checkpoint.replay_corpus.clone(),
                    ));
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
                        checkpoint.replay_corpus.clone(),
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
                    alternate_pools,
                    generation,
                )?;
                ensure_not_cancelled(config)?;
                let replay_corpus = append_generation_replay(
                    root,
                    campaign,
                    config,
                    parent,
                    parent_bytes,
                    &resume,
                    checkpoint.replay_corpus.as_ref(),
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
                    Some(replay_corpus),
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
                    if checkpoint.replay_corpus.is_none() && resume.completed_candidates > 0 {
                        let replay_corpus = append_generation_replay(
                            root,
                            campaign,
                            config,
                            parent,
                            parent_bytes,
                            &resume,
                            None,
                            generation,
                        )?;
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
                            Some(replay_corpus),
                        )
                        .map_err(native_error)?;
                        continue;
                    }
                    return Ok(summary(
                        config,
                        &resume,
                        generation,
                        &archive,
                        checkpoint.replay_corpus.clone(),
                    ));
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
                        checkpoint.replay_corpus.clone(),
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
                    alternate_pools,
                    generation,
                )?;
                ensure_not_cancelled(config)?;
                let ranked =
                    generation_rank(root, config, parent, parent_bytes, &resume, generation)?;
                cem.tell(&ranked).map_err(native_error)?;
                let replay_corpus = append_generation_replay(
                    root,
                    campaign,
                    config,
                    parent,
                    parent_bytes,
                    &resume,
                    checkpoint.replay_corpus.as_ref(),
                    generation,
                )?;
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
                    Some(replay_corpus),
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
    replay_corpus: Option<ResidualReplayCheckpoint>,
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
        replay_corpus,
    }
}

fn pretty_json(value: &impl Serialize) -> Result<Vec<u8>, serde_json::Error> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

#[derive(Debug)]
pub struct NativeResidualCampaignRunnerError {
    message: String,
    cancelled: bool,
}

impl NativeResidualCampaignRunnerError {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }
}

fn ensure_not_cancelled(
    config: &NativeResidualCampaignRunConfig<'_>,
) -> Result<(), NativeResidualCampaignRunnerError> {
    if config
        .cancellation
        .is_some_and(|cancelled| cancelled.load(Ordering::Acquire))
    {
        Err(native_cancelled(
            "native residual campaign cancelled at a durable boundary",
        ))
    } else {
        Ok(())
    }
}

fn native_message(message: impl Into<String>) -> NativeResidualCampaignRunnerError {
    NativeResidualCampaignRunnerError {
        message: message.into(),
        cancelled: false,
    }
}

fn native_cancelled(message: impl Into<String>) -> NativeResidualCampaignRunnerError {
    NativeResidualCampaignRunnerError {
        message: message.into(),
        cancelled: true,
    }
}

fn native_error(error: impl fmt::Display) -> NativeResidualCampaignRunnerError {
    native_message(error.to_string())
}

impl fmt::Display for NativeResidualCampaignRunnerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for NativeResidualCampaignRunnerError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_residual_campaign::NATIVE_RESIDUAL_EXECUTION_SCHEMA_V1;
    use crate::residual_campaign_runner::prepare_batch;
    use dusklight_harness_contracts::objective_suite::ArtifactReference;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_ROOT: AtomicU64 = AtomicU64::new(0);

    fn repository() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../..")
            .canonicalize()
            .unwrap()
    }

    fn optimization(root: &Path) -> OptimizationRequest {
        serde_json::from_slice(
            &fs::read(root.join(
                "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json",
            ))
            .unwrap(),
        )
        .unwrap()
    }

    fn placeholder(path: &str, byte: u8) -> ArtifactReference {
        ArtifactReference {
            path: path.into(),
            sha256: Digest([byte; 32]),
        }
    }

    fn execution(optimization: &OptimizationRequest) -> NativeResidualExecutionBinding {
        NativeResidualExecutionBinding {
            schema: NATIVE_RESIDUAL_EXECUTION_SCHEMA_V1.into(),
            content_sha256: Digest([9; 32]),
            optimization_request_sha256: optimization.content_sha256,
            executable: placeholder("build/test/Dusklight", 1),
            game_data: placeholder("build/test/game.iso", 2),
            process_boot_tape: placeholder("build/test/process.tape", 3),
            milestone_program: placeholder("build/test/terminal.dmsp", 4),
            world_context: placeholder("build/test/world.context.json", 5),
            card_fixture_manifest: placeholder("build/test/card-fixture.json", 6),
            checkpoint_validation_ticks: 8,
            verify_state_hashes: false,
        }
    }

    fn prepared_generation(
        root: &Path,
        optimization: &OptimizationRequest,
    ) -> (InputTape, Vec<u8>, Vec<PreparedCandidate>) {
        let incumbent = optimization.incumbent.as_ref().unwrap();
        let parent_bytes = fs::read(root.join(&incumbent.tape.path)).unwrap();
        let parent = InputTape::decode(&parent_bytes).unwrap().tape;
        let mut optimizer = new_optimizer(optimization, &parent_bytes).unwrap();
        let ResidualCampaignOptimizer::Cem(cem) = &mut optimizer else {
            panic!("checked Ordon request must use CEM")
        };
        let proposal = cem.ask(&parent, &parent_bytes).unwrap();
        let prepared = prepare_batch(optimization, &parent, &parent_bytes, 0, proposal).unwrap();
        (parent, parent_bytes, prepared)
    }

    #[test]
    fn native_batch_losslessly_bridges_residual_tapes_at_the_route_checkpoint() {
        let root = repository();
        let optimization = optimization(&root);
        let execution = execution(&optimization);
        let (_parent, _parent_bytes, prepared) = prepared_generation(&root, &optimization);
        let selected = prepared.iter().take(4).collect::<Vec<_>>();
        let batch = native_batch(
            &optimization,
            &execution,
            segment_profile(&root, &optimization).unwrap(),
            &selected,
            1,
        )
        .unwrap();

        assert_eq!(batch.source_frame, 440);
        assert_eq!(
            batch.source_boundary_fingerprint,
            optimization.route.native_source_boundary_fingerprint
        );
        assert_eq!(batch.maximum_ticks, 160);
        assert_eq!(batch.checkpoint_validation.ticks, 8);
        assert_eq!(batch.candidates.len(), selected.len());
        for (actual, expected) in batch.candidates.iter().zip(selected) {
            let imported = Candidate::from_absolute_tape(
                segment_profile(&root, &optimization).unwrap(),
                &expected.compiled.tape,
            )
            .unwrap();
            assert_eq!(actual.actions, imported.actions);
            assert_eq!(actual.id, wire_candidate_id(&expected.envelope.id, 1));
        }
    }

    #[test]
    fn residual_evaluation_charges_and_binds_alternate_terminal_attempts() {
        let root = repository();
        let optimization = optimization(&root);
        let execution = execution(&optimization);
        let (_parent, _parent_bytes, prepared) = prepared_generation(&root, &optimization);
        let alternate = optimization
            .alternate_terminal_predicates(&root)
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        let attempt = |first_hit_tick: Option<u64>, byte: u8| NativeResidualAttempt {
            repetition: 1,
            worker_seed: optimization.execution.deterministic_seeds[0],
            wire_candidate_id: wire_candidate_id(&prepared[0].envelope.id, 1),
            batch_request: placeholder("build/test/request.json", byte),
            batch_result: placeholder("build/test/result.json", byte.saturating_add(1)),
            episode_shard: placeholder("build/test/episodes.dseps", byte.saturating_add(2)),
            restore_identity: "7".repeat(32),
            checkpoint_bytes: 1,
            simulated_ticks: first_hit_tick.unwrap_or(160),
            first_hit_tick,
            terminal_boundary_fingerprint: "8".repeat(32),
            behavior_sha256: Digest([byte.saturating_add(3); 32]),
        };
        let evaluation = NativeResidualCampaignEvaluation::seal_with_alternate_terminals(
            &optimization,
            &execution,
            &prepared[0].envelope,
            vec![attempt(None, 10)],
            vec![NativeAlternateTerminalEvaluation {
                terminal: NativeTerminalBinding {
                    goal: alternate.goal,
                    program_sha256: alternate.program_sha256,
                    definition_sha256: alternate.definition_sha256,
                },
                attempts: vec![attempt(Some(113), 20)],
            }],
        )
        .unwrap();

        assert_eq!(evaluation.simulated_ticks, 273);
        assert_eq!(evaluation.alternate_terminals.len(), 1);
        assert_eq!(
            evaluation.alternate_terminals[0].attempts[0].first_hit_tick,
            Some(113)
        );
        evaluation
            .validate(&optimization, &execution, &prepared[0].envelope)
            .unwrap();
    }

    #[test]
    fn crash_recovery_never_reuses_a_partial_native_result_path() {
        let root = repository();
        let optimization = optimization(&root);
        let execution = execution(&optimization);
        let (_parent, _parent_bytes, prepared) = prepared_generation(&root, &optimization);
        let batch = native_batch(
            &optimization,
            &execution,
            segment_profile(&root, &optimization).unwrap(),
            &[&prepared[0]],
            1,
        )
        .unwrap();
        let nonce = NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed);
        let batch_root = root.join("build").join(format!(
            "native-residual-crash-path-test-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&batch_root).unwrap();
        fs::write(batch_root.join("result-try001.json"), b"{\"partial\":").unwrap();
        fs::write(
            batch_root.join("result-try001.json.episodes.dseps"),
            b"partial episode",
        )
        .unwrap();
        let terminal = NativeTerminalBinding {
            goal: optimization.terminal_predicate.goal.clone(),
            program_sha256: optimization.terminal_predicate.program_sha256,
            definition_sha256: optimization.terminal_predicate.definition_sha256,
        };

        let (path, adopted) = select_result_path(&batch_root, &batch, &terminal).unwrap();
        assert_eq!(path, batch_root.join("result-try002.json"));
        assert!(adopted.is_none());
        assert_eq!(
            fs::read(batch_root.join("result-try001.json")).unwrap(),
            b"{\"partial\":"
        );
        fs::remove_dir_all(batch_root).unwrap();
    }

    #[test]
    fn pre_cancelled_campaign_returns_a_typed_outcome_without_launching_workers() {
        let root = repository();
        let optimization = optimization(&root);
        let execution = execution(&optimization);
        let cancellation = AtomicBool::new(true);

        let error = run_native_residual_campaign(&NativeResidualCampaignRunConfig {
            repository_root: &root,
            optimization: &optimization,
            execution: &execution,
            cancellation: Some(&cancellation),
        })
        .unwrap_err();

        assert!(error.is_cancelled());
        assert!(error.to_string().contains("durable boundary"));
    }

    #[test]
    fn worker_pool_drop_shuts_down_and_removes_its_ephemeral_session_tree() {
        let root = repository();
        let optimization = optimization(&root);
        let execution = execution(&optimization);
        let nonce = NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed);
        let session_root = root.join("build").join(format!(
            "native-residual-session-cleanup-test-{}-{nonce}/native-sessions/run-test",
            std::process::id()
        ));
        fs::create_dir_all(session_root.join("worker-000/renderer-cache")).unwrap();
        fs::write(session_root.join("worker-000/transient.bin"), b"transient").unwrap();
        {
            let _pool = WorkerPool {
                root: &root,
                optimization: &optimization,
                execution: &execution,
                terminal: NativeTerminalBinding {
                    goal: optimization.terminal_predicate.goal.clone(),
                    program_sha256: optimization.terminal_predicate.program_sha256,
                    definition_sha256: optimization.terminal_predicate.definition_sha256,
                },
                milestone_program: root.join(&execution.milestone_program.path),
                card_fixture_root: root.clone(),
                session_root: session_root.clone(),
                lanes: Vec::new(),
            };
        }
        assert!(!session_root.exists());
        fs::remove_dir_all(session_root.ancestors().nth(2).expect("test campaign root")).unwrap();
    }
}
