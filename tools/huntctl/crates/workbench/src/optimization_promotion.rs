//! Exact cold-replay verification and Git-owned installation of campaign winners.

use super::*;
use dusklight_automation_contracts::artifact::Digest as ArtifactDigest;
use dusklight_automation_contracts::native_fidelity::FIXED_AUTOMATION_CVARS;
use dusklight_harness_contracts::evaluation::BoundaryFingerprint;
use dusklight_harness_contracts::objective_suite::ArtifactReference;
use dusklight_orchestration::native_residual_campaign::{
    NativeResidualCampaignEvaluation, NativeResidualExecutionBinding,
};
use dusklight_orchestration::optimization_request::OptimizationRequest;
use dusklight_orchestration::optimization_resume::OptimizationResumeState;
use dusklight_orchestration::residual_campaign::ResidualCampaignCandidate;
use dusklight_search::residual_retention::ExactTerminalVerdict;
use std::fs::OpenOptions;
use std::process::Stdio;
use std::time::Instant;

const OPTIMIZATION_PROMOTION_SCHEMA: &str = "dusklight-optimization-promotion-proof/v1";
const OPTIMIZATION_PROMOTION_RESPONSE_SCHEMA: &str =
    "dusklight.route-workbench.optimization-promotion.v1";
const OPTIMIZATION_PROMOTION_REPETITIONS: u32 = 5;
const OPTIMIZATION_PROMOTION_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserOptimizationPromoteRequest {
    pub candidate: String,
    pub segment_id: String,
    pub label: String,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct OptimizationPromotionResponse {
    pub schema: &'static str,
    pub candidate: String,
    pub segment: String,
    pub status: &'static str,
    pub repetitions: u32,
}

#[derive(Clone, Debug)]
struct OptimizationPromotionStatus {
    status: &'static str,
    segment: Option<String>,
    error: Option<String>,
}

fn optimization_promotions() -> &'static Mutex<BTreeMap<String, OptimizationPromotionStatus>> {
    static PROMOTIONS: OnceLock<Mutex<BTreeMap<String, OptimizationPromotionStatus>>> =
        OnceLock::new();
    PROMOTIONS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn promotion_key(request_sha256: &str, candidate_id: &str) -> String {
    format!("{request_sha256}:{candidate_id}")
}

pub(super) fn optimization_candidate_promotion(
    request_sha256: &str,
    candidate_id: &str,
    eligible: bool,
) -> Option<GraphCandidatePromotion> {
    let status = optimization_promotions()
        .lock()
        .ok()
        .and_then(|promotions| {
            promotions
                .get(&promotion_key(request_sha256, candidate_id))
                .cloned()
        });
    Some(match status {
        Some(status) => GraphCandidatePromotion {
            eligible,
            status: status.status.into(),
            segment: status.segment,
            error: status.error,
        },
        None => GraphCandidatePromotion {
            eligible,
            status: if eligible { "ready" } else { "ineligible" }.into(),
            segment: None,
            error: None,
        },
    })
}

fn set_promotion_status(key: &str, status: OptimizationPromotionStatus) {
    if let Ok(mut promotions) = optimization_promotions().lock() {
        promotions.insert(key.into(), status);
    }
}

pub(super) fn optimization_request_promotion_active(request_sha256: &str) -> bool {
    let prefix = format!("{request_sha256}:");
    optimization_promotions().lock().is_ok_and(|promotions| {
        promotions.iter().any(|(key, status)| {
            key.starts_with(&prefix) && matches!(status.status, "verifying" | "installing")
        })
    })
}

pub(super) fn forget_optimization_promotions(request_sha256: &str) {
    let prefix = format!("{request_sha256}:");
    if let Ok(mut promotions) = optimization_promotions().lock() {
        promotions.retain(|key, _| !key.starts_with(&prefix));
    }
}

#[derive(Clone, Debug)]
struct PreparedOptimizationPromotion {
    root: PathBuf,
    timeline_path: PathBuf,
    timeline_source_sha256: String,
    request: OptimizationRequest,
    execution: NativeResidualExecutionBinding,
    candidate: ResidualCampaignCandidate,
    candidate_artifact_sha256: ArtifactDigest,
    evaluation: NativeResidualCampaignEvaluation,
    evaluation_artifact_sha256: ArtifactDigest,
    graph_candidate_id: String,
    promoted_segment_id: String,
    promoted_label: String,
    promoted_goal_id: String,
    promoted_lineage_id: String,
    parent_segment: String,
    profile: String,
    tape_path: PathBuf,
    tape_relative: String,
    proof_path: PathBuf,
    proof_relative: String,
    predicate_source_relative: String,
    lineage_dsl: String,
    local_tape: InputTape,
    full_tape: InputTape,
    first_hit_tick: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct OptimizationColdReplayAttempt {
    repetition: u32,
    milestone_result_sha256: ArtifactDigest,
    sim_tick: u64,
    tape_frame: u64,
    boundary_index: u64,
    boundary_fingerprint: BoundaryFingerprint,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct OptimizationPromotionProof {
    schema: String,
    content_sha256: ArtifactDigest,
    optimization_request_sha256: ArtifactDigest,
    execution_binding_sha256: ArtifactDigest,
    candidate_id: String,
    candidate_artifact_sha256: ArtifactDigest,
    candidate_envelope_sha256: ArtifactDigest,
    residual_candidate_sha256: ArtifactDigest,
    evaluation_artifact_sha256: ArtifactDigest,
    evaluation_sha256: ArtifactDigest,
    source_lineage: String,
    promoted_lineage: String,
    promoted_segment: String,
    parent_segment: String,
    source_boundary_index: u64,
    source_boundary_fingerprint: String,
    native_source_boundary_fingerprint: String,
    terminal_boundary_fingerprint: String,
    goal: String,
    terminal_program_sha256: ArtifactDigest,
    terminal_definition_sha256: ArtifactDigest,
    first_hit_tick: u64,
    promotion_before_tick: u64,
    promoted_tape: String,
    promoted_tape_sha256: ArtifactDigest,
    promoted_tape_frames: u64,
    full_tape_sha256: ArtifactDigest,
    full_tape_frames: u64,
    executable: ArtifactReference,
    game_data: ArtifactReference,
    milestone_program: ArtifactReference,
    card_fixture_manifest: ArtifactReference,
    repetitions: Vec<OptimizationColdReplayAttempt>,
}

impl OptimizationPromotionProof {
    fn seal(
        prepared: &PreparedOptimizationPromotion,
        repetitions: Vec<OptimizationColdReplayAttempt>,
    ) -> Result<Self, WorkbenchError> {
        let promoted_bytes = prepared.local_tape.encode().map_err(promotion_error)?;
        let full_bytes = prepared.full_tape.encode().map_err(promotion_error)?;
        let mut proof = Self {
            schema: OPTIMIZATION_PROMOTION_SCHEMA.into(),
            content_sha256: ArtifactDigest::ZERO,
            optimization_request_sha256: prepared.request.content_sha256,
            execution_binding_sha256: prepared.execution.content_sha256,
            candidate_id: prepared.candidate.id.clone(),
            candidate_artifact_sha256: prepared.candidate_artifact_sha256,
            candidate_envelope_sha256: prepared.candidate.content_sha256,
            residual_candidate_sha256: prepared.candidate.candidate.content_sha256,
            evaluation_artifact_sha256: prepared.evaluation_artifact_sha256,
            evaluation_sha256: prepared.evaluation.content_sha256,
            source_lineage: prepared.request.route.lineage.clone(),
            promoted_lineage: prepared.promoted_lineage_id.clone(),
            promoted_segment: prepared.promoted_segment_id.clone(),
            parent_segment: prepared.parent_segment.clone(),
            source_boundary_index: prepared.request.route.source_boundary_index,
            source_boundary_fingerprint: prepared.request.route.source_boundary_fingerprint.clone(),
            native_source_boundary_fingerprint: prepared
                .request
                .route
                .native_source_boundary_fingerprint
                .clone(),
            terminal_boundary_fingerprint: prepared
                .evaluation
                .terminal_boundary_fingerprint
                .clone(),
            goal: prepared.request.terminal_predicate.goal.clone(),
            terminal_program_sha256: prepared.request.terminal_predicate.program_sha256,
            terminal_definition_sha256: prepared.request.terminal_predicate.definition_sha256,
            first_hit_tick: prepared.first_hit_tick,
            promotion_before_tick: prepared.request.budgets.promotion_before_tick,
            promoted_tape: prepared.tape_relative.clone(),
            promoted_tape_sha256: ArtifactDigest(Sha256::digest(&promoted_bytes).into()),
            promoted_tape_frames: u64::try_from(prepared.local_tape.frames.len())
                .map_err(promotion_error)?,
            full_tape_sha256: ArtifactDigest(Sha256::digest(&full_bytes).into()),
            full_tape_frames: u64::try_from(prepared.full_tape.frames.len())
                .map_err(promotion_error)?,
            executable: prepared.execution.executable.clone(),
            game_data: prepared.execution.game_data.clone(),
            milestone_program: prepared.execution.milestone_program.clone(),
            card_fixture_manifest: prepared.execution.card_fixture_manifest.clone(),
            repetitions,
        };
        proof.content_sha256 = proof.identity()?;
        proof.validate()?;
        Ok(proof)
    }

    fn validate(&self) -> Result<(), WorkbenchError> {
        let first = self.repetitions.first();
        let exact_repetitions = self.repetitions.len()
            == usize::try_from(OPTIMIZATION_PROMOTION_REPETITIONS).map_err(promotion_error)?
            && first.is_some()
            && self.repetitions.iter().enumerate().all(|(index, attempt)| {
                usize::try_from(attempt.repetition).ok() == Some(index + 1)
                    && first.is_some_and(|first| {
                        attempt.sim_tick == first.sim_tick
                            && attempt.tape_frame == first.tape_frame
                            && attempt.boundary_index == first.boundary_index
                            && attempt.boundary_fingerprint == first.boundary_fingerprint
                    })
                    && attempt.boundary_fingerprint.digest == self.terminal_boundary_fingerprint
                    && exact_boundary_fingerprint(&attempt.boundary_fingerprint)
                    && attempt.milestone_result_sha256 != ArtifactDigest::ZERO
            });
        if self.schema != OPTIMIZATION_PROMOTION_SCHEMA
            || self.content_sha256 == ArtifactDigest::ZERO
            || self.content_sha256 != self.identity()?
            || self.optimization_request_sha256 == ArtifactDigest::ZERO
            || self.execution_binding_sha256 == ArtifactDigest::ZERO
            || self.candidate_artifact_sha256 == ArtifactDigest::ZERO
            || self.candidate_envelope_sha256 == ArtifactDigest::ZERO
            || self.residual_candidate_sha256 == ArtifactDigest::ZERO
            || self.evaluation_artifact_sha256 == ArtifactDigest::ZERO
            || self.evaluation_sha256 == ArtifactDigest::ZERO
            || self.first_hit_tick == 0
            || self.first_hit_tick >= self.promotion_before_tick
            || self.promoted_tape_frames != self.first_hit_tick
            || self.full_tape_frames
                != self
                    .source_boundary_index
                    .checked_add(self.promoted_tape_frames)
                    .ok_or_else(|| promotion_message("promotion tape frame count overflowed"))?
            || !native_fingerprint(&self.source_boundary_fingerprint)
            || !native_fingerprint(&self.native_source_boundary_fingerprint)
            || !native_fingerprint(&self.terminal_boundary_fingerprint)
            || first.is_none_or(|attempt| {
                attempt.tape_frame.checked_add(1) != Some(self.full_tape_frames)
                    || attempt.boundary_index != self.full_tape_frames
            })
            || !exact_repetitions
        {
            return Err(promotion_message(
                "optimization promotion proof is invalid, detached, or nonexact",
            ));
        }
        Ok(())
    }

    fn identity(&self) -> Result<ArtifactDigest, WorkbenchError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = ArtifactDigest::ZERO;
        let bytes = serde_json::to_vec(&canonical).map_err(promotion_error)?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.optimization-promotion-proof/v1\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
        Ok(ArtifactDigest(hasher.finalize().into()))
    }

    fn to_pretty_json(&self) -> Result<Vec<u8>, WorkbenchError> {
        let mut bytes = serde_json::to_vec_pretty(self).map_err(promotion_error)?;
        bytes.push(b'\n');
        Ok(bytes)
    }
}

pub(super) fn start_optimization_promotion(
    config: &WorkbenchConfig,
    request: &BrowserOptimizationPromoteRequest,
) -> Result<OptimizationPromotionResponse, WorkbenchError> {
    let _lifecycle = optimization_lifecycle_edits()
        .lock()
        .map_err(|_| promotion_message("optimization lifecycle lock is unavailable"))?;
    let prepared = prepare_optimization_promotion(config, request)?;
    let key = promotion_key(
        &prepared.request.content_sha256.to_string(),
        &prepared.candidate.id,
    );
    {
        let mut promotions = optimization_promotions()
            .lock()
            .map_err(|_| promotion_message("optimization promotion registry is unavailable"))?;
        if promotions
            .get(&key)
            .is_some_and(|status| matches!(status.status, "verifying" | "installing"))
        {
            return Err(promotion_message(
                "optimization candidate promotion is already running",
            ));
        }
        if promotions
            .get(&key)
            .is_some_and(|status| status.status == "promoted")
        {
            return Err(promotion_message(
                "optimization candidate is already promoted",
            ));
        }
        promotions.insert(
            key.clone(),
            OptimizationPromotionStatus {
                status: "verifying",
                segment: Some(prepared.promoted_segment_id.clone()),
                error: None,
            },
        );
    }
    let segment = prepared.promoted_segment_id.clone();
    let candidate = prepared.graph_candidate_id.clone();
    let thread_config = config.clone();
    let thread_key = key.clone();
    let spawn = thread::Builder::new()
        .name(format!("promote-{}", prepared.candidate.id))
        .spawn(move || {
            let result =
                run_optimization_cold_replays(&thread_config, &prepared).and_then(|repetitions| {
                    set_promotion_status(
                        &thread_key,
                        OptimizationPromotionStatus {
                            status: "installing",
                            segment: Some(prepared.promoted_segment_id.clone()),
                            error: None,
                        },
                    );
                    let proof = OptimizationPromotionProof::seal(&prepared, repetitions)?;
                    install_optimization_promotion(&prepared, &proof)
                });
            let status = match result {
                Ok(()) => OptimizationPromotionStatus {
                    status: "promoted",
                    segment: Some(prepared.promoted_segment_id),
                    error: None,
                },
                Err(error) => OptimizationPromotionStatus {
                    status: "failed",
                    segment: Some(prepared.promoted_segment_id),
                    error: Some(error.to_string()),
                },
            };
            set_promotion_status(&thread_key, status);
        });
    if let Err(error) = spawn {
        let message = format!("cannot start optimization promotion thread: {error}");
        set_promotion_status(
            &key,
            OptimizationPromotionStatus {
                status: "failed",
                segment: Some(segment.clone()),
                error: Some(message.clone()),
            },
        );
        return Err(promotion_message(message));
    }
    Ok(OptimizationPromotionResponse {
        schema: OPTIMIZATION_PROMOTION_RESPONSE_SCHEMA,
        candidate,
        segment,
        status: "verifying",
        repetitions: OPTIMIZATION_PROMOTION_REPETITIONS,
    })
}

fn prepare_optimization_promotion(
    config: &WorkbenchConfig,
    browser: &BrowserOptimizationPromoteRequest,
) -> Result<PreparedOptimizationPromotion, WorkbenchError> {
    validate_promoted_segment_id(&browser.segment_id)?;
    let label = validate_segment_name(&browser.label)?;
    let root = config
        .repository_root
        .canonicalize()
        .map_err(promotion_error)?;
    let timeline_path = validated_timeline_edit_path(&config.timeline_path)?;
    if !timeline_path.starts_with(&root) {
        return Err(promotion_message(
            "promotion timeline is outside the repository",
        ));
    }
    let projection = optimization_candidate_projections(&root, &timeline_path)?
        .into_iter()
        .find(|projection| projection.segment.id == browser.candidate)
        .ok_or_else(|| promotion_message("unknown, deleted, or expired optimization candidate"))?;
    let generated = projection
        .segment
        .generated
        .as_ref()
        .filter(|generated| generated.kind == "optimization_candidate")
        .ok_or_else(|| promotion_message("selected segment is not an optimization candidate"))?;
    let request = load_optimization_request(&root, &timeline_path, &generated.objective_sha256)?;
    let first_hit_tick = projection
        .segment
        .first_hit_tick
        .filter(|tick| *tick > 0 && *tick < request.budgets.promotion_before_tick)
        .ok_or_else(|| {
            promotion_message(format!(
                "candidate is not an exact success before the {}-tick promotion boundary",
                request.budgets.promotion_before_tick
            ))
        })?;
    let state: OptimizationResumeState =
        bounded_json::<OptimizationResumeState>(&root.join(&request.resume.state_path))
            .filter(|state| {
                state.request_sha256 == request.content_sha256 && state.validate().is_ok()
            })
            .ok_or_else(|| promotion_message("optimization resume state is invalid or detached"))?;
    let row = state
        .candidates
        .iter()
        .find(|row| row.id == generated.candidate_id && row.result.is_some())
        .ok_or_else(|| promotion_message("optimization candidate is absent from resume state"))?;
    let candidate: ResidualCampaignCandidate = bound_artifact_json(&root, &row.candidate)
        .filter(|candidate: &ResidualCampaignCandidate| candidate.validate().is_ok())
        .ok_or_else(|| promotion_message("optimization candidate artifact is invalid"))?;
    let evaluation_reference = row
        .result
        .as_ref()
        .ok_or_else(|| promotion_message("optimization candidate has no evaluation"))?;
    let campaign_root = root
        .join(&request.resume.state_path)
        .parent()
        .ok_or_else(|| promotion_message("optimization campaign has no artifact root"))?
        .to_path_buf();
    let execution: NativeResidualExecutionBinding =
        bounded_json(&campaign_root.join("execution/execution.json"))
            .filter(|execution: &NativeResidualExecutionBinding| {
                execution.validate_files(&root, &request).is_ok()
            })
            .ok_or_else(|| promotion_message("optimization execution binding is invalid"))?;
    let evaluation: NativeResidualCampaignEvaluation =
        bound_artifact_json(&root, evaluation_reference)
            .filter(|evaluation: &NativeResidualCampaignEvaluation| {
                evaluation
                    .validate(&request, &execution, &candidate)
                    .is_ok()
            })
            .ok_or_else(|| promotion_message("optimization evaluation is invalid"))?;
    if evaluation.evidence.verdict != (ExactTerminalVerdict::Reached { first_hit_tick })
        || evaluation.terminal_boundary_fingerprint != projection.segment.boundary_fingerprint
    {
        return Err(promotion_message(
            "optimization projection differs from its exact evaluation",
        ));
    }
    let mut local_tape = InputTape::decode(
        &bound_artifact_bytes(&root, &row.compiled_tape)
            .ok_or_else(|| promotion_message("compiled candidate tape is invalid"))?,
    )
    .map_err(promotion_error)?
    .tape;
    local_tape.frames.truncate(
        usize::try_from(first_hit_tick).map_err(|_| promotion_message("hit tick is too large"))?,
    );
    if local_tape.frames.len() as u64 != first_hit_tick {
        return Err(promotion_message(
            "candidate tape ends before its exact hit",
        ));
    }
    let source_index = usize::try_from(request.route.source_boundary_index)
        .map_err(|_| promotion_message("source boundary is too large"))?;
    if projection.full_tape.frames.len() < source_index {
        return Err(promotion_message("candidate parent tape is invalid"));
    }
    let mut full_tape = projection.full_tape.clone();
    full_tape.frames.truncate(
        source_index
            .checked_add(local_tape.frames.len())
            .ok_or_else(|| promotion_message("promotion tape length overflowed"))?,
    );
    if full_tape.frames.len() != source_index + local_tape.frames.len() {
        return Err(promotion_message(
            "candidate full tape ends before its exact hit",
        ));
    }

    let source = fs::read(&timeline_path).map_err(promotion_error)?;
    let source_text = String::from_utf8(source.clone())
        .map_err(|_| promotion_message("timeline source is not UTF-8"))?;
    let timeline = Timeline::parse(&source_text).map_err(promotion_error)?;
    if timeline.segments.contains_key(&browser.segment_id) {
        return Err(promotion_message("promoted segment ID already exists"));
    }
    let parent_segment = projection
        .segment
        .parent
        .clone()
        .ok_or_else(|| promotion_message("optimization candidate has no authored parent"))?;
    let promoted_goal_id = format!("{}_goal", browser.segment_id);
    let promoted_lineage_id = format!("promoted_{}", browser.segment_id);
    if timeline.goals.contains_key(&promoted_goal_id)
        || timeline.continuations.contains_key(&promoted_lineage_id)
        || timeline.branches.contains_key(&promoted_lineage_id)
    {
        return Err(promotion_message(
            "derived promotion goal or lineage ID already exists",
        ));
    }
    let lineage_dsl = promotion_lineage_dsl(
        &timeline,
        &request.route.lineage,
        &request.route.segment,
        &parent_segment,
        &promoted_lineage_id,
        &browser.segment_id,
        &request.route.source_boundary_fingerprint,
    )?;
    let timeline_directory = timeline_path
        .parent()
        .ok_or_else(|| promotion_message("timeline has no artifact root"))?;
    let promotion_directory = validated_promotion_directory(&root, &timeline_path, false)?;
    let tape_path = promotion_directory.join(format!("{}.tape", browser.segment_id));
    let proof_path = promotion_directory.join(format!("{}.promotion.json", browser.segment_id));
    let tape_relative = relative_timeline_artifact(timeline_directory, &tape_path)?;
    let proof_relative = relative_timeline_artifact(timeline_directory, &proof_path)?;
    let predicate_source_relative = relative_timeline_artifact(
        timeline_directory,
        &root.join(&request.terminal_predicate.source.path),
    )?;
    Ok(PreparedOptimizationPromotion {
        root,
        timeline_path,
        timeline_source_sha256: source_revision(&source),
        request,
        execution,
        candidate,
        candidate_artifact_sha256: row.candidate.sha256,
        evaluation,
        evaluation_artifact_sha256: evaluation_reference.sha256,
        graph_candidate_id: browser.candidate.clone(),
        promoted_segment_id: browser.segment_id.clone(),
        promoted_label: label,
        promoted_goal_id,
        promoted_lineage_id,
        parent_segment,
        profile: projection.segment.profile,
        tape_path,
        tape_relative,
        proof_path,
        proof_relative,
        predicate_source_relative,
        lineage_dsl,
        local_tape,
        full_tape,
        first_hit_tick,
    })
}

pub(super) fn load_optimization_request(
    root: &Path,
    timeline_path: &Path,
    request_sha256: &str,
) -> Result<OptimizationRequest, WorkbenchError> {
    let benchmark_root = timeline_path.with_extension("").join("benchmarks");
    let mut paths = fs::read_dir(benchmark_root)
        .map_err(promotion_error)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".request.json"))
        })
        .collect::<Vec<_>>();
    paths.sort();
    for path in paths {
        let Some(request) = bounded_json::<OptimizationRequest>(&path) else {
            continue;
        };
        if request.content_sha256.to_string() == request_sha256
            && request.validate_files(root).is_ok()
        {
            return Ok(request);
        }
    }
    Err(promotion_message(
        "optimization request for candidate is missing or invalid",
    ))
}

