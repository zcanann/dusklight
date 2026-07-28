//! Workbench projection and launch boundary for tactic-level route learning.

use super::*;
use dusklight_learning::tactic_exploration::TacticProposalPolicy;
use dusklight_orchestration::native_tactic_route_runner::{
    NATIVE_TACTIC_DECISION_SUMMARY_SCHEMA_V1, NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V4,
    NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V5, NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V6,
    NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V7, NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V8,
    NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V9, NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V10,
    NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V11, NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V12,
    NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V13, NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V14,
    NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V15, NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V16,
    NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V17, NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V18,
    NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V19, NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V20,
    NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V21, NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V22,
    NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V23, NativeTacticDecisionTrace, NativeTacticExecutionPlan,
    NativeTacticExecutionPlanRequest, NativeTacticPlanBudgets, NativeTacticReplaySharingPlan,
    NativeTacticResourceLimit, NativeTacticRouteRunConfig, has_tactic_decision_journal,
    materialize_tactic_decision_route, project_tactic_decision_graph, read_tactic_decision_journal,
    run_native_tactic_route,
};
use dusklight_orchestration::native_tactic_worker::NativeGenericExecutionStrategy;
use dusklight_orchestration::optimization_request::OptimizationRequest;
use dusklight_orchestration::tactic_q_campaign::TACTIC_Q_CHECKPOINT_EXTENSION;
use serde_json::Value;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const TACTIC_ROUTE_START_SCHEMA: &str = "dusklight.route-workbench.tactic-route-start.v1";
const TACTIC_ROUTE_LIFECYCLE_SCHEMA: &str = "dusklight.route-workbench.tactic-route-lifecycle.v1";
const TACTIC_ROUTE_CANCEL_MARKER_SCHEMA: &str =
    "dusklight.route-workbench.tactic-route-cancelled.v1";
const TACTIC_ROUTE_DECISION_DETAIL_SCHEMA: &str =
    "dusklight.route-workbench.tactic-route-decision-detail.v1";
