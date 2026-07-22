//! Resumable collect/train/freeze/native-execute/ingest goal-learning loop.

use crate::campaign_replay::{PolicyReplayRollout, append_policy_replay_generation, load_corpus};
use crate::native_goal_learning_loop::{
    NativeGoalLearningLoopEvent, NativeGoalLearningLoopRequest, NativeGoalLearningLoopState,
    NativeGoalLearningProposalSource, NativeGoalLearningStopReason,
    append_native_goal_learning_loop_event, initialize_native_goal_learning_loop,
    load_native_goal_learning_loop,
};
use crate::native_residual_campaign::NativeResidualExecutionBinding;
use crate::native_suffix_result::NativeTerminalBinding;
use crate::native_suffix_worker::{
    NativeFrozenPolicyWorkerLaunch, NativeSuffixWorkerSession, ValidatedNativeFrozenPolicyBatch,
    validate_native_frozen_policy_artifacts,
};
use crate::optimization_request::OptimizationRequest;
use crate::residual_campaign::ResidualReplayCheckpoint;
use crate::residual_campaign_runner::{artifact_reference, read_artifact, write_exact_or_new};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::InputTape;
use dusklight_evidence::native_episode_shard::NativeEpisodeShard;
use dusklight_harness_contracts::objective_suite::ArtifactReference;
use dusklight_learning::compiled_goal_graph::CompiledGoalGraph;
use dusklight_learning::factorized_policy_suffix_batch::NativeFactorizedPolicyBatchConfig;
use dusklight_learning::native_frozen_policy_reinference::realize_native_frozen_policy_tape;
use dusklight_learning::native_frozen_policy_suffix_batch::NativeFrozenPolicySuffixBatch;
use dusklight_learning::native_goal_frozen_policy::{
    NativeGoalFrozenPolicyAdmission, NativeGoalFrozenPolicyExport, NativeGoalFrozenPolicyManifest,
};
use dusklight_learning::native_goal_reachability::{
    NativeGoalReachabilityAdmission, NativeGoalReachabilityModel,
};
use dusklight_learning::native_goal_trajectory::NativeGoalTrajectoryDataset;
use dusklight_learning::native_policy_collapse::{
    NativePolicyCollapseReport, NativePolicyCollapseWarning,
};
use dusklight_learning::native_replay_corpus::NativeReplayCorpus;
use serde::Serialize;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
pub struct NativeGoalLearningLoopRunConfig<'a> {
    pub repository_root: &'a Path,
    pub request: &'a NativeGoalLearningLoopRequest,
    pub optimization: &'a OptimizationRequest,
    pub execution: &'a NativeResidualExecutionBinding,
    /// Cancellation is observed only between durable phases. An in-flight
    /// native batch finishes and can be adopted on resume.
    pub cancellation: Option<&'a AtomicBool>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeGoalLearningLoopRunSummary {
    pub request_sha256: Digest,
    pub optimization_request_sha256: Digest,
    pub native_execution_sha256: Digest,
    pub committed_generations: u16,
    pub charged_simulated_ticks: u64,
    pub active_corpus_sha256: Digest,
    pub stopped_reason: NativeGoalLearningStopReason,
    pub proposal_source: NativeGoalLearningProposalSource,
    pub collapse_warning_generations: u16,
    pub latest_collapse_warnings: Vec<NativePolicyCollapseWarning>,
    pub journal_sha256: Digest,
    pub state_sha256: Digest,
}

struct FrozenWorkerLane {
    index: usize,
    session: Option<NativeSuffixWorkerSession>,
    state_root: PathBuf,
}

struct FrozenWorkerPool<'a> {
    root: &'a Path,
    optimization: &'a OptimizationRequest,
    execution: &'a NativeResidualExecutionBinding,
    terminal: NativeTerminalBinding,
    card_fixture_root: PathBuf,
    session_root: PathBuf,
    lanes: Vec<FrozenWorkerLane>,
}

struct FrozenBatchJob {
    rollout: usize,
    lane: usize,
    batch_path: PathBuf,
    result_path: PathBuf,
}

struct FrozenBatchOutput {
    rollout: usize,
    batch_path: PathBuf,
    result_path: PathBuf,
    validated: ValidatedNativeFrozenPolicyBatch,
}

struct LoadedGenerationRollout {
    batch: NativeFrozenPolicySuffixBatch,
    validated: ValidatedNativeFrozenPolicyBatch,
    shard: NativeEpisodeShard,
}

