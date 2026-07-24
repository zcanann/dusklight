//! Workbench projection and launch boundary for tactic-level route learning.

use super::*;
use dusklight_orchestration::native_tactic_route_runner::{
    NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V4, NativeTacticRouteRunConfig, run_native_tactic_route,
};
use dusklight_orchestration::optimization_request::OptimizationRequest;
use serde_json::Value;

const TACTIC_ROUTE_START_SCHEMA: &str = "dusklight.route-workbench.tactic-route-start.v1";
const TACTIC_ROUTE_DECISIONS_PER_SEED: u64 = 256;
const TACTIC_ROUTE_BRANCH_EVERY_DECISIONS: u64 = 8;
const TACTIC_ROUTE_REFIT_EVERY_DECISIONS: u64 = 32;
const TACTIC_ROUTE_EPSILON_PER_MILLION: u32 = 600_000;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserTacticRouteStartRequest {
    pub campaign: String,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct TacticRouteStartResponse {
    pub schema: &'static str,
    pub campaign: String,
    pub optimization_request_sha256: String,
    pub output: String,
    pub status: &'static str,
}

#[derive(Clone, Debug)]
struct TacticRouteRuntimeStatus {
    status: &'static str,
    error: Option<String>,
}

fn tactic_route_runs() -> &'static Mutex<BTreeMap<String, TacticRouteRuntimeStatus>> {
    static RUNS: OnceLock<Mutex<BTreeMap<String, TacticRouteRuntimeStatus>>> = OnceLock::new();
    RUNS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub(super) fn tactic_route_campaign_active(optimization_request_sha256: &str) -> bool {
    tactic_route_runs().lock().ok().is_some_and(|runs| {
        runs.get(optimization_request_sha256)
            .is_some_and(|run| matches!(run.status, "preparing" | "running"))
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
                if report.get("schema").and_then(Value::as_str)
                    == Some(NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V4)
                    && report
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
        projection.status = "interrupted".into();
        projection.error = Some(
            "the tactic-route output is incomplete; resumable runner state is not implemented yet"
                .into(),
        );
    }

    if let Ok(runs) = tactic_route_runs().lock()
        && let Some(runtime) = runs.get(&optimization.content_sha256.to_string())
    {
        projection.status = runtime.status.into();
        if runtime.error.is_some() {
            projection.error = runtime.error.clone();
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
    let world_context = config.world_context.as_ref().ok_or_else(|| {
        WorkbenchError::new(
            "route learning requires a sealed world context; restart the workbench with --world-context WORLD.json",
        )
    })?;
    let root = config
        .repository_root
        .canonicalize()
        .map_err(tactic_route_error)?;
    let optimization = selected_optimization(config, &root, &browser.campaign)?;
    let request_sha256 = optimization.content_sha256.to_string();
    if optimization_runtime_status(&request_sha256)
        .is_some_and(|status| matches!(status.status, "preparing" | "running" | "cancelling"))
    {
        return Err(WorkbenchError::new(
            "residual optimization must stop before route learning starts",
        ));
    }
    if optimization_request_promotion_active(&request_sha256) {
        return Err(WorkbenchError::new(
            "candidate promotion must finish before route learning starts",
        ));
    }
    let output = tactic_route_output_root(&root, &optimization)?;
    if output.exists() {
        return Err(WorkbenchError::new(
            "route-learning evidence already exists for this start and goal",
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
        if runs
            .get(&request_sha256)
            .is_some_and(|run| matches!(run.status, "preparing" | "running"))
        {
            return Err(WorkbenchError::new("route learning is already running"));
        }
        runs.insert(
            request_sha256.clone(),
            TacticRouteRuntimeStatus {
                status: "running",
                error: None,
            },
        );
    }
    let seeds = optimization.execution.deterministic_seeds.clone();
    let thread_request_sha256 = request_sha256.clone();
    let spawn = thread::Builder::new()
        .name(format!("tactic-route-{}", optimization.id))
        .spawn(move || {
            let result = run_native_tactic_route(&NativeTacticRouteRunConfig {
                repository_root: &root,
                optimization: &optimization,
                execution: &execution,
                output_root: &output,
                exploration_seeds: &seeds,
                decisions_per_seed: TACTIC_ROUTE_DECISIONS_PER_SEED,
                branch_every_decisions: TACTIC_ROUTE_BRANCH_EVERY_DECISIONS,
                refit_every_decisions: TACTIC_ROUTE_REFIT_EVERY_DECISIONS,
                epsilon_per_million: TACTIC_ROUTE_EPSILON_PER_MILLION,
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
                Err(error) => TacticRouteRuntimeStatus {
                    status: "failed",
                    error: Some(error.to_string()),
                },
            };
            if let Ok(mut runs) = tactic_route_runs().lock() {
                runs.insert(thread_request_sha256, status);
            }
        });
    if let Err(error) = spawn {
        let message = format!("cannot start route-learning thread: {error}");
        if let Ok(mut runs) = tactic_route_runs().lock() {
            runs.insert(
                request_sha256.clone(),
                TacticRouteRuntimeStatus {
                    status: "failed",
                    error: Some(message.clone()),
                },
            );
        }
        return Err(WorkbenchError::new(message));
    }
    Ok(TacticRouteStartResponse {
        schema: TACTIC_ROUTE_START_SCHEMA,
        campaign: browser.campaign.clone(),
        optimization_request_sha256: request_sha256,
        output: output_text,
        status: "running",
    })
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

fn project_latest_decision(output: &Path, seeds: &[u64]) -> Option<GraphTacticDecisionTrace> {
    for (index, seed) in seeds.iter().enumerate().rev() {
        let directory = output
            .join(format!("seed-{index:03}-{seed}"))
            .join("decision-trace");
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
        if let Some(trace) = bounded_json(&latest) {
            return Some(trace);
        }
    }
    None
}

fn project_latest_graph(output: &Path, seeds: &[u64]) -> Option<GraphTacticKnowledgeGraph> {
    for (index, seed) in seeds.iter().enumerate().rev() {
        let seed_root = output.join(format!("seed-{index:03}-{seed}"));
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
        if let Ok(graph) = serde_json::from_value(value) {
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
        fs::write(
            output.join("report.json"),
            serde_json::to_vec(&serde_json::json!({
                "schema": NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V4,
                "optimization_request_sha256": optimization.content_sha256,
                "successful_seeds": 1,
                "total_decisions": 70,
                "total_native_ticks": 2266,
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
        let expected_report = format!("{relative}/tactic-route/report.json");
        assert_eq!(projection.report.as_deref(), Some(expected_report.as_str()));
        fs::remove_dir_all(root.join(relative)).unwrap();
    }

    #[test]
    fn active_registry_only_blocks_live_route_learning() {
        let key = format!("tactic-route-active-test-{}", std::process::id());
        tactic_route_runs().lock().unwrap().insert(
            key.clone(),
            TacticRouteRuntimeStatus {
                status: "running",
                error: None,
            },
        );
        assert!(tactic_route_campaign_active(&key));
        tactic_route_runs().lock().unwrap().insert(
            key.clone(),
            TacticRouteRuntimeStatus {
                status: "completed",
                error: None,
            },
        );
        assert!(!tactic_route_campaign_active(&key));
        forget_tactic_route_campaign(&key);
    }
}

fn repository_path_text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