const TACTIC_ROUTE_DECISIONS_PER_SEED: u64 = 256;
const TACTIC_ROUTE_BRANCH_EVERY_DECISIONS: u64 = 8;
const TACTIC_ROUTE_REFIT_EVERY_DECISIONS: u64 = 32;
const TACTIC_ROUTE_EPSILON_PER_MILLION: u32 = 600_000;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserTacticRouteStartRequest {
    pub campaign: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserTacticRouteReplayRequest {
    pub campaign: String,
    pub request_sha256: String,
    pub seed: u64,
    pub edge_index: u64,
    #[serde(default = "default_takeover")]
    pub handoff: bool,
    #[serde(
        default = "default_speed_percent",
        deserialize_with = "deserialize_playback_speed_percent"
    )]
    pub speed_percent: u16,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserTacticRouteDecisionDetailRequest {
    pub campaign: String,
    pub request_sha256: String,
    pub seed: u64,
    pub decision_index: u64,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct TacticRouteStartResponse {
    pub schema: &'static str,
    pub campaign: String,
    pub optimization_request_sha256: String,
    pub output: String,
    pub status: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct TacticRouteLifecycleResponse {
    pub schema: &'static str,
    pub campaign: String,
    pub optimization_request_sha256: String,
    pub status: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct TacticRouteDecisionDetailResponse {
    pub schema: &'static str,
    pub campaign: String,
    pub optimization_request_sha256: String,
    pub seed: u64,
    pub decision: GraphTacticDecisionTrace,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredTacticDecisionSummary {
    schema: String,
    decision_index: u64,
    episode: u64,
    selected_option_id: String,
    selection_reason: String,
    reward: f32,
    duration_ticks: u32,
    goal_distance_before: f32,
    goal_distance_after: f32,
    terminal: bool,
}

#[derive(Clone, Debug)]
struct TacticRouteRuntimeStatus {
    status: &'static str,
    error: Option<String>,
}

#[derive(Clone, Debug)]
struct TacticRouteRuntimeEntry {
    status: TacticRouteRuntimeStatus,
    cancellation: Arc<AtomicBool>,
}

fn tactic_route_runs() -> &'static Mutex<BTreeMap<String, TacticRouteRuntimeEntry>> {
    static RUNS: OnceLock<Mutex<BTreeMap<String, TacticRouteRuntimeEntry>>> = OnceLock::new();
    RUNS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub(super) fn tactic_route_campaign_active(optimization_request_sha256: &str) -> bool {
    tactic_route_runs().lock().ok().is_some_and(|runs| {
        runs.get(optimization_request_sha256).is_some_and(|run| {
            matches!(
                run.status.status,
                "preparing" | "running" | "pausing" | "cancelling"
            )
        })
    })
}

pub(super) fn forget_tactic_route_campaign(optimization_request_sha256: &str) {
    if let Ok(mut runs) = tactic_route_runs().lock() {
        runs.remove(optimization_request_sha256);
    }
}

pub(super) fn tactic_route_learning_projection(
    root: &Path,
    optimization: &OptimizationRequest,
    runtime_config: Option<&WorkbenchConfig>,
) -> GraphTacticRouteLearning {
    let seeds = optimization.execution.deterministic_seeds.clone();
    let mut projection = GraphTacticRouteLearning {
        status: "ready".into(),
        goal: optimization.terminal_predicate.goal.clone(),
        source_boundary_index: optimization.route.source_boundary_index,
        exploration_seeds: seeds.clone(),
        decisions_per_seed: TACTIC_ROUTE_DECISIONS_PER_SEED,
        branch_every_decisions: TACTIC_ROUTE_BRANCH_EVERY_DECISIONS,
        refit_every_decisions: TACTIC_ROUTE_REFIT_EVERY_DECISIONS,
        epsilon_per_million: TACTIC_ROUTE_EPSILON_PER_MILLION,
        completed_seeds: 0,
        successful_seeds: 0,
        total_decisions: 0,
        total_native_ticks: 0,
        useful_decisions: 0,
        throughput: None,
        latest_decision: None,
        learned_graph: None,
        output: None,
        report: None,
        blocker: None,
        error: None,
    };
    if runtime_config
        .and_then(|config| config.world_context.as_ref())
        .is_none()
    {
        projection.status = "blocked".into();
        projection.blocker = Some(
            "restart the workbench with --world-context WORLD.json to learn this route".into(),
        );
    } else if let Some(config) = runtime_config
        && let Some(blocker) = optimization_runtime_blocker(root, config)
    {
        projection.status = "blocked".into();
        projection.blocker = Some(blocker);
    }

    let output = match tactic_route_output_root(root, optimization) {
        Ok(output) => output,
        Err(error) => {
            projection.status = "invalid".into();
            projection.error = Some(error.to_string());
            return projection;
        }
    };
    projection.output = output.strip_prefix(root).ok().map(repository_path_text);
    project_completed_seed_results(&output, &seeds, &mut projection);
    projection.latest_decision = project_latest_decision(&output, &seeds);
    projection.learned_graph = project_latest_graph(&output, &seeds);
    let report_path = output.join("report.json");
    if report_path.exists() {
        match bounded_json::<Value>(&report_path) {
            Some(report)
                if matches!(
                    report.get("schema").and_then(Value::as_str),
                    Some(schema)
                        if schema == NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V4
                            || schema == NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V5
                            || schema == NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V6
                            || schema == NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V7
                            || schema == NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V8
                            || schema == NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V9
                            || schema == NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V10
                            || schema == NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V11
                            || schema == NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V12
                            || schema == NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V13
                            || schema == NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V14
                            || schema == NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V15
                            || schema == NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V16
                            || schema == NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V17
                            || schema == NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V18
                            || schema == NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V19
                            || schema == NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V20
                            || schema == NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V21
                            || schema == NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V22
                            || schema == NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V23
                ) && report
                    .get("optimization_request_sha256")
                    .and_then(Value::as_str)
                    == Some(&optimization.content_sha256.to_string()) =>
            {
                projection.report = report_path
                    .strip_prefix(root)
                    .ok()
                    .map(repository_path_text);
                projection.completed_seeds = report
                    .get("seeds")
                    .and_then(Value::as_array)
                    .map_or(0, |seeds| seeds.len() as u64);
                projection.successful_seeds = report
                    .get("successful_seeds")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                projection.total_decisions = report
                    .get("total_decisions")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                projection.total_native_ticks = report
                    .get("total_native_ticks")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                projection.useful_decisions = report
                    .get("useful_decisions")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                projection.throughput = report.get("timing").and_then(|timing| {
                    serde_json::from_value::<GraphTacticRouteThroughput>(timing.clone()).ok()
                });
                projection.status = if projection.successful_seeds > 0 {
                    "succeeded"
                } else {
                    "completed"
                }
                .into();
            }
            _ => {
                projection.status = "invalid".into();
                projection.error = Some(
                    "tactic-route report is invalid, oversized, or belongs to another request"
                        .into(),
                );
            }
        }
    } else if output.exists() {
        if tactic_route_cancel_marker_valid(&output, &optimization.content_sha256.to_string()) {
            projection.status = "cancelled".into();
        } else if tactic_route_pause_evidence_exists(&output) {
            projection.status = "paused".into();
        } else {
            projection.status = "interrupted".into();
            projection.error = Some(
                "the tactic-route output is incomplete and has no immutable pause checkpoint"
                    .into(),
            );
        }
    }

    if let Ok(runs) = tactic_route_runs().lock()
        && let Some(runtime) = runs.get(&optimization.content_sha256.to_string())
    {
        projection.status = runtime.status.status.into();
        if runtime.status.error.is_some() {
            projection.error = runtime.status.error.clone();
        }
    }
    projection
}

pub(super) fn start_tactic_route_learning(
    config: &WorkbenchConfig,
    browser: &BrowserTacticRouteStartRequest,
) -> Result<TacticRouteStartResponse, WorkbenchError> {
    let _lifecycle = optimization_lifecycle_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("optimization lifecycle lock is unavailable"))?;
    let root = config
        .repository_root
        .canonicalize()
        .map_err(tactic_route_error)?;
    let optimization = selected_optimization(config, &root, &browser.campaign)?;
    launch_tactic_route_learning(config, root, optimization, browser.campaign.clone(), false)
}

pub(super) fn resume_tactic_route_learning(
    config: &WorkbenchConfig,
    browser: &BrowserOptimizationLifecycleRequest,
) -> Result<TacticRouteStartResponse, WorkbenchError> {
    let _lifecycle = optimization_lifecycle_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("optimization lifecycle lock is unavailable"))?;
    let (root, optimization) = checked_optimization_request(config, browser)?;
    launch_tactic_route_learning(config, root, optimization, browser.campaign.clone(), true)
}

fn launch_tactic_route_learning(
    config: &WorkbenchConfig,
    root: PathBuf,
    optimization: OptimizationRequest,
    campaign: String,
    resume: bool,
) -> Result<TacticRouteStartResponse, WorkbenchError> {
    let world_context = config.world_context.as_ref().ok_or_else(|| {
        WorkbenchError::new(
            "route learning requires a sealed world context; restart the workbench with --world-context WORLD.json",
        )
    })?;
    let request_sha256 = optimization.content_sha256.to_string();
    if optimization_runtime_status(&request_sha256)
        .is_some_and(|status| matches!(status.status, "preparing" | "running" | "cancelling"))
    {
        return Err(WorkbenchError::new(
            "residual optimization must stop before route learning starts",
        ));
    }
    let cancellation = Arc::new(AtomicBool::new(false));
    if optimization_request_promotion_active(&request_sha256) {
        return Err(WorkbenchError::new(
            "candidate promotion must finish before route learning starts",
        ));
    }
    let output = tactic_route_output_root(&root, &optimization)?;
    if output.exists() && !resume {
        return Err(WorkbenchError::new(
            "route-learning evidence already exists for this start and goal",
        ));
    }
    if resume
        && (tactic_route_cancel_marker_valid(&output, &request_sha256)
            || !tactic_route_pause_evidence_exists(&output))
    {
        return Err(WorkbenchError::new(
            "route learning has no immutable paused checkpoint to resume",
        ));
    }
    let execution = prepare_optimization_execution(
        &root,
        &optimization,
        &config.game,
        &config.dvd,
        world_context,
    )?;
    let output_text = output
        .strip_prefix(&root)
        .map(repository_path_text)
        .unwrap_or_else(|_| "build/campaigns".into());
    {
        let mut runs = tactic_route_runs()
            .lock()
            .map_err(|_| WorkbenchError::new("route-learning runtime registry is unavailable"))?;
        if runs.get(&request_sha256).is_some_and(|run| {
            matches!(
                run.status.status,
                "preparing" | "running" | "pausing" | "cancelling"
            )
        }) {
            return Err(WorkbenchError::new("route learning is already running"));
        }
        runs.insert(
            request_sha256.clone(),
            TacticRouteRuntimeEntry {
                status: TacticRouteRuntimeStatus {
                    status: "running",
                    error: None,
                },
                cancellation: Arc::clone(&cancellation),
            },
        );
    }
    let seeds = optimization.execution.deterministic_seeds.clone();
    let thread_request_sha256 = request_sha256.clone();
    let thread_cancellation = Arc::clone(&cancellation);
    let spawn = thread::Builder::new()
        .name(format!("tactic-route-{}", optimization.id))
        .spawn(move || {
            let execution_plan =
                NativeTacticExecutionPlan::build(NativeTacticExecutionPlanRequest {
                    seeds,
                    proposal_policy: TacticProposalPolicy::Learned,
                    execution_strategy: NativeGenericExecutionStrategy::NativeController,
                    lanes_per_generation: optimization
                        .execution
                        .deterministic_seeds
                        .len()
                        .min(4)
                        .max(1),
                    proposal_width_per_decision: 4,
                    branch_every_decisions: TACTIC_ROUTE_BRANCH_EVERY_DECISIONS,
                    refit_every_decisions: TACTIC_ROUTE_REFIT_EVERY_DECISIONS,
                    root_refresh_cadence: 4,
                    epsilon_per_million: TACTIC_ROUTE_EPSILON_PER_MILLION,
                    demonstration_chunk_ticks: None,
                    replay_sharing: NativeTacticReplaySharingPlan::GenerationBarrier,
                    budgets: NativeTacticPlanBudgets {
                        decisions_per_lane: TACTIC_ROUTE_DECISIONS_PER_SEED,
                        native_ticks: NativeTacticResourceLimit::Bounded(
                            optimization.budgets.simulated_tick_budget,
                        ),
                        memory_bytes: NativeTacticResourceLimit::Unbounded,
                        wall_micros: NativeTacticResourceLimit::Unbounded,
                    },
                });
            let result = execution_plan.and_then(|execution_plan| {
                run_native_tactic_route(&NativeTacticRouteRunConfig {
                    repository_root: &root,
                    optimization: &optimization,
                    execution: &execution,
                    execution_plan: &execution_plan,
                    output_root: &output,
                    workers: usize::from(optimization.execution.workers),
                    cancellation: Some(&thread_cancellation),
                    resume,
                })
            });
            let status = match result {
                Ok(report) if report.successful_seeds > 0 => TacticRouteRuntimeStatus {
                    status: "succeeded",
                    error: None,
                },
                Ok(_) => TacticRouteRuntimeStatus {
                    status: "completed",
                    error: None,
                },
                Err(error) if error.is_cancelled() => TacticRouteRuntimeStatus {
                    status: "paused",
                    error: None,
                },
                Err(error) => TacticRouteRuntimeStatus {
                    status: "failed",
                    error: Some(error.to_string()),
                },
            };
            if let Ok(mut runs) = tactic_route_runs().lock() {
                if let Some(entry) = runs.get_mut(&thread_request_sha256) {
                    entry.status =
                        if status.status == "paused" && entry.status.status == "cancelling" {
                            TacticRouteRuntimeStatus {
                                status: "cancelled",
                                error: None,
                            }
                        } else {
                            status
                        };
                }
            }
        });
    if let Err(error) = spawn {
        let message = format!("cannot start route-learning thread: {error}");
        if let Ok(mut runs) = tactic_route_runs().lock() {
            runs.insert(
                request_sha256.clone(),
                TacticRouteRuntimeEntry {
                    status: TacticRouteRuntimeStatus {
                        status: "failed",
                        error: Some(message.clone()),
                    },
                    cancellation,
                },
            );
        }
        return Err(WorkbenchError::new(message));
    }
    Ok(TacticRouteStartResponse {
        schema: TACTIC_ROUTE_START_SCHEMA,
        campaign,
        optimization_request_sha256: request_sha256,
        output: output_text,
        status: "running",
    })
}

pub(super) fn pause_tactic_route_learning(
    config: &WorkbenchConfig,
    browser: &BrowserOptimizationLifecycleRequest,
) -> Result<TacticRouteLifecycleResponse, WorkbenchError> {
    let _lifecycle = optimization_lifecycle_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("optimization lifecycle lock is unavailable"))?;
    let (_, optimization) = checked_optimization_request(config, browser)?;
    let request_sha256 = optimization.content_sha256.to_string();
    let status = request_tactic_route_pause(&request_sha256)?;
    Ok(TacticRouteLifecycleResponse {
        schema: TACTIC_ROUTE_LIFECYCLE_SCHEMA,
        campaign: optimization.id,
        optimization_request_sha256: request_sha256,
        status,
    })
}

pub(super) fn cancel_tactic_route_learning(
    config: &WorkbenchConfig,
    browser: &BrowserOptimizationLifecycleRequest,
) -> Result<TacticRouteLifecycleResponse, WorkbenchError> {
    let _lifecycle = optimization_lifecycle_edits()
        .lock()
        .map_err(|_| WorkbenchError::new("optimization lifecycle lock is unavailable"))?;
    let (root, optimization) = checked_optimization_request(config, browser)?;
    let request_sha256 = optimization.content_sha256.to_string();
    let output = tactic_route_output_root(&root, &optimization)?;
    let status = request_tactic_route_cancel(&request_sha256, &output)?;
    write_tactic_route_cancel_marker(&output, &request_sha256)?;
    Ok(TacticRouteLifecycleResponse {
        schema: TACTIC_ROUTE_LIFECYCLE_SCHEMA,
        campaign: optimization.id,
        optimization_request_sha256: request_sha256,
        status,
    })
}

fn request_tactic_route_pause(
    optimization_request_sha256: &str,
) -> Result<&'static str, WorkbenchError> {
    let mut runs = tactic_route_runs()
        .lock()
        .map_err(|_| WorkbenchError::new("route-learning runtime registry is unavailable"))?;
    let entry = runs
        .get_mut(optimization_request_sha256)
        .ok_or_else(|| WorkbenchError::new("route learning is not running"))?;
    match entry.status.status {
        "preparing" | "running" => {
            entry.cancellation.store(true, Ordering::Release);
            entry.status = TacticRouteRuntimeStatus {
                status: "pausing",
                error: None,
            };
            Ok("pausing")
        }
        "pausing" => Ok("pausing"),
        "paused" => Ok("paused"),
        _ => Err(WorkbenchError::new("route learning is not running")),
    }
}