impl<'a> FrozenWorkerPool<'a> {
    fn new(
        root: &'a Path,
        campaign: &Path,
        optimization: &'a OptimizationRequest,
        execution: &'a NativeResidualExecutionBinding,
    ) -> Result<Self, NativeGoalLearningLoopRunnerError> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(runner_error)?
            .as_nanos();
        let session_root = campaign
            .join("native-sessions")
            .join(format!("run-{}-{nonce}", std::process::id()));
        let lanes = (0..usize::from(optimization.execution.workers))
            .map(|index| FrozenWorkerLane {
                index,
                session: None,
                state_root: session_root.join(format!("worker-{index:03}")),
            })
            .collect();
        Ok(Self {
            root,
            optimization,
            execution,
            terminal: NativeTerminalBinding {
                goal: optimization.terminal_predicate.goal.clone(),
                program_sha256: optimization.terminal_predicate.program_sha256,
                definition_sha256: optimization.terminal_predicate.definition_sha256,
            },
            card_fixture_root: execution
                .card_fixture_root(root, optimization)
                .map_err(runner_error)?,
            session_root,
            lanes,
        })
    }

    fn run_jobs(
        &mut self,
        jobs: Vec<FrozenBatchJob>,
    ) -> Result<Vec<FrozenBatchOutput>, NativeGoalLearningLoopRunnerError> {
        let mut pending = jobs;
        let mut outputs = Vec::new();
        while !pending.is_empty() {
            let mut lane_jobs = Vec::new();
            for lane in 0..self.lanes.len() {
                if let Some(index) = pending.iter().position(|job| job.lane == lane) {
                    lane_jobs.push(pending.remove(index));
                }
            }
            if lane_jobs.is_empty() {
                return Err(runner_message(
                    "frozen policy batch targets an unknown worker lane",
                ));
            }
            let root = self.root;
            let optimization = self.optimization;
            let execution = self.execution;
            let terminal = &self.terminal;
            let card_fixture_root = &self.card_fixture_root;
            let mut by_lane = lane_jobs
                .into_iter()
                .map(|job| (job.lane, job))
                .collect::<BTreeMap<_, _>>();
            let round = std::thread::scope(|scope| {
                let mut handles = Vec::new();
                for lane in &mut self.lanes {
                    let Some(job) = by_lane.remove(&lane.index) else {
                        continue;
                    };
                    handles.push(scope.spawn(move || {
                        run_frozen_lane_job(
                            root,
                            optimization,
                            execution,
                            terminal,
                            card_fixture_root,
                            lane,
                            job,
                        )
                    }));
                }
                handles
                    .into_iter()
                    .map(|handle| {
                        handle.join().map_err(|_| {
                            runner_message("native frozen policy worker thread panicked")
                        })?
                    })
                    .collect::<Result<Vec<_>, _>>()
            })?;
            if !by_lane.is_empty() {
                return Err(runner_message(
                    "frozen policy batch targets an unknown worker lane",
                ));
            }
            outputs.extend(round);
            self.validate_build_identity()?;
        }
        outputs.sort_by_key(|output| output.rollout);
        Ok(outputs)
    }

    fn validate_build_identity(&self) -> Result<(), NativeGoalLearningLoopRunnerError> {
        let mut expected: Option<&dusklight_worker_protocol::client::HelloResponse> = None;
        for lane in &self.lanes {
            let Some(session) = &lane.session else {
                continue;
            };
            if let Some(expected) = expected {
                let differences = expected.identity_differences(session.hello());
                if !differences.is_empty() {
                    return Err(runner_message(format!(
                        "native frozen policy worker pool build identity differs: {}",
                        differences
                            .iter()
                            .map(|difference| difference.message())
                            .collect::<Vec<_>>()
                            .join("; ")
                    )));
                }
            } else {
                expected = Some(session.hello());
            }
        }
        Ok(())
    }

    fn shutdown(&mut self) -> Result<(), NativeGoalLearningLoopRunnerError> {
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
            Err(runner_message(format!(
                "native frozen policy worker shutdown failed: {}",
                failures.join("; ")
            )))
        }
    }
}

impl Drop for FrozenWorkerPool<'_> {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn run_frozen_lane_job(
    root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    terminal: &NativeTerminalBinding,
    card_fixture_root: &Path,
    lane: &mut FrozenWorkerLane,
    job: FrozenBatchJob,
) -> Result<FrozenBatchOutput, NativeGoalLearningLoopRunnerError> {
    let validated = if let Some(session) = &mut lane.session {
        session
            .run_frozen_batch(&job.batch_path, &job.result_path)
            .map_err(runner_error)?
    } else {
        let launch = NativeFrozenPolicyWorkerLaunch {
            executable: root.join(&execution.executable.path),
            game_data: root.join(&execution.game_data.path),
            input_tape: root.join(&execution.process_boot_tape.path),
            milestone_program: root.join(&execution.milestone_program.path),
            card_fixture: card_fixture_root.to_path_buf(),
            card_fixture_sha256: execution.card_fixture_manifest.sha256,
            working_directory: root.to_path_buf(),
            state_root: lane.state_root.clone(),
            world_context_sha256: execution.world_context.sha256,
            terminal: terminal.clone(),
            initial_batch: job.batch_path.clone(),
            initial_result: job.result_path.clone(),
        };
        let (session, validated) =
            NativeSuffixWorkerSession::launch_frozen(&launch).map_err(runner_error)?;
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
            return Err(runner_message(
                "native frozen policy worker identity differs from sealed execution",
            ));
        }
        lane.session = Some(session);
        validated
    };
    Ok(FrozenBatchOutput {
        rollout: job.rollout,
        batch_path: job.batch_path,
        result_path: job.result_path,
        validated,
    })
}

