//! Read-only projection of sealed optimization campaigns into the workbench graph.

use super::*;
use dusklight_harness_contracts::objective_suite::ArtifactReference;
use dusklight_orchestration::native_residual_campaign::{
    NativeResidualAttempt, NativeResidualCampaignEvaluation, NativeResidualExecutionBinding,
};
use dusklight_orchestration::optimization_request::{OptimizationRequest, ResidualOptimizerConfig};
use dusklight_orchestration::optimization_resume::{
    OptimizationResumeCandidate, OptimizationResumeState,
};
use dusklight_orchestration::residual_campaign::{
    ResidualCampaignCandidate, ResidualCampaignCheckpoint,
};
use dusklight_routes::timeline_materialization::materialize_segment_chain;
use dusklight_search::residual_retention::{ExactTerminalVerdict, ResidualRetentionSnapshot};

const MAX_CAMPAIGN_REQUESTS: usize = 256;
const MAX_PROJECTED_OPTIMIZATION_CANDIDATES: usize = 16;
const MAX_PROJECTED_OPTIMIZATION_SUCCESSES: usize = 8;
const OPTIMIZATION_CANDIDATE_DETAIL_SCHEMA: &str =
    "dusklight.route-workbench.optimization-candidate-detail.v1";

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserOptimizationCandidateDetailRequest {
    pub candidate: String,
    pub request_sha256: String,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct OptimizationCandidateDetailResponse {
    pub schema: &'static str,
    pub candidate: String,
    pub candidate_id: String,
    pub request_sha256: String,
    pub execution_sha256: String,
    pub candidate_envelope_sha256: String,
    pub residual_candidate_sha256: String,
    pub evaluation_sha256: String,
    pub generation: u64,
    pub proposer_seed: u64,
    pub status: String,
    pub first_hit_tick: Option<u64>,
    pub promotion_eligible: bool,
    pub terminal_boundary_fingerprint: String,
    pub candidate_artifact: ArtifactReference,
    pub compiled_tape: ArtifactReference,
    pub evaluation_artifact: ArtifactReference,
    pub attempts: Vec<NativeResidualAttempt>,
}

pub(super) struct OptimizationCandidateProjection {
    pub segment: GraphSegment,
    pub full_tape: InputTape,
}

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
        let mut campaign = campaign_projection(&root, relative, &request, runtime_config);
        if validation_error.is_some() {
            campaign.status = "invalid".into();
            campaign.error = validation_error;
        } else if campaign.status != "completed"
            && let Some(config) = runtime_config
        {
            campaign.blocker = optimization_runtime_blocker(&root, config);
        }
        if campaign.status != "invalid" {
            graph.segments.extend(
                project_request_candidates(&root, &timeline, &request)
                    .into_iter()
                    .map(|projection| projection.segment),
            );
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

pub(super) fn optimization_candidate_projections(
    repository_root: &Path,
    timeline_path: &Path,
) -> Result<Vec<OptimizationCandidateProjection>, WorkbenchError> {
    let root = repository_root.canonicalize().map_err(optimization_error)?;
    let timeline_path = timeline_path.canonicalize().map_err(optimization_error)?;
    if !timeline_path.starts_with(&root) || !timeline_path.is_file() {
        return Err(WorkbenchError::new(
            "optimization timeline is outside the workbench repository",
        ));
    }
    let benchmark_root = timeline_path.with_extension("").join("benchmarks");
    let Ok(metadata) = fs::symlink_metadata(&benchmark_root) else {
        return Ok(Vec::new());
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Ok(Vec::new());
    }
    let mut paths = fs::read_dir(benchmark_root)
        .map_err(optimization_error)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            (name.ends_with(".request.json") && entry.file_type().ok()?.is_file())
                .then_some(entry.path())
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths.truncate(MAX_CAMPAIGN_REQUESTS);
    let mut projections = Vec::new();
    for path in paths {
        let Some(request) = bounded_json::<OptimizationRequest>(&path) else {
            continue;
        };
        if root
            .join(&request.route.timeline.path)
            .canonicalize()
            .ok()
            .as_ref()
            != Some(&timeline_path)
            || request.validate_files(&root).is_err()
        {
            continue;
        }
        projections.extend(project_request_candidates(&root, &timeline_path, &request));
    }
    Ok(projections)
}

pub(super) fn project_request_candidates(
    root: &Path,
    timeline_path: &Path,
    request: &OptimizationRequest,
) -> Vec<OptimizationCandidateProjection> {
    let Some(state) = bounded_json::<OptimizationResumeState>(
        &root.join(&request.resume.state_path),
    )
    .filter(|state| state.request_sha256 == request.content_sha256 && state.validate().is_ok()) else {
        return Vec::new();
    };
    let campaign_root = root
        .join(&request.resume.state_path)
        .parent()
        .unwrap_or(root)
        .to_path_buf();
    let execution_path = campaign_root.join("execution/execution.json");
    let Some(execution) = bounded_json::<NativeResidualExecutionBinding>(&execution_path)
        .filter(|execution| execution.validate_files(root, request).is_ok())
    else {
        return Vec::new();
    };
    let ranked_successes = state
        .latest_optimizer_checkpoint
        .as_ref()
        .and_then(|latest| {
            bound_artifact_json::<ResidualCampaignCheckpoint>(root, &latest.artifact).filter(
                |checkpoint| {
                    checkpoint.content_sha256 == latest.artifact_sha256
                        && checkpoint
                            .validate(request, execution.content_sha256)
                            .is_ok()
                },
            )
        })
        .map(|checkpoint| {
            checkpoint
                .retention
                .successes
                .iter()
                .map(|success| success.candidate_sha256)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut selected = Vec::new();
    let mut selected_ids = BTreeSet::new();
    for candidate_sha256 in ranked_successes
        .iter()
        .take(MAX_PROJECTED_OPTIMIZATION_SUCCESSES)
    {
        if let Some(row) = state.candidates.iter().find(|row| {
            row.result.is_some()
                && bound_artifact_json::<ResidualCampaignCandidate>(root, &row.candidate)
                    .is_some_and(|candidate| {
                        candidate.validate().is_ok()
                            && candidate.candidate.content_sha256 == *candidate_sha256
                    })
        }) {
            selected_ids.insert(row.id.clone());
            selected.push(row);
        }
    }
    append_recent_completed_candidates(&state, &mut selected_ids, &mut selected);

    let Ok(timeline) = load_authoritative_timeline(timeline_path) else {
        return Vec::new();
    };
    let Some(authored) = timeline.segments.get(&request.route.segment) else {
        return Vec::new();
    };
    let Some(parent_id) = authored.parent.as_deref() else {
        return Vec::new();
    };
    let artifact_root = timeline_path.parent().unwrap_or(root);
    let Ok(parent) = materialize_segment_chain(&timeline, artifact_root, parent_id) else {
        return Vec::new();
    };
    if parent.tape.frames.len() as u64 != request.route.source_boundary_index {
        return Vec::new();
    }
    let display_base = authored
        .name
        .clone()
        .unwrap_or_else(|| request.route.segment.replace('_', " "));
    let campaign_relative = campaign_root
        .strip_prefix(root)
        .map(path_text)
        .unwrap_or_else(|_| campaign_root.display().to_string());
    let mut projections = selected
        .into_iter()
        .filter_map(|row| {
            let candidate = bound_artifact_json::<ResidualCampaignCandidate>(root, &row.candidate)?;
            if candidate.validate().is_err()
                || candidate.id != row.id
                || candidate.generation != row.generation
                || candidate.proposer_seed != row.proposer_seed
                || candidate.compilation.realized_tape_sha256 != row.compiled_tape_sha256
            {
                return None;
            }
            let tape_bytes = bound_artifact_bytes(root, &row.compiled_tape)?;
            let tape = InputTape::decode(&tape_bytes).ok()?.tape;
            if tape.frames.len() as u64 != candidate.compilation.frame_count {
                return None;
            }
            let result = row.result.as_ref()?;
            let evaluation = bound_artifact_json::<NativeResidualCampaignEvaluation>(root, result)?;
            if evaluation
                .validate(request, &execution, &candidate)
                .is_err()
                || row.result_sha256 != Some(result.sha256)
            {
                return None;
            }
            let full_tape = concatenate(vec![
                ChainSegment::all(parent.tape.clone()),
                ChainSegment::all(tape.clone()),
            ])
            .ok()?
            .tape;
            let materialization_sha256 = tape_digest(&full_tape).ok()?;
            let (status, first_hit_tick) = match evaluation.evidence.verdict {
                ExactTerminalVerdict::Reached { first_hit_tick } => {
                    ("success", Some(first_hit_tick))
                }
                ExactTerminalVerdict::Miss => ("miss", None),
            };
            let proof = GraphGoalProof {
                goal: request.terminal_predicate.goal.clone(),
                predicate: request.terminal_predicate.goal.clone(),
                program_sha256: request.terminal_predicate.program_sha256.to_string(),
                definition_sha256: request.terminal_predicate.definition_sha256.to_string(),
                status: if first_hit_tick.is_some() {
                    "verified"
                } else {
                    "failed"
                }
                .into(),
                first_hit_tick,
            };
            let generation = u32::try_from(row.generation).ok()?;
            let short = candidate.id.chars().rev().take(6).collect::<String>();
            let short = short.chars().rev().collect::<String>();
            let name = first_hit_tick.map_or_else(
                || format!("{display_base} · miss · g{generation} · {short}"),
                |tick| format!("{display_base} · {tick}f · g{generation} · {short}"),
            );
            Some(OptimizationCandidateProjection {
                segment: GraphSegment {
                    id: format!(
                        "optimization-{}-{}",
                        &request.content_sha256.to_string()[..12],
                        candidate.id
                    ),
                    name: Some(name),
                    parent: Some(parent_id.into()),
                    profile: authored.profile.as_str().into(),
                    artifact: GraphArtifact {
                        kind: "tape".into(),
                        value: row.compiled_tape.path.clone(),
                    },
                    start_fingerprint: request.route.source_boundary_fingerprint.clone(),
                    boundary_fingerprint: evaluation.terminal_boundary_fingerprint.clone(),
                    materialization_sha256,
                    goal_proofs: vec![proof],
                    predicate_proof: if first_hit_tick.is_some() {
                        "verified"
                    } else {
                        "failed"
                    }
                    .into(),
                    first_hit_tick,
                    frame_count: Some(tape.frames.len() as u64),
                    start_tick: 0,
                    end_tick: (tape.frames.len() as u64).checked_sub(1),
                    ticks: first_hit_tick,
                    playable: true,
                    recordable: false,
                    record_anchors: Vec::new(),
                    option_visualization: Vec::new(),
                    option_diagnostic_error: None,
                    generated: Some(GraphGeneratedSegment {
                        kind: "optimization_candidate".into(),
                        status: status.into(),
                        uncommitted: true,
                        run: campaign_relative.clone(),
                        generation,
                        candidate_id: candidate.id.clone(),
                        candidate: row.candidate.path.clone(),
                        tape: row.compiled_tape.path.clone(),
                        objective_sha256: request.content_sha256.to_string(),
                        source_predicate: request.route.native_source_boundary_fingerprint.clone(),
                        goal_predicate: request.terminal_predicate.goal.clone(),
                        proof_attempts: u32::try_from(evaluation.attempts.len()).ok()?,
                        promotion: optimization_candidate_promotion(
                            &request.content_sha256.to_string(),
                            &candidate.id,
                            first_hit_tick
                                .is_some_and(|tick| tick < request.budgets.promotion_before_tick),
                        ),
                    }),
                    thumbnail: None,
                    error: None,
                },
                full_tape,
            })
        })
        .collect::<Vec<_>>();
    projections.sort_by(|left, right| {
        left.segment
            .first_hit_tick
            .is_none()
            .cmp(&right.segment.first_hit_tick.is_none())
            .then_with(|| {
                left.segment
                    .first_hit_tick
                    .cmp(&right.segment.first_hit_tick)
            })
            .then_with(|| left.segment.id.cmp(&right.segment.id))
    });
    projections
}

fn append_recent_completed_candidates<'a>(
    state: &'a OptimizationResumeState,
    selected_ids: &mut BTreeSet<String>,
    selected: &mut Vec<&'a OptimizationResumeCandidate>,
) {
    for row in state
        .candidates
        .iter()
        .rev()
        .filter(|row| row.result.is_some())
    {
        if selected.len() >= MAX_PROJECTED_OPTIMIZATION_CANDIDATES {
            break;
        }
        if selected_ids.insert(row.id.clone()) {
            selected.push(row);
        }
    }
}

pub(super) fn optimization_candidate_detail(
    repository_root: &Path,
    timeline_path: &Path,
    browser: &BrowserOptimizationCandidateDetailRequest,
) -> Result<OptimizationCandidateDetailResponse, WorkbenchError> {
    let root = repository_root.canonicalize().map_err(optimization_error)?;
    let timeline = timeline_path.canonicalize().map_err(optimization_error)?;
    let request = load_optimization_request(&root, &timeline, &browser.request_sha256)
        .map_err(|_| WorkbenchError::new("unknown, deleted, or expired optimization candidate"))?;
    optimization_candidate_detail_for_request(&root, &timeline, &request, browser)
}

pub(super) fn optimization_candidate_detail_for_request(
    root: &Path,
    timeline: &Path,
    request: &OptimizationRequest,
    browser: &BrowserOptimizationCandidateDetailRequest,
) -> Result<OptimizationCandidateDetailResponse, WorkbenchError> {
    if request.content_sha256.to_string() != browser.request_sha256 {
        return Err(WorkbenchError::new(
            "optimization candidate changed; refresh before loading detail",
        ));
    }
    let projection = project_request_candidates(root, timeline, request)
        .into_iter()
        .find(|projection| projection.segment.id == browser.candidate)
        .ok_or_else(|| {
            WorkbenchError::new("unknown, deleted, or expired optimization candidate")
        })?;
    let generated = projection
        .segment
        .generated
        .as_ref()
        .filter(|generated| generated.kind == "optimization_candidate")
        .ok_or_else(|| WorkbenchError::new("selected segment is not an optimization candidate"))?;
    if generated.objective_sha256 != browser.request_sha256 {
        return Err(WorkbenchError::new(
            "optimization candidate changed; refresh before loading detail",
        ));
    }
    let state: OptimizationResumeState = bounded_json(&root.join(&request.resume.state_path))
        .filter(|state: &OptimizationResumeState| {
            state.request_sha256 == request.content_sha256 && state.validate().is_ok()
        })
        .ok_or_else(|| WorkbenchError::new("optimization resume state is invalid or detached"))?;
    let row = state
        .candidates
        .iter()
        .find(|row| row.id == generated.candidate_id && row.result.is_some())
        .ok_or_else(|| WorkbenchError::new("optimization candidate is absent from resume state"))?;
    let candidate: ResidualCampaignCandidate = bound_artifact_json(root, &row.candidate)
        .filter(|candidate: &ResidualCampaignCandidate| candidate.validate().is_ok())
        .ok_or_else(|| WorkbenchError::new("optimization candidate artifact is invalid"))?;
    let campaign_root = root
        .join(&request.resume.state_path)
        .parent()
        .ok_or_else(|| WorkbenchError::new("optimization campaign has no artifact root"))?
        .to_path_buf();
    let execution: NativeResidualExecutionBinding =
        bounded_json(&campaign_root.join("execution/execution.json"))
            .filter(|execution: &NativeResidualExecutionBinding| {
                execution.validate_files(root, request).is_ok()
            })
            .ok_or_else(|| WorkbenchError::new("optimization execution binding is invalid"))?;
    let evaluation_artifact = row
        .result
        .clone()
        .ok_or_else(|| WorkbenchError::new("optimization candidate has no evaluation"))?;
    let evaluation: NativeResidualCampaignEvaluation =
        bound_artifact_json(root, &evaluation_artifact)
            .filter(|evaluation: &NativeResidualCampaignEvaluation| {
                evaluation.validate(request, &execution, &candidate).is_ok()
            })
            .ok_or_else(|| WorkbenchError::new("optimization evaluation is invalid"))?;
    if candidate.id != generated.candidate_id
        || candidate.compilation.realized_tape_sha256 != row.compiled_tape_sha256
        || evaluation.terminal_boundary_fingerprint != projection.segment.boundary_fingerprint
    {
        return Err(WorkbenchError::new(
            "optimization candidate detail differs from its projected summary",
        ));
    }
    let first_hit_tick = projection.segment.first_hit_tick;
    Ok(OptimizationCandidateDetailResponse {
        schema: OPTIMIZATION_CANDIDATE_DETAIL_SCHEMA,
        candidate: projection.segment.id,
        candidate_id: candidate.id.clone(),
        request_sha256: request.content_sha256.to_string(),
        execution_sha256: execution.content_sha256.to_string(),
        candidate_envelope_sha256: candidate.content_sha256.to_string(),
        residual_candidate_sha256: candidate.candidate.content_sha256.to_string(),
        evaluation_sha256: evaluation.content_sha256.to_string(),
        generation: candidate.generation,
        proposer_seed: candidate.proposer_seed,
        status: generated.status.clone(),
        first_hit_tick,
        promotion_eligible: first_hit_tick
            .is_some_and(|tick| tick < request.budgets.promotion_before_tick),
        terminal_boundary_fingerprint: evaluation.terminal_boundary_fingerprint,
        candidate_artifact: row.candidate.clone(),
        compiled_tape: row.compiled_tape.clone(),
        evaluation_artifact,
        attempts: evaluation.attempts,
    })
}

fn campaign_projection(
    root: &Path,
    request_path: &Path,
    request: &OptimizationRequest,
    runtime_config: Option<&WorkbenchConfig>,
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
        replay_generation: None,
        replay_entries: 0,
        replay_transitions: 0,
        replay_successes: 0,
        replay_failures: 0,
        replay_corpus: None,
        uncheckpointed_completions: 0,
        artifacts_present: optimization_campaign_artifacts_present(root, request),
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
        learning: goal_learning_projection(root, request, runtime_config),
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
                    if let Some(replay) = checkpoint.replay_corpus {
                        projection.replay_generation = Some(replay.generation);
                        projection.replay_entries = replay.entries;
                        projection.replay_transitions = replay.transitions;
                        projection.replay_successes = replay.successes;
                        projection.replay_failures = replay.failures;
                        projection.replay_corpus = Some(replay.artifact.path);
                    }
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

pub(super) fn optimization_runtime_blocker(
    root: &Path,
    config: &WorkbenchConfig,
) -> Option<String> {
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

pub(super) fn bound_artifact_json<T: for<'de> Deserialize<'de>>(
    root: &Path,
    reference: &ArtifactReference,
) -> Option<T> {
    serde_json::from_slice(&bound_artifact_bytes(root, reference)?).ok()
}

pub(super) fn bound_artifact_bytes(root: &Path, reference: &ArtifactReference) -> Option<Vec<u8>> {
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
    Some(bytes)
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

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_automation_contracts::artifact::Digest;

    #[test]
    fn ten_thousand_completed_rows_select_only_the_recent_bounded_window() {
        let artifact = ArtifactReference {
            path: "build/campaigns/scale/artifact.json".into(),
            sha256: Digest([1; 32]),
        };
        let candidates = (0..10_000)
            .map(|index| OptimizationResumeCandidate {
                id: format!("candidate-{index:05}"),
                candidate: artifact.clone(),
                candidate_sha256: artifact.sha256,
                compiled_tape: artifact.clone(),
                compiled_tape_sha256: artifact.sha256,
                generation: index / 100,
                proposer_seed: index,
                result: Some(artifact.clone()),
                result_sha256: Some(artifact.sha256),
                simulated_ticks: Some(1),
            })
            .collect();
        let state = OptimizationResumeState {
            schema: "test".into(),
            request_sha256: Digest([2; 32]),
            journal_sha256: Digest([3; 32]),
            valid_journal_bytes: 0,
            record_count: 0,
            last_record_sha256: Digest([4; 32]),
            next_sequence: 1,
            candidates,
            completed_candidates: 10_000,
            charged_simulated_ticks: 10_000,
            pending_candidate_ids: Vec::new(),
            latest_optimizer_checkpoint: None,
            uncheckpointed_completions: 10_000,
            state_sha256: Digest([5; 32]),
        };
        let mut selected_ids = BTreeSet::new();
        let mut selected = Vec::new();

        append_recent_completed_candidates(&state, &mut selected_ids, &mut selected);

        assert_eq!(selected.len(), MAX_PROJECTED_OPTIMIZATION_CANDIDATES);
        assert_eq!(selected[0].id, "candidate-09999");
        assert_eq!(selected[15].id, "candidate-09984");
    }
}