#[allow(clippy::too_many_arguments)]
fn promotion_lineage_dsl(
    timeline: &Timeline,
    source_lineage: &str,
    source_segment: &str,
    parent_segment: &str,
    promoted_lineage: &str,
    promoted_segment: &str,
    source_boundary: &str,
) -> Result<String, WorkbenchError> {
    let inspection = timeline.inspect().map_err(promotion_error)?;
    let lineage = inspection
        .lineages
        .iter()
        .find(|lineage| lineage.name == source_lineage)
        .ok_or_else(|| promotion_message("optimization source lineage is absent"))?;
    let source_index = lineage
        .steps
        .iter()
        .position(|step| step.segment == source_segment)
        .ok_or_else(|| promotion_message("optimization source segment is absent from lineage"))?;
    if source_index == 0 || lineage.steps[source_index - 1].segment != parent_segment {
        return Err(promotion_message(
            "optimization parent is not the immediate source-lineage checkpoint",
        ));
    }
    let mut output = format!(
        "continuation {promoted_lineage} starts root@{}\n",
        lineage.root_fingerprint
    );
    for step in &lineage.steps[..source_index] {
        output.push_str(&format!(
            "continue {promoted_lineage} with {} after {}@{}\n",
            step.segment, step.after.parent_segment, step.after.checkpoint_fingerprint
        ));
    }
    output.push_str(&format!(
        "continue {promoted_lineage} with {promoted_segment} after {parent_segment}@{source_boundary}\n"
    ));
    Ok(output)
}