fn request_tactic_route_cancel(
    optimization_request_sha256: &str,
    output: &Path,
) -> Result<&'static str, WorkbenchError> {
    let mut runs = tactic_route_runs()
        .lock()
        .map_err(|_| WorkbenchError::new("route-learning runtime registry is unavailable"))?;
    let Some(entry) = runs.get_mut(optimization_request_sha256) else {
        return if tactic_route_pause_evidence_exists(output) {
            Ok("cancelled")
        } else {
            Err(WorkbenchError::new(
                "route learning is not running or paused",
            ))
        };
    };
    match entry.status.status {
        "preparing" | "running" | "pausing" => {
            entry.cancellation.store(true, Ordering::Release);
            entry.status = TacticRouteRuntimeStatus {
                status: "cancelling",
                error: None,
            };
            Ok("cancelling")
        }
        "cancelling" => Ok("cancelling"),
        "paused" | "cancelled" => {
            entry.status = TacticRouteRuntimeStatus {
                status: "cancelled",
                error: None,
            };
            Ok("cancelled")
        }
        _ => Err(WorkbenchError::new(
            "route learning is not running or paused",
        )),
    }
}

pub(super) fn tactic_route_decision_detail(
    config: &WorkbenchConfig,
    browser: &BrowserTacticRouteDecisionDetailRequest,
) -> Result<TacticRouteDecisionDetailResponse, WorkbenchError> {
    let (root, optimization) = checked_optimization_request(
        config,
        &BrowserOptimizationLifecycleRequest {
            campaign: browser.campaign.clone(),
            request_sha256: browser.request_sha256.clone(),
        },
    )?;
    let request_sha256 = optimization.content_sha256.to_string();
    let seed_index = optimization
        .execution
        .deterministic_seeds
        .iter()
        .position(|seed| *seed == browser.seed)
        .ok_or_else(|| WorkbenchError::new("route-learning seed is not part of this campaign"))?;
    if browser.decision_index >= TACTIC_ROUTE_DECISIONS_PER_SEED {
        return Err(WorkbenchError::new(
            "route-learning decision is outside the campaign budget",
        ));
    }
    let output = tactic_route_output_root(&root, &optimization)?;
    let decision = load_tactic_route_decision_trace(
        &output,
        seed_index,
        browser.seed,
        browser.decision_index,
    )?;
    Ok(TacticRouteDecisionDetailResponse {
        schema: TACTIC_ROUTE_DECISION_DETAIL_SCHEMA,
        campaign: optimization.id,
        optimization_request_sha256: request_sha256,
        seed: browser.seed,
        decision,
    })
}

