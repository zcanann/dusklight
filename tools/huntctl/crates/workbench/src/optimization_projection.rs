//! Read-only projection of sealed optimization campaigns into the workbench graph.

use super::*;
use dusklight_harness_contracts::objective_suite::ArtifactReference;
use dusklight_orchestration::native_residual_campaign::NativeResidualExecutionBinding;
use dusklight_orchestration::optimization_request::{OptimizationRequest, ResidualOptimizerConfig};
use dusklight_orchestration::optimization_resume::OptimizationResumeState;
use dusklight_orchestration::residual_campaign::ResidualCampaignCheckpoint;
use dusklight_search::residual_retention::ResidualRetentionSnapshot;

const MAX_CAMPAIGN_REQUESTS: usize = 256;

pub(super) fn append_optimization_campaigns(
    graph: &mut WorkbenchGraph,
    repository_root: &Path,
    timeline_path: &Path,
    runtime_config: Option<&WorkbenchConfig>,
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
        } else if campaign.status != "completed"
            && let Some(config) = runtime_config
        {
            campaign.blocker = optimization_runtime_blocker(&root, config);
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
        retained_successes: 0,
        retained_failures: 0,
        best_first_hit_tick: None,
        uncheckpointed_completions: 0,
        proposal_sources: vec![
            match request.proposal.optimizer {
                ResidualOptimizerConfig::Random { .. } => "random",
                ResidualOptimizerConfig::Cem { .. } => "cem",
            }
            .into(),
        ],
        execution: None,
        blocker: None,
        error: None,
    };
    let state_path = root.join(&request.resume.state_path);
    let mut resume_state = None;
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
                projection.uncheckpointed_completions = state.uncheckpointed_completions;
                projection.status = if campaign_complete(request, &state) {
                    "completed"
                } else {
                    "resumable"
                }
                .into();
                resume_state = Some(state);
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
    let execution = bounded_json::<NativeResidualExecutionBinding>(&execution_path)
        .filter(|execution| execution.validate_files(root, request).is_ok());
    if let Some(execution) = &execution {
        if let Ok(relative) = execution_path.strip_prefix(root) {
            projection.execution = Some(path_text(relative));
        }
        if let Some(state) = &resume_state
            && let Some(latest) = &state.latest_optimizer_checkpoint
        {
            match bound_artifact_json::<ResidualCampaignCheckpoint>(root, &latest.artifact).filter(
                |checkpoint| {
                    checkpoint.content_sha256 == latest.artifact_sha256
                        && checkpoint.completed_candidates == latest.completed_candidates
                        && checkpoint
                            .validate(request, execution.content_sha256)
                            .is_ok()
                },
            ) {
                Some(checkpoint) => {
                    projection.generation = checkpoint.generation;
                    apply_retention_projection(&mut projection, &checkpoint.retention);
                }
                None => {
                    projection.status = "invalid".into();
                    projection.error =
                        Some("optimization checkpoint is invalid or detached".into());
                }
            }
        }
    } else if resume_state.is_some() {
        projection.blocker = Some(
            "resume state has no valid native execution binding; restore its immutable execution artifacts"
                .into(),
        );
    }
    if let Some(runtime) = optimization_runtime_status(&request.content_sha256.to_string()) {
        projection.status = runtime.status.into();
        if runtime.error.is_some() {
            projection.error = runtime.error;
        }
    }
    projection
}

pub(super) fn apply_retention_projection(
    projection: &mut GraphOptimizationCampaign,
    retention: &ResidualRetentionSnapshot,
) {
    projection.retained_successes = retention.successes.len() as u64;
    projection.retained_failures = retention.failures.len() as u64;
    projection.best_first_hit_tick = retention
        .successes
        .first()
        .map(|success| success.first_hit_tick);
}

fn optimization_runtime_blocker(root: &Path, config: &WorkbenchConfig) -> Option<String> {
    let Some(world_context) = config.world_context.as_ref() else {
        return Some("restart the workbench with --world-context WORLD.json".into());
    };
    for (label, path) in [
        ("game executable", config.game.as_path()),
        ("game data", config.dvd.as_path()),
        ("world context", world_context.as_path()),
    ] {
        let resolved = path.canonicalize().ok();
        if resolved
            .as_ref()
            .is_none_or(|path| !path.starts_with(root) || !path.is_file())
        {
            return Some(format!(
                "{label} is absent or outside the repository: {}",
                path.display()
            ));
        }
    }
    None
}

fn bound_artifact_json<T: for<'de> Deserialize<'de>>(
    root: &Path,
    reference: &ArtifactReference,
) -> Option<T> {
    let relative = Path::new(&reference.path);
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return None;
    }
    let path = root.join(relative);
    let metadata = fs::symlink_metadata(&path).ok()?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_SEARCH_ARTIFACT_BYTES
    {
        return None;
    }
    let canonical = path.canonicalize().ok()?;
    if !canonical.starts_with(root) {
        return None;
    }
    let bytes = fs::read(canonical).ok()?;
    let digest = crate::artifact::Digest(<Sha256 as sha2::Digest>::digest(&bytes).into());
    if digest != reference.sha256 {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
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