pub fn run_native_goal_learning_loop(
    config: &NativeGoalLearningLoopRunConfig<'_>,
) -> Result<NativeGoalLearningLoopRunSummary, NativeGoalLearningLoopRunnerError> {
    ensure_not_cancelled(config)?;
    let root = config
        .repository_root
        .canonicalize()
        .map_err(runner_error)?;
    let execution_report = config
        .request
        .validate_files(&root, config.optimization, config.execution)
        .map_err(runner_error)?;
    let campaign = root.join(&config.request.resume.artifact_root);
    fs::create_dir_all(&campaign).map_err(runner_error)?;
    let mut pool = FrozenWorkerPool::new(&root, &campaign, config.optimization, config.execution)?;
    let result = run_loop(config, &root, &campaign, &execution_report, &mut pool);
    let shutdown = pool.shutdown();
    match (result, shutdown) {
        (Ok(summary), Ok(())) => Ok(summary),
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(shutdown)) => Err(runner_message(format!(
            "{error}; worker teardown reported: {shutdown}"
        ))),
        (Ok(_), Err(error)) => Err(error),
    }
}

fn run_loop(
    config: &NativeGoalLearningLoopRunConfig<'_>,
    root: &Path,
    campaign: &Path,
    _execution_report: &crate::native_goal_learning_loop::NativeGoalLearningLoopValidationReport,
    pool: &mut FrozenWorkerPool<'_>,
) -> Result<NativeGoalLearningLoopRunSummary, NativeGoalLearningLoopRunnerError> {
    let mut state = if root.join(&config.request.resume.journal_path).exists() {
        load_native_goal_learning_loop(config.request, root, config.optimization, config.execution)
    } else {
        initialize_native_goal_learning_loop(
            config.request,
            root,
            config.optimization,
            config.execution,
        )
    }
    .map_err(runner_error)?;
    let graph = compiled_goal(root, config.optimization, config.execution)?;

    loop {
        ensure_not_cancelled(config)?;
        if state.stopped.is_some() {
            return summary(config, &state);
        }
        if let Some(current) = state.generations.last()
            && current.committed_record_sha256.is_none()
        {
            if current.executed_record_sha256.is_none() {
                state = execute_prepared_generation(config, root, campaign, pool, &state)?;
            } else {
                state = ingest_executed_generation(config, root, campaign, &state)?;
            }
            continue;
        }
        if state.committed_generations == config.request.generation_limit {
            state = append_native_goal_learning_loop_event(
                config.request,
                root,
                config.optimization,
                config.execution,
                NativeGoalLearningLoopEvent::LoopStopped {
                    next_generation: state.committed_generations.saturating_add(1),
                    reason: NativeGoalLearningStopReason::GenerationLimitReached,
                    active_corpus_sha256: state.active_corpus_sha256,
                    evidence: None,
                    proposal_source: NativeGoalLearningProposalSource::FrozenGoalPolicy,
                },
            )
            .map_err(runner_error)?;
            continue;
        }
        if state.charged_simulated_ticks == config.request.simulated_tick_budget {
            state = append_native_goal_learning_loop_event(
                config.request,
                root,
                config.optimization,
                config.execution,
                NativeGoalLearningLoopEvent::LoopStopped {
                    next_generation: state.committed_generations.saturating_add(1),
                    reason: NativeGoalLearningStopReason::SimulatedTickBudgetReached,
                    active_corpus_sha256: state.active_corpus_sha256,
                    evidence: None,
                    proposal_source: NativeGoalLearningProposalSource::FrozenGoalPolicy,
                },
            )
            .map_err(runner_error)?;
            continue;
        }
        state = prepare_generation(config, root, campaign, &graph, &state)?;
    }
}