fn run_optimization_cold_replays(
    config: &WorkbenchConfig,
    prepared: &PreparedOptimizationPromotion,
) -> Result<Vec<OptimizationColdReplayAttempt>, WorkbenchError> {
    prepared
        .execution
        .validate_files(&prepared.root, &prepared.request)
        .map_err(promotion_error)?;
    let session = random_session_token()?;
    let evidence_root = config
        .state_root
        .join("optimization-promotions")
        .join(&prepared.candidate.id)
        .join(session);
    fs::create_dir_all(&evidence_root).map_err(promotion_error)?;
    let tape_path = evidence_root.join("cold-boot.tape");
    fs::write(
        &tape_path,
        prepared.full_tape.encode().map_err(promotion_error)?,
    )
    .map_err(promotion_error)?;
    let program = prepared
        .root
        .join(&prepared.execution.milestone_program.path);
    let executable = prepared.root.join(&prepared.execution.executable.path);
    let game_data = prepared.root.join(&prepared.execution.game_data.path);
    let card_fixture = prepared
        .execution
        .card_fixture_root(&prepared.root, &prepared.request)
        .map_err(promotion_error)?;
    let logical_ticks = prepared.full_tape.frames.len().to_string();
    let mut attempts = Vec::new();
    for repetition in 1..=OPTIMIZATION_PROMOTION_REPETITIONS {
        let trial = evidence_root.join(format!("repeat-{repetition:03}"));
        let state = trial.join("state");
        let renderer = trial.join("renderer-cache");
        let result_path = trial.join("milestones.json");
        fs::create_dir_all(&state).map_err(promotion_error)?;
        fs::create_dir_all(&renderer).map_err(promotion_error)?;
        let stdout = fs::File::create(trial.join("stdout.txt")).map_err(promotion_error)?;
        let stderr = fs::File::create(trial.join("stderr.txt")).map_err(promotion_error)?;
        let mut command = Command::new(&executable);
        command
            .current_dir(&config.working_directory)
            .arg("--dvd")
            .arg(&game_data)
            .arg("--input-tape")
            .arg(&tape_path)
            .arg("--input-tape-end")
            .arg("hold")
            .arg("--automation-tick-budget")
            .arg(&logical_ticks)
            .arg("--automation-data-root")
            .arg(&state)
            .arg("--renderer-cache-root")
            .arg(&renderer)
            .arg("--automation-card-fixture")
            .arg(&card_fixture)
            .arg("--milestone-program")
            .arg(&program)
            .arg("--milestones")
            .arg(&prepared.request.terminal_predicate.goal)
            .arg("--milestone-goal")
            .arg(&prepared.request.terminal_predicate.goal)
            .arg("--milestone-result")
            .arg(&result_path)
            .arg("--headless")
            .arg("--fixed-step")
            .arg("--unpaced")
            .arg("--exit-after-tape")
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        for cvar in FIXED_AUTOMATION_CVARS {
            command.arg("--cvar").arg(cvar);
        }
        let started = Instant::now();
        let mut child = command.spawn().map_err(promotion_error)?;
        let status = loop {
            if let Some(status) = child.try_wait().map_err(promotion_error)? {
                break status;
            }
            if started.elapsed() >= OPTIMIZATION_PROMOTION_TIMEOUT {
                child.kill().map_err(promotion_error)?;
                let _ = child.wait();
                return Err(promotion_message(format!(
                    "promotion cold replay {repetition} timed out"
                )));
            }
            thread::sleep(Duration::from_millis(10));
        };
        if !status.success() {
            return Err(promotion_message(format!(
                "promotion cold replay {repetition} exited with {:?}",
                status.code()
            )));
        }
        let result = fs::read(&result_path).map_err(promotion_error)?;
        attempts.push(validate_cold_replay_result(prepared, repetition, &result)?);
    }
    if attempts.windows(2).any(|pair| {
        pair[0].sim_tick != pair[1].sim_tick
            || pair[0].tape_frame != pair[1].tape_frame
            || pair[0].boundary_index != pair[1].boundary_index
            || pair[0].boundary_fingerprint != pair[1].boundary_fingerprint
    }) {
        return Err(promotion_message(
            "five cold replays disagreed on the exact terminal proof",
        ));
    }
    Ok(attempts)
}

