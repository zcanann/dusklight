//! Workbench projection and explicit lifecycle for native goal learning.

use super::*;
use dusklight_harness_contracts::objective_suite::ArtifactReference;
use dusklight_learning::native_goal_frozen_policy::NativeGoalFrozenPolicyConfig;
use dusklight_learning::native_goal_reachability::NativeGoalReachabilityConfig;
use dusklight_learning::native_goal_trajectory::NativeGoalTrajectoryConfig;
use dusklight_learning::native_policy_collapse::{
    NativePolicyCollapseReport, NativePolicyCollapseWarning,
};
use dusklight_learning::native_replay_corpus::NativeReplayCorpus;
use dusklight_orchestration::native_goal_learning_loop::{
    NativeGoalLearningLoopRequest, NativeGoalLearningLoopResume, NativeGoalLearningProposalSource,
    NativeGoalLearningStopReason, inspect_native_goal_learning_loop,
};
use dusklight_orchestration::native_goal_learning_loop_runner::{
    NativeGoalLearningLoopRunConfig, run_native_goal_learning_loop,
};
use dusklight_orchestration::native_residual_campaign::{
    NativeIncumbentDemonstration, NativeResidualCampaignEvaluation, NativeResidualExecutionBinding,
};
use dusklight_orchestration::optimization_request::OptimizationRequest;
use dusklight_orchestration::optimization_resume::OptimizationResumeState;
use dusklight_orchestration::residual_campaign::{
    ResidualCampaignCheckpoint, ResidualReplayCheckpoint,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const GOAL_LEARNING_START_SCHEMA: &str = "dusklight.route-workbench.goal-learning-start.v1";
const GOAL_LEARNING_LIFECYCLE_SCHEMA: &str = "dusklight.route-workbench.goal-learning-lifecycle.v1";
const GOAL_LEARNING_GENERATIONS: u16 = 3;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserGoalLearningStartRequest {
    pub campaign: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserGoalLearningLifecycleRequest {
    pub campaign: String,
    pub request_sha256: String,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct GoalLearningStartResponse {
    pub schema: &'static str,
    pub campaign: String,
    pub request_sha256: String,
    pub status: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct GoalLearningLifecycleResponse {
    pub schema: &'static str,
    pub campaign: String,
    pub request_sha256: String,
    pub status: &'static str,
}

#[derive(Clone, Debug)]
struct GoalLearningRuntimeStatus {
    status: &'static str,
    error: Option<String>,
}

#[derive(Clone, Debug)]
struct GoalLearningRuntimeEntry {
    optimization_request_sha256: String,
    status: GoalLearningRuntimeStatus,
    cancellation: Arc<AtomicBool>,
}

fn goal_learning_runs() -> &'static Mutex<BTreeMap<String, GoalLearningRuntimeEntry>> {
    static RUNS: OnceLock<Mutex<BTreeMap<String, GoalLearningRuntimeEntry>>> = OnceLock::new();
    RUNS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub(super) fn goal_learning_campaign_active(optimization_request_sha256: &str) -> bool {
    goal_learning_runs().lock().ok().is_some_and(|runs| {
        runs.values().any(|entry| {
            entry.optimization_request_sha256 == optimization_request_sha256
                && matches!(entry.status.status, "preparing" | "running" | "cancelling")
        })
    })
}

pub(super) fn forget_goal_learning_campaign(optimization_request_sha256: &str) {
    if let Ok(mut runs) = goal_learning_runs().lock() {
        runs.retain(|_, entry| entry.optimization_request_sha256 != optimization_request_sha256);
    }
}

fn goal_learning_runtime_status(
    optimization_request_sha256: &str,
) -> Option<GoalLearningRuntimeStatus> {
    goal_learning_runs()
        .lock()
        .ok()?
        .values()
        .find_map(|entry| {
            (entry.optimization_request_sha256 == optimization_request_sha256)
                .then(|| entry.status.clone())
        })
}

pub(super) fn goal_learning_projection(
    root: &Path,
    optimization: &OptimizationRequest,
    runtime_config: Option<&WorkbenchConfig>,
) -> GraphGoalLearningLoop {
    let mut projection = GraphGoalLearningLoop {
        status: "blocked".into(),
        request: None,
        request_sha256: None,
        generation_limit: GOAL_LEARNING_GENERATIONS,
        committed_generations: 0,
        rollouts_per_generation: optimization.execution.workers,
        charged_simulated_ticks: 0,
        simulated_tick_budget: minimum_tick_budget(optimization).unwrap_or(0),
        active_corpus_sha256: None,
        proposal_source: None,
        stopped_reason: None,
        cold_replayable_tapes: 0,
        collapse: None,
        blocker: None,
        error: None,
    };
    let execution = match load_goal_learning_execution(root, optimization) {
        Ok(execution) => execution,
        Err(error) => {
            projection.blocker = Some(error.to_string());
            apply_runtime_projection(&mut projection, optimization);
            return projection;
        }
    };
    if let Err(error) = goal_learning_initial_inputs(root, optimization, &execution) {
        projection.blocker = Some(error.to_string());
        apply_runtime_projection(&mut projection, optimization);
        return projection;
    }
    if let Some(config) = runtime_config
        && let Some(blocker) = optimization_runtime_blocker(root, config)
    {
        projection.blocker = Some(blocker);
    } else {
        projection.status = "ready".into();
    }

    let request_path = match goal_learning_request_path(root, optimization) {
        Ok(path) => path,
        Err(error) => {
            projection.status = "invalid".into();
            projection.error = Some(error.to_string());
            apply_runtime_projection(&mut projection, optimization);
            return projection;
        }
    };
    if request_path.exists() {
        let request: NativeGoalLearningLoopRequest = match bounded_json(&request_path) {
            Some(request) => request,
            None => {
                projection.status = "invalid".into();
                projection.error = Some("goal-learning request is invalid or oversized".into());
                apply_runtime_projection(&mut projection, optimization);
                return projection;
            }
        };
        projection.request = request_path
            .strip_prefix(root)
            .ok()
            .map(repository_path_text);
        projection.request_sha256 = Some(request.content_sha256.to_string());
        projection.generation_limit = request.generation_limit;
        projection.rollouts_per_generation = request.rollouts_per_generation;
        projection.simulated_tick_budget = request.simulated_tick_budget;
        if let Err(error) = request.validate_files(root, optimization, &execution) {
            projection.status = "invalid".into();
            projection.error = Some(error.to_string());
        } else if root.join(&request.resume.journal_path).exists() {
            match inspect_native_goal_learning_loop(&request, root, optimization, &execution) {
                Ok(state) => {
                    projection.committed_generations = state.committed_generations;
                    projection.charged_simulated_ticks = state.charged_simulated_ticks;
                    projection.active_corpus_sha256 = Some(state.active_corpus_sha256.to_string());
                    projection.cold_replayable_tapes = state
                        .generations
                        .iter()
                        .filter_map(|generation| generation.realized_tapes.as_ref())
                        .map(|tapes| tapes.len() as u64)
                        .sum();
                    projection.collapse = state.generations.iter().rev().find_map(|generation| {
                        let reference = generation.collapse_diagnostics.as_ref()?;
                        let report: NativePolicyCollapseReport =
                            bound_artifact_json(root, reference)?;
                        report.validate().ok()?;
                        Some(GraphGoalLearningCollapse {
                            generation: report.generation,
                            collapse_detected: report.collapse_detected,
                            warnings: report
                                .warnings
                                .iter()
                                .map(|warning| collapse_warning_name(*warning).into())
                                .collect(),
                            rollouts: report.rollouts,
                            unique_parent_states: report.unique_parent_states,
                            unique_consumed_actions: report.unique_consumed_actions,
                            unique_action_trajectories: report.unique_action_trajectories,
                            unique_state_identities: report.unique_state_identities,
                            contact_observations: report.contact_observations,
                            unique_contact_signatures: report.unique_contact_signatures,
                            successes: report.successes,
                            failures: report.failures,
                            unique_success_ticks: report.unique_success_ticks,
                            artifact: reference.path.clone(),
                        })
                    });
                    if let Some(stopped) = state.stopped {
                        projection.proposal_source =
                            Some(proposal_source_name(stopped.proposal_source).into());
                        projection.stopped_reason = Some(stop_reason_name(stopped.reason).into());
                        projection.status = match stopped.reason {
                            NativeGoalLearningStopReason::GenerationLimitReached => "completed",
                            NativeGoalLearningStopReason::SimulatedTickBudgetReached => "completed",
                            NativeGoalLearningStopReason::HeldOutReachabilityRejected
                            | NativeGoalLearningStopReason::HeldOutPolicyRejected => "fallback",
                        }
                        .into();
                    } else {
                        projection.status = "resumable".into();
                    }
                }
                Err(error) => {
                    projection.status = "invalid".into();
                    projection.error = Some(error.to_string());
                }
            }
        }
    }
    apply_runtime_projection(&mut projection, optimization);
    projection
}

fn apply_runtime_projection(
    projection: &mut GraphGoalLearningLoop,
    optimization: &OptimizationRequest,
) {
    if let Some(runtime) = goal_learning_runtime_status(&optimization.content_sha256.to_string()) {
        projection.status = runtime.status.into();
        if runtime.error.is_some() {
            projection.error = runtime.error;
        }
    }
}

pub(super) fn start_goal_learning_campaign(
    config: &WorkbenchConfig,
    browser: &BrowserGoalLearningStartRequest,
) -> Result<GoalLearningStartResponse, WorkbenchError> {
    let _lifecycle = optimization_lifecycle_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("optimization lifecycle lock is unavailable"))?;
    let root = config
        .repository_root
        .canonicalize()
        .map_err(goal_learning_error)?;
    let lifecycle = BrowserOptimizationLifecycleRequest {
        campaign: browser.campaign.clone(),
        request_sha256: campaign_request_sha256(config, &browser.campaign)?,
    };
    let (_, optimization) = checked_optimization_request(config, &lifecycle)?;
    if optimization_runtime_status(&optimization.content_sha256.to_string())
        .is_some_and(|status| matches!(status.status, "preparing" | "running" | "cancelling"))
    {
        return Err(WorkbenchError::new(
            "residual optimization must stop before goal learning starts",
        ));
    }
    if optimization_request_promotion_active(&optimization.content_sha256.to_string()) {
        return Err(WorkbenchError::new(
            "candidate promotion must finish before goal learning starts",
        ));
    }
    let world_context = config.world_context.as_ref().ok_or_else(|| {
        WorkbenchError::new(
            "goal learning requires a sealed world context; restart the workbench with --world-context WORLD.json",
        )
    })?;
    let execution = prepare_optimization_execution(
        &root,
        &optimization,
        &config.game,
        &config.dvd,
        world_context,
    )?;
    let request = prepare_goal_learning_request(&root, &optimization, &execution)?;
    let request_sha256 = request.content_sha256.to_string();
    let cancellation = Arc::new(AtomicBool::new(false));
    {
        let mut runs = goal_learning_runs()
            .lock()
            .map_err(|_| WorkbenchError::new("goal-learning runtime registry is unavailable"))?;
        if runs.values().any(|entry| {
            entry.optimization_request_sha256 == optimization.content_sha256.to_string()
                && matches!(entry.status.status, "preparing" | "running" | "cancelling")
        }) {
            return Err(WorkbenchError::new("goal-learning loop is already running"));
        }
        runs.retain(|_, entry| {
            entry.optimization_request_sha256 != optimization.content_sha256.to_string()
        });
        runs.insert(
            request_sha256.clone(),
            GoalLearningRuntimeEntry {
                optimization_request_sha256: optimization.content_sha256.to_string(),
                status: GoalLearningRuntimeStatus {
                    status: "running",
                    error: None,
                },
                cancellation: Arc::clone(&cancellation),
            },
        );
    }
    let thread_request_sha256 = request_sha256.clone();
    let thread_cancellation = Arc::clone(&cancellation);
    let spawn = thread::Builder::new()
        .name(format!("goal-learning-{}", optimization.id))
        .spawn(move || {
            let result = run_native_goal_learning_loop(&NativeGoalLearningLoopRunConfig {
                repository_root: &root,
                request: &request,
                optimization: &optimization,
                execution: &execution,
                cancellation: Some(&thread_cancellation),
            });
            let status = match result {
                Ok(summary) => GoalLearningRuntimeStatus {
                    status: match summary.stopped_reason {
                        NativeGoalLearningStopReason::HeldOutReachabilityRejected
                        | NativeGoalLearningStopReason::HeldOutPolicyRejected => "fallback",
                        NativeGoalLearningStopReason::GenerationLimitReached
                        | NativeGoalLearningStopReason::SimulatedTickBudgetReached => "completed",
                    },
                    error: None,
                },
                Err(error) if error.is_cancelled() => GoalLearningRuntimeStatus {
                    status: "cancelled",
                    error: None,
                },
                Err(error) => GoalLearningRuntimeStatus {
                    status: "failed",
                    error: Some(error.to_string()),
                },
            };
            set_goal_learning_runtime_status(&thread_request_sha256, status);
        });
    if let Err(error) = spawn {
        let message = format!("cannot start goal-learning thread: {error}");
        set_goal_learning_runtime_status(
            &request_sha256,
            GoalLearningRuntimeStatus {
                status: "failed",
                error: Some(message.clone()),
            },
        );
        return Err(WorkbenchError::new(message));
    }
    Ok(GoalLearningStartResponse {
        schema: GOAL_LEARNING_START_SCHEMA,
        campaign: browser.campaign.clone(),
        request_sha256,
        status: "running",
    })
}

pub(super) fn cancel_goal_learning_campaign(
    config: &WorkbenchConfig,
    browser: &BrowserGoalLearningLifecycleRequest,
) -> Result<GoalLearningLifecycleResponse, WorkbenchError> {
    let _lifecycle = optimization_lifecycle_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("optimization lifecycle lock is unavailable"))?;
    let expected = campaign_request_sha256(config, &browser.campaign)?;
    let lifecycle = BrowserOptimizationLifecycleRequest {
        campaign: browser.campaign.clone(),
        request_sha256: expected,
    };
    let (_, optimization) = checked_optimization_request(config, &lifecycle)?;
    let request_path = goal_learning_request_path(
        &config
            .repository_root
            .canonicalize()
            .map_err(goal_learning_error)?,
        &optimization,
    )?;
    let request: NativeGoalLearningLoopRequest = bounded_json(&request_path)
        .ok_or_else(|| WorkbenchError::new("goal-learning request is absent or invalid"))?;
    if request.content_sha256.to_string() != browser.request_sha256 {
        return Err(WorkbenchError::new(
            "goal-learning request changed; refresh before cancelling",
        ));
    }
    let mut runs = goal_learning_runs()
        .lock()
        .map_err(|_| WorkbenchError::new("goal-learning runtime registry is unavailable"))?;
    let entry = runs
        .get_mut(&browser.request_sha256)
        .ok_or_else(|| WorkbenchError::new("goal-learning loop is not running"))?;
    match entry.status.status {
        "preparing" | "running" => {
            entry.cancellation.store(true, Ordering::Release);
            entry.status = GoalLearningRuntimeStatus {
                status: "cancelling",
                error: None,
            };
        }
        "cancelling" | "cancelled" => {}
        _ => return Err(WorkbenchError::new("goal-learning loop is not running")),
    }
    Ok(GoalLearningLifecycleResponse {
        schema: GOAL_LEARNING_LIFECYCLE_SCHEMA,
        campaign: browser.campaign.clone(),
        request_sha256: browser.request_sha256.clone(),
        status: entry.status.status,
    })
}

fn campaign_request_sha256(
    config: &WorkbenchConfig,
    campaign_id: &str,
) -> Result<String, WorkbenchError> {
    let root = config
        .repository_root
        .canonicalize()
        .map_err(goal_learning_error)?;
    let timeline = load_authoritative_timeline(&config.timeline_path)?;
    let artifact_root = configured_artifact_root(config)?;
    let mut graph = graph_with_drafts(&timeline, &artifact_root, &config.state_root)?;
    append_optimization_campaigns(&mut graph, &root, &config.timeline_path, Some(config))?;
    graph
        .campaigns
        .iter()
        .find(|campaign| campaign.id == campaign_id)
        .map(|campaign| campaign.request_sha256.clone())
        .ok_or_else(|| WorkbenchError::new("unknown optimization campaign"))
}

fn prepare_goal_learning_request(
    root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
) -> Result<NativeGoalLearningLoopRequest, WorkbenchError> {
    let path = goal_learning_request_path(root, optimization)?;
    if path.exists() {
        let request: NativeGoalLearningLoopRequest = bounded_json(&path)
            .ok_or_else(|| WorkbenchError::new("existing goal-learning request is invalid"))?;
        request
            .validate_files(root, optimization, execution)
            .map_err(goal_learning_error)?;
        return Ok(request);
    }
    let (initial_replay_corpus, initial_episode_shards) =
        goal_learning_initial_inputs(root, optimization, execution)?;
    let campaign = optimization_campaign_root(root, optimization)?;
    let learning = campaign.join("learning-loop");
    let relative = learning
        .strip_prefix(root)
        .map_err(goal_learning_error)?
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/");
    let request = NativeGoalLearningLoopRequest::seal(
        optimization,
        execution,
        initial_replay_corpus,
        initial_episode_shards,
        GOAL_LEARNING_GENERATIONS,
        optimization.execution.workers,
        minimum_tick_budget(optimization)?,
        NativeGoalTrajectoryConfig::default(),
        NativeGoalReachabilityConfig::default(),
        NativeGoalFrozenPolicyConfig::default(),
        NativeGoalLearningLoopResume {
            journal_path: format!("{relative}/journal.jsonl"),
            state_path: format!("{relative}/state.json"),
            artifact_root: format!("{relative}/artifacts"),
        },
    )
    .map_err(goal_learning_error)?;
    request
        .validate_files(root, optimization, execution)
        .map_err(goal_learning_error)?;
    write_exact_or_new(
        &path,
        &request.to_pretty_json().map_err(goal_learning_error)?,
    )?;
    Ok(request)
}

fn goal_learning_initial_inputs(
    root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
) -> Result<(ArtifactReference, Vec<ArtifactReference>), WorkbenchError> {
    let state: OptimizationResumeState = bounded_json(&root.join(&optimization.resume.state_path))
        .filter(|state: &OptimizationResumeState| {
            state.request_sha256 == optimization.content_sha256 && state.validate().is_ok()
        })
        .ok_or_else(|| {
            WorkbenchError::new("run residual optimization to create authenticated replay first")
        })?;
    let checkpoint_reference = state
        .latest_optimizer_checkpoint
        .as_ref()
        .map(|checkpoint| &checkpoint.artifact)
        .ok_or_else(|| WorkbenchError::new("residual optimization has no checkpoint"))?;
    let checkpoint: ResidualCampaignCheckpoint = bound_artifact_json(root, checkpoint_reference)
        .filter(|checkpoint: &ResidualCampaignCheckpoint| {
            checkpoint
                .validate(optimization, execution.content_sha256)
                .is_ok()
        })
        .ok_or_else(|| WorkbenchError::new("residual checkpoint is invalid or detached"))?;
    let replay: ResidualReplayCheckpoint = checkpoint.replay_corpus.ok_or_else(|| {
        WorkbenchError::new("complete one residual generation to create replay experience")
    })?;
    let corpus: NativeReplayCorpus = bound_artifact_json(root, &replay.artifact)
        .filter(|corpus: &NativeReplayCorpus| replay.validate_corpus(corpus).is_ok())
        .ok_or_else(|| WorkbenchError::new("residual replay corpus is invalid or detached"))?;
    let required = corpus
        .entries
        .iter()
        .map(|entry| entry.shard_sha256)
        .collect::<BTreeSet<_>>();
    let mut shards = BTreeMap::new();
    if let Some(reference) = &state.demonstration {
        let demonstration: NativeIncumbentDemonstration = bound_artifact_json(root, reference)
            .filter(|demonstration: &NativeIncumbentDemonstration| {
                demonstration.validate(optimization, execution).is_ok()
            })
            .ok_or_else(|| WorkbenchError::new("incumbent demonstration is invalid or detached"))?;
        let shard = demonstration.attempt.episode_shard;
        if required.contains(&shard.sha256) {
            shards.insert(shard.sha256, shard);
        }
    }
    for result in state
        .candidates
        .iter()
        .filter_map(|candidate| candidate.result.as_ref())
    {
        let Some(evaluation) =
            bound_artifact_json::<NativeResidualCampaignEvaluation>(root, result)
        else {
            continue;
        };
        for attempt in evaluation.attempts {
            if required.contains(&attempt.episode_shard.sha256)
                && let Some(previous) =
                    shards.insert(attempt.episode_shard.sha256, attempt.episode_shard.clone())
                && previous != attempt.episode_shard
            {
                return Err(WorkbenchError::new(
                    "residual replay shard identity has conflicting paths",
                ));
            }
        }
    }
    if shards.keys().copied().collect::<BTreeSet<_>>() != required {
        return Err(WorkbenchError::new(
            "residual replay corpus does not resolve every native episode shard",
        ));
    }
    Ok((replay.artifact, shards.into_values().collect()))
}

fn load_goal_learning_execution(
    root: &Path,
    optimization: &OptimizationRequest,
) -> Result<NativeResidualExecutionBinding, WorkbenchError> {
    let path = optimization_campaign_root(root, optimization)?.join("execution/execution.json");
    bounded_json::<NativeResidualExecutionBinding>(&path)
        .filter(|execution| execution.validate_files(root, optimization).is_ok())
        .ok_or_else(|| {
            WorkbenchError::new(
                "run residual optimization to materialize its native execution binding",
            )
        })
}

fn goal_learning_request_path(
    root: &Path,
    optimization: &OptimizationRequest,
) -> Result<PathBuf, WorkbenchError> {
    Ok(optimization_campaign_root(root, optimization)?
        .join("learning-loop")
        .join("request.json"))
}

fn minimum_tick_budget(optimization: &OptimizationRequest) -> Result<u64, WorkbenchError> {
    u64::from(GOAL_LEARNING_GENERATIONS)
        .checked_mul(u64::from(optimization.execution.workers))
        .and_then(|rollouts| rollouts.checked_mul(optimization.budgets.exploration_horizon_ticks))
        .ok_or_else(|| WorkbenchError::new("goal-learning tick budget overflowed"))
}

fn set_goal_learning_runtime_status(key: &str, status: GoalLearningRuntimeStatus) {
    if let Ok(mut runs) = goal_learning_runs().lock()
        && let Some(entry) = runs.get_mut(key)
    {
        entry.status = status;
    }
}

fn proposal_source_name(source: NativeGoalLearningProposalSource) -> &'static str {
    match source {
        NativeGoalLearningProposalSource::FrozenGoalPolicy => "frozen_goal_policy",
        NativeGoalLearningProposalSource::RetainedBaseline => "retained_baseline",
    }
}

fn stop_reason_name(reason: NativeGoalLearningStopReason) -> &'static str {
    match reason {
        NativeGoalLearningStopReason::GenerationLimitReached => "generation_limit_reached",
        NativeGoalLearningStopReason::SimulatedTickBudgetReached => "simulated_tick_budget_reached",
        NativeGoalLearningStopReason::HeldOutReachabilityRejected => {
            "held_out_reachability_rejected"
        }
        NativeGoalLearningStopReason::HeldOutPolicyRejected => "held_out_policy_rejected",
    }
}