fn prepare_generation(
    config: &NativeGoalLearningLoopRunConfig<'_>,
    root: &Path,
    campaign: &Path,
    graph: &CompiledGoalGraph,
    state: &NativeGoalLearningLoopState,
) -> Result<NativeGoalLearningLoopState, NativeGoalLearningLoopRunnerError> {
    let generation = state.committed_generations.saturating_add(1);
    let generation_root = generation_root(campaign, generation);
    fs::create_dir_all(&generation_root).map_err(runner_error)?;
    let corpus = load_active_corpus(config, root, state)?;
    let shards = load_all_shards(config, root, state)?;
    let dataset =
        NativeGoalTrajectoryDataset::build(&corpus, &shards, graph, config.request.trajectory)
            .map_err(runner_error)?;
    let dataset_path = generation_root.join(format!("dataset-{}.json", dataset.dataset_sha256));
    write_exact_or_new(&dataset_path, &pretty_json(&dataset)?).map_err(runner_error)?;
    let dataset_reference = artifact_reference(root, &dataset_path).map_err(runner_error)?;

    let reachability = NativeGoalReachabilityModel::fit(
        std::slice::from_ref(&dataset),
        &shards,
        config.request.reachability,
    )
    .map_err(runner_error)?;
    let reachability_path =
        generation_root.join(format!("reachability-{}.json", reachability.model_sha256));
    write_exact_or_new(&reachability_path, &pretty_json(&reachability)?).map_err(runner_error)?;
    let reachability_reference =
        artifact_reference(root, &reachability_path).map_err(runner_error)?;
    if reachability.admission != NativeGoalReachabilityAdmission::GoalConditionedCandidate {
        return append_native_goal_learning_loop_event(
            config.request,
            root,
            config.optimization,
            config.execution,
            NativeGoalLearningLoopEvent::LoopStopped {
                next_generation: generation,
                reason: NativeGoalLearningStopReason::HeldOutReachabilityRejected,
                active_corpus_sha256: state.active_corpus_sha256,
                evidence: Some(reachability_reference),
                proposal_source: NativeGoalLearningProposalSource::RetainedBaseline,
            },
        )
        .map_err(runner_error);
    }

    let export =
        NativeGoalFrozenPolicyExport::fit(&dataset, &shards, &reachability, config.request.policy)
            .map_err(runner_error)?;
    let model_path = generation_root.join(format!(
        "policy-{}.dsfrozen",
        export.manifest.frozen_model_xxh3_128
    ));
    let manifest_path = generation_root.join(format!(
        "policy-manifest-{}.json",
        export.manifest.manifest_sha256
    ));
    write_exact_or_new(&model_path, &export.model_bytes).map_err(runner_error)?;
    write_exact_or_new(&manifest_path, &pretty_json(&export.manifest)?).map_err(runner_error)?;
    let model_reference = artifact_reference(root, &model_path).map_err(runner_error)?;
    let manifest_reference = artifact_reference(root, &manifest_path).map_err(runner_error)?;
    if export.manifest.admission != NativeGoalFrozenPolicyAdmission::FrozenPolicyCandidate {
        return append_native_goal_learning_loop_event(
            config.request,
            root,
            config.optimization,
            config.execution,
            NativeGoalLearningLoopEvent::LoopStopped {
                next_generation: generation,
                reason: NativeGoalLearningStopReason::HeldOutPolicyRejected,
                active_corpus_sha256: state.active_corpus_sha256,
                evidence: Some(manifest_reference),
                proposal_source: NativeGoalLearningProposalSource::RetainedBaseline,
            },
        )
        .map_err(runner_error);
    }

    let model_path = model_path.canonicalize().map_err(runner_error)?;
    let mut batches = Vec::with_capacity(usize::from(config.request.rollouts_per_generation));
    for rollout in 0..config.request.rollouts_per_generation {
        let batch = NativeFrozenPolicySuffixBatch::build(
            &export.model_bytes,
            model_path.to_string_lossy().into_owned(),
            config.optimization.terminal_predicate.definition_sha256,
            format!("goal-policy-g{generation:04}-r{rollout:04}"),
            NativeFactorizedPolicyBatchConfig {
                source_frame: usize::try_from(config.optimization.route.source_boundary_index)
                    .map_err(runner_error)?,
                source_boundary_fingerprint: config
                    .optimization
                    .route
                    .native_source_boundary_fingerprint
                    .clone(),
                checkpoint_validation_ticks: usize::try_from(
                    config.execution.checkpoint_validation_ticks,
                )
                .map_err(runner_error)?,
                maximum_ticks: usize::try_from(
                    config.optimization.budgets.exploration_horizon_ticks,
                )
                .map_err(runner_error)?,
                verify_state_hashes: config.execution.verify_state_hashes,
            },
        )
        .map_err(runner_error)?;
        let batch_path = generation_root
            .join("rollouts")
            .join(format!("rollout-{rollout:04}"))
            .join("request.json");
        write_exact_or_new(&batch_path, &pretty_json(&batch)?).map_err(runner_error)?;
        batches.push(artifact_reference(root, &batch_path).map_err(runner_error)?);
    }
    append_native_goal_learning_loop_event(
        config.request,
        root,
        config.optimization,
        config.execution,
        NativeGoalLearningLoopEvent::GenerationPrepared {
            generation,
            input_corpus_sha256: corpus.corpus_sha256,
            dataset_sha256: dataset.dataset_sha256,
            reachability_model_sha256: reachability.model_sha256,
            policy_manifest_sha256: export.manifest.manifest_sha256,
            frozen_model_xxh3_128: export.manifest.frozen_model_xxh3_128,
            dataset: dataset_reference,
            reachability_model: reachability_reference,
            policy_manifest: manifest_reference,
            frozen_model: model_reference,
            native_batches: batches,
        },
    )
    .map_err(runner_error)
}

