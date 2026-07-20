//! Native, cross-platform population evaluation and evidence interpretation.

use crate::artifact::Digest as ArtifactDigest;
use crate::behavior_archive::BehaviorContext;
use crate::content_store::{ContentBlob, ContentKind, ContentStore};
use crate::dataset::{DATASET_SOURCE_SCHEMA_V1, DatasetSourceDescriptor};
use crate::episode::{
    EPISODE_CONTEXT_SCHEMA_V1, EpisodeContext, EpisodeIntervention, EpisodeLedger, EpisodeLineage,
    EpisodeManifest, EpisodeManifestBuild, EpisodeObjectiveIdentity, EpisodeOutcome,
    EpisodeOutcomeClass, EpisodeProducerIdentity, EpisodeProducerKind, EpisodeSeed,
    RunBuildIdentity,
};
use crate::episode_store::{EpisodeBundleSources, EpisodeStore};
pub use crate::harness::evaluation::{AnchoredObjectiveIdentity, BoundaryFingerprint};
use crate::harness::execution::execute_request;
use crate::harness::objective_suite::{ArtifactReference, ObjectiveSeed};
use crate::harness::run_contract::{HarnessRunRequest, HarnessRunResult, HarnessTerminalReason};
use crate::offline_rl::{
    ExploratoryExtractConfig, extract_exploratory_v2_from_bytes, extract_exploratory_v3_from_bytes,
    movement_action_schema_digest_v2, movement_action_schema_digest_v3,
};
use crate::search::{
    Ancestry, Candidate, CandidateResult, LeaderboardEntry, LexicographicScore, POPULATION_SCHEMA,
    PopulationManifest, RESULTS_SCHEMA, SearchResults, SegmentProfile, rank_population,
    tape_input_complexity,
};
use crate::semantic_novelty::{BoundaryFingerprintFact, SemanticNoveltyDescriptor};
use crate::tape::{InputTape, TapeBoot};
use crate::tape_chain::{ChainSegment, concatenate};
use crate::transition_corpus::{StateReference, StateReferenceKind, TransitionCorpus};
use crate::transition_evidence::{
    ImmutableEpisodeArtifact, ImmutableEpisodeBuild, TerminalReasonEvidence,
    TransitionEvidenceBuild, TransitionEvidenceBundle,
};
pub use dusklight_evaluation_plan::{
    EvaluationWorkerSchedule, PlannedWorkerAssignment, WORKER_SCHEDULE_SCHEMA,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const EVALUATION_SCHEMA: &str = "dusklight-search-evaluation/v5";
pub const ATTEMPT_SCHEMA: &str = "dusklight-search-attempt/v5";
pub const ANCHORED_RESULTS_SCHEMA: &str = "dusklight-anchored-search-results/v2";
const NATIVE_GOAL_MISS_EXIT_CODE: i32 = 2;

fn is_anchored_profile(profile: SegmentProfile) -> bool {
    matches!(
        profile,
        SegmentProfile::BootToFsp103
            | SegmentProfile::Fsp103ToFsp104
            | SegmentProfile::LinkControlToTunnelCrawlStart
    )
}

/// Immutable proof inputs for a clean-boot suffix search. The prefix is an
/// absolute compact tape, compiled DMSP, game executable, and DVD image;
/// callers may materialize the route inputs through any management UX.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnchoredObjectiveConfig {
    pub segment: SegmentProfile,
    pub prefix_tape: PathBuf,
    pub milestone_program: PathBuf,
    pub game: PathBuf,
    pub dvd: PathBuf,
    pub source_milestone: String,
    pub source_boundary_fingerprint: String,
    pub goal_milestone: String,
}