fn load_tactic_route_decision_trace(
    output: &Path,
    seed_index: usize,
    seed: u64,
    decision_index: u64,
) -> Result<GraphTacticDecisionTrace, WorkbenchError> {
    let seed_root = output.join(format!("seed-{seed_index:03}-{seed}"));
    if has_tactic_decision_journal(&seed_root) {
        let decisions = read_tactic_decision_journal(&seed_root).map_err(tactic_route_error)?;
        let decision = decisions
            .get(usize::try_from(decision_index).map_err(tactic_route_error)?)
            .ok_or_else(|| WorkbenchError::new("route-learning decision is not in the journal"))?;
        return project_native_decision_trace(decision);
    }
    let path = seed_root
        .join("decision-trace")
        .join(format!("decision-{decision_index:06}.json"));
    let metadata = fs::symlink_metadata(&path).map_err(tactic_route_error)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(WorkbenchError::new(
            "route-learning decision trace is absent or not a physical file",
        ));
    }
    let canonical_path = path.canonicalize().map_err(tactic_route_error)?;
    let canonical_output = output.canonicalize().map_err(tactic_route_error)?;
    if !canonical_path.starts_with(&canonical_output) {
        return Err(WorkbenchError::new(
            "route-learning decision trace escapes its campaign",
        ));
    }
    let decision: GraphTacticDecisionTrace = bounded_json(&canonical_path).ok_or_else(|| {
        WorkbenchError::new("route-learning decision trace is invalid or oversized")
    })?;
    if decision.decision_index != decision_index {
        return Err(WorkbenchError::new(
            "route-learning decision trace has a detached index",
        ));
    }
    Ok(decision)
}

fn project_native_decision_trace(
    decision: &NativeTacticDecisionTrace,
) -> Result<GraphTacticDecisionTrace, WorkbenchError> {
    serde_json::from_value(serde_json::to_value(decision).map_err(tactic_route_error)?)
        .map_err(tactic_route_error)
}