fn execute_prepared_generation(
    config: &NativeGoalLearningLoopRunConfig<'_>,
    root: &Path,
    campaign: &Path,
    pool: &mut FrozenWorkerPool<'_>,
    state: &NativeGoalLearningLoopState,
) -> Result<NativeGoalLearningLoopState, NativeGoalLearningLoopRunnerError> {
    let current = state
        .generations
        .last()
        .ok_or_else(|| runner_message("prepared generation disappeared"))?;
    let model_bytes = read_artifact(root, &current.frozen_model).map_err(runner_error)?;
    let terminal = NativeTerminalBinding {
        goal: config.optimization.terminal_predicate.goal.clone(),
        program_sha256: config.optimization.terminal_predicate.program_sha256,
        definition_sha256: config.optimization.terminal_predicate.definition_sha256,
    };
    let mut outputs = Vec::new();
    let mut jobs = Vec::new();
    for (rollout, reference) in current.native_batches.iter().enumerate() {
        let batch_path = root.join(&reference.path);
        let batch: NativeFrozenPolicySuffixBatch =
            serde_json::from_slice(&read_artifact(root, reference).map_err(runner_error)?)
                .map_err(runner_error)?;
        batch.validate(&model_bytes).map_err(runner_error)?;
        let rollout_root = generation_root(campaign, current.generation)
            .join("rollouts")
            .join(format!("rollout-{rollout:04}"));
        let (result_path, adopted) =
            select_frozen_result_path(&rollout_root, &batch, &model_bytes, &terminal)?;
        if let Some(validated) = adopted {
            outputs.push(FrozenBatchOutput {
                rollout,
                batch_path,
                result_path,
                validated,
            });
        } else {
            jobs.push(FrozenBatchJob {
                rollout,
                lane: rollout % pool.lanes.len(),
                batch_path,
                result_path,
            });
        }
    }
    ensure_not_cancelled(config)?;
    outputs.extend(pool.run_jobs(jobs)?);
    outputs.sort_by_key(|output| output.rollout);
    if outputs.len() != usize::from(config.request.rollouts_per_generation) {
        return Err(runner_message(
            "native frozen policy generation did not complete every rollout",
        ));
    }
    let mut native_results = Vec::with_capacity(outputs.len());
    let mut episode_shards = Vec::with_capacity(outputs.len());
    let mut reinference_reports = Vec::with_capacity(outputs.len());
    let mut realized_tapes = Vec::with_capacity(outputs.len());
    let mut simulated_ticks = 0_u64;
    let mut successes = 0_u16;
    let source_tape_bytes =
        read_artifact(root, &config.execution.process_boot_tape).map_err(runner_error)?;
    let source_tape = InputTape::decode(&source_tape_bytes)
        .map_err(runner_error)?
        .tape;
    for output in outputs {
        let request_reference =
            artifact_reference(root, &output.batch_path).map_err(runner_error)?;
        if request_reference != current.native_batches[output.rollout] {
            return Err(runner_message(
                "executed frozen policy request differs from prepared journal artifact",
            ));
        }
        native_results.push(artifact_reference(root, &output.result_path).map_err(runner_error)?);
        let shard_path = Path::new(&output.validated.execution.episode_shard_path);
        episode_shards.push(artifact_reference(root, shard_path).map_err(runner_error)?);
        let shard = NativeEpisodeShard::read(shard_path).map_err(runner_error)?;
        let episode_id = output
            .validated
            .execution
            .candidates
            .first()
            .filter(|_| output.validated.execution.candidates.len() == 1)
            .ok_or_else(|| runner_message("frozen policy rollout lacks one exact candidate"))?
            .id
            .as_str();
        let realized = realize_native_frozen_policy_tape(&source_tape, &shard, episode_id)
            .map_err(runner_error)?;
        let realized_path = output.result_path.with_file_name("realized.tape");
        write_exact_or_new(&realized_path, &realized.encode().map_err(runner_error)?)
            .map_err(runner_error)?;
        realized_tapes.push(artifact_reference(root, &realized_path).map_err(runner_error)?);
        let report_path = output.result_path.with_file_name("reinference-report.json");
        write_exact_or_new(&report_path, &pretty_json(&output.validated.reinference)?)
            .map_err(runner_error)?;
        reinference_reports.push(artifact_reference(root, &report_path).map_err(runner_error)?);
        simulated_ticks = simulated_ticks
            .checked_add(output.validated.execution.simulated_ticks)
            .ok_or_else(|| runner_message("frozen policy simulated tick total overflowed"))?;
        successes = successes
            .checked_add(
                u16::try_from(
                    output
                        .validated
                        .execution
                        .candidates
                        .iter()
                        .filter(|candidate| candidate.first_hit_tick.is_some())
                        .count(),
                )
                .map_err(runner_error)?,
            )
            .ok_or_else(|| runner_message("frozen policy success count overflowed"))?;
    }
    append_native_goal_learning_loop_event(
        config.request,
        root,
        config.optimization,
        config.execution,
        NativeGoalLearningLoopEvent::GenerationExecuted {
            generation: current.generation,
            prepared_record_sha256: current.prepared_record_sha256,
            native_results,
            episode_shards,
            reinference_reports,
            realized_tapes,
            simulated_ticks,
            successes,
        },
    )
    .map_err(runner_error)
}