fn validate_cold_replay_result(
    prepared: &PreparedOptimizationPromotion,
    repetition: u32,
    bytes: &[u8],
) -> Result<OptimizationColdReplayAttempt, WorkbenchError> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(promotion_error)?;
    if value
        .pointer("/schema/name")
        .and_then(serde_json::Value::as_str)
        != Some("dusklight.automation.milestones")
        || value
            .pointer("/schema/version")
            .and_then(serde_json::Value::as_u64)
            != Some(5)
        || value
            .get("boot_origin_established")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || serde_json::from_value::<TapeBoot>(value["boot"].clone()).ok()
            != Some(prepared.full_tape.boot.clone())
        || value.get("goal").and_then(serde_json::Value::as_str)
            != Some(prepared.request.terminal_predicate.goal.as_str())
        || value
            .get("goal_reached")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || value
            .get("program_digest")
            .and_then(serde_json::Value::as_str)
            != Some(
                prepared
                    .request
                    .terminal_predicate
                    .program_sha256
                    .to_string()
                    .as_str(),
            )
    {
        return Err(promotion_message(
            "cold replay returned unauthenticated milestone authority",
        ));
    }
    let milestones = value
        .get("milestones")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| promotion_message("cold replay omitted milestones"))?;
    let matching = milestones
        .iter()
        .filter(|milestone| {
            milestone.get("id").and_then(serde_json::Value::as_str)
                == Some(prepared.request.terminal_predicate.goal.as_str())
        })
        .collect::<Vec<_>>();
    if matching.len() != 1 {
        return Err(promotion_message(
            "cold replay did not return exactly one terminal goal",
        ));
    }
    let milestone = matching[0];
    let sim_tick = milestone
        .get("sim_tick")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| promotion_message("cold replay goal omitted sim_tick"))?;
    let tape_frame = milestone
        .get("tape_frame")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| promotion_message("cold replay goal omitted tape_frame"))?;
    let boundary_index = milestone
        .get("boundary_index")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| promotion_message("cold replay goal omitted boundary index"))?;
    let boundary_fingerprint: BoundaryFingerprint = serde_json::from_value(
        milestone
            .pointer("/evidence/boundary_fingerprint")
            .cloned()
            .ok_or_else(|| promotion_message("cold replay goal omitted boundary fingerprint"))?,
    )
    .map_err(promotion_error)?;
    let full_frames = u64::try_from(prepared.full_tape.frames.len()).map_err(promotion_error)?;
    if milestone.get("hit").and_then(serde_json::Value::as_bool) != Some(true)
        || milestone.get("phase").and_then(serde_json::Value::as_str) != Some("post_sim")
        || milestone
            .get("definition_digest")
            .and_then(serde_json::Value::as_str)
            != Some(
                prepared
                    .request
                    .terminal_predicate
                    .definition_sha256
                    .to_string()
                    .as_str(),
            )
        || milestone
            .get("program_digest")
            .and_then(serde_json::Value::as_str)
            != Some(
                prepared
                    .request
                    .terminal_predicate
                    .program_sha256
                    .to_string()
                    .as_str(),
            )
        || tape_frame.checked_add(1) != Some(full_frames)
        || boundary_index != full_frames
        || !exact_boundary_fingerprint(&boundary_fingerprint)
        || boundary_fingerprint.digest != prepared.evaluation.terminal_boundary_fingerprint
    {
        return Err(promotion_message(
            "cold replay terminal proof differs from the checkpoint evaluation",
        ));
    }
    Ok(OptimizationColdReplayAttempt {
        repetition,
        milestone_result_sha256: ArtifactDigest(Sha256::digest(bytes).into()),
        sim_tick,
        tape_frame,
        boundary_index,
        boundary_fingerprint,
    })
}