pub(super) fn replay_tactic_route_edge(
    config: &WorkbenchConfig,
    browser: &BrowserTacticRouteReplayRequest,
) -> Result<PlayResponse, WorkbenchError> {
    let root = config
        .repository_root
        .canonicalize()
        .map_err(tactic_route_error)?;
    let optimization = selected_optimization(config, &root, &browser.campaign)?;
    if optimization.content_sha256.to_string() != browser.request_sha256 {
        return Err(WorkbenchError::new(
            "route-learning request changed; refresh before replaying",
        ));
    }
    let seed_index = optimization
        .execution
        .deterministic_seeds
        .iter()
        .position(|seed| *seed == browser.seed)
        .ok_or_else(|| WorkbenchError::new("route-learning seed is not part of this campaign"))?;
    let output = tactic_route_output_root(&root, &optimization)?;
    let graph = project_latest_graph(&output, &optimization.execution.deterministic_seeds)
        .filter(|graph| graph.seed_index == seed_index && graph.seed == browser.seed)
        .ok_or_else(|| WorkbenchError::new("route-learning graph is not available"))?;
    let edge = graph
        .edges
        .iter()
        .find(|edge| edge.edge_index == browser.edge_index)
        .ok_or_else(|| WorkbenchError::new("route-learning edge is not available"))?;
    let tape = load_tactic_route_edge_tape(&output, seed_index, browser.seed, edge)?;
    let timeline = load_authoritative_timeline(&config.timeline_path)?;
    let materialized = MaterializedPlayback {
        lineage: None,
        segment: Some(format!(
            "tactic-route:{}:{}:{}",
            browser.campaign, browser.seed, browser.edge_index
        )),
        tape,
        seed_stage: None,
        native_oracle: NativePlaybackOracle::None,
    };
    let (response, _child) = launch_materialized(
        &timeline,
        config,
        materialized,
        MaterializedLaunchOptions {
            takeover: browser.handoff,
            origin: PlaybackOrigin::Boot,
            fast_forward_frames: None,
            thumbnail: None,
            playback: PlaybackSettings {
                speed_percent: browser.speed_percent,
                fast: false,
            },
        },
    )?;
    Ok(response)
}

fn load_tactic_route_edge_tape(
    output: &Path,
    seed_index: usize,
    seed: u64,
    edge: &GraphTacticKnowledgeEdge,
) -> Result<InputTape, WorkbenchError> {
    let seed_root = output.join(format!("seed-{seed_index:03}-{seed}"));
    if has_tactic_decision_journal(&seed_root) {
        let tape = materialize_tactic_decision_route(&seed_root, edge.edge_index)
            .map_err(tactic_route_error)?;
        if tape.frames.len() as u64 != edge.end_frame_exclusive {
            return Err(WorkbenchError::new(
                "materialized route-learning edge does not end at its authenticated boundary",
            ));
        }
        return Ok(tape);
    }
    let tape_path = output
        .join(format!("seed-{seed_index:03}-{seed}"))
        .join("edge-tapes")
        .join(format!("edge-{:06}.tape", edge.edge_index));
    let metadata = fs::symlink_metadata(&tape_path).map_err(tactic_route_error)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(WorkbenchError::new(
            "route-learning edge tape is absent or not a physical file",
        ));
    }
    let canonical_tape = tape_path.canonicalize().map_err(tactic_route_error)?;
    let canonical_output = output.canonicalize().map_err(tactic_route_error)?;
    if !canonical_tape.starts_with(&canonical_output) {
        return Err(WorkbenchError::new(
            "route-learning edge tape escapes its campaign",
        ));
    }
    let tape = InputTape::decode(&fs::read(&canonical_tape).map_err(tactic_route_error)?)
        .map_err(tactic_route_error)?
        .tape;
    if tape.frames.len() as u64 != edge.end_frame_exclusive {
        return Err(WorkbenchError::new(
            "route-learning edge tape does not end at the authenticated edge boundary",
        ));
    }
    Ok(tape)
}

fn selected_optimization(
    config: &WorkbenchConfig,
    root: &Path,
    campaign_id: &str,
) -> Result<OptimizationRequest, WorkbenchError> {
    let timeline = load_authoritative_timeline(&config.timeline_path)?;
    let artifact_root = configured_artifact_root(config)?;
    let mut graph = graph_with_drafts(&timeline, &artifact_root, &config.state_root)?;
    append_optimization_campaigns(&mut graph, root, &config.timeline_path, Some(config))?;
    let campaign = graph
        .campaigns
        .iter()
        .find(|campaign| campaign.id == campaign_id)
        .ok_or_else(|| WorkbenchError::new("unknown optimization campaign"))?;
    if campaign.status == "invalid" {
        return Err(WorkbenchError::new(
            campaign
                .error
                .as_deref()
                .unwrap_or("optimization campaign is invalid"),
        ));
    }
    if let Some(blocker) = &campaign.blocker {
        return Err(WorkbenchError::new(blocker));
    }
    let request_path = root.join(&campaign.request);
    let optimization: OptimizationRequest =
        serde_json::from_slice(&fs::read(request_path).map_err(tactic_route_error)?)
            .map_err(tactic_route_error)?;
    optimization
        .validate_files(root)
        .map_err(tactic_route_error)?;
    if optimization.id != campaign_id {
        return Err(WorkbenchError::new(
            "optimization request identity changed while launching route learning",
        ));
    }
    Ok(optimization)
}

fn tactic_route_output_root(
    root: &Path,
    optimization: &OptimizationRequest,
) -> Result<PathBuf, WorkbenchError> {
    Ok(optimization_campaign_root(root, optimization)?.join("tactic-route"))
}

fn tactic_route_pause_evidence_exists(output: &Path) -> bool {
    fs::read_dir(output)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_type()
                .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
                && entry.file_name().to_string_lossy().starts_with("seed-")
        })
        .any(|seed| {
            fs::read_dir(seed.path().join("pause-checkpoints"))
                .ok()
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry
                        .file_type()
                        .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
                })
                .any(|checkpoint| {
                    fs::read_dir(checkpoint.path())
                        .ok()
                        .into_iter()
                        .flatten()
                        .filter_map(Result::ok)
                        .any(|file| {
                            file.file_type()
                                .is_ok_and(|kind| kind.is_file() && !kind.is_symlink())
                                && file.file_name().to_string_lossy().starts_with("tactic-q-")
                                && file
                                    .path()
                                    .extension()
                                    .is_some_and(|value| value == TACTIC_Q_CHECKPOINT_EXTENSION)
                        })
                })
        })
}