fn ingest_executed_generation(
    config: &NativeGoalLearningLoopRunConfig<'_>,
    root: &Path,
    campaign: &Path,
    state: &NativeGoalLearningLoopState,
) -> Result<NativeGoalLearningLoopState, NativeGoalLearningLoopRunnerError> {
    let current = state
        .generations
        .last()
        .ok_or_else(|| runner_message("executed generation disappeared"))?;
    let manifest: NativeGoalFrozenPolicyManifest = serde_json::from_slice(
        &read_artifact(root, &current.policy_manifest).map_err(runner_error)?,
    )
    .map_err(runner_error)?;
    let model_bytes = read_artifact(root, &current.frozen_model).map_err(runner_error)?;
    manifest.validate(&model_bytes).map_err(runner_error)?;
    let batches = current
        .native_batches
        .iter()
        .map(|reference| {
            serde_json::from_slice(&read_artifact(root, reference).map_err(runner_error)?)
                .map_err(runner_error)
        })
        .collect::<Result<Vec<NativeFrozenPolicySuffixBatch>, _>>()?;
    let result_references = current
        .native_results
        .as_ref()
        .ok_or_else(|| runner_message("executed generation lacks result references"))?;
    let shard_references = current
        .episode_shards
        .as_ref()
        .ok_or_else(|| runner_message("executed generation lacks episode references"))?;
    let report_references = current
        .reinference_reports
        .as_ref()
        .ok_or_else(|| runner_message("executed generation lacks reinference references"))?;
    let realized_references = current
        .realized_tapes
        .as_ref()
        .ok_or_else(|| runner_message("executed generation lacks realized tape references"))?;
    let source_tape_bytes =
        read_artifact(root, &config.execution.process_boot_tape).map_err(runner_error)?;
    let source_tape = InputTape::decode(&source_tape_bytes)
        .map_err(runner_error)?
        .tape;
    let terminal = NativeTerminalBinding {
        goal: config.optimization.terminal_predicate.goal.clone(),
        program_sha256: config.optimization.terminal_predicate.program_sha256,
        definition_sha256: config.optimization.terminal_predicate.definition_sha256,
    };
    let mut loaded = Vec::with_capacity(batches.len());
    for index in 0..batches.len() {
        let validated = validate_native_frozen_policy_artifacts(
            &batches[index],
            &model_bytes,
            &root.join(&result_references[index].path),
            &terminal,
        )
        .map_err(runner_error)?;
        let shard = NativeEpisodeShard::read(root.join(&shard_references[index].path))
            .map_err(runner_error)?;
        if shard.content_sha256 != shard_references[index].sha256 {
            return Err(runner_message(
                "journaled policy episode differs from its content identity",
            ));
        }
        let report: dusklight_learning::native_frozen_policy_reinference::NativeFrozenPolicyReinferenceReport =
            serde_json::from_slice(&read_artifact(root, &report_references[index]).map_err(runner_error)?)
                .map_err(runner_error)?;
        report.validate().map_err(runner_error)?;
        if report != validated.reinference {
            return Err(runner_message(
                "journaled reinference report differs from independent replay",
            ));
        }
        let episode_id = validated
            .execution
            .candidates
            .first()
            .filter(|_| validated.execution.candidates.len() == 1)
            .ok_or_else(|| runner_message("frozen policy rollout lacks one exact candidate"))?
            .id
            .as_str();
        let expected_realized = realize_native_frozen_policy_tape(&source_tape, &shard, episode_id)
            .map_err(runner_error)?
            .encode()
            .map_err(runner_error)?;
        if read_artifact(root, &realized_references[index]).map_err(runner_error)?
            != expected_realized
        {
            return Err(runner_message(
                "journaled cold-replayable tape differs from exact consumed policy PADs",
            ));
        }
        loaded.push(LoadedGenerationRollout {
            batch: batches[index].clone(),
            validated,
            shard,
        });
    }
    let rollouts = loaded
        .iter()
        .map(|rollout| PolicyReplayRollout {
            batch: &rollout.batch,
            validated: &rollout.validated,
            shard: &rollout.shard,
        })
        .collect::<Vec<_>>();
    let previous_corpus = load_active_corpus(config, root, state)?;
    let previous = ResidualReplayCheckpoint::seal(
        active_corpus_reference(config, state).clone(),
        &previous_corpus,
    )
    .map_err(runner_error)?;
    let replay = append_policy_replay_generation(
        root,
        campaign,
        config.optimization,
        &previous,
        &manifest,
        &model_bytes,
        &rollouts,
    )
    .map_err(runner_error)?;
    let corpus = load_corpus(root, &replay.artifact).map_err(runner_error)?;
    let realized_shards = loaded
        .iter()
        .map(|rollout| rollout.shard.clone())
        .collect::<Vec<_>>();
    let collapse = NativePolicyCollapseReport::build(current.generation, &realized_shards)
        .map_err(runner_error)?;
    let collapse_path = generation_root(campaign, current.generation)
        .join(format!("collapse-{}.json", collapse.report_sha256));
    write_exact_or_new(&collapse_path, &pretty_json(&collapse)?).map_err(runner_error)?;
    let collapse_diagnostics = artifact_reference(root, &collapse_path).map_err(runner_error)?;
    append_native_goal_learning_loop_event(
        config.request,
        root,
        config.optimization,
        config.execution,
        NativeGoalLearningLoopEvent::GenerationCommitted {
            generation: current.generation,
            executed_record_sha256: current
                .executed_record_sha256
                .ok_or_else(|| runner_message("executed record identity is absent"))?,
            output_corpus_sha256: corpus.corpus_sha256,
            output_corpus: replay.artifact,
            collapse_diagnostics: Some(collapse_diagnostics),
            entries: corpus.report.entries as u64,
            transitions: corpus.report.transitions,
        },
    )
    .map_err(runner_error)
}

fn select_frozen_result_path(
    rollout_root: &Path,
    batch: &NativeFrozenPolicySuffixBatch,
    model_bytes: &[u8],
    terminal: &NativeTerminalBinding,
) -> Result<(PathBuf, Option<ValidatedNativeFrozenPolicyBatch>), NativeGoalLearningLoopRunnerError>
{
    for trial in 1..=100_u32 {
        let result = rollout_root.join(format!("result-try{trial:03}.json"));
        if result.is_file()
            && let Ok(validated) =
                validate_native_frozen_policy_artifacts(batch, model_bytes, &result, terminal)
        {
            return Ok((result, Some(validated)));
        }
        let mut episode = result.as_os_str().to_os_string();
        episode.push(".episodes.dseps");
        if !result.exists() && !Path::new(&episode).exists() {
            return Ok((result, None));
        }
    }
    Err(runner_message(
        "native frozen policy batch exhausted crash-recovery result paths",
    ))
}