fn install_optimization_promotion(
    prepared: &PreparedOptimizationPromotion,
    proof: &OptimizationPromotionProof,
) -> Result<(), WorkbenchError> {
    proof.validate()?;
    let _edit = timeline_edits()
        .lock()
        .map_err(|_| promotion_message("timeline promotion lock is poisoned"))?;
    let timeline_path = validated_timeline_edit_path(&prepared.timeline_path)?;
    let original = fs::read(&timeline_path).map_err(promotion_error)?;
    if source_revision(&original) != prepared.timeline_source_sha256 {
        return Err(promotion_message(
            "timeline changed during cold replay; reload before promoting",
        ));
    }
    let source = String::from_utf8(original.clone())
        .map_err(|_| promotion_message("timeline source is not UTF-8"))?;
    let replacement = promotion_timeline_source(prepared, proof, &source)?;
    let parsed = Timeline::parse(&replacement).map_err(promotion_error)?;
    if parsed
        .segments
        .get(&prepared.promoted_segment_id)
        .is_none_or(|segment| {
            segment.parent.as_deref() != Some(prepared.parent_segment.as_str())
                || segment.start_fingerprint != prepared.request.route.source_boundary_fingerprint
                || segment.end_fingerprint != prepared.evaluation.terminal_boundary_fingerprint
        })
        || parsed
            .goals
            .get(&prepared.promoted_goal_id)
            .is_none_or(|goal| goal.segment != prepared.promoted_segment_id)
        || !parsed
            .continuations
            .contains_key(&prepared.promoted_lineage_id)
    {
        return Err(promotion_message(
            "promoted timeline did not preserve the candidate proof and lineage",
        ));
    }
    if prepared.tape_path.exists() || prepared.proof_path.exists() {
        return Err(promotion_message(
            "promotion destination already exists; no files were replaced",
        ));
    }
    let destination_parent = prepared
        .tape_path
        .parent()
        .ok_or_else(|| promotion_message("promotion tape has no parent directory"))?;
    let validated_destination =
        validated_promotion_directory(&prepared.root, &timeline_path, true)?;
    if destination_parent != validated_destination {
        return Err(promotion_message(
            "promotion destination differs from its validated repository path",
        ));
    }
    let destination_parent = validated_destination;
    let nonce = random_session_token()?;
    let tape_temporary = destination_parent.join(format!(".promotion-{nonce}.tape.tmp"));
    let proof_temporary = destination_parent.join(format!(".promotion-{nonce}.proof.tmp"));
    let timeline_directory = timeline_path
        .parent()
        .ok_or_else(|| promotion_message("timeline has no parent directory"))?;
    let timeline_temporary = timeline_directory.join(format!(".promotion-{nonce}.timeline.tmp"));
    let timeline_backup = timeline_directory.join(format!(".promotion-{nonce}.rollback"));
    write_new_synced(
        &tape_temporary,
        &prepared.local_tape.encode().map_err(promotion_error)?,
    )?;
    write_new_synced(&proof_temporary, &proof.to_pretty_json()?)?;
    write_new_synced(&timeline_temporary, replacement.as_bytes())?;
    let mut tape_cleanup = RemoveFileOnDrop(Some(tape_temporary.clone()));
    let mut proof_cleanup = RemoveFileOnDrop(Some(proof_temporary.clone()));
    let mut timeline_cleanup = RemoveFileOnDrop(Some(timeline_temporary.clone()));
    if fs::read(&timeline_path).ok() != Some(original.clone()) {
        return Err(promotion_message(
            "timeline changed while staging promotion; reload and retry",
        ));
    }
    fs::rename(&tape_temporary, &prepared.tape_path).map_err(promotion_error)?;
    tape_cleanup.0 = None;
    if let Err(error) = fs::rename(&proof_temporary, &prepared.proof_path) {
        let _ = fs::remove_file(&prepared.tape_path);
        return Err(promotion_error(error));
    }
    proof_cleanup.0 = None;
    if let Err(error) = fs::rename(&timeline_path, &timeline_backup) {
        let _ = fs::remove_file(&prepared.tape_path);
        let _ = fs::remove_file(&prepared.proof_path);
        return Err(promotion_error(error));
    }
    if let Err(error) = fs::rename(&timeline_temporary, &timeline_path) {
        let _ = fs::rename(&timeline_backup, &timeline_path);
        let _ = fs::remove_file(&prepared.tape_path);
        let _ = fs::remove_file(&prepared.proof_path);
        return Err(promotion_error(error));
    }
    timeline_cleanup.0 = None;
    let _ = fs::remove_file(timeline_backup);
    Ok(())
}

