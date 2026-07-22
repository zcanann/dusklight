//! Read-only projection of sealed optimization campaigns into the workbench graph.

use super::*;
use dusklight_orchestration::native_residual_campaign::NativeResidualExecutionBinding;
use dusklight_orchestration::optimization_request::{OptimizationRequest, ResidualOptimizerConfig};
use dusklight_orchestration::optimization_resume::OptimizationResumeState;

const MAX_CAMPAIGN_REQUESTS: usize = 256;

pub(super) fn append_optimization_campaigns(
    graph: &mut WorkbenchGraph,
    repository_root: &Path,
    timeline_path: &Path,
) -> Result<(), WorkbenchError> {
    let root = repository_root.canonicalize().map_err(optimization_error)?;
    let timeline = timeline_path.canonicalize().map_err(optimization_error)?;
    if !timeline.starts_with(&root) || !timeline.is_file() {
        return Err(WorkbenchError::new(
            "optimization timeline is outside the workbench repository",
        ));
    }
    let benchmark_root = timeline.with_extension("").join("benchmarks");
    let Ok(metadata) = fs::symlink_metadata(&benchmark_root) else {
        return Ok(());
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(WorkbenchError::new(
            "optimization benchmark root is not a physical directory",
        ));
    }
    let mut request_paths = fs::read_dir(&benchmark_root)
        .map_err(optimization_error)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            (name.ends_with(".request.json") && entry.file_type().ok()?.is_file())
                .then_some(entry.path())
        })
        .collect::<Vec<_>>();
    request_paths.sort();
    request_paths.truncate(MAX_CAMPAIGN_REQUESTS);

    for path in request_paths {
        let relative = path.strip_prefix(&root).map_err(optimization_error)?;
        let request: OptimizationRequest = match bounded_json(&path) {
            Some(request) => request,
            None => continue,
        };
        if root
            .join(&request.route.timeline.path)
            .canonicalize()
            .ok()
            .as_ref()
            != Some(&timeline)
        {
            continue;
        }
        let validation_error = request
            .validate_files(&root)
            .err()
            .map(|error| error.to_string());
        let mut campaign = campaign_projection(&root, relative, &request);
        if validation_error.is_some() {
            campaign.status = "invalid".into();
            campaign.error = validation_error;
        }
        graph.campaigns.push(campaign);
    }
    graph.campaigns.sort_by(|left, right| {
        left.segment
            .cmp(&right.segment)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(())
}

fn campaign_projection(
    root: &Path,
    request_path: &Path,
    request: &OptimizationRequest,
) -> GraphOptimizationCampaign {
    let mut projection = GraphOptimizationCampaign {
        id: request.id.clone(),
        request: path_text(request_path),
        request_sha256: request.content_sha256.to_string(),
        segment: request.route.segment.clone(),
        goal: request.terminal_predicate.goal.clone(),
        optimizer: match request.proposal.optimizer {
            ResidualOptimizerConfig::Random { .. } => "random",
            ResidualOptimizerConfig::Cem { .. } => "cem",
        }
        .into(),
        status: "ready".into(),
        exploration_horizon_ticks: request.budgets.exploration_horizon_ticks,
        promotion_before_tick: request.budgets.promotion_before_tick,
        candidate_budget: request.budgets.candidate_budget,
        simulated_tick_budget: request.budgets.simulated_tick_budget,
        workers: request.execution.workers,
        sealed_candidates: 0,
        completed_candidates: 0,
        pending_candidates: 0,
        charged_simulated_ticks: 0,
        generation: 0,
        execution: None,
        error: None,
    };
    let state_path = root.join(&request.resume.state_path);
    if state_path.exists() {
        match bounded_json::<OptimizationResumeState>(&state_path) {
            Some(state)
                if state.request_sha256 == request.content_sha256 && state.validate().is_ok() =>
            {
                projection.sealed_candidates = state.candidates.len() as u64;
                projection.completed_candidates = state.completed_candidates;
                projection.pending_candidates = state.pending_candidate_ids.len() as u64;
                projection.charged_simulated_ticks = state.charged_simulated_ticks;
                projection.generation = state
                    .latest_optimizer_checkpoint
                    .as_ref()
                    .map_or(0, |checkpoint| checkpoint.generation);
                projection.status = if campaign_complete(request, &state) {
                    "completed"
                } else {
                    "resumable"
                }
                .into();
            }
            _ => {
                projection.status = "invalid".into();
                projection.error = Some("optimization resume state is invalid or detached".into());
            }
        }
    }
    let execution_path = state_path
        .parent()
        .unwrap_or(root)
        .join("execution/execution.json");
    if let Some(execution) = bounded_json::<NativeResidualExecutionBinding>(&execution_path)
        && execution.validate_files(root, request).is_ok()
        && let Ok(relative) = execution_path.strip_prefix(root)
    {
        projection.execution = Some(path_text(relative));
    }
    if let Some(runtime) = optimization_runtime_status(&request.content_sha256.to_string()) {
        projection.status = runtime.status.into();
        if runtime.error.is_some() {
            projection.error = runtime.error;
        }
    }
    projection
}

fn campaign_complete(request: &OptimizationRequest, state: &OptimizationResumeState) -> bool {
    if !state.pending_candidate_ids.is_empty() {
        return false;
    }
    match request.proposal.optimizer {
        ResidualOptimizerConfig::Random { samples } => state.completed_candidates >= samples,
        ResidualOptimizerConfig::Cem {
            population,
            generations,
            ..
        } => {
            state.completed_candidates >= u64::from(population) * u64::from(generations)
                && state
                    .latest_optimizer_checkpoint
                    .as_ref()
                    .is_some_and(|checkpoint| checkpoint.generation >= u64::from(generations))
        }
    }
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
}

fn optimization_error(error: impl fmt::Display) -> WorkbenchError {
    WorkbenchError::new(error.to_string())
}