fn load_active_corpus(
    config: &NativeGoalLearningLoopRunConfig<'_>,
    root: &Path,
    state: &NativeGoalLearningLoopState,
) -> Result<NativeReplayCorpus, NativeGoalLearningLoopRunnerError> {
    load_corpus(root, active_corpus_reference(config, state)).map_err(runner_error)
}

fn active_corpus_reference<'a>(
    config: &'a NativeGoalLearningLoopRunConfig<'_>,
    state: &'a NativeGoalLearningLoopState,
) -> &'a ArtifactReference {
    state
        .generations
        .iter()
        .rev()
        .find_map(|generation| generation.output_corpus.as_ref())
        .unwrap_or(&config.request.initial_replay_corpus)
}

fn load_all_shards(
    config: &NativeGoalLearningLoopRunConfig<'_>,
    root: &Path,
    state: &NativeGoalLearningLoopState,
) -> Result<Vec<NativeEpisodeShard>, NativeGoalLearningLoopRunnerError> {
    config
        .request
        .initial_episode_shards
        .iter()
        .chain(
            state
                .generations
                .iter()
                .filter_map(|generation| generation.episode_shards.as_ref())
                .flatten(),
        )
        .map(|reference| {
            let bytes = read_artifact(root, reference).map_err(runner_error)?;
            let shard = NativeEpisodeShard::decode(&bytes).map_err(runner_error)?;
            if shard.content_sha256 != reference.sha256 {
                return Err(runner_message(
                    "learning-loop episode shard differs from its artifact identity",
                ));
            }
            Ok(shard)
        })
        .collect()
}

fn compiled_goal(
    root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
) -> Result<CompiledGoalGraph, NativeGoalLearningLoopRunnerError> {
    let bytes = read_artifact(root, &execution.milestone_program).map_err(runner_error)?;
    let decoded = dusklight_objectives::milestone_dsl::decode(&bytes).map_err(runner_error)?;
    let definition = decoded
        .definitions
        .iter()
        .position(|definition| definition.name == optimization.terminal_predicate.goal)
        .ok_or_else(|| runner_message("learning-loop milestone goal is absent"))?;
    let compiled = dusklight_objectives::milestone_dsl::CompiledMilestones {
        bytes,
        program_sha256: decoded.program_sha256,
        definitions: decoded.definitions,
    };
    CompiledGoalGraph::from_compiled(&compiled, definition).map_err(runner_error)
}

fn generation_root(campaign: &Path, generation: u16) -> PathBuf {
    campaign.join(format!("generation-{generation:04}"))
}