fn promotion_timeline_source(
    prepared: &PreparedOptimizationPromotion,
    proof: &OptimizationPromotionProof,
    source: &str,
) -> Result<String, WorkbenchError> {
    let mut output = source.to_owned();
    if !output.ends_with('\n') {
        output.push('\n');
    }
    output.push_str(&format!(
        "\n# Authenticated optimization promotion; proof {} sha256 {}\n",
        prepared.proof_relative, proof.content_sha256
    ));
    output.push_str(&format!(
        "segment {} after {} profile {} uses tape {} starts {} produces {}\n",
        prepared.promoted_segment_id,
        prepared.parent_segment,
        prepared.profile,
        prepared.tape_relative,
        prepared.request.route.source_boundary_fingerprint,
        prepared.evaluation.terminal_boundary_fingerprint
    ));
    output.push_str(&format!(
        "label {} \"{}\"\n",
        prepared.promoted_segment_id, prepared.promoted_label
    ));
    output.push_str(&format!(
        "goal {} on {} predicate {} source {}\n",
        prepared.promoted_goal_id,
        prepared.promoted_segment_id,
        prepared.request.terminal_predicate.goal,
        prepared.predicate_source_relative
    ));
    output.push_str(&format!(
        "proof {} satisfies {} program {} predicate {} ticks {}\n\n",
        prepared.promoted_segment_id,
        prepared.promoted_goal_id,
        prepared.request.terminal_predicate.program_sha256,
        prepared.request.terminal_predicate.definition_sha256,
        prepared.first_hit_tick
    ));
    output.push_str(&prepared.lineage_dsl);
    Ok(output)
}

fn write_new_synced(path: &Path, bytes: &[u8]) -> Result<(), WorkbenchError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(promotion_error)?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(promotion_error)
}

fn relative_timeline_artifact(root: &Path, path: &Path) -> Result<String, WorkbenchError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| promotion_message("promotion artifact is outside the timeline root"))?;
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(promotion_message("promotion artifact path is noncanonical"));
    }
    Ok(relative
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/"))
}

fn validated_promotion_directory(
    root: &Path,
    timeline_path: &Path,
    create_missing: bool,
) -> Result<PathBuf, WorkbenchError> {
    let base = timeline_path.with_extension("");
    let metadata = fs::symlink_metadata(&base).map_err(|error| {
        promotion_message(format!(
            "cannot inspect promotion artifact root {}: {error}",
            base.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(promotion_message(format!(
            "promotion artifact root {} is not a physical directory",
            base.display()
        )));
    }
    let mut current = base.canonicalize().map_err(promotion_error)?;
    if !current.starts_with(root) {
        return Err(promotion_message(
            "promotion artifact root is outside the repository",
        ));
    }
    let mut parent_missing = false;
    for component in ["segments", "optimized"] {
        let next = current.join(component);
        if parent_missing && !create_missing {
            current = next;
            continue;
        }
        let metadata = match fs::symlink_metadata(&next) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && create_missing => {
                match fs::create_dir(&next) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(promotion_error(error)),
                }
                fs::symlink_metadata(&next).map_err(promotion_error)?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                parent_missing = true;
                current = next;
                continue;
            }
            Err(error) => return Err(promotion_error(error)),
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(promotion_message(format!(
                "promotion artifact path {} is not a physical directory",
                next.display()
            )));
        }
        current = next.canonicalize().map_err(promotion_error)?;
        if !current.starts_with(root) {
            return Err(promotion_message(
                "promotion artifact directory is outside the repository",
            ));
        }
    }
    Ok(current)
}

fn validate_promoted_segment_id(id: &str) -> Result<(), WorkbenchError> {
    if id.is_empty()
        || id.len() > 80
        || !id
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase())
        || id.bytes().any(|byte| {
            !(byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'_' | b'-' | b'.'))
        })
    {
        return Err(promotion_message(
            "promoted segment ID must start with a lowercase letter and contain only lowercase letters, digits, '.', '-', or '_'",
        ));
    }
    Ok(())
}

fn exact_boundary_fingerprint(fingerprint: &BoundaryFingerprint) -> bool {
    fingerprint.algorithm == "xxh3-128"
        && native_fingerprint(&fingerprint.digest)
        && matches!(
            (
                fingerprint.schema.as_str(),
                fingerprint.canonical_encoding.as_str()
            ),
            ("dusklight.milestone-boundary/v4", "little-endian-fixed-v4")
                | ("dusklight.milestone-boundary/v5", "little-endian-fixed-v5")
                | ("dusklight.milestone-boundary/v6", "little-endian-fixed-v6")
        )
}