fn tactic_route_cancel_marker_path(output: &Path) -> PathBuf {
    output.join("lifecycle").join("cancelled.json")
}

fn tactic_route_cancel_marker_valid(output: &Path, request_sha256: &str) -> bool {
    bounded_json::<Value>(&tactic_route_cancel_marker_path(output)).is_some_and(|marker| {
        marker.get("schema").and_then(Value::as_str) == Some(TACTIC_ROUTE_CANCEL_MARKER_SCHEMA)
            && marker
                .get("optimization_request_sha256")
                .and_then(Value::as_str)
                == Some(request_sha256)
    })
}

fn write_tactic_route_cancel_marker(
    output: &Path,
    request_sha256: &str,
) -> Result<(), WorkbenchError> {
    let path = tactic_route_cancel_marker_path(output);
    let parent = path
        .parent()
        .ok_or_else(|| WorkbenchError::new("route-learning cancel marker has no parent"))?;
    fs::create_dir_all(parent).map_err(tactic_route_error)?;
    let bytes = serde_json::to_vec_pretty(&serde_json::json!({
        "schema": TACTIC_ROUTE_CANCEL_MARKER_SCHEMA,
        "optimization_request_sha256": request_sha256,
    }))
    .map_err(tactic_route_error)?;
    if path.exists() {
        if fs::read(&path).map_err(tactic_route_error)? == bytes {
            return Ok(());
        }
        return Err(WorkbenchError::new(
            "route-learning cancel marker contains different evidence",
        ));
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(tactic_route_error)?
        .as_nanos();
    let partial = parent.join(format!(".cancelled.{}.{nonce}.partial", std::process::id()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&partial)
        .map_err(tactic_route_error)?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(tactic_route_error)?;
    fs::rename(partial, path).map_err(tactic_route_error)
}

fn project_completed_seed_results(
    output: &Path,
    seeds: &[u64],
    projection: &mut GraphTacticRouteLearning,
) {
    for (index, seed) in seeds.iter().enumerate() {
        let result_path = output
            .join(format!("seed-{index:03}-{seed}"))
            .join("seed-result.json");
        let Some(result) = bounded_json::<Value>(&result_path) else {
            continue;
        };
        projection.completed_seeds += 1;
        projection.successful_seeds += u64::from(
            result
                .get("success")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        );
        projection.total_decisions += result.get("decisions").and_then(Value::as_u64).unwrap_or(0);
        projection.total_native_ticks += result
            .get("native_ticks")
            .and_then(Value::as_u64)
            .unwrap_or(0);
    }
}

fn project_latest_decision(output: &Path, seeds: &[u64]) -> Option<GraphTacticDecisionSummary> {
    for (index, seed) in seeds.iter().enumerate().rev() {
        let seed_root = output.join(format!("seed-{index:03}-{seed}"));
        if has_tactic_decision_journal(&seed_root) {
            let decision = read_tactic_decision_journal(&seed_root)
                .ok()?
                .into_iter()
                .last()?;
            let selection_reason = serde_json::to_value(decision.selection_reason)
                .ok()?
                .as_str()?
                .to_owned();
            return Some(GraphTacticDecisionSummary {
                seed_index: index,
                seed: *seed,
                decision_index: decision.decision_index,
                episode: decision.episode,
                selected_option_id: decision.selected_option_id,
                selection_reason,
                reward: decision.reward,
                duration_ticks: decision.reward_components.duration_ticks,
                goal_distance_before: decision.goal_distance_before,
                goal_distance_after: decision.goal_distance_after,
                terminal: decision.terminal,
            });
        }
        let directory = seed_root.join("decision-summary");
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        let Some(latest) = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("decision-") && name.ends_with(".json"))
            })
            .max()
        else {
            continue;
        };
        let expected_index = latest
            .file_stem()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix("decision-"))
            .and_then(|index| index.parse::<u64>().ok());
        if let Some(summary) = bounded_json::<StoredTacticDecisionSummary>(&latest)
            && summary.schema == NATIVE_TACTIC_DECISION_SUMMARY_SCHEMA_V1
            && Some(summary.decision_index) == expected_index
        {
            return Some(GraphTacticDecisionSummary {
                seed_index: index,
                seed: *seed,
                decision_index: summary.decision_index,
                episode: summary.episode,
                selected_option_id: summary.selected_option_id,
                selection_reason: summary.selection_reason,
                reward: summary.reward,
                duration_ticks: summary.duration_ticks,
                goal_distance_before: summary.goal_distance_before,
                goal_distance_after: summary.goal_distance_after,
                terminal: summary.terminal,
            });
        }
    }
    None
}

fn project_latest_graph(output: &Path, seeds: &[u64]) -> Option<GraphTacticKnowledgeGraph> {
    for (index, seed) in seeds.iter().enumerate().rev() {
        let seed_root = output.join(format!("seed-{index:03}-{seed}"));
        if has_tactic_decision_journal(&seed_root)
            && let Ok(Some(projected)) = project_tactic_decision_graph(&seed_root)
        {
            let mut graph: GraphTacticKnowledgeGraph =
                serde_json::from_value(serde_json::to_value(projected).ok()?).ok()?;
            graph.seed_index = index;
            graph.seed = *seed;
            return Some(graph);
        }
        let final_graph = seed_root.join("graph.json");
        let graph_path = if final_graph.is_file() {
            final_graph
        } else {
            let Ok(entries) = fs::read_dir(seed_root.join("knowledge-graph")) else {
                continue;
            };
            let Some(latest) = entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("graph-") && name.ends_with(".json"))
                })
                .max()
            else {
                continue;
            };
            latest
        };
        let Some(value) = bounded_json::<Value>(&graph_path) else {
            continue;
        };
        if value.get("schema").and_then(Value::as_str)
            != Some("dusklight-tactic-campaign-graph-projection/v1")
        {
            continue;
        }
        if let Ok(mut graph) = serde_json::from_value::<GraphTacticKnowledgeGraph>(value) {
            graph.seed_index = index;
            graph.seed = *seed;
            return Some(graph);
        }
    }
    None
}