fn summary(
    config: &NativeGoalLearningLoopRunConfig<'_>,
    state: &NativeGoalLearningLoopState,
) -> Result<NativeGoalLearningLoopRunSummary, NativeGoalLearningLoopRunnerError> {
    let stopped = state
        .stopped
        .as_ref()
        .ok_or_else(|| runner_message("completed learning loop has no stopping state"))?;
    let root = config
        .repository_root
        .canonicalize()
        .map_err(runner_error)?;
    let collapse_reports = state
        .generations
        .iter()
        .filter_map(|generation| generation.collapse_diagnostics.as_ref())
        .map(|reference| {
            serde_json::from_slice::<NativePolicyCollapseReport>(
                &read_artifact(&root, reference).map_err(runner_error)?,
            )
            .map_err(runner_error)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let collapse_warning_generations = u16::try_from(
        collapse_reports
            .iter()
            .filter(|report| report.collapse_detected)
            .count(),
    )
    .map_err(runner_error)?;
    let latest_collapse_warnings = collapse_reports
        .last()
        .map(|report| report.warnings.clone())
        .unwrap_or_default();
    Ok(NativeGoalLearningLoopRunSummary {
        request_sha256: config.request.content_sha256,
        optimization_request_sha256: config.optimization.content_sha256,
        native_execution_sha256: config.execution.content_sha256,
        committed_generations: state.committed_generations,
        charged_simulated_ticks: state.charged_simulated_ticks,
        active_corpus_sha256: state.active_corpus_sha256,
        stopped_reason: stopped.reason,
        proposal_source: stopped.proposal_source,
        collapse_warning_generations,
        latest_collapse_warnings,
        journal_sha256: state.journal_sha256,
        state_sha256: state.state_sha256,
    })
}

fn ensure_not_cancelled(
    config: &NativeGoalLearningLoopRunConfig<'_>,
) -> Result<(), NativeGoalLearningLoopRunnerError> {
    if config
        .cancellation
        .is_some_and(|cancelled| cancelled.load(Ordering::Acquire))
    {
        return Err(runner_message("native goal learning loop was cancelled"));
    }
    Ok(())
}

fn pretty_json(value: &impl Serialize) -> Result<Vec<u8>, NativeGoalLearningLoopRunnerError> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(runner_error)?;
    bytes.push(b'\n');
    Ok(bytes)
}

#[derive(Debug)]
pub struct NativeGoalLearningLoopRunnerError(String);

impl NativeGoalLearningLoopRunnerError {
    pub fn is_cancelled(&self) -> bool {
        self.0 == "native goal learning loop was cancelled"
    }
}

fn runner_message(message: impl Into<String>) -> NativeGoalLearningLoopRunnerError {
    NativeGoalLearningLoopRunnerError(message.into())
}

fn runner_error(error: impl fmt::Display) -> NativeGoalLearningLoopRunnerError {
    runner_message(error.to_string())
}

impl fmt::Display for NativeGoalLearningLoopRunnerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeGoalLearningLoopRunnerError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_goal_learning_loop::NativeGoalLearningLoopResume;
    use dusklight_learning::native_frozen_policy_suffix_batch::native_frozen_policy_probe_model;
    use dusklight_learning::native_goal_frozen_policy::NativeGoalFrozenPolicyConfig;
    use dusklight_learning::native_goal_reachability::NativeGoalReachabilityConfig;
    use dusklight_learning::native_goal_trajectory::NativeGoalTrajectoryConfig;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::AtomicU64;

    static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

    fn test_root() -> PathBuf {
        let nonce = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "dusklight-native-goal-runner-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        root.canonicalize().unwrap()
    }

    fn reference(path: &str, byte: u8) -> ArtifactReference {
        ArtifactReference {
            path: path.into(),
            sha256: Digest([byte; 32]),
        }
    }

    #[test]
    fn pre_cancelled_loop_returns_without_validating_or_launching_workers() {
        let root = test_root();
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../..")
            .canonicalize()
            .unwrap();
        let optimization: OptimizationRequest = serde_json::from_slice(
            &fs::read(repository.join(
                "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json",
            ))
            .unwrap(),
        )
        .unwrap();
        let execution = NativeResidualExecutionBinding {
            schema: "test-invalid-execution".into(),
            content_sha256: Digest([2; 32]),
            optimization_request_sha256: optimization.content_sha256,
            executable: reference("missing/executable", 3),
            game_data: reference("missing/game", 4),
            process_boot_tape: reference("missing/tape", 5),
            milestone_program: reference("missing/milestones", 6),
            world_context: reference("missing/world", 7),
            card_fixture_manifest: reference("missing/cards", 8),
            checkpoint_validation_ticks: 1,
            verify_state_hashes: false,
        };
        let request = NativeGoalLearningLoopRequest {
            schema: "test-invalid-request".into(),
            content_sha256: Digest([9; 32]),
            optimization_request_sha256: optimization.content_sha256,
            native_execution_sha256: execution.content_sha256,
            initial_replay_corpus: reference("missing/corpus", 10),
            initial_episode_shards: vec![reference("missing/shard", 11)],
            generation_limit: 3,
            rollouts_per_generation: 1,
            simulated_tick_budget: 1,
            trajectory: NativeGoalTrajectoryConfig::default(),
            reachability: NativeGoalReachabilityConfig::default(),
            policy: NativeGoalFrozenPolicyConfig::default(),
            resume: NativeGoalLearningLoopResume {
                journal_path: "journal.jsonl".into(),
                state_path: "state.json".into(),
                artifact_root: "artifacts".into(),
            },
        };
        let cancelled = AtomicBool::new(true);
        let error = run_native_goal_learning_loop(&NativeGoalLearningLoopRunConfig {
            repository_root: &root,
            request: &request,
            optimization: &optimization,
            execution: &execution,
            cancellation: Some(&cancelled),
        })
        .unwrap_err();
        assert!(error.to_string().contains("cancelled"));
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn crash_recovery_never_reuses_partial_frozen_result_paths() {
        let root = test_root();
        let objective = Digest([12; 32]);
        let model = native_frozen_policy_probe_model(objective).unwrap();
        let model_bytes = model.to_bytes().unwrap();
        let model_path = root.join("policy.dsfrozen");
        fs::write(&model_path, &model_bytes).unwrap();
        let batch = NativeFrozenPolicySuffixBatch::build(
            &model_bytes,
            model_path.to_string_lossy().into_owned(),
            objective,
            "generation-1-rollout-0".into(),
            NativeFactorizedPolicyBatchConfig {
                source_frame: 1,
                source_boundary_fingerprint: "1".repeat(32),
                checkpoint_validation_ticks: 1,
                maximum_ticks: 2,
                verify_state_hashes: false,
            },
        )
        .unwrap();
        let terminal = NativeTerminalBinding {
            goal: "goal".into(),
            program_sha256: Digest([13; 32]),
            definition_sha256: objective,
        };
        fs::write(root.join("result-try001.json"), b"partial").unwrap();
        let orphan = root.join("result-try002.json.episodes.dseps");
        fs::write(&orphan, b"orphan").unwrap();
        let (selected, adopted) =
            select_frozen_result_path(&root, &batch, &model_bytes, &terminal).unwrap();
        assert_eq!(selected, root.join("result-try003.json"));
        assert!(adopted.is_none());
        assert_eq!(
            fs::read(root.join("result-try001.json")).unwrap(),
            b"partial"
        );
        assert_eq!(fs::read(orphan).unwrap(), b"orphan");
        fs::remove_dir_all(root).unwrap();
    }
}