fn collapse_warning_name(warning: NativePolicyCollapseWarning) -> &'static str {
    match warning {
        NativePolicyCollapseWarning::InsufficientRollouts => "insufficient_rollouts",
        NativePolicyCollapseWarning::SingleParentState => "single_parent_state",
        NativePolicyCollapseWarning::SingleConsumedAction => "single_consumed_action",
        NativePolicyCollapseWarning::SingleActionTrajectory => "single_action_trajectory",
        NativePolicyCollapseWarning::SingleStateIdentity => "single_state_identity",
        NativePolicyCollapseWarning::SingleContactSignature => "single_contact_signature",
        NativePolicyCollapseWarning::NoTerminalSuccess => "no_terminal_success",
    }
}

fn goal_learning_error(error: impl fmt::Display) -> WorkbenchError {
    WorkbenchError::new(error.to_string())
}

fn repository_path_text(path: &Path) -> String {
    path.to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_registry_is_scoped_to_one_learning_request() {
        let key = format!("goal-learning-cancel-test-{}", std::process::id());
        let optimization = format!("optimization-{}", std::process::id());
        let cancellation = Arc::new(AtomicBool::new(false));
        goal_learning_runs().lock().unwrap().insert(
            key.clone(),
            GoalLearningRuntimeEntry {
                optimization_request_sha256: optimization.clone(),
                status: GoalLearningRuntimeStatus {
                    status: "running",
                    error: None,
                },
                cancellation: Arc::clone(&cancellation),
            },
        );
        assert!(goal_learning_campaign_active(&optimization));
        {
            let mut runs = goal_learning_runs().lock().unwrap();
            let entry = runs.get_mut(&key).unwrap();
            entry.cancellation.store(true, Ordering::Release);
            entry.status.status = "cancelling";
        }
        assert!(cancellation.load(Ordering::Acquire));
        assert!(goal_learning_campaign_active(&optimization));
        set_goal_learning_runtime_status(
            &key,
            GoalLearningRuntimeStatus {
                status: "cancelled",
                error: None,
            },
        );
        assert!(!goal_learning_campaign_active(&optimization));
        goal_learning_runs().lock().unwrap().remove(&key);
    }
}