#[derive(Clone, Debug)]
pub struct AnchoredEvaluateConfig {
    pub evaluation: EvaluateConfig,
    pub objective: AnchoredObjectiveConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AnchoredSearchResults {
    pub schema: String,
    pub objective: AnchoredObjectiveIdentity,
    pub results: SearchResults,
}

#[derive(Clone, Debug)]
pub struct EvaluateConfig {
    pub population_path: PathBuf,
    pub game: PathBuf,
    pub dvd: PathBuf,
    pub output_root: PathBuf,
    pub episode_store: Option<PathBuf>,
    pub results_path: PathBuf,
    pub working_directory: PathBuf,
    pub game_args_prefix: Vec<String>,
    pub workers: usize,
    pub repetitions: u32,
    pub timeout: Duration,
    /// Sole authenticated execution authority for migrated entry points.
    pub harness: Option<HarnessEvaluateConfig>,
}

#[derive(Clone, Debug)]
pub struct HarnessEvaluateConfig {
    pub repository_root: PathBuf,
    pub request_template: HarnessRunRequest,
}

/// Derive one candidate-specific request without weakening any identity from
/// the authenticated template. The candidate tape and destination become part
/// of the new request digest; every other objective/build/protocol binding is
/// retained byte-for-byte.
pub fn derive_candidate_request(
    template: &HarnessRunRequest,
    repository_root: &Path,
    tape_path: &Path,
    artifact_destination: &str,
    rng_seed: u64,
) -> Result<HarnessRunRequest, EvaluateError> {
    template.validate_files(repository_root).map_err(|error| {
        EvaluateError::InvalidConfig(format!("invalid harness request template: {error}"))
    })?;
    let repository_root = fs::canonicalize(repository_root)?;
    let tape_path = fs::canonicalize(tape_path)?;
    let relative = tape_path.strip_prefix(&repository_root).map_err(|_| {
        EvaluateError::InvalidConfig(format!(
            "candidate tape is outside the harness repository root: {}",
            tape_path.display()
        ))
    })?;
    let path = relative
        .to_str()
        .ok_or_else(|| EvaluateError::InvalidConfig("candidate tape path is not UTF-8".into()))?
        .replace(std::path::MAIN_SEPARATOR, "/");
    let bytes = fs::read(&tape_path)?;
    let tape = InputTape::decode(&bytes)?;
    let ticks = u64::try_from(tape.tape.frames.len()).map_err(|_| {
        EvaluateError::InvalidConfig("candidate tape length does not fit u64".into())
    })?;
    if ticks == 0 || ticks > template.logical_tick_budget {
        return Err(EvaluateError::InvalidConfig(format!(
            "candidate tape requires {ticks} ticks but template budget is {}",
            template.logical_tick_budget
        )));
    }

    let mut request = template.clone();
    request.input = ObjectiveSeed::Tape {
        artifact: ArtifactReference {
            path,
            sha256: ArtifactDigest(Sha256::digest(&bytes).into()),
        },
    };
    request.rng_seed = rng_seed;
    request.artifact_destination = artifact_destination.into();
    request.content_sha256 = ArtifactDigest::ZERO;
    request
        .refresh_content_sha256()
        .map_err(|error| EvaluateError::InvalidConfig(error.to_string()))?;
    request.validate_files(&repository_root).map_err(|error| {
        EvaluateError::InvalidConfig(format!("candidate request is invalid: {error}"))
    })?;
    Ok(request)
}

#[derive(Clone, Debug, Serialize)]
pub struct EvaluationReport {
    pub schema: &'static str,
    pub population: PathBuf,
    pub results: PathBuf,
    pub worker_schedule: PathBuf,
    pub worker_schedule_sha256: ArtifactDigest,
    pub segment: SegmentProfile,
    pub boot: TapeBoot,
    pub workers: usize,
    pub repetitions: u32,
    pub planned_attempts: usize,
    pub completed_attempts: usize,
    pub infrastructure_faults: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub episode_ledger: Option<PathBuf>,
    pub attempts: Vec<AttemptEvidence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub objective: Option<AnchoredObjectiveIdentity>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AttemptEvidence {
    pub schema: &'static str,
    pub candidate_id: String,
    pub ancestry: Ancestry,
    pub attempt: u32,
    pub worker_id: String,
    pub segment: SegmentProfile,
    pub boot: TapeBoot,
    pub tape: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub realized_tape: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suffix_tape: Option<PathBuf>,
    pub artifact_root: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness_request: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness_request_sha256: Option<ArtifactDigest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness_result: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness_result_sha256: Option<ArtifactDigest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness_terminal: Option<HarnessTerminalReason>,
    pub state_root: PathBuf,
    pub milestone_result: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gameplay_trace: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gameplay_trace_blob: Option<ContentBlob>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gameplay_trace_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition_corpus: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition_evidence: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub episode_manifest: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub episode_store_entry: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub immutable_episode: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dataset_source: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition_corpus_error: Option<String>,
    pub stdout: PathBuf,
    pub stderr: PathBuf,
    pub elapsed_millis: u128,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub cancelled: bool,
    pub infrastructure_error: Option<String>,
    pub outcome: EpisodeOutcome,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub crash_artifacts: Vec<ContentBlob>,
    pub milestone_depth: u16,
    pub deepest_milestone: String,
    pub first_hit_tick: Option<u64>,
    pub goal_reached: bool,
    pub milestone_observations: BTreeMap<String, MilestoneObservation>,
    pub boundary_fingerprints: BTreeMap<String, BoundaryFingerprint>,
    pub value_projections: BTreeMap<String, BTreeMap<String, ValueProjectionEvidence>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MilestoneObservation {
    pub sim_tick: u64,
    pub tape_frame: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boundary_index: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stable_ticks: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program_digest: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ValueProjectionEvidence {
    pub name: String,
    pub identity: String,
    pub available: bool,
    pub value_fingerprint: Option<BoundaryFingerprint>,
    pub values: Vec<serde_json::Value>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueParityComparison {
    Equal,
    Different,
    Incomparable,
}

/// Compare one exact named value axis. Route ancestry is deliberately absent.
pub fn compare_value_projections(
    left: &ValueProjectionEvidence,
    right: &ValueProjectionEvidence,
) -> ValueParityComparison {
    if left.name != right.name
        || left.identity != right.identity
        || !left.available
        || !right.available
        || left.value_fingerprint.is_none()
        || right.value_fingerprint.is_none()
    {
        return ValueParityComparison::Incomparable;
    }
    if left.value_fingerprint == right.value_fingerprint {
        ValueParityComparison::Equal
    } else {
        ValueParityComparison::Different
    }
}

#[derive(Clone, Debug)]
struct AuthoredDefinitionExpectation {
    name: String,
    phase: String,
    stable_ticks: u16,
    digest: String,
    projections: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct PreparedAnchoredEvaluator {
    config: AnchoredObjectiveConfig,
    identity: AnchoredObjectiveIdentity,
    prefix: InputTape,
    program_bytes: Vec<u8>,
    source: AuthoredDefinitionExpectation,
    progress: Vec<AuthoredDefinitionExpectation>,
    goal: AuthoredDefinitionExpectation,
    runtime_program: PathBuf,
}

/// Resolve and validate all immutable inputs, returning the content identity
/// that binds a population and its results to this exact objective.
pub fn anchored_objective_identity(
    config: &AnchoredObjectiveConfig,
) -> Result<AnchoredObjectiveIdentity, EvaluateError> {
    Ok(prepare_anchored_objective(config, PathBuf::new())?.identity)
}

/// Prepare and authenticate immutable anchored-objective inputs once for a
/// caller that will evaluate several populations against the same contract.
pub fn prepare_anchored_evaluator(
    config: &AnchoredObjectiveConfig,
) -> Result<PreparedAnchoredEvaluator, EvaluateError> {
    prepare_anchored_objective(config, PathBuf::new())
}

impl PreparedAnchoredEvaluator {
    pub fn identity(&self) -> &AnchoredObjectiveIdentity {
        &self.identity
    }

    /// Materialize the exact immutable prefix plus a validated candidate
    /// suffix without exposing mutable prepared-evaluator internals.
    pub fn realize_suffix(&self, suffix: InputTape) -> Result<InputTape, EvaluateError> {
        Ok(concatenate(vec![
            ChainSegment::all(self.prefix.clone()),
            ChainSegment::all(suffix),
        ])
        .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?
        .tape)
    }
}

fn prepare_anchored_objective(
    config: &AnchoredObjectiveConfig,
    runtime_program: PathBuf,
) -> Result<PreparedAnchoredEvaluator, EvaluateError> {
    if !is_anchored_profile(config.segment) {
        return Err(EvaluateError::InvalidConfig(format!(
            "anchored objective requires a supported suffix segment, got {}",
            config.segment.as_str()
        )));
    }
    if config.source_milestone.is_empty()
        || config.goal_milestone.is_empty()
        || config.source_milestone == config.goal_milestone
    {
        return Err(EvaluateError::InvalidConfig(
            "anchored source and goal milestone names must be nonempty and distinct".into(),
        ));
    }
    validate_lower_hex(
        &config.source_boundary_fingerprint,
        32,
        "source boundary fingerprint",
    )?;
    let prefix_bytes = fs::read(&config.prefix_tape).map_err(|error| {
        EvaluateError::InvalidConfig(format!(
            "cannot read anchored prefix {}: {error}",
            config.prefix_tape.display()
        ))
    })?;
    let prefix = InputTape::decode(&prefix_bytes)?.tape;
    if prefix.frames.is_empty() {
        return Err(EvaluateError::InvalidConfig(
            "anchored prefix tape must contain at least one frame".into(),
        ));
    }
    // A one-segment chain applies the same absolute/non-reactive validation as
    // the later prefix+suffix composition.
    concatenate(vec![ChainSegment::all(prefix.clone())])
        .map_err(|error| EvaluateError::InvalidConfig(error.to_string()))?;

    let program_bytes = fs::read(&config.milestone_program).map_err(|error| {
        EvaluateError::InvalidConfig(format!(
            "cannot read authored DMSP {}: {error}",
            config.milestone_program.display()
        ))
    })?;
    let decoded = crate::milestone_dsl::decode(&program_bytes)
        .map_err(|error| EvaluateError::InvalidConfig(error.to_string()))?;
    let definition = |name: &str| -> Result<AuthoredDefinitionExpectation, EvaluateError> {
        let index = decoded
            .program
            .definitions
            .iter()
            .position(|definition| definition.name == name)
            .ok_or_else(|| {
                EvaluateError::InvalidConfig(format!(
                    "authored DMSP does not define milestone {name:?}"
                ))
            })?;
        let ast = &decoded.program.definitions[index];
        let identity = &decoded.definitions[index];
        Ok(AuthoredDefinitionExpectation {
            name: name.into(),
            phase: match ast.phase {
                crate::milestone_dsl::EvaluationPhase::PreInput => "pre_input",
                crate::milestone_dsl::EvaluationPhase::PostSim => "post_sim",
            }
            .into(),
            stable_ticks: ast.stable_ticks,
            digest: hex_bytes(&identity.sha256),
            projections: ast
                .projections
                .iter()
                .map(|projection| {
                    crate::milestone_dsl::value_projection_identity(projection)
                        .map(|identity| (projection.name.clone(), hex_bytes(&identity)))
                        .map_err(|error| EvaluateError::InvalidConfig(error.to_string()))
                })
                .collect::<Result<_, _>>()?,
        })
    };
    let source = definition(&config.source_milestone)?;
    let goal = definition(&config.goal_milestone)?;
    let source_index = decoded
        .program
        .definitions
        .iter()
        .position(|definition| definition.name == config.source_milestone)
        .expect("source definition was resolved above");
    let goal_index = decoded
        .program
        .definitions
        .iter()
        .position(|definition| definition.name == config.goal_milestone)
        .expect("goal definition was resolved above");
    if source_index >= goal_index {
        return Err(EvaluateError::InvalidConfig(
            "anchored milestone programs must author source, then optional progress milestones, then goal"
                .into(),
        ));
    }
    let progress = decoded.program.definitions[source_index + 1..goal_index]
        .iter()
        .map(|definition_ast| definition(&definition_ast.name))
        .collect::<Result<Vec<_>, _>>()?;
    let prefix_frames = prefix.frames.len() as u64;
    let source_tape_frame = prefix_frames - 1;
    let source_boundary_index = prefix_frames;
    let prefix_sha256 = hex_bytes(&Sha256::digest(&prefix_bytes));
    let milestone_program_sha256 = hex_bytes(&decoded.program_sha256);
    let game_sha256 = sha256_file(&config.game, "game executable")?;
    let dvd_sha256 = sha256_file(&config.dvd, "DVD image")?;
    let digest_payload = serde_json::to_vec(&serde_json::json!({
        "schema": "dusklight-anchored-search-objective/v2",
        "segment": config.segment,
        "prefix_sha256": prefix_sha256,
        "prefix_frames": prefix_frames,
        "milestone_program_sha256": milestone_program_sha256,
        "game_sha256": game_sha256,
        "dvd_sha256": dvd_sha256,
        "source_milestone": config.source_milestone,
        "source_definition_sha256": source.digest,
        "source_boundary_fingerprint": config.source_boundary_fingerprint,
        "source_tape_frame": source_tape_frame,
        "source_boundary_index": source_boundary_index,
        "goal_milestone": config.goal_milestone,
        "goal_definition_sha256": goal.digest,
    }))?;
    let identity = AnchoredObjectiveIdentity {
        schema: "dusklight-anchored-search-objective/v2".into(),
        segment: config.segment,
        digest: hex_bytes(
            &Sha256::new()
                .chain_update(b"dusklight.anchored-search-objective/v2\0")
                .chain_update(digest_payload)
                .finalize(),
        ),
        prefix_sha256,
        prefix_frames,
        milestone_program_sha256,
        game_sha256,
        dvd_sha256,
        source_milestone: config.source_milestone.clone(),
        source_definition_sha256: source.digest.clone(),
        source_boundary_fingerprint: config.source_boundary_fingerprint.clone(),
        source_tape_frame,
        source_boundary_index,
        goal_milestone: config.goal_milestone.clone(),
        goal_definition_sha256: goal.digest.clone(),
    };
    Ok(PreparedAnchoredEvaluator {
        config: config.clone(),
        identity,
        prefix,
        program_bytes,
        source,
        progress,
        goal,
        runtime_program,
    })
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sha256_file(path: &Path, label: &str) -> Result<String, EvaluateError> {
    let mut file = File::open(path).map_err(|error| {
        EvaluateError::InvalidConfig(format!(
            "cannot read anchored {label} {}: {error}",
            path.display()
        ))
    })?;
    if !file.metadata()?.is_file() {
        return Err(EvaluateError::InvalidConfig(format!(
            "anchored {label} is not a regular file: {}",
            path.display()
        )));
    }
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(hex_bytes(&digest.finalize()))
}

fn validate_lower_hex(value: &str, length: usize, label: &str) -> Result<(), EvaluateError> {
    if value.len() != length
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(EvaluateError::InvalidConfig(format!(
            "{label} must be exactly {length} lowercase hexadecimal characters"
        )));
    }
    Ok(())
}

fn validate_attempt_worker_assignments(
    schedule: &EvaluationWorkerSchedule,
    attempts: &[AttemptEvidence],
) -> Result<(), EvaluateError> {
    schedule
        .validate_completed_claims(attempts.iter().map(|attempt| {
            (
                attempt.candidate_id.as_str(),
                attempt.attempt,
                attempt.worker_id.as_str(),
            )
        }))
        .map_err(|error| EvaluateError::InvalidResult(error.to_string()))
}

pub fn evaluate_population(config: &EvaluateConfig) -> Result<EvaluationReport, EvaluateError> {
    let config = normalize_evaluate_config(config)?;
    validate_evaluate_config(&config)?;
    let population_bytes = fs::read(&config.population_path)?;
    let manifest: PopulationManifest = serde_json::from_slice(&population_bytes)?;
    validate_manifest(&manifest, &config.population_path)?;
    if manifest.segment == SegmentProfile::LinkControlToTunnelCrawlStart {
        return Err(EvaluateError::InvalidConfig(
            "link_control_to_tunnel_crawl_start requires evaluate_anchored_population".into(),
        ));
    }
    let population_root = canonical_parent(&config.population_path)?;
    let trials = build_trials(
        &manifest,
        &population_root,
        &config.output_root,
        config.repetitions,
    )?;
    fs::create_dir_all(&config.output_root)?;
    write_json(
        &config.output_root.join("plan.json"),
        &serde_json::json!({
            "schema": "dusklight-search-evaluation-plan/v4",
            "segment": manifest.segment,
            "boot": manifest.boot,
            "population": config.population_path,
            "game": config.game,
            "dvd": config.dvd,
            "workers": config.workers,
            "repetitions": config.repetitions,
            "timeout_millis": config.timeout.as_millis(),
            "attempts": trials.len(),
            "run_request_template_sha256": config
                .harness
                .as_ref()
                .map(|harness| harness.request_template.content_sha256),
            "execution_boundary": config
                .harness
                .as_ref()
                .map(|_| "dusklight-harness-run-request/v2"),
        }),
    )?;

    let worker_count = config.workers.min(trials.len()).max(1);
    let worker_schedule = EvaluationWorkerSchedule::build(
        worker_count,
        trials
            .iter()
            .map(|trial| (trial.candidate_id.clone(), trial.attempt)),
    )
    .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
    let worker_schedule_sha256 = worker_schedule
        .sha256()
        .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
    let worker_schedule_path = config.output_root.join("worker-schedule.json");
    write_json(&worker_schedule_path, &worker_schedule)?;
    let trials = Arc::new(trials);
    let cancelled = Arc::new(AtomicBool::new(false));
    let outcomes = Arc::new(Mutex::new(Vec::with_capacity(trials.len())));

    thread::scope(|scope| {
        let config = &config;
        let segment = manifest.segment;
        for worker_index in 0..worker_count {
            let worker_schedule = &worker_schedule;
            let trials = Arc::clone(&trials);
            let cancelled = Arc::clone(&cancelled);
            let outcomes = Arc::clone(&outcomes);
            scope.spawn(move || {
                let assignments = worker_schedule
                    .assignments_for_lane(worker_index)
                    .expect("validated schedule contains every launched worker lane");
                for assignment in assignments {
                    if cancelled.load(Ordering::Acquire) {
                        break;
                    }
                    let index = assignment.trial_index;
                    let trial = &trials[index];
                    let mut evidence = run_trial(
                        config,
                        segment,
                        trial,
                        &assignment.worker_id,
                        &cancelled,
                        None,
                    );
                    if let Err(error) = write_json(&trial.root.join("attempt.json"), &evidence) {
                        evidence.infrastructure_error =
                            Some(format!("could not persist attempt evidence: {error}"));
                    }
                    if evidence.infrastructure_error.is_some() {
                        cancelled.store(true, Ordering::Release);
                    }
                    outcomes.lock().unwrap().push(evidence);
                }
            });
        }
    });

    let mut attempts = Arc::try_unwrap(outcomes)
        .expect("evaluation workers still own outcomes")
        .into_inner()
        .unwrap();
    attempts.sort_by(|left, right| {
        left.candidate_id
            .cmp(&right.candidate_id)
            .then(left.attempt.cmp(&right.attempt))
    });
    validate_attempt_worker_assignments(&worker_schedule, &attempts)?;
    let episode_store = config
        .episode_store
        .clone()
        .unwrap_or_else(|| config.output_root.join("content"));
    address_attempt_artifacts(&episode_store, &mut attempts)?;
    let faults = attempts
        .iter()
        .filter(|attempt| attempt.infrastructure_error.is_some())
        .count();
    let episode_ledger = write_episode_ledger(&config.output_root, &attempts)?;
    let report = EvaluationReport {
        schema: EVALUATION_SCHEMA,
        population: config.population_path.clone(),
        results: config.results_path.clone(),
        worker_schedule: worker_schedule_path,
        worker_schedule_sha256,
        segment: manifest.segment,
        boot: manifest.boot.clone(),
        workers: config.workers,
        repetitions: config.repetitions,
        planned_attempts: trials.len(),
        completed_attempts: attempts.len(),
        infrastructure_faults: faults,
        episode_ledger,
        attempts,
        objective: None,
    };
    write_json(&config.output_root.join("evaluation.json"), &report)?;
    if faults != 0 || report.completed_attempts != report.planned_attempts {
        return Err(EvaluateError::Infrastructure {
            faults,
            completed: report.completed_attempts,
            planned: report.planned_attempts,
            evidence: config.output_root.join("evaluation.json"),
        });
    }
    let results = aggregate_results(&manifest, &report.attempts)?;
    if let Some(parent) = config.results_path.parent() {
        fs::create_dir_all(parent)?;
    }
    write_json(&config.results_path, &results)?;
    // Ranking also validates the population/result pairing and all counts.
    rank_population(&manifest, &results)?;
    Ok(report)
}

/// Evaluate a suffix population by prepending the exact immutable prefix and
/// proving both the source boundary and authored goal from a clean process.
pub fn evaluate_anchored_population(
    config: &AnchoredEvaluateConfig,
) -> Result<(EvaluationReport, AnchoredSearchResults), EvaluateError> {
    evaluate_anchored_population_internal(config, None)
}

/// Evaluate a suffix population through an already-authenticated anchored
/// objective. The prepared identity is rechecked against the requested
/// objective before any candidate can be admitted.
pub fn evaluate_prepared_anchored_population(
    config: &AnchoredEvaluateConfig,
    prepared: &PreparedAnchoredEvaluator,
) -> Result<(EvaluationReport, AnchoredSearchResults), EvaluateError> {
    if config.objective != prepared.config {
        return Err(EvaluateError::InvalidConfig(
            "prepared anchored evaluator does not match the requested objective configuration"
                .into(),
        ));
    }
    evaluate_anchored_population_internal(config, Some(prepared))
}

fn evaluate_anchored_population_internal(
    config: &AnchoredEvaluateConfig,
    prepared: Option<&PreparedAnchoredEvaluator>,
) -> Result<(EvaluationReport, AnchoredSearchResults), EvaluateError> {
    let base = normalize_evaluate_config(&config.evaluation)?;
    validate_evaluate_config(&base)?;
    validate_anchored_game_args(&base.game_args_prefix)?;
    validate_anchored_execution_paths(&config.objective, &base.game, &base.dvd)?;
    let manifest: PopulationManifest = serde_json::from_slice(&fs::read(&base.population_path)?)?;
    validate_manifest(&manifest, &base.population_path)?;
    if !is_anchored_profile(manifest.segment) || manifest.segment != config.objective.segment {
        return Err(EvaluateError::InvalidConfig(format!(
            "anchored evaluation segment {} does not match objective {}",
            manifest.segment.as_str(),
            config.objective.segment.as_str()
        )));
    }
    let runtime_program = base.output_root.join("objective.dmsp");
    let objective = if let Some(prepared) = prepared {
        let mut objective = prepared.clone();
        objective.runtime_program = runtime_program.clone();
        objective
    } else {
        prepare_anchored_objective(&config.objective, runtime_program.clone())?
    };
    let population_root = canonical_parent(&base.population_path)?;
    bind_population_objective(&population_root, &objective.identity)?;
    fs::create_dir_all(&base.output_root)?;
    fs::write(&runtime_program, &objective.program_bytes)?;
    let trials = build_anchored_trials(
        &manifest,
        &population_root,
        &base.output_root,
        base.repetitions,
        &objective,
    )?;
    write_json(
        &base.output_root.join("plan.json"),
        &serde_json::json!({
            "schema": "dusklight-search-evaluation-plan/v4",
            "segment": manifest.segment,
            "boot": manifest.boot,
            "objective": objective.identity,
            "population": base.population_path,
            "game": base.game,
            "dvd": base.dvd,
            "workers": base.workers,
            "repetitions": base.repetitions,
            "timeout_millis": base.timeout.as_millis(),
            "attempts": trials.len(),
            "launch_mode": "clean_boot_prefix_plus_suffix",
        }),
    )?;

    let worker_count = base.workers.min(trials.len()).max(1);
    let worker_schedule = EvaluationWorkerSchedule::build(
        worker_count,
        trials
            .iter()
            .map(|trial| (trial.candidate_id.clone(), trial.attempt)),
    )
    .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
    let worker_schedule_sha256 = worker_schedule
        .sha256()
        .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
    let worker_schedule_path = base.output_root.join("worker-schedule.json");
    write_json(&worker_schedule_path, &worker_schedule)?;
    let trials = Arc::new(trials);
    let objective = Arc::new(objective);
    let cancelled = Arc::new(AtomicBool::new(false));
    let outcomes = Arc::new(Mutex::new(Vec::with_capacity(trials.len())));
    let segment = manifest.segment;
    thread::scope(|scope| {
        for worker_index in 0..worker_count {
            let worker_schedule = &worker_schedule;
            let trials = Arc::clone(&trials);
            let objective = Arc::clone(&objective);
            let cancelled = Arc::clone(&cancelled);
            let outcomes = Arc::clone(&outcomes);
            let base = &base;
            scope.spawn(move || {
                let assignments = worker_schedule
                    .assignments_for_lane(worker_index)
                    .expect("validated schedule contains every launched worker lane");
                for assignment in assignments {
                    if cancelled.load(Ordering::Acquire) {
                        break;
                    }
                    let index = assignment.trial_index;
                    let trial = &trials[index];
                    let mut evidence = run_trial(
                        base,
                        segment,
                        trial,
                        &assignment.worker_id,
                        &cancelled,
                        Some(&objective),
                    );
                    if let Err(error) = write_json(&trial.root.join("attempt.json"), &evidence) {
                        evidence.infrastructure_error =
                            Some(format!("could not persist attempt evidence: {error}"));
                    }
                    if evidence.infrastructure_error.is_some() {
                        cancelled.store(true, Ordering::Release);
                    }
                    outcomes.lock().unwrap().push(evidence);
                }
            });
        }
    });
    let mut attempts = Arc::try_unwrap(outcomes)
        .expect("evaluation workers still own outcomes")
        .into_inner()
        .unwrap();
    attempts.sort_by(|left, right| {
        left.candidate_id
            .cmp(&right.candidate_id)
            .then(left.attempt.cmp(&right.attempt))
    });
    validate_attempt_worker_assignments(&worker_schedule, &attempts)?;
    let episode_store = base
        .episode_store
        .clone()
        .unwrap_or_else(|| base.output_root.join("content"));
    address_attempt_artifacts(&episode_store, &mut attempts)?;
    let faults = attempts
        .iter()
        .filter(|attempt| attempt.infrastructure_error.is_some())
        .count();
    let episode_ledger = write_episode_ledger(&base.output_root, &attempts)?;
    let report = EvaluationReport {
        schema: EVALUATION_SCHEMA,
        population: base.population_path.clone(),
        results: base.results_path.clone(),
        worker_schedule: worker_schedule_path,
        worker_schedule_sha256,
        segment: manifest.segment,
        boot: manifest.boot.clone(),
        workers: base.workers,
        repetitions: base.repetitions,
        planned_attempts: trials.len(),
        completed_attempts: attempts.len(),
        infrastructure_faults: faults,
        episode_ledger,
        attempts,
        objective: Some(objective.identity.clone()),
    };
    write_json(&base.output_root.join("evaluation.json"), &report)?;
    if faults != 0 || report.completed_attempts != report.planned_attempts {
        return Err(EvaluateError::Infrastructure {
            faults,
            completed: report.completed_attempts,
            planned: report.planned_attempts,
            evidence: base.output_root.join("evaluation.json"),
        });
    }
    let results = aggregate_results(&manifest, &report.attempts)?;
    rank_population(&manifest, &results)?;
    let anchored_results = AnchoredSearchResults {
        schema: ANCHORED_RESULTS_SCHEMA.into(),
        objective: objective.identity.clone(),
        results,
    };
    write_json(&base.results_path, &anchored_results)?;
    Ok((report, anchored_results))
}

fn bind_population_objective(
    population_root: &Path,
    identity: &AnchoredObjectiveIdentity,
) -> Result<(), EvaluateError> {
    let path = population_root.join("objective.json");
    let bytes = serde_json::to_vec_pretty(identity)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = population_root.join(format!(".objective.{}.{nonce}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    match fs::hard_link(&temporary, &path) {
        Ok(()) => {
            fs::remove_file(&temporary)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            fs::remove_file(&temporary)?;
            let existing: AnchoredObjectiveIdentity = serde_json::from_slice(&fs::read(&path)?)?;
            if existing != *identity {
                return Err(EvaluateError::InvalidManifest(format!(
                    "population objective binding {} does not match requested objective {}",
                    existing.digest, identity.digest
                )));
            }
        }
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            return Err(error.into());
        }
    }
    Ok(())
}

fn validate_anchored_game_args(arguments: &[String]) -> Result<(), EvaluateError> {
    if !arguments.is_empty() {
        return Err(EvaluateError::InvalidConfig(
            "anchored evaluation rejects game_args_prefix so CVars, timing, stage, and proof inputs cannot diverge from its execution contract".into(),
        ));
    }
    Ok(())
}

fn validate_anchored_execution_paths(
    objective: &AnchoredObjectiveConfig,
    game: &Path,
    dvd: &Path,
) -> Result<(), EvaluateError> {
    if fs::canonicalize(&objective.game)? != fs::canonicalize(game)?
        || fs::canonicalize(&objective.dvd)? != fs::canonicalize(dvd)?
    {
        return Err(EvaluateError::InvalidConfig(
            "anchored objective game/DVD paths do not match the launched execution paths".into(),
        ));
    }
    Ok(())
}

mod proposal_readiness;
#[cfg(test)]
use proposal_readiness::{
    learned_holdout_scores_adequate, native_terminals_support_required_facts,
};
use proposal_readiness::{learned_proposal_held_out_performance, required_native_facts_supported};

/// Derive portable novelty evidence from one authenticated native attempt.
pub fn attempt_semantic_novelty_descriptor(
    evidence: &AttemptEvidence,
) -> Result<Option<SemanticNoveltyDescriptor>, EvaluateError> {
    semantic_novelty_descriptor(evidence)
}

/// Derive archive context from authenticated attempt evidence and descriptor.
pub fn attempt_behavior_context(
    evidence: &AttemptEvidence,
    descriptor: &SemanticNoveltyDescriptor,
) -> BehaviorContext {
    archive_behavior_context(evidence, descriptor)
}

/// Admit learned proposals only when required native facts were observable.
pub fn attempts_support_required_native_facts(attempts: &[AttemptEvidence]) -> bool {
    required_native_facts_supported(attempts)
}

/// Evaluate the held-out learned-vs-baseline ordering without owning a search
/// generation or proposal schedule.
pub fn learned_proposals_pass_holdout(
    manifest: &PopulationManifest,
    leaderboard: &[LeaderboardEntry],
) -> bool {
    learned_proposal_held_out_performance(manifest, leaderboard)
}

mod native_result;
use native_result::*;

mod trial;
use trial::*;
fn aggregate_results(
    manifest: &PopulationManifest,
    attempts: &[AttemptEvidence],
) -> Result<SearchResults, EvaluateError> {
    let mut candidates = BTreeMap::new();
    for member in &manifest.members {
        let samples: Vec<_> = attempts
            .iter()
            .filter(|attempt| attempt.candidate_id == member.candidate_id)
            .collect();
        if samples.is_empty()
            || samples
                .iter()
                .any(|sample| sample.infrastructure_error.is_some())
        {
            return Err(EvaluateError::InvalidResult(format!(
                "candidate {} does not have a complete valid sample set",
                member.candidate_id
            )));
        }
        let reference = samples[0];
        if samples.iter().skip(1).any(|sample| {
            sample.milestone_depth != reference.milestone_depth
                || sample.deepest_milestone != reference.deepest_milestone
                || sample.first_hit_tick != reference.first_hit_tick
                || sample.goal_reached != reference.goal_reached
                || sample.milestone_observations != reference.milestone_observations
                || sample.boundary_fingerprints != reference.boundary_fingerprints
                || sample.value_projections != reference.value_projections
        }) {
            return Err(EvaluateError::InvalidResult(format!(
                "candidate {} produced nondeterministic milestone evidence across identical trials",
                member.candidate_id
            )));
        }
        let depth = reference.milestone_depth;
        let ticks = if depth == 0 {
            Vec::new()
        } else {
            vec![
                reference.first_hit_tick.ok_or_else(|| {
                    EvaluateError::InvalidResult(format!(
                        "candidate {} reached depth {depth} without a score tick",
                        member.candidate_id
                    ))
                })?;
                samples.len()
            ]
        };
        candidates.insert(
            member.candidate_id.clone(),
            CandidateResult {
                goal_reached: Some(reference.goal_reached),
                milestone_depth: depth,
                attempts: samples.len() as u32,
                successes: if depth == 0 { 0 } else { samples.len() as u32 },
                first_hit_ticks: ticks,
                risk_events: None,
                boundary_compatibility: crate::search::BoundaryCompatibility::Unknown,
            },
        );
    }
    Ok(SearchResults {
        schema: RESULTS_SCHEMA.into(),
        segment: manifest.segment,
        boot: manifest.boot.clone(),
        candidates,
    })
}

fn validate_evaluate_config(config: &EvaluateConfig) -> Result<(), EvaluateError> {
    if config.workers == 0 || config.repetitions == 0 || config.timeout.is_zero() {
        return Err(EvaluateError::InvalidConfig(
            "workers, repetitions, and timeout must be greater than zero".into(),
        ));
    }
    if !config.game.is_file() {
        return Err(EvaluateError::InvalidConfig(format!(
            "--game is not a file: {}",
            config.game.display()
        )));
    }
    if !config.dvd.is_file() {
        return Err(EvaluateError::InvalidConfig(format!(
            "--dvd is not a file: {}",
            config.dvd.display()
        )));
    }
    if !config.working_directory.is_dir() {
        return Err(EvaluateError::InvalidConfig(format!(
            "working directory does not exist: {}",
            config.working_directory.display()
        )));
    }
    if let Some(harness) = &config.harness {
        harness
            .request_template
            .validate_files(&harness.repository_root)
            .map_err(|error| {
                EvaluateError::InvalidConfig(format!(
                    "invalid authenticated run-request template: {error}"
                ))
            })?;
        let expected_game = fs::canonicalize(
            harness
                .repository_root
                .join(&harness.request_template.executable.path),
        )?;
        let expected_dvd = fs::canonicalize(
            harness
                .repository_root
                .join(&harness.request_template.game_data.path),
        )?;
        let expected_timeout =
            Duration::from_secs(u64::from(harness.request_template.host_timeout_seconds));
        if config.game != expected_game
            || config.dvd != expected_dvd
            || config.working_directory != harness.repository_root
            || config.timeout != expected_timeout
            || !config.game_args_prefix.is_empty()
        {
            return Err(EvaluateError::InvalidConfig(
                "authenticated evaluation must derive executable, game data, working directory, host timeout, and game arguments exclusively from its run request"
                    .into(),
            ));
        }
        if !config.population_path.starts_with(&harness.repository_root)
            || !config.output_root.starts_with(&harness.repository_root)
        {
            return Err(EvaluateError::InvalidConfig(
                "authenticated evaluation population and output must be beneath the repository root"
                    .into(),
            ));
        }
    }
    if directory_is_nonempty(&config.output_root)? {
        return Err(EvaluateError::InvalidConfig(format!(
            "output root must be new or empty: {}",
            config.output_root.display()
        )));
    }
    Ok(())
}

fn normalize_evaluate_config(config: &EvaluateConfig) -> Result<EvaluateConfig, EvaluateError> {
    let absolute = |path: &Path| -> Result<PathBuf, EvaluateError> {
        if path.is_absolute() {
            Ok(path.to_path_buf())
        } else {
            Ok(std::env::current_dir()?.join(path))
        }
    };
    let harness = config
        .harness
        .as_ref()
        .map(|harness| -> Result<HarnessEvaluateConfig, EvaluateError> {
            Ok(HarnessEvaluateConfig {
                repository_root: fs::canonicalize(&harness.repository_root)?,
                request_template: harness.request_template.clone(),
            })
        })
        .transpose()?;
    let output_root = absolute(&config.output_root)?;
    let output_root = if harness.is_some() && !output_root.exists() {
        let parent = output_root.parent().ok_or_else(|| {
            EvaluateError::InvalidConfig("search output has no parent directory".into())
        })?;
        let name = output_root.file_name().ok_or_else(|| {
            EvaluateError::InvalidConfig("search output has no final component".into())
        })?;
        fs::canonicalize(parent)?.join(name)
    } else if harness.is_some() {
        fs::canonicalize(&output_root)?
    } else {
        output_root
    };
    Ok(EvaluateConfig {
        population_path: fs::canonicalize(&config.population_path)?,
        game: fs::canonicalize(&config.game)?,
        dvd: fs::canonicalize(&config.dvd)?,
        output_root,
        episode_store: config.episode_store.as_deref().map(absolute).transpose()?,
        results_path: absolute(&config.results_path)?,
        working_directory: fs::canonicalize(&config.working_directory)?,
        game_args_prefix: config.game_args_prefix.clone(),
        workers: config.workers,
        repetitions: config.repetitions,
        timeout: config.timeout,
        harness,
    })
}

fn directory_is_nonempty(path: &Path) -> Result<bool, EvaluateError> {
    Ok(path.exists() && fs::read_dir(path)?.next().is_some())
}

fn validate_manifest(manifest: &PopulationManifest, path: &Path) -> Result<(), EvaluateError> {
    if manifest.schema != POPULATION_SCHEMA || manifest.members.is_empty() {
        return Err(EvaluateError::InvalidManifest(format!(
            "invalid population manifest {}",
            path.display()
        )));
    }
    let mut ids = HashSet::new();
    if manifest
        .members
        .iter()
        .any(|member| !ids.insert(&member.candidate_id))
    {
        return Err(EvaluateError::InvalidManifest(
            "population contains duplicate candidate IDs".into(),
        ));
    }
    Ok(())
}

fn canonical_parent(path: &Path) -> Result<PathBuf, EvaluateError> {
    let parent = path
        .parent()
        .ok_or_else(|| EvaluateError::InvalidManifest("manifest has no parent".into()))?;
    Ok(fs::canonicalize(parent)?)
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), EvaluateError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

#[cfg(windows)]
fn hide_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_window(_: &mut Command) {}

#[derive(Debug)]
pub enum EvaluateError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Search(crate::search::SearchError),
    Tape(crate::tape::TapeError),
    InvalidConfig(String),
    InvalidManifest(String),
    InvalidResult(String),
    NativeResult(String),
    Launch(std::io::Error),
    Timeout(Duration),
    Cancelled,
    Infrastructure {
        faults: usize,
        completed: usize,
        planned: usize,
        evidence: PathBuf,
    },
    EmptyLeaderboard,
}

impl fmt::Display for EvaluateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "search evaluator I/O failed: {error}"),
            Self::Json(error) => write!(formatter, "invalid search evaluator JSON: {error}"),
            Self::Search(error) => error.fmt(formatter),
            Self::Tape(error) => error.fmt(formatter),
            Self::InvalidConfig(message) => {
                write!(formatter, "invalid evaluator config: {message}")
            }
            Self::InvalidManifest(message) => write!(formatter, "invalid population: {message}"),
            Self::InvalidResult(message) => write!(formatter, "invalid search result: {message}"),
            Self::NativeResult(message) => {
                write!(formatter, "invalid native milestone result: {message}")
            }
            Self::Launch(error) => write!(formatter, "could not launch Dusklight: {error}"),
            Self::Timeout(duration) => write!(
                formatter,
                "Dusklight timed out after {} ms",
                duration.as_millis()
            ),
            Self::Cancelled => {
                formatter.write_str("trial cancelled after another infrastructure fault")
            }
            Self::Infrastructure {
                faults,
                completed,
                planned,
                evidence,
            } => write!(
                formatter,
                "population evaluation failed: {faults} infrastructure fault(s), {completed}/{planned} attempts completed; evidence: {}",
                evidence.display()
            ),
            Self::EmptyLeaderboard => formatter.write_str("search produced an empty leaderboard"),
        }
    }
}

impl Error for EvaluateError {}

impl From<std::io::Error> for EvaluateError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<serde_json::Error> for EvaluateError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}
impl From<crate::search::SearchError> for EvaluateError {
    fn from(value: crate::search::SearchError) -> Self {
        Self::Search(value)
    }
}
impl From<crate::tape::TapeError> for EvaluateError {
    fn from(value: crate::tape::TapeError) -> Self {
        Self::Tape(value)
    }
}
impl From<crate::continuous_search::ContinuousSearchError> for EvaluateError {
    fn from(value: crate::continuous_search::ContinuousSearchError) -> Self {
        Self::InvalidConfig(value.to_string())
    }
}
impl From<crate::bayesian_search::BayesianError> for EvaluateError {
    fn from(value: crate::bayesian_search::BayesianError) -> Self {
        Self::InvalidConfig(value.to_string())
    }
}

#[cfg(test)]
#[path = "search_evaluator/tests.rs"]
mod minimize_tests;