fn promotion_message(message: impl Into<String>) -> WorkbenchError {
    WorkbenchError::new(message)
}

fn promotion_error(error: impl fmt::Display) -> WorkbenchError {
    WorkbenchError::new(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_automation_contracts::tape::InputFrame;
    use dusklight_orchestration::native_residual_campaign::{
        NATIVE_RESIDUAL_EXECUTION_SCHEMA_V1, NativeResidualAttempt,
    };
    use dusklight_search::residual_action::{
        AnalogChannel, AnalogResidual, ResidualCandidate, TemporalBasis,
        compile_residual_candidate_to_horizon,
    };
    use dusklight_search::residual_optimizer::ResidualGenome;

    fn test_root(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "dusklight-promotion-{name}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        root.canonicalize().unwrap()
    }

    fn reference(path: &str, byte: u8) -> ArtifactReference {
        ArtifactReference {
            path: path.into(),
            sha256: ArtifactDigest([byte; 32]),
        }
    }

    fn prepared_fixture(name: &str) -> PreparedOptimizationPromotion {
        let root = test_root(name);
        let timeline_path = root.join("route.timeline");
        let parent_tape = InputTape {
            frames: vec![
                InputFrame {
                    owned_ports: 1,
                    ..InputFrame::default()
                };
                2
            ],
            ..InputTape::default()
        };
        let parent_bytes = parent_tape.encode().unwrap();
        fs::write(root.join("parent.tape"), &parent_bytes).unwrap();
        fs::write(root.join("incumbent.tape"), &parent_bytes).unwrap();
        fs::write(
            &timeline_path,
            format!(
                "timeline promotion\nsegment parent root profile boot_to_fsp103 uses tape parent.tape starts {} produces {}\nsegment incumbent after parent profile fsp103_to_fsp104 uses tape incumbent.tape starts {} produces {}\ncontinuation main starts root@{}\ncontinue main with parent after root@{}\ncontinue main with incumbent after parent@{}\n",
                "1".repeat(32),
                "a".repeat(32),
                "a".repeat(32),
                "c".repeat(32),
                "1".repeat(32),
                "1".repeat(32),
                "a".repeat(32),
            ),
        )
        .unwrap();
        fs::write(
            root.join("goal.milestones"),
            "milestones 1.3\nmilestone terminal { phase post_sim when player.exists }\n",
        )
        .unwrap();
        fs::create_dir(root.join("route")).unwrap();

        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../..")
            .canonicalize()
            .unwrap();
        let mut request: OptimizationRequest = serde_json::from_slice(
            &fs::read(repository.join(
                "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json",
            ))
            .unwrap(),
        )
        .unwrap();
        request.route.timeline.path = "route.timeline".into();
        request.route.timeline.sha256 =
            ArtifactDigest(Sha256::digest(fs::read(&timeline_path).unwrap()).into());
        request.route.lineage = "main".into();
        request.route.segment = "incumbent".into();
        request.route.source_boundary_index = 2;
        request.route.source_boundary_fingerprint = "a".repeat(32);
        request.route.native_source_boundary_fingerprint = "d".repeat(32);
        request.terminal_predicate.goal = "terminal".into();
        request.terminal_predicate.source = ArtifactReference {
            path: "goal.milestones".into(),
            sha256: ArtifactDigest(
                Sha256::digest(fs::read(root.join("goal.milestones")).unwrap()).into(),
            ),
        };
        let compiled_goal = milestone_dsl::compile_source(
            &fs::read_to_string(root.join("goal.milestones")).unwrap(),
        )
        .unwrap();
        request.terminal_predicate.program_sha256 = ArtifactDigest(compiled_goal.program_sha256);
        request.terminal_predicate.definition_sha256 =
            ArtifactDigest(compiled_goal.definitions[0].sha256);
        request.budgets.exploration_horizon_ticks = 10;
        request.budgets.promotion_before_tick = 5;
        request.execution.workers = 1;
        request.execution.deterministic_seeds = vec![17];
        request.execution.repetitions = 1;
        request.refresh_content_sha256().unwrap();

        let residual = ResidualCandidate::seal(
            &parent_bytes,
            vec![AnalogResidual {
                port: 0,
                channel: AnalogChannel::MainX,
                basis: TemporalBasis::ExactFrame { frame: 0, delta: 8 },
            }],
            Vec::new(),
        )
        .unwrap();
        let compiled = compile_residual_candidate_to_horizon(
            &parent_tape,
            &parent_bytes,
            &residual,
            request.budgets.exploration_horizon_ticks,
        )
        .unwrap();
        let candidate = ResidualCampaignCandidate::seal(
            "g000001-s00000-promote".into(),
            1,
            0,
            17,
            ResidualGenome { genes: Vec::new() },
            residual,
            &compiled,
        )
        .unwrap();
        let execution = NativeResidualExecutionBinding {
            schema: NATIVE_RESIDUAL_EXECUTION_SCHEMA_V1.into(),
            content_sha256: ArtifactDigest([2; 32]),
            optimization_request_sha256: request.content_sha256,
            executable: reference("build/Dusklight", 3),
            game_data: reference("build/game.iso", 4),
            process_boot_tape: reference("build/process.tape", 5),
            milestone_program: reference("build/goal.dmsp", 6),
            world_context: reference("build/world.json", 7),
            card_fixture_manifest: reference("build/card.json", 8),
            checkpoint_validation_ticks: 2,
            verify_state_hashes: false,
        };
        let evaluation = NativeResidualCampaignEvaluation::seal(
            &request,
            &execution,
            &candidate,
            vec![NativeResidualAttempt {
                repetition: 1,
                worker_seed: 17,
                wire_candidate_id: "candidate-1".into(),
                batch_request: reference("build/request.json", 9),
                batch_result: reference("build/result.json", 10),
                episode_shard: reference("build/episode.bin", 11),
                restore_identity: "e".repeat(32),
                checkpoint_bytes: 64,
                simulated_ticks: 2,
                first_hit_tick: Some(2),
                terminal_boundary_fingerprint: "b".repeat(32),
                behavior_sha256: ArtifactDigest([12; 32]),
            }],
        )
        .unwrap();
        let mut local_tape = InputTape::decode(&compiled.bytes).unwrap().tape;
        local_tape.frames.truncate(2);
        let full_tape = concatenate(vec![
            ChainSegment::all(parent_tape),
            ChainSegment::all(local_tape.clone()),
        ])
        .unwrap()
        .tape;
        let promotion_root = root.join("route/segments/optimized");
        let timeline_source = fs::read(&timeline_path).unwrap();
        PreparedOptimizationPromotion {
            root: root.clone(),
            timeline_path,
            timeline_source_sha256: source_revision(&timeline_source),
            request,
            execution,
            candidate,
            candidate_artifact_sha256: ArtifactDigest([13; 32]),
            evaluation,
            evaluation_artifact_sha256: ArtifactDigest([14; 32]),
            graph_candidate_id: "optimization-candidate".into(),
            promoted_segment_id: "optimized_terminal_fast".into(),
            promoted_label: "Optimized terminal 2f".into(),
            promoted_goal_id: "optimized_terminal_fast_goal".into(),
            promoted_lineage_id: "promoted_optimized_terminal_fast".into(),
            parent_segment: "parent".into(),
            profile: "fsp103_to_fsp104".into(),
            tape_path: promotion_root.join("optimized_terminal_fast.tape"),
            tape_relative: "route/segments/optimized/optimized_terminal_fast.tape".into(),
            proof_path: promotion_root.join("optimized_terminal_fast.promotion.json"),
            proof_relative: "route/segments/optimized/optimized_terminal_fast.promotion.json"
                .into(),
            predicate_source_relative: "goal.milestones".into(),
            lineage_dsl: format!(
                "continuation promoted_optimized_terminal_fast starts root@{}\ncontinue promoted_optimized_terminal_fast with parent after root@{}\ncontinue promoted_optimized_terminal_fast with optimized_terminal_fast after parent@{}\n",
                "1".repeat(32),
                "1".repeat(32),
                "a".repeat(32),
            ),
            local_tape,
            full_tape,
            first_hit_tick: 2,
        }
    }

    fn attempts(prepared: &PreparedOptimizationPromotion) -> Vec<OptimizationColdReplayAttempt> {
        (1..=OPTIMIZATION_PROMOTION_REPETITIONS)
            .map(|repetition| OptimizationColdReplayAttempt {
                repetition,
                milestone_result_sha256: ArtifactDigest([repetition as u8; 32]),
                sim_tick: 3,
                tape_frame: 3,
                boundary_index: 4,
                boundary_fingerprint: BoundaryFingerprint {
                    schema: "dusklight.milestone-boundary/v6".into(),
                    algorithm: "xxh3-128".into(),
                    canonical_encoding: "little-endian-fixed-v6".into(),
                    digest: prepared.evaluation.terminal_boundary_fingerprint.clone(),
                },
            })
            .collect()
    }

    fn cold_result(prepared: &PreparedOptimizationPromotion) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "schema": {"name": "dusklight.automation.milestones", "version": 5},
            "boot": prepared.full_tape.boot,
            "boot_origin_established": true,
            "goal": prepared.request.terminal_predicate.goal,
            "goal_reached": true,
            "program_digest": prepared.request.terminal_predicate.program_sha256,
            "milestones": [{
                "id": prepared.request.terminal_predicate.goal,
                "hit": true,
                "sim_tick": 3,
                "tape_frame": 3,
                "phase": "post_sim",
                "definition_digest": prepared.request.terminal_predicate.definition_sha256,
                "program_digest": prepared.request.terminal_predicate.program_sha256,
                "boundary_index": 4,
                "evidence": {
                    "boundary_fingerprint": {
                        "schema": "dusklight.milestone-boundary/v6",
                        "algorithm": "xxh3-128",
                        "canonical_encoding": "little-endian-fixed-v6",
                        "digest": prepared.evaluation.terminal_boundary_fingerprint,
                    }
                }
            }]
        }))
        .unwrap()
    }

    #[test]
    fn cold_replay_result_requires_the_exact_trimmed_terminal_boundary() {
        let prepared = prepared_fixture("cold-result");
        let result = cold_result(&prepared);
        let attempt = validate_cold_replay_result(&prepared, 1, &result).unwrap();
        assert_eq!(attempt.tape_frame, 3);
        assert_eq!(attempt.boundary_index, 4);
        assert_eq!(
            attempt.boundary_fingerprint.digest,
            prepared.evaluation.terminal_boundary_fingerprint
        );

        let mut tampered: serde_json::Value = serde_json::from_slice(&result).unwrap();
        tampered["milestones"][0]["evidence"]["boundary_fingerprint"]["digest"] =
            serde_json::Value::String("f".repeat(32));
        assert!(
            validate_cold_replay_result(&prepared, 1, &serde_json::to_vec(&tampered).unwrap())
                .is_err()
        );
        fs::remove_dir_all(&prepared.root).unwrap();
    }

    #[test]
    fn promotion_installs_compact_tape_sealed_proof_and_explicit_lineage() {
        let prepared = prepared_fixture("install");
        let proof = OptimizationPromotionProof::seal(&prepared, attempts(&prepared)).unwrap();
        install_optimization_promotion(&prepared, &proof).unwrap();

        let source = fs::read_to_string(&prepared.timeline_path).unwrap();
        let timeline = Timeline::parse(&source).unwrap();
        let promoted = &timeline.segments[&prepared.promoted_segment_id];
        assert_eq!(promoted.parent.as_deref(), Some("parent"));
        assert_eq!(promoted.start_fingerprint, "a".repeat(32));
        assert_eq!(promoted.end_fingerprint, "b".repeat(32));
        assert_eq!(
            timeline.goals[&prepared.promoted_goal_id].predicate,
            "terminal"
        );
        assert!(
            timeline
                .continuations
                .contains_key(&prepared.promoted_lineage_id)
        );
        let decoded = InputTape::decode(&fs::read(&prepared.tape_path).unwrap())
            .unwrap()
            .tape;
        assert_eq!(decoded.frames.len(), 2);
        let stored: OptimizationPromotionProof =
            serde_json::from_slice(&fs::read(&prepared.proof_path).unwrap()).unwrap();
        stored.validate().unwrap();
        assert_eq!(stored, proof);
        assert_eq!(stored.repetitions.len(), 5);
        assert!(source.contains(&format!("proof {} satisfies", prepared.promoted_segment_id)));
        assert!(source.contains(&prepared.proof_relative));
        fs::remove_dir_all(&prepared.root).unwrap();
    }

    #[test]
    fn promotion_rejects_tampered_proof_and_stale_timeline_without_partial_files() {
        let prepared = prepared_fixture("stale");
        let proof = OptimizationPromotionProof::seal(&prepared, attempts(&prepared)).unwrap();
        let mut tampered = proof.clone();
        tampered.repetitions[4].boundary_fingerprint.digest = "f".repeat(32);
        assert!(tampered.validate().is_err());

        fs::write(&prepared.timeline_path, "timeline changed\n").unwrap();
        assert!(install_optimization_promotion(&prepared, &proof).is_err());
        assert!(!prepared.tape_path.exists());
        assert!(!prepared.proof_path.exists());
        fs::remove_dir_all(&prepared.root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn promotion_rejects_symlinked_artifact_directories_before_writing() {
        use std::os::unix::fs::symlink;

        let prepared = prepared_fixture("symlink");
        let outside = test_root("symlink-outside");
        symlink(&outside, prepared.root.join("route/segments")).unwrap();
        let proof = OptimizationPromotionProof::seal(&prepared, attempts(&prepared)).unwrap();

        assert!(install_optimization_promotion(&prepared, &proof).is_err());
        assert!(fs::read_dir(&outside).unwrap().next().is_none());
        assert!(!prepared.tape_path.exists());
        assert!(!prepared.proof_path.exists());

        fs::remove_dir_all(&prepared.root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }
}