fn tactic_route_error(error: impl fmt::Display) -> WorkbenchError {
    WorkbenchError::new(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_report_projects_the_tactic_route_summary_without_loading_checkpoints() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../..")
            .canonicalize()
            .unwrap();
        let mut optimization: OptimizationRequest = serde_json::from_slice(
            &fs::read(root.join(
                "routes/Glitch Exhibition/intro/benchmarks/ordon-tactic-q-discovery-v1.request.json",
            ))
            .unwrap(),
        )
        .unwrap();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let relative = format!(
            "build/campaigns/tactic-route-projection-test-{}-{nonce}",
            std::process::id()
        );
        optimization.resume.state_path = format!("{relative}/state.json");
        optimization.resume.journal_path = format!("{relative}/journal.jsonl");
        optimization.refresh_content_sha256().unwrap();
        let output = root.join(&relative).join("tactic-route");
        fs::create_dir_all(&output).unwrap();
        let trace_root = output.join("seed-003-181081/decision-trace");
        fs::create_dir_all(&trace_root).unwrap();
        let state = GraphTacticStateTrace {
            snapshot_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .into(),
            stage: "F_SP103".into(),
            room: 1,
            layer: Some(0),
            point: Some(0),
            simulation_tick: 100,
            tape_frame: 506,
            player_position: [1.0, 2.0, 3.0],
            player_velocity: Some([0.0, 0.0, 1.0]),
            player_procedure: Some(7),
            player_contacts: Some(1),
            event_running: Some(false),
            event_id: Some(-1),
            terminal_reached: Some(false),
            actor_count: 3,
            same_room_actor_count: 2,
            recent_option_id: None,
        };
        fs::write(
            trace_root.join("decision-000069.json"),
            serde_json::to_vec(&GraphTacticDecisionTrace {
                decision_index: 69,
                episode: 18,
                selected_option_id: "goal.seek.coordinate.17".into(),
                selection_reason: "epsilon".into(),
                selected_q: Some(4.5),
                best_q: Some(5.0),
                reward: 96.0,
                reward_components: GraphTacticRewardTrace {
                    terminal_observed: true,
                    endpoint_novel: true,
                    duration_ticks: 32,
                    terminal_component: 100.0,
                    tick_cost_component: -0.032,
                    novelty_component: 0.05,
                    base_reward: 100.018,
                    potential: Some(GraphTacticPotentialRewardTrace {
                        source_potential: -4.0,
                        next_potential: 0.0,
                        effective_next_potential: 0.0,
                        shaping_reward: 4.0,
                        components: vec![GraphTacticPotentialComponentTrace {
                            name: "goal_distance".into(),
                            source_fact: 10.0,
                            next_fact: 0.0,
                            shaping_reward: 4.0,
                        }],
                    }),
                    training_reward: 96.0,
                },
                goal_distance_before: 10.0,
                goal_distance_after: 0.0,
                terminal: true,
                frontier_cells: 52,
                visited_states: 39,
                before: state.clone(),
                after: GraphTacticStateTrace {
                    terminal_reached: Some(true),
                    ..state
                },
                measurements: vec![GraphTacticMeasurementTrace {
                    name: "goal_planar_distance".into(),
                    before: 10.0,
                    after: 0.0,
                }],
                applicable_tactics: vec![GraphTacticValueTrace {
                    option_id: "goal.seek.coordinate.17".into(),
                    mean_q: Some(4.5),
                    ensemble_variance: Some(0.25),
                    selected: true,
                }],
            })
            .unwrap(),
        )
        .unwrap();
        let summary_root = output.join("seed-003-181081/decision-summary");
        fs::create_dir_all(&summary_root).unwrap();
        fs::write(
            summary_root.join("decision-000069.json"),
            serde_json::to_vec(&serde_json::json!({
                "schema": NATIVE_TACTIC_DECISION_SUMMARY_SCHEMA_V1,
                "decision_index": 69,
                "episode": 18,
                "selected_option_id": "goal.seek.coordinate.17",
                "selection_reason": "epsilon",
                "reward": 96.0,
                "duration_ticks": 32,
                "goal_distance_before": 10.0,
                "goal_distance_after": 0.0,
                "terminal": true
            }))
            .unwrap(),
        )
        .unwrap();
        let graph_root = output.join("seed-003-181081/knowledge-graph");
        fs::create_dir_all(&graph_root).unwrap();
        fs::write(
            graph_root.join("graph-000070.json"),
            serde_json::to_vec(&serde_json::json!({
                "schema": "dusklight-tactic-campaign-graph-projection/v1",
                "root_checkpoint_sha256":
                    "1111111111111111111111111111111111111111111111111111111111111111",
                "root_state_sha256":
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "root_connected": true,
                "frontier_cells": 1,
                "nodes": [
                    {
                        "checkpoint_sha256":
                            "1111111111111111111111111111111111111111111111111111111111111111",
                        "state_sha256":
                            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                        "stage": "F_SP103",
                        "room": 1,
                        "player_position": [1.0, 2.0, 3.0],
                        "terminal": false,
                        "retained_frontier": false,
                        "current": false
                    },
                    {
                        "checkpoint_sha256":
                            "2222222222222222222222222222222222222222222222222222222222222222",
                        "state_sha256":
                            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                        "stage": "F_SP103",
                        "room": 1,
                        "player_position": [9.0, 2.0, 3.0],
                        "terminal": true,
                        "retained_frontier": true,
                        "current": true
                    }
                ],
                "edges": [{
                    "edge_index": 69,
                    "episode_group": 18,
                    "before_state_sha256":
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "after_state_sha256":
                        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    "source_checkpoint_sha256":
                        "1111111111111111111111111111111111111111111111111111111111111111",
                    "next_checkpoint_sha256":
                        "2222222222222222222222222222222222222222222222222222222222222222",
                    "option_id": "goal.seek.coordinate.17",
                    "reward": 96.0,
                    "duration_ticks": 32,
                    "terminal": true,
                    "start_frame": 506,
                    "end_frame_exclusive": 538
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        let edge_tape_root = output.join("seed-003-181081/edge-tapes");
        fs::create_dir_all(&edge_tape_root).unwrap();
        fs::write(
            edge_tape_root.join("edge-000069.tape"),
            InputTape {
                frames: vec![crate::tape::InputFrame::default(); 538],
                ..InputTape::default()
            }
            .encode()
            .unwrap(),
        )
        .unwrap();
        fs::write(
            output.join("report.json"),
            serde_json::to_vec(&serde_json::json!({
                "schema": NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V19,
                "optimization_request_sha256": optimization.content_sha256,
                "successful_seeds": 1,
                "total_decisions": 70,
                "total_native_ticks": 2266,
                "useful_decisions": 23,
                "timing": {
                    "wall_micros": 2_000_000,
                    "tactic_selection_micros": 10,
                    "checkpoint_branching_micros": 20,
                    "tactic_execution_micros": 1_500_000,
                    "native_simulation_micros": 1_400_000,
                    "tactic_preparation_and_fact_extraction_micros": 100_000,
                    "model_update_micros": 200_000,
                    "evidence_projection_and_persistence_micros": 250_000,
                    "useful_decisions_per_second_millionths": 11_500_000,
                    "native_ticks_per_second_millionths": 1_133_000_000,
                    "episodes_per_second_millionths": 9_000_000
                },
                "seeds": [{"seed": 181081}]
            }))
            .unwrap(),
        )
        .unwrap();

        let projection = tactic_route_learning_projection(&root, &optimization, None);
        assert_eq!(projection.status, "succeeded");
        assert_eq!(projection.completed_seeds, 1);
        assert_eq!(projection.successful_seeds, 1);
        assert_eq!(projection.total_decisions, 70);
        assert_eq!(projection.total_native_ticks, 2266);
        assert_eq!(projection.useful_decisions, 23);
        assert_eq!(
            projection
                .throughput
                .as_ref()
                .map(|timing| timing.native_simulation_micros),
            Some(1_400_000)
        );
        assert_eq!(
            projection
                .latest_decision
                .as_ref()
                .map(|decision| decision.selected_option_id.as_str()),
            Some("goal.seek.coordinate.17")
        );
        let graph = projection.learned_graph.as_ref().unwrap();
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.frontier_cells, 1);
        assert_eq!(graph.seed_index, 3);
        assert_eq!(graph.seed, 181081);
        assert_eq!(
            load_tactic_route_edge_tape(&output, 3, 181081, &graph.edges[0])
                .unwrap()
                .frames
                .len(),
            538
        );
        let detail = load_tactic_route_decision_trace(&output, 3, 181081, 69).unwrap();
        assert_eq!(detail.selected_option_id, "goal.seek.coordinate.17");
        assert_eq!(detail.measurements.len(), 1);
        assert_eq!(detail.applicable_tactics.len(), 1);
        let expected_report = format!("{relative}/tactic-route/report.json");
        assert_eq!(projection.report.as_deref(), Some(expected_report.as_str()));
        fs::remove_dir_all(root.join(relative)).unwrap();
    }

    #[test]
    fn active_registry_only_blocks_live_route_learning() {
        let key = format!("tactic-route-active-test-{}", std::process::id());
        tactic_route_runs().lock().unwrap().insert(
            key.clone(),
            TacticRouteRuntimeEntry {
                status: TacticRouteRuntimeStatus {
                    status: "running",
                    error: None,
                },
                cancellation: Arc::new(AtomicBool::new(false)),
            },
        );
        assert!(tactic_route_campaign_active(&key));
        tactic_route_runs().lock().unwrap().insert(
            key.clone(),
            TacticRouteRuntimeEntry {
                status: TacticRouteRuntimeStatus {
                    status: "completed",
                    error: None,
                },
                cancellation: Arc::new(AtomicBool::new(false)),
            },
        );
        assert!(!tactic_route_campaign_active(&key));
        forget_tactic_route_campaign(&key);
    }

    #[test]
    fn pause_request_signals_the_live_route_runner() {
        let key = format!("tactic-route-pause-test-{}", std::process::id());
        let cancellation = Arc::new(AtomicBool::new(false));
        tactic_route_runs().lock().unwrap().insert(
            key.clone(),
            TacticRouteRuntimeEntry {
                status: TacticRouteRuntimeStatus {
                    status: "running",
                    error: None,
                },
                cancellation: Arc::clone(&cancellation),
            },
        );

        assert_eq!(request_tactic_route_pause(&key).unwrap(), "pausing");
        assert!(cancellation.load(Ordering::Acquire));
        assert_eq!(request_tactic_route_pause(&key).unwrap(), "pausing");
        assert!(tactic_route_campaign_active(&key));
        assert_eq!(
            request_tactic_route_cancel(&key, Path::new("unused")).unwrap(),
            "cancelling"
        );
        assert!(tactic_route_campaign_active(&key));

        forget_tactic_route_campaign(&key);
    }

    #[test]
    fn only_immutable_pause_checkpoints_make_route_evidence_resumable() {
        let root = std::env::temp_dir().join(format!(
            "dusklight-tactic-route-resume-evidence-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("seed-000-42").join("native")).unwrap();
        assert!(!tactic_route_pause_evidence_exists(&root));

        let pause = root
            .join("seed-000-42")
            .join("pause-checkpoints")
            .join("decision-000003");
        fs::create_dir_all(&pause).unwrap();
        fs::write(
            pause.join(format!("tactic-q-proof.{TACTIC_Q_CHECKPOINT_EXTENSION}")),
            b"checkpoint",
        )
        .unwrap();
        assert!(tactic_route_pause_evidence_exists(&root));
        assert_eq!(
            request_tactic_route_cancel("unregistered-pause", &root).unwrap(),
            "cancelled"
        );
        let request_sha256 = "a".repeat(64);
        write_tactic_route_cancel_marker(&root, &request_sha256).unwrap();
        assert!(tactic_route_cancel_marker_valid(&root, &request_sha256));
        assert!(!tactic_route_cancel_marker_valid(&root, &"b".repeat(64)));

        fs::remove_dir_all(root).unwrap();
    }
}

fn repository_path_text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
