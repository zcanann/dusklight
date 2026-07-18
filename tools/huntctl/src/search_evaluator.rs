//! Native, cross-platform population evaluation and multi-generation search.

use crate::artifact::Digest as ArtifactDigest;
use crate::bayesian_search::{
    BayesianConfig, BayesianObservation, BayesianOptimizer, BayesianProposal, BayesianSnapshot,
};
use crate::behavior_archive::{BehaviorArchive, BehaviorContext, describe_behavior_with_context};
use crate::content_store::{ContentBlob, ContentKind, ContentStore};
use crate::continuous_search::{
    ContinuousAxes, ContinuousMethod, ContinuousOptimizer, ContinuousOptimizerConfig,
    ContinuousOptimizerSnapshot, ContinuousSample, ContinuousTemplate,
};
use crate::dataset::{DATASET_SOURCE_SCHEMA_V1, DatasetSourceDescriptor};
use crate::episode::{
    EPISODE_CONTEXT_SCHEMA_V1, EpisodeContext, EpisodeIntervention, EpisodeLedger, EpisodeLineage,
    EpisodeManifest, EpisodeManifestBuild, EpisodeObjectiveIdentity, EpisodeOutcome,
    EpisodeOutcomeClass, EpisodeProducerIdentity, EpisodeProducerKind, EpisodeSeed,
    RunBuildIdentity,
};
use crate::harness::execution::execute_request;
use crate::harness::objective_suite::{ArtifactReference, ObjectiveSeed};
use crate::harness::run_contract::{HarnessRunRequest, HarnessRunResult, HarnessTerminalReason};
use crate::learning::evaluation_isolation::{EvaluationAttemptInput, EvaluationGenerationSeal};
use crate::learning::online_lineage::{OnlineDatasetGeneration, OnlineModelLineage};
use crate::learning::planning_priors::{QBeamPriorTable, option_catalog_sha256};
use crate::offline_rl::{ExploratoryExtractConfig, extract_exploratory_from_bytes};
use crate::q_search::{QEpisode, QProposalConfig, propose_q_candidates_with_lineage};
use crate::search::{
    Ancestry, Candidate, CandidateResult, EvolutionConfig, InterventionRange, LexicographicScore,
    MacroAction, POPULATION_SCHEMA, PopulationManifest, RESULTS_SCHEMA, SearchResults,
    SegmentProfile, evolve_population, evolve_population_with_retained_and_proposals,
    rank_population, tape_input_complexity, write_explicit_population, write_seed_population,
};
use crate::semantic_novelty::catalog::{
    SemanticNoveltyAssessment, SemanticNoveltyCatalog, SemanticNoveltyCatalogConfig,
};
use crate::semantic_novelty::proposal_signal::{
    SemanticNoveltyProposalSignal, SemanticNoveltyProposalSignalConfig,
};
use crate::semantic_novelty::{BoundaryFingerprintFact, SemanticNoveltyDescriptor};
use crate::tape::{InputTape, RawPadState, TapeBoot};
use crate::tape_chain::{ChainSegment, concatenate};
use crate::transition_corpus::{StateReference, StateReferenceKind, TransitionCorpus};
use crate::transition_evidence::{
    ImmutableEpisodeArtifact, ImmutableEpisodeBuild, TerminalReasonEvidence,
    TransitionEvidenceBuild, TransitionEvidenceBundle,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::error::Error;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const EVALUATION_SCHEMA: &str = "dusklight-search-evaluation/v5";
pub const ATTEMPT_SCHEMA: &str = "dusklight-search-attempt/v5";
pub const SEARCH_RUN_SCHEMA: &str = "dusklight-search-run/v2";
pub const ANCHORED_RESULTS_SCHEMA: &str = "dusklight-anchored-search-results/v2";
pub const ANCHORED_RUN_SCHEMA: &str = "dusklight-anchored-search-run/v2";
const NATIVE_GOAL_MISS_EXIT_CODE: i32 = 2;

fn is_anchored_profile(profile: SegmentProfile) -> bool {
    matches!(
        profile,
        SegmentProfile::Fsp103ToFsp104 | SegmentProfile::LinkControlToTunnelCrawlStart
    )
}

/// Immutable proof inputs for a clean-boot suffix search. The prefix is an
/// absolute compact tape, compiled DMSP, game executable, and DVD image;
/// callers may materialize the route inputs through any management UX.
#[derive(Clone, Debug)]
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

#[derive(Clone, Debug)]
pub struct AnchoredSearchRunConfig {
    pub search: SearchRunConfig,
    pub objective: AnchoredObjectiveConfig,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AnchoredObjectiveIdentity {
    pub schema: String,
    pub segment: SegmentProfile,
    pub digest: String,
    pub prefix_sha256: String,
    pub prefix_frames: u64,
    pub milestone_program_sha256: String,
    pub game_sha256: String,
    pub dvd_sha256: String,
    pub source_milestone: String,
    pub source_definition_sha256: String,
    pub source_boundary_fingerprint: String,
    pub source_tape_frame: u64,
    pub source_boundary_index: u64,
    pub goal_milestone: String,
    pub goal_definition_sha256: String,
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
    pub results_path: PathBuf,
    pub working_directory: PathBuf,
    pub game_args_prefix: Vec<String>,
    pub workers: usize,
    pub repetitions: u32,
    pub timeout: Duration,
    /// Authenticated request template used to turn each candidate into an
    /// ordinary core-harness run. Legacy callers may omit this while they are
    /// migrated, but new evaluator entry points should provide it.
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

#[derive(Clone, Debug)]
pub struct SearchRunConfig {
    pub segment: SegmentProfile,
    pub seed_candidate: Option<Candidate>,
    pub game: PathBuf,
    pub dvd: PathBuf,
    pub output_root: PathBuf,
    pub working_directory: PathBuf,
    pub game_args_prefix: Vec<String>,
    pub generations: u32,
    pub population_size: usize,
    pub elite_count: usize,
    pub workers: usize,
    pub repetitions: u32,
    pub timeout: Duration,
    pub rng_seed: u64,
    pub harness: Option<HarnessEvaluateConfig>,
}

#[derive(Clone, Debug)]
pub struct BeamSearchConfig {
    pub segment: SegmentProfile,
    pub seed_candidate: Candidate,
    pub options: Vec<MacroAction>,
    pub q_priors: Option<QBeamPriorTable>,
    pub game: PathBuf,
    pub dvd: PathBuf,
    pub output_root: PathBuf,
    pub working_directory: PathBuf,
    pub game_args_prefix: Vec<String>,
    pub beam_width: usize,
    pub maximum_depth: u32,
    pub candidate_budget: usize,
    pub workers: usize,
    pub repetitions: u32,
    pub timeout: Duration,
}

#[derive(Clone, Debug, Serialize)]
pub struct BeamSearchSummary {
    pub schema: &'static str,
    pub segment: SegmentProfile,
    pub beam_width: usize,
    pub maximum_depth: u32,
    pub candidate_budget: usize,
    pub evaluated_candidates: usize,
    pub simulator_episodes: usize,
    pub duplicate_proposals: usize,
    pub beam_pruned_prefixes: usize,
    pub terminal_bound_pruned_children: usize,
    pub q_prior_table_sha256: Option<ArtifactDigest>,
    pub q_prior_model_sha256: Option<ArtifactDigest>,
    pub q_prior_ranked_children: usize,
    pub q_prior_role: &'static str,
    pub native_rollout_ranking_authority: bool,
    pub policy_owns_route: bool,
    pub depths_evaluated: u32,
    pub champion_id: String,
    pub champion_score: LexicographicScore,
    pub champion_candidate: PathBuf,
    pub champion_tape: PathBuf,
    pub output_root: PathBuf,
}

#[derive(Clone, Debug)]
pub struct ContinuousSearchRunConfig {
    pub method: ContinuousMethod,
    pub seed_candidate: Candidate,
    pub axes: ContinuousAxes,
    pub game: PathBuf,
    pub dvd: PathBuf,
    pub output_root: PathBuf,
    pub working_directory: PathBuf,
    pub game_args_prefix: Vec<String>,
    pub generations: u32,
    pub population_size: usize,
    pub elite_count: usize,
    pub initial_sigma: f64,
    pub candidate_budget: usize,
    pub rng_seed: u64,
    pub workers: usize,
    pub repetitions: u32,
    pub timeout: Duration,
}

#[derive(Clone, Debug, Serialize)]
pub struct ContinuousSearchRunSummary {
    pub schema: &'static str,
    pub method: ContinuousMethod,
    pub segment: SegmentProfile,
    pub generations_requested: u32,
    pub generations_completed: u32,
    pub population_size: usize,
    pub elite_count: usize,
    pub candidate_budget: usize,
    pub evaluated_candidates: usize,
    pub simulator_episodes: usize,
    pub duplicate_proposals: usize,
    pub invalid_proposals: usize,
    pub rng_seed: u64,
    pub final_optimizer: ContinuousOptimizerSnapshot,
    pub champion_id: String,
    pub champion_score: LexicographicScore,
    pub champion_values: Vec<f64>,
    pub champion_candidate: PathBuf,
    pub champion_tape: PathBuf,
    pub output_root: PathBuf,
}

#[derive(Clone, Debug)]
pub struct BayesianSearchRunConfig {
    pub seed_candidate: Candidate,
    pub axes: ContinuousAxes,
    pub game: PathBuf,
    pub dvd: PathBuf,
    pub output_root: PathBuf,
    pub working_directory: PathBuf,
    pub game_args_prefix: Vec<String>,
    pub generations: u32,
    pub batch_size: usize,
    pub initial_samples: usize,
    pub acquisition_pool: usize,
    pub length_scale: f64,
    pub observation_noise: f64,
    pub exploration: f64,
    pub candidate_budget: usize,
    pub rng_seed: u64,
    pub workers: usize,
    pub repetitions: u32,
    pub timeout: Duration,
}

#[derive(Clone, Debug, Serialize)]
pub struct BayesianSearchRunSummary {
    pub schema: &'static str,
    pub segment: SegmentProfile,
    pub generations_requested: u32,
    pub generations_completed: u32,
    pub batch_size: usize,
    pub candidate_budget: usize,
    pub evaluated_candidates: usize,
    pub simulator_episodes: usize,
    pub duplicate_proposals: usize,
    pub invalid_proposals: usize,
    pub rng_seed: u64,
    pub final_optimizer: BayesianSnapshot,
    pub champion_id: String,
    pub champion_score: LexicographicScore,
    pub champion_values: Vec<f64>,
    pub champion_candidate: PathBuf,
    pub champion_tape: PathBuf,
    pub output_root: PathBuf,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TournamentBudgetUnit {
    Episodes,
    CandidateTicks,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TournamentProposerKind {
    IncumbentMutation,
    BlindExploration,
    Structured,
    Learned,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TournamentProposer {
    pub name: String,
    pub kind: TournamentProposerKind,
    pub population: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TournamentDefinition {
    pub schema: String,
    pub budget_unit: TournamentBudgetUnit,
    pub budget_per_proposer: u64,
    pub proposers: Vec<TournamentProposer>,
}

#[derive(Clone, Debug)]
pub struct ProposerTournamentConfig {
    pub definition: TournamentDefinition,
    pub definition_directory: PathBuf,
    pub game: PathBuf,
    pub dvd: PathBuf,
    pub output_root: PathBuf,
    pub working_directory: PathBuf,
    pub game_args_prefix: Vec<String>,
    pub workers: usize,
    pub repetitions: u32,
    pub timeout: Duration,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProposerTournamentRow {
    pub name: String,
    pub kind: TournamentProposerKind,
    pub selected_candidates: usize,
    pub charged_episodes: u64,
    pub charged_candidate_ticks: u64,
    pub observed_simulator_ticks: u64,
    pub shared_duplicate_proposals: usize,
    pub improvements_over_incumbent: usize,
    pub misses: usize,
    pub crashes: usize,
    pub predicate_hits: usize,
    pub predicate_hit_rate: f64,
    pub frame_wins: usize,
    pub boundary_diversity: usize,
    pub cold_replay_pass_rate: f64,
    pub best_candidate_id: String,
    pub best_score: LexicographicScore,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProposerTournamentSummary {
    pub schema: &'static str,
    pub segment: SegmentProfile,
    pub boot: TapeBoot,
    pub budget_unit: TournamentBudgetUnit,
    pub budget_per_proposer: u64,
    pub repetitions: u32,
    pub physical_candidates: usize,
    pub physical_episodes: usize,
    pub physical_candidate_ticks: u64,
    pub physical_simulator_ticks: u64,
    pub evaluation_wall_millis: u128,
    pub incumbent_score: LexicographicScore,
    pub rows: Vec<ProposerTournamentRow>,
    pub champion_id: String,
    pub champion_score: LexicographicScore,
    pub output_root: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
pub struct EvaluationReport {
    pub schema: &'static str,
    pub population: PathBuf,
    pub results: PathBuf,
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
pub struct CandidateSemanticNoveltyAssessment {
    pub candidate_id: String,
    pub assessment: SemanticNoveltyAssessment,
    pub proposal_signal: SemanticNoveltyProposalSignal,
}

#[derive(Clone, Debug, Serialize)]
pub struct SemanticNoveltyGenerationReport {
    pub schema: &'static str,
    pub generation: u32,
    pub baseline_observed_episodes: u64,
    pub candidates: Vec<CandidateSemanticNoveltyAssessment>,
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
pub struct BoundaryFingerprint {
    pub schema: String,
    pub algorithm: String,
    pub canonical_encoding: String,
    pub digest: String,
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

#[derive(Clone, Debug, Serialize)]
pub struct SearchRunSummary {
    pub schema: &'static str,
    pub segment: SegmentProfile,
    pub generations: u32,
    pub population_size: usize,
    pub repetitions: u32,
    pub rng_seed: u64,
    pub champion_id: String,
    pub champion_candidate: PathBuf,
    pub champion_tape: PathBuf,
    pub score: crate::search::LexicographicScore,
    pub output_root: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
pub struct AnchoredSearchRunSummary {
    pub schema: &'static str,
    pub segment: SegmentProfile,
    pub objective: AnchoredObjectiveIdentity,
    pub generations: u32,
    pub population_size: usize,
    pub repetitions: u32,
    pub rng_seed: u64,
    pub champion_id: String,
    pub champion_candidate: PathBuf,
    pub champion_suffix_tape: PathBuf,
    pub champion_tape: PathBuf,
    pub score: crate::search::LexicographicScore,
    pub output_root: PathBuf,
}

#[derive(Clone, Debug)]
pub struct BootMinimizeConfig {
    pub candidate: Candidate,
    pub game: PathBuf,
    pub dvd: PathBuf,
    pub output_root: PathBuf,
    pub working_directory: PathBuf,
    pub game_args_prefix: Vec<String>,
    pub workers: usize,
    pub repetitions: u32,
    pub timeout: Duration,
}

#[derive(Clone, Debug, Serialize)]
pub struct BootMinimizeSummary {
    pub schema: &'static str,
    pub source_candidate_id: String,
    pub minimized_candidate_id: String,
    pub source_frames: u64,
    pub minimized_frames: u64,
    pub source_pulse_frames: usize,
    pub minimized_pulse_frames: usize,
    pub goal_sim_tick: u64,
    pub goal_tape_frame: u64,
    pub goal_boundary_fingerprint: String,
    pub candidate: PathBuf,
    pub tape: PathBuf,
    pub proof: PathBuf,
    pub output_root: PathBuf,
}

#[derive(Clone, Debug)]
pub struct BootGolfConfig {
    pub candidate: Candidate,
    pub game: PathBuf,
    pub dvd: PathBuf,
    pub output_root: PathBuf,
    pub working_directory: PathBuf,
    pub game_args_prefix: Vec<String>,
    pub workers: usize,
    pub repetitions: u32,
    pub timeout: Duration,
}

#[derive(Clone, Debug, Serialize)]
pub struct BootGolfSummary {
    pub schema: &'static str,
    pub source_candidate_id: String,
    pub golfed_candidate_id: String,
    pub source_goal_sim_tick: u64,
    pub goal_sim_tick: u64,
    pub goal_tape_frame: u64,
    pub goal_boundary_fingerprint: String,
    pub source_pulse_timestamps: Vec<u64>,
    pub golfed_pulse_timestamps: Vec<u64>,
    pub accepted_moves: u32,
    pub evaluated_candidates: usize,
    pub candidate: PathBuf,
    pub tape: PathBuf,
    pub proof: PathBuf,
    pub output_root: PathBuf,
}

#[derive(Clone, Debug)]
struct AuthoredDefinitionExpectation {
    phase: String,
    stable_ticks: u16,
    digest: String,
    projections: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
struct PreparedAnchoredObjective {
    identity: AnchoredObjectiveIdentity,
    prefix: InputTape,
    program_bytes: Vec<u8>,
    source: AuthoredDefinitionExpectation,
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

fn prepare_anchored_objective(
    config: &AnchoredObjectiveConfig,
    runtime_program: PathBuf,
) -> Result<PreparedAnchoredObjective, EvaluateError> {
    if !is_anchored_profile(config.segment) {
        return Err(EvaluateError::InvalidConfig(format!(
            "anchored objective requires a movement segment, got {}",
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
    Ok(PreparedAnchoredObjective {
        identity,
        prefix,
        program_bytes,
        source,
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

    let trials = Arc::new(trials);
    let next = Arc::new(AtomicUsize::new(0));
    let cancelled = Arc::new(AtomicBool::new(false));
    let outcomes = Arc::new(Mutex::new(Vec::with_capacity(trials.len())));
    let worker_count = config.workers.min(trials.len()).max(1);

    thread::scope(|scope| {
        let config = &config;
        let segment = manifest.segment;
        for worker_index in 0..worker_count {
            let trials = Arc::clone(&trials);
            let next = Arc::clone(&next);
            let cancelled = Arc::clone(&cancelled);
            let outcomes = Arc::clone(&outcomes);
            scope.spawn(move || {
                loop {
                    if cancelled.load(Ordering::Acquire) {
                        break;
                    }
                    let index = next.fetch_add(1, Ordering::AcqRel);
                    let Some(trial) = trials.get(index) else {
                        break;
                    };
                    let mut evidence = run_trial(
                        config,
                        segment,
                        trial,
                        &format!("evaluation/worker-{worker_index}"),
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
    address_attempt_artifacts(&config.output_root, &mut attempts)?;
    let faults = attempts
        .iter()
        .filter(|attempt| attempt.infrastructure_error.is_some())
        .count();
    let episode_ledger = write_episode_ledger(&config.output_root, &attempts)?;
    let report = EvaluationReport {
        schema: EVALUATION_SCHEMA,
        population: config.population_path.clone(),
        results: config.results_path.clone(),
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

fn evaluate_anchored_population_internal(
    config: &AnchoredEvaluateConfig,
    prepared: Option<&PreparedAnchoredObjective>,
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

    let trials = Arc::new(trials);
    let objective = Arc::new(objective);
    let next = Arc::new(AtomicUsize::new(0));
    let cancelled = Arc::new(AtomicBool::new(false));
    let outcomes = Arc::new(Mutex::new(Vec::with_capacity(trials.len())));
    let worker_count = base.workers.min(trials.len()).max(1);
    let segment = manifest.segment;
    thread::scope(|scope| {
        for worker_index in 0..worker_count {
            let trials = Arc::clone(&trials);
            let objective = Arc::clone(&objective);
            let next = Arc::clone(&next);
            let cancelled = Arc::clone(&cancelled);
            let outcomes = Arc::clone(&outcomes);
            let base = &base;
            scope.spawn(move || {
                loop {
                    if cancelled.load(Ordering::Acquire) {
                        break;
                    }
                    let index = next.fetch_add(1, Ordering::AcqRel);
                    let Some(trial) = trials.get(index) else {
                        break;
                    };
                    let mut evidence = run_trial(
                        base,
                        segment,
                        trial,
                        &format!("evaluation/worker-{worker_index}"),
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
    address_attempt_artifacts(&base.output_root, &mut attempts)?;
    let faults = attempts
        .iter()
        .filter(|attempt| attempt.infrastructure_error.is_some())
        .count();
    let episode_ledger = write_episode_ledger(&base.output_root, &attempts)?;
    let report = EvaluationReport {
        schema: EVALUATION_SCHEMA,
        population: base.population_path.clone(),
        results: base.results_path.clone(),
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

pub fn run_search(config: &SearchRunConfig) -> Result<SearchRunSummary, EvaluateError> {
    if config.generations == 0
        || config.population_size == 0
        || config.elite_count == 0
        || config.elite_count > config.population_size
    {
        return Err(EvaluateError::InvalidConfig(
            "generations, population size, and elites must be valid and nonzero".into(),
        ));
    }
    if !config.game.is_file()
        || !config.dvd.is_file()
        || !config.working_directory.is_dir()
        || directory_is_nonempty(&config.output_root)?
    {
        return Err(EvaluateError::InvalidConfig(
            "game, DVD, working directory, and a new/empty output root are required".into(),
        ));
    }
    fs::create_dir_all(&config.output_root)?;
    let seed_candidate = config
        .seed_candidate
        .clone()
        .unwrap_or_else(|| Candidate::baseline(config.segment));
    if seed_candidate.segment != config.segment {
        return Err(EvaluateError::InvalidConfig(
            "seed candidate segment does not match the search segment".into(),
        ));
    }
    seed_candidate.validate()?;
    let mut population_root = config.output_root.join("g000");
    let mut manifest = write_seed_population(
        &population_root,
        seed_candidate,
        config.population_size,
        config.rng_seed,
    )?;
    let mut final_results = None;
    for generation in 0..config.generations {
        let manifest_path = population_root.join("manifest.json");
        let results_path = population_root.join("results.json");
        evaluate_population(&EvaluateConfig {
            population_path: manifest_path.clone(),
            game: config.game.clone(),
            dvd: config.dvd.clone(),
            output_root: population_root.join("evaluations"),
            results_path: results_path.clone(),
            working_directory: config.working_directory.clone(),
            game_args_prefix: config.game_args_prefix.clone(),
            workers: config.workers,
            repetitions: config.repetitions,
            timeout: config.timeout,
            harness: config.harness.clone(),
        })?;
        let results: SearchResults = serde_json::from_slice(&fs::read(&results_path)?)?;
        let leaderboard = rank_population(&manifest, &results)?;
        write_json(&population_root.join("leaderboard.json"), &leaderboard)?;
        final_results = Some(results);
        if generation + 1 < config.generations {
            let next_root = config.output_root.join(format!("g{:03}", generation + 1));
            manifest = evolve_population(
                &manifest_path,
                final_results.as_ref().unwrap(),
                &next_root,
                EvolutionConfig {
                    population_size: config.population_size,
                    elite_count: config.elite_count,
                    rng_seed: config.rng_seed + u64::from(generation) + 1,
                },
            )?;
            population_root = next_root;
        }
    }
    let results = final_results.expect("nonzero generations");
    let leaderboard = rank_population(&manifest, &results)?;
    let champion = leaderboard.first().ok_or(EvaluateError::EmptyLeaderboard)?;
    let member = manifest
        .members
        .iter()
        .find(|member| member.candidate_id == champion.candidate_id)
        .ok_or(EvaluateError::EmptyLeaderboard)?;
    let source = population_root.join(&member.tape_file);
    let champion_tape = config.output_root.join("champion.tape");
    fs::copy(source, &champion_tape)?;
    let champion_candidate = config.output_root.join("champion.candidate.json");
    fs::copy(
        population_root.join(&member.candidate_file),
        &champion_candidate,
    )?;
    let summary = SearchRunSummary {
        schema: SEARCH_RUN_SCHEMA,
        segment: config.segment,
        generations: config.generations,
        population_size: config.population_size,
        repetitions: config.repetitions,
        rng_seed: config.rng_seed,
        champion_id: champion.candidate_id.clone(),
        champion_candidate,
        champion_tape,
        score: champion.score,
        output_root: config.output_root.clone(),
    };
    write_json(&config.output_root.join("run.summary.json"), &summary)?;
    Ok(summary)
}

/// Beam search over a finite discrete option catalog. Every score comes from
/// the ordinary native evaluator. Branch-and-bound prunes only descendants of
/// a prefix whose terminal goal was already proved: appending inputs after its
/// first hit cannot improve that hit and can only make the tape larger.
pub fn run_beam_search(config: &BeamSearchConfig) -> Result<BeamSearchSummary, EvaluateError> {
    if config.seed_candidate.segment != config.segment
        || config.options.is_empty()
        || config.options.len() > 128
        || config.beam_width == 0
        || config.beam_width > 256
        || config.maximum_depth == 0
        || config.maximum_depth > 32
        || config.candidate_budget == 0
        || config.candidate_budget > 100_000
        || config.workers == 0
        || config.repetitions < 2
        || config.timeout.is_zero()
        || !config.game.is_file()
        || !config.dvd.is_file()
        || !config.working_directory.is_dir()
        || directory_is_nonempty(&config.output_root)?
    {
        return Err(EvaluateError::InvalidConfig(
            "beam search requires a matching seed, 1..=128 options, bounded positive beam/depth/budget, native inputs, at least two repetitions, and a new output root"
                .into(),
        ));
    }
    config.seed_candidate.validate()?;
    let option_catalog_sha256 = option_catalog_sha256(&config.options)
        .map_err(|error| EvaluateError::InvalidConfig(error.to_string()))?;
    if let Some(priors) = &config.q_priors {
        priors
            .validate_for_catalog(option_catalog_sha256, config.options.len())
            .map_err(|error| EvaluateError::InvalidConfig(error.to_string()))?;
    }
    fs::create_dir_all(&config.output_root)?;
    let mut seen = HashSet::new();
    seen.insert(config.seed_candidate.id()?);
    let mut batch = vec![config.seed_candidate.clone()];
    let mut evaluated = 0_usize;
    let mut duplicate_proposals = 0_usize;
    let mut beam_pruned_prefixes = 0_usize;
    let mut terminal_bound_pruned_children = 0_usize;
    let mut q_prior_ranked_children = 0_usize;
    let mut depths_evaluated = 0_u32;
    let mut champion: Option<(LexicographicScore, String, Candidate)> = None;

    for depth in 0..=config.maximum_depth {
        if batch.is_empty() || evaluated >= config.candidate_budget {
            break;
        }
        let remaining = config.candidate_budget - evaluated;
        batch.truncate(remaining);
        let depth_root = config.output_root.join(format!("d{depth:03}"));
        let manifest =
            write_explicit_population(&depth_root, config.segment, depth, batch.clone())?;
        let manifest_path = depth_root.join("manifest.json");
        let results_path = depth_root.join("results.json");
        evaluate_population(&EvaluateConfig {
            population_path: manifest_path,
            game: config.game.clone(),
            dvd: config.dvd.clone(),
            output_root: depth_root.join("evaluations"),
            results_path: results_path.clone(),
            working_directory: config.working_directory.clone(),
            game_args_prefix: config.game_args_prefix.clone(),
            workers: config.workers,
            repetitions: config.repetitions,
            timeout: config.timeout,
            harness: None,
        })?;
        let results: SearchResults = serde_json::from_slice(&fs::read(&results_path)?)?;
        let leaderboard = rank_population(&manifest, &results)?;
        write_json(&depth_root.join("leaderboard.json"), &leaderboard)?;
        evaluated += batch.len();
        depths_evaluated += 1;
        let candidates = batch
            .drain(..)
            .map(|candidate| Ok((candidate.id()?, candidate)))
            .collect::<Result<BTreeMap<_, _>, EvaluateError>>()?;
        for row in &leaderboard {
            let candidate = candidates
                .get(&row.candidate_id)
                .ok_or(EvaluateError::EmptyLeaderboard)?;
            if champion.as_ref().is_none_or(|(score, prior_id, _)| {
                row.score > *score || (row.score == *score && row.candidate_id < *prior_id)
            }) {
                champion = Some((row.score, row.candidate_id.clone(), candidate.clone()));
            }
        }
        if depth == config.maximum_depth || evaluated >= config.candidate_budget {
            break;
        }

        let mut frontier = Vec::new();
        for row in &leaderboard {
            let result = results
                .candidates
                .get(&row.candidate_id)
                .ok_or(EvaluateError::EmptyLeaderboard)?;
            if result.goal_reached == Some(true) {
                terminal_bound_pruned_children =
                    terminal_bound_pruned_children.saturating_add(config.options.len());
                continue;
            }
            if frontier.len() < config.beam_width {
                frontier.push(
                    candidates
                        .get(&row.candidate_id)
                        .ok_or(EvaluateError::EmptyLeaderboard)?
                        .clone(),
                );
            } else {
                beam_pruned_prefixes += 1;
            }
        }
        let mut next = Vec::new();
        'parents: for parent in frontier {
            let parent_id = parent.id()?;
            let parent_frames = parent.frame_count();
            let option_indices = config.q_priors.as_ref().map_or_else(
                || (0..config.options.len()).collect::<Vec<_>>(),
                |priors| priors.ranked_option_indices(&parent_id, config.options.len()),
            );
            for option_index in option_indices {
                if evaluated + next.len() >= config.candidate_budget {
                    break 'parents;
                }
                let option = &config.options[option_index];
                if config
                    .q_priors
                    .as_ref()
                    .is_some_and(|priors| priors.has_prior(&parent_id, option_index))
                {
                    q_prior_ranked_children += 1;
                }
                let mut child = parent.clone();
                child.actions.push(option.clone());
                let child_frames = child.frame_count();
                child.ancestry = Ancestry {
                    generation: depth + 1,
                    parent_id: Some(parent_id.clone()),
                    mutation: Some(format!("beam discrete option {option_index}")),
                    intervention: Some(InterventionRange {
                        start_frame: parent_frames,
                        end_frame_exclusive: child_frames,
                        parent_end_frame_exclusive: parent_frames,
                    }),
                };
                if child.validate().is_err() {
                    continue;
                }
                let id = child.id()?;
                if seen.insert(id) {
                    next.push(child);
                } else {
                    duplicate_proposals += 1;
                }
            }
        }
        batch = next;
    }

    let (champion_score, champion_id, champion) =
        champion.ok_or(EvaluateError::EmptyLeaderboard)?;
    let champion_candidate = config.output_root.join("champion.candidate.json");
    let champion_tape = config.output_root.join("champion.tape");
    fs::write(&champion_candidate, serde_json::to_vec_pretty(&champion)?)?;
    fs::write(&champion_tape, champion.compile()?.encode()?)?;
    let summary = BeamSearchSummary {
        schema: "dusklight-beam-search/v2",
        segment: config.segment,
        beam_width: config.beam_width,
        maximum_depth: config.maximum_depth,
        candidate_budget: config.candidate_budget,
        evaluated_candidates: evaluated,
        simulator_episodes: evaluated.saturating_mul(config.repetitions as usize),
        duplicate_proposals,
        beam_pruned_prefixes,
        terminal_bound_pruned_children,
        q_prior_table_sha256: config.q_priors.as_ref().map(|priors| priors.table_sha256),
        q_prior_model_sha256: config.q_priors.as_ref().map(|priors| priors.model_sha256),
        q_prior_ranked_children,
        q_prior_role: "supported_child_ordering_only",
        native_rollout_ranking_authority: true,
        policy_owns_route: false,
        depths_evaluated,
        champion_id,
        champion_score,
        champion_candidate,
        champion_tape,
        output_root: config.output_root.clone(),
    };
    write_json(&config.output_root.join("beam.summary.json"), &summary)?;
    Ok(summary)
}

/// Runs seeded CEM or full-covariance CMA-ES while keeping native repeated
/// rollout evidence as the only ranking signal.
pub fn run_continuous_search(
    config: &ContinuousSearchRunConfig,
) -> Result<ContinuousSearchRunSummary, EvaluateError> {
    if config.generations == 0
        || config.generations > 1_000
        || config.candidate_budget == 0
        || config.candidate_budget > 100_000
        || config.workers == 0
        || config.repetitions < 2
        || config.timeout.is_zero()
        || !config.game.is_file()
        || !config.dvd.is_file()
        || !config.working_directory.is_dir()
        || directory_is_nonempty(&config.output_root)?
    {
        return Err(EvaluateError::InvalidConfig(
            "continuous search requires bounded generations/budget, native inputs, at least two repetitions, and a new output root"
                .into(),
        ));
    }
    let template = ContinuousTemplate::new(config.seed_candidate.clone(), config.axes.clone())?;
    let mut optimizer = ContinuousOptimizer::new(
        template.clone(),
        ContinuousOptimizerConfig {
            method: config.method,
            population_size: config.population_size,
            elite_count: config.elite_count,
            initial_sigma: config.initial_sigma,
            seed: config.rng_seed,
        },
    )?;
    fs::create_dir_all(&config.output_root)?;
    let seed_id = config.seed_candidate.id()?;
    let seed_tape = config.seed_candidate.compile()?;
    let mut seen = HashSet::new();
    let mut evaluated = 0_usize;
    let mut duplicate_proposals = 0_usize;
    let mut invalid_proposals = 0_usize;
    let mut generations_completed = 0_u32;
    let mut champion: Option<(LexicographicScore, String, Candidate, Vec<f64>)> = None;

    for generation in 0..config.generations {
        if evaluated >= config.candidate_budget {
            break;
        }
        let samples = optimizer.ask()?;
        let mut sample_by_candidate = BTreeMap::<String, ContinuousSample>::new();
        let mut candidates = Vec::new();
        for sample in samples {
            if evaluated + candidates.len() >= config.candidate_budget {
                break;
            }
            let Ok(mut candidate) = template.candidate(&sample.values) else {
                invalid_proposals += 1;
                continue;
            };
            let tape = candidate.compile()?;
            let Some(intervention) = tape_intervention(&seed_tape, &tape) else {
                duplicate_proposals += 1;
                continue;
            };
            candidate.ancestry = Ancestry {
                generation,
                parent_id: Some(seed_id.clone()),
                mutation: Some(format!("{:?} bounded continuous sample", config.method)),
                intervention: Some(intervention),
            };
            let id = candidate.id()?;
            if !seen.insert(id.clone()) {
                duplicate_proposals += 1;
                continue;
            }
            sample_by_candidate.insert(id, sample);
            candidates.push(candidate);
        }
        if candidates.len() < config.elite_count {
            if generations_completed == 0 {
                return Err(EvaluateError::InvalidConfig(format!(
                    "continuous bounds produced only {} unique valid candidates; at least {} are required",
                    candidates.len(),
                    config.elite_count
                )));
            }
            break;
        }
        let generation_root = config.output_root.join(format!("g{generation:03}"));
        let manifest = write_explicit_population(
            &generation_root,
            config.seed_candidate.segment,
            generation,
            candidates,
        )?;
        let results_path = generation_root.join("results.json");
        evaluate_population(&EvaluateConfig {
            population_path: generation_root.join("manifest.json"),
            game: config.game.clone(),
            dvd: config.dvd.clone(),
            output_root: generation_root.join("evaluations"),
            results_path: results_path.clone(),
            working_directory: config.working_directory.clone(),
            game_args_prefix: config.game_args_prefix.clone(),
            workers: config.workers,
            repetitions: config.repetitions,
            timeout: config.timeout,
            harness: None,
        })?;
        let results: SearchResults = serde_json::from_slice(&fs::read(results_path)?)?;
        let leaderboard = rank_population(&manifest, &results)?;
        write_json(&generation_root.join("leaderboard.json"), &leaderboard)?;
        let ranked_samples = leaderboard
            .iter()
            .map(|row| {
                sample_by_candidate
                    .get(&row.candidate_id)
                    .cloned()
                    .ok_or(EvaluateError::EmptyLeaderboard)
            })
            .collect::<Result<Vec<_>, _>>()?;
        for row in &leaderboard {
            let sample = sample_by_candidate
                .get(&row.candidate_id)
                .ok_or(EvaluateError::EmptyLeaderboard)?;
            let member = manifest
                .members
                .iter()
                .find(|member| member.candidate_id == row.candidate_id)
                .ok_or(EvaluateError::EmptyLeaderboard)?;
            let candidate: Candidate =
                serde_json::from_slice(&fs::read(generation_root.join(&member.candidate_file))?)?;
            if champion.as_ref().is_none_or(|(score, id, _, _)| {
                row.score > *score || (row.score == *score && row.candidate_id < *id)
            }) {
                champion = Some((
                    row.score,
                    row.candidate_id.clone(),
                    candidate,
                    sample.values.clone(),
                ));
            }
        }
        optimizer.tell(&ranked_samples)?;
        write_json(
            &generation_root.join("optimizer.json"),
            &serde_json::json!({
                "schema": "dusklight-continuous-generation/v1",
                "method": config.method,
                "axes": config.axes,
                "ranked_samples": ranked_samples,
                "next_state": optimizer.snapshot(),
            }),
        )?;
        evaluated += manifest.members.len();
        generations_completed += 1;
    }

    let (champion_score, champion_id, champion, champion_values) =
        champion.ok_or(EvaluateError::EmptyLeaderboard)?;
    let champion_candidate = config.output_root.join("champion.candidate.json");
    let champion_tape = config.output_root.join("champion.tape");
    fs::write(&champion_candidate, serde_json::to_vec_pretty(&champion)?)?;
    fs::write(&champion_tape, champion.compile()?.encode()?)?;
    let summary = ContinuousSearchRunSummary {
        schema: "dusklight-continuous-search/v1",
        method: config.method,
        segment: config.seed_candidate.segment,
        generations_requested: config.generations,
        generations_completed,
        population_size: config.population_size,
        elite_count: config.elite_count,
        candidate_budget: config.candidate_budget,
        evaluated_candidates: evaluated,
        simulator_episodes: evaluated.saturating_mul(config.repetitions as usize),
        duplicate_proposals,
        invalid_proposals,
        rng_seed: config.rng_seed,
        final_optimizer: optimizer.snapshot(),
        champion_id,
        champion_score,
        champion_values,
        champion_candidate,
        champion_tape,
        output_root: config.output_root.clone(),
    };
    write_json(
        &config.output_root.join("continuous.summary.json"),
        &summary,
    )?;
    Ok(summary)
}

/// Bounded Gaussian-process expected-improvement search. The surrogate models
/// empirical native rank utility only; final ordering and proof remain native.
pub fn run_bayesian_search(
    config: &BayesianSearchRunConfig,
) -> Result<BayesianSearchRunSummary, EvaluateError> {
    if config.generations == 0
        || config.generations > 1_000
        || config.batch_size == 0
        || config.batch_size > 512
        || config.candidate_budget == 0
        || config.candidate_budget > 100_000
        || config.workers == 0
        || config.repetitions < 2
        || config.timeout.is_zero()
        || !config.game.is_file()
        || !config.dvd.is_file()
        || !config.working_directory.is_dir()
        || directory_is_nonempty(&config.output_root)?
    {
        return Err(EvaluateError::InvalidConfig(
            "Bayesian search requires bounded batches/generations/budget, native inputs, at least two repetitions, and a new output root"
                .into(),
        ));
    }
    let template = ContinuousTemplate::new(config.seed_candidate.clone(), config.axes.clone())?;
    let mut optimizer = BayesianOptimizer::new(BayesianConfig {
        dimensions: template.dimensions(),
        initial_samples: config.initial_samples,
        acquisition_pool: config.acquisition_pool,
        length_scale: config.length_scale,
        observation_noise: config.observation_noise,
        exploration: config.exploration,
        seed: config.rng_seed,
    })?;
    fs::create_dir_all(&config.output_root)?;
    let seed_id = config.seed_candidate.id()?;
    let seed_tape = config.seed_candidate.compile()?;
    let mut seen = HashSet::new();
    let mut evaluated = 0_usize;
    let mut duplicate_proposals = 0_usize;
    let mut invalid_proposals = 0_usize;
    let mut generations_completed = 0_u32;
    let mut champion: Option<(LexicographicScore, String, Candidate, Vec<f64>)> = None;

    for generation in 0..config.generations {
        if evaluated >= config.candidate_budget {
            break;
        }
        let request = config
            .batch_size
            .min(config.candidate_budget.saturating_sub(evaluated));
        let proposals = optimizer.ask(request)?;
        let mut proposal_by_candidate = BTreeMap::<String, (BayesianProposal, Vec<f64>)>::new();
        let mut candidates = Vec::new();
        for proposal in proposals {
            let values = template.values_from_normalized(&proposal.normalized)?;
            let Ok(mut candidate) = template.candidate(&values) else {
                invalid_proposals += 1;
                continue;
            };
            let tape = candidate.compile()?;
            let Some(intervention) = tape_intervention(&seed_tape, &tape) else {
                duplicate_proposals += 1;
                continue;
            };
            candidate.ancestry = Ancestry {
                generation,
                parent_id: Some(seed_id.clone()),
                mutation: Some("Gaussian-process expected-improvement proposal".into()),
                intervention: Some(intervention),
            };
            let id = candidate.id()?;
            if !seen.insert(id.clone()) {
                duplicate_proposals += 1;
                continue;
            }
            proposal_by_candidate.insert(id, (proposal, values));
            candidates.push(candidate);
        }
        if candidates.is_empty() {
            if generations_completed == 0 {
                return Err(EvaluateError::InvalidConfig(
                    "Bayesian bounds produced no unique valid candidates".into(),
                ));
            }
            break;
        }
        let generation_root = config.output_root.join(format!("g{generation:03}"));
        let manifest = write_explicit_population(
            &generation_root,
            config.seed_candidate.segment,
            generation,
            candidates,
        )?;
        let results_path = generation_root.join("results.json");
        evaluate_population(&EvaluateConfig {
            population_path: generation_root.join("manifest.json"),
            game: config.game.clone(),
            dvd: config.dvd.clone(),
            output_root: generation_root.join("evaluations"),
            results_path: results_path.clone(),
            working_directory: config.working_directory.clone(),
            game_args_prefix: config.game_args_prefix.clone(),
            workers: config.workers,
            repetitions: config.repetitions,
            timeout: config.timeout,
            harness: None,
        })?;
        let results: SearchResults = serde_json::from_slice(&fs::read(results_path)?)?;
        let leaderboard = rank_population(&manifest, &results)?;
        write_json(&generation_root.join("leaderboard.json"), &leaderboard)?;
        let denominator = leaderboard.len().saturating_sub(1).max(1) as f64;
        let mut observations = Vec::with_capacity(leaderboard.len());
        for (rank, row) in leaderboard.iter().enumerate() {
            let (proposal, values) = proposal_by_candidate
                .get(&row.candidate_id)
                .ok_or(EvaluateError::EmptyLeaderboard)?;
            let utility = if leaderboard.len() == 1 {
                1.0
            } else {
                (leaderboard.len() - rank - 1) as f64 / denominator
            };
            observations.push(BayesianObservation {
                normalized: proposal.normalized.clone(),
                rank_utility: utility,
            });
            let member = manifest
                .members
                .iter()
                .find(|member| member.candidate_id == row.candidate_id)
                .ok_or(EvaluateError::EmptyLeaderboard)?;
            let candidate: Candidate =
                serde_json::from_slice(&fs::read(generation_root.join(&member.candidate_file))?)?;
            if champion.as_ref().is_none_or(|(score, id, _, _)| {
                row.score > *score || (row.score == *score && row.candidate_id < *id)
            }) {
                champion = Some((
                    row.score,
                    row.candidate_id.clone(),
                    candidate,
                    values.clone(),
                ));
            }
        }
        optimizer.tell(observations.clone())?;
        write_json(
            &generation_root.join("optimizer.json"),
            &serde_json::json!({
                "schema": "dusklight-bayesian-generation/v1",
                "axes": config.axes,
                "proposals": proposal_by_candidate,
                "rank_observations": observations,
                "next_state": optimizer.snapshot(),
            }),
        )?;
        evaluated += manifest.members.len();
        generations_completed += 1;
    }

    let (champion_score, champion_id, champion, champion_values) =
        champion.ok_or(EvaluateError::EmptyLeaderboard)?;
    let champion_candidate = config.output_root.join("champion.candidate.json");
    let champion_tape = config.output_root.join("champion.tape");
    fs::write(&champion_candidate, serde_json::to_vec_pretty(&champion)?)?;
    fs::write(&champion_tape, champion.compile()?.encode()?)?;
    let summary = BayesianSearchRunSummary {
        schema: "dusklight-bayesian-search/v1",
        segment: config.seed_candidate.segment,
        generations_requested: config.generations,
        generations_completed,
        batch_size: config.batch_size,
        candidate_budget: config.candidate_budget,
        evaluated_candidates: evaluated,
        simulator_episodes: evaluated.saturating_mul(config.repetitions as usize),
        duplicate_proposals,
        invalid_proposals,
        rng_seed: config.rng_seed,
        final_optimizer: optimizer.snapshot(),
        champion_id,
        champion_score,
        champion_values,
        champion_candidate,
        champion_tape,
        output_root: config.output_root.clone(),
    };
    write_json(&config.output_root.join("bayesian.summary.json"), &summary)?;
    Ok(summary)
}

/// Evaluate several proposer populations under the same declared cap. Every
/// selected candidate enters one deduplicated native population, so no proposer
/// can bypass the evaluator or spend simulator time twice on a shared tape.
pub fn run_proposer_tournament(
    config: &ProposerTournamentConfig,
) -> Result<ProposerTournamentSummary, EvaluateError> {
    let definition = &config.definition;
    if definition.schema != "dusklight-proposer-tournament-definition/v1"
        || definition.budget_per_proposer == 0
        || definition.proposers.len() < 2
        || definition.proposers.len() > 16
        || config.workers == 0
        || config.repetitions < 2
        || config.timeout.is_zero()
        || !config.definition_directory.is_dir()
        || !config.game.is_file()
        || !config.dvd.is_file()
        || !config.working_directory.is_dir()
        || directory_is_nonempty(&config.output_root)?
    {
        return Err(EvaluateError::InvalidConfig(
            "tournaments require a bounded v1 definition, 2..=16 proposers, native inputs, at least two repetitions, and a new output root"
                .into(),
        ));
    }
    if !definition
        .proposers
        .iter()
        .any(|proposer| proposer.kind == TournamentProposerKind::IncumbentMutation)
        || !definition
            .proposers
            .iter()
            .any(|proposer| proposer.kind == TournamentProposerKind::BlindExploration)
    {
        return Err(EvaluateError::InvalidConfig(
            "a fair tournament must retain incumbent_mutation and blind_exploration proposers"
                .into(),
        ));
    }
    let episode_slots = match definition.budget_unit {
        TournamentBudgetUnit::Episodes => {
            let repetitions = u64::from(config.repetitions);
            if definition.budget_per_proposer % repetitions != 0 {
                return Err(EvaluateError::InvalidConfig(
                    "episode budget must be an exact multiple of repetitions".into(),
                ));
            }
            Some(
                usize::try_from(definition.budget_per_proposer / repetitions).map_err(|_| {
                    EvaluateError::InvalidConfig("episode budget is too large".into())
                })?,
            )
        }
        TournamentBudgetUnit::CandidateTicks => None,
    };
    if episode_slots == Some(0) {
        return Err(EvaluateError::InvalidConfig(
            "episode budget cannot select zero candidates".into(),
        ));
    }

    struct SelectedProposer {
        name: String,
        kind: TournamentProposerKind,
        candidate_ids: Vec<String>,
        candidate_ticks: u64,
    }

    let mut names = HashSet::new();
    let mut segment = None;
    let mut boot = None;
    let mut selected = Vec::new();
    let mut union = BTreeMap::<String, Candidate>::new();
    for proposer in &definition.proposers {
        if proposer.name.is_empty()
            || proposer.name.len() > 64
            || !proposer
                .name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            || !names.insert(proposer.name.clone())
        {
            return Err(EvaluateError::InvalidConfig(
                "proposer names must be unique 1..=64 byte identifiers".into(),
            ));
        }
        let population_path = if proposer.population.is_absolute() {
            proposer.population.clone()
        } else {
            config.definition_directory.join(&proposer.population)
        };
        let population_path = fs::canonicalize(population_path)?;
        let manifest: PopulationManifest = serde_json::from_slice(&fs::read(&population_path)?)?;
        validate_manifest(&manifest, &population_path)?;
        if segment.is_some_and(|value| value != manifest.segment)
            || boot.as_ref().is_some_and(|value| *value != manifest.boot)
        {
            return Err(EvaluateError::InvalidManifest(
                "tournament populations must share one segment and boot origin".into(),
            ));
        }
        segment = Some(manifest.segment);
        boot.get_or_insert_with(|| manifest.boot.clone());
        let population_root = canonical_parent(&population_path)?;
        let mut ids = Vec::new();
        let mut candidate_ticks = 0_u64;
        for member in &manifest.members {
            let candidate_path = fs::canonicalize(population_root.join(&member.candidate_file))?;
            if !candidate_path.starts_with(&population_root) {
                return Err(EvaluateError::InvalidManifest(
                    "tournament candidate escapes its population root".into(),
                ));
            }
            let candidate: Candidate = serde_json::from_slice(&fs::read(candidate_path)?)?;
            candidate.validate()?;
            let id = candidate.id()?;
            if id != member.candidate_id
                || candidate.segment != manifest.segment
                || candidate.boot != manifest.boot
                || candidate.frame_count() != member.frame_count
            {
                return Err(EvaluateError::InvalidManifest(format!(
                    "proposer {:?} contains a candidate/manifest identity mismatch",
                    proposer.name
                )));
            }
            let cost = member
                .frame_count
                .checked_mul(u64::from(config.repetitions))
                .ok_or_else(|| {
                    EvaluateError::InvalidConfig("candidate-tick cost overflowed".into())
                })?;
            let accept = match episode_slots {
                Some(slots) => ids.len() < slots,
                None => candidate_ticks
                    .checked_add(cost)
                    .is_some_and(|total| total <= definition.budget_per_proposer),
            };
            if !accept {
                continue;
            }
            candidate_ticks += cost;
            ids.push(id.clone());
            union.entry(id).or_insert(candidate);
            if episode_slots.is_some_and(|slots| ids.len() == slots) {
                break;
            }
        }
        if ids.is_empty() || episode_slots.is_some_and(|slots| ids.len() != slots) {
            return Err(EvaluateError::InvalidConfig(format!(
                "proposer {:?} cannot fill its declared budget with valid candidates",
                proposer.name
            )));
        }
        selected.push(SelectedProposer {
            name: proposer.name.clone(),
            kind: proposer.kind,
            candidate_ids: ids,
            candidate_ticks,
        });
    }
    if union.len() > 10_000 {
        return Err(EvaluateError::InvalidConfig(
            "tournament union exceeds 10,000 physical candidates".into(),
        ));
    }
    let segment = segment.ok_or_else(|| EvaluateError::InvalidConfig("empty tournament".into()))?;
    let boot = boot.ok_or_else(|| EvaluateError::InvalidConfig("empty tournament".into()))?;
    fs::create_dir_all(&config.output_root)?;
    let population_root = config.output_root.join("population");
    let manifest =
        write_explicit_population(&population_root, segment, 0, union.into_values().collect())?;
    if manifest.boot != boot {
        return Err(EvaluateError::InvalidManifest(
            "deduplicated tournament changed the boot origin".into(),
        ));
    }
    let results_path = config.output_root.join("results.json");
    let started = Instant::now();
    let report = evaluate_population(&EvaluateConfig {
        population_path: population_root.join("manifest.json"),
        game: config.game.clone(),
        dvd: config.dvd.clone(),
        output_root: config.output_root.join("evaluations"),
        results_path: results_path.clone(),
        working_directory: config.working_directory.clone(),
        game_args_prefix: config.game_args_prefix.clone(),
        workers: config.workers,
        repetitions: config.repetitions,
        timeout: config.timeout,
        harness: None,
    })?;
    let evaluation_wall_millis = started.elapsed().as_millis();
    let results: SearchResults = serde_json::from_slice(&fs::read(results_path)?)?;
    let leaderboard = rank_population(&manifest, &results)?;
    write_json(&config.output_root.join("leaderboard.json"), &leaderboard)?;
    let scores = leaderboard
        .iter()
        .map(|row| (row.candidate_id.as_str(), row.score))
        .collect::<BTreeMap<_, _>>();
    let incumbent_score = selected
        .iter()
        .filter(|proposer| proposer.kind == TournamentProposerKind::IncumbentMutation)
        .flat_map(|proposer| &proposer.candidate_ids)
        .filter_map(|id| scores.get(id.as_str()).copied())
        .max()
        .ok_or(EvaluateError::EmptyLeaderboard)?;
    let incidence = selected
        .iter()
        .flat_map(|proposer| &proposer.candidate_ids)
        .fold(BTreeMap::<String, usize>::new(), |mut counts, id| {
            *counts.entry(id.clone()).or_default() += 1;
            counts
        });
    let frame_counts = manifest
        .members
        .iter()
        .map(|member| (member.candidate_id.as_str(), member.frame_count))
        .collect::<BTreeMap<_, _>>();
    let mut rows = Vec::new();
    for proposer in selected {
        let best = leaderboard
            .iter()
            .filter(|row| proposer.candidate_ids.contains(&row.candidate_id))
            .max_by(|left, right| left.score.cmp(&right.score))
            .ok_or(EvaluateError::EmptyLeaderboard)?;
        let predicate_hits = proposer
            .candidate_ids
            .iter()
            .filter(|id| results.candidates[*id].goal_reached == Some(true))
            .count();
        let misses = proposer.candidate_ids.len() - predicate_hits;
        let boundaries = report
            .attempts
            .iter()
            .filter(|attempt| proposer.candidate_ids.contains(&attempt.candidate_id))
            .flat_map(|attempt| attempt.boundary_fingerprints.values())
            .map(|fingerprint| fingerprint.digest.as_str())
            .collect::<HashSet<_>>();
        let improvements_over_incumbent = proposer
            .candidate_ids
            .iter()
            .filter(|id| scores[id.as_str()] > incumbent_score)
            .count();
        let frame_wins = proposer
            .candidate_ids
            .iter()
            .map(|id| scores[id.as_str()])
            .filter(|score| {
                score.goal_feasible
                    && incumbent_score.goal_feasible
                    && score.milestone_depth >= incumbent_score.milestone_depth
                    && score.median_first_hit_tick < incumbent_score.median_first_hit_tick
            })
            .count();
        let observed_simulator_ticks = report
            .attempts
            .iter()
            .filter(|attempt| proposer.candidate_ids.contains(&attempt.candidate_id))
            .map(|attempt| {
                attempt
                    .first_hit_tick
                    .map(|tick| tick.saturating_add(1))
                    .unwrap_or(frame_counts[attempt.candidate_id.as_str()])
            })
            .sum();
        rows.push(ProposerTournamentRow {
            name: proposer.name,
            kind: proposer.kind,
            selected_candidates: proposer.candidate_ids.len(),
            charged_episodes: proposer.candidate_ids.len() as u64 * u64::from(config.repetitions),
            charged_candidate_ticks: proposer.candidate_ticks,
            observed_simulator_ticks,
            shared_duplicate_proposals: proposer
                .candidate_ids
                .iter()
                .filter(|id| incidence[id.as_str()] > 1)
                .count(),
            improvements_over_incumbent,
            misses,
            crashes: 0,
            predicate_hits,
            predicate_hit_rate: predicate_hits as f64 / proposer.candidate_ids.len() as f64,
            frame_wins,
            boundary_diversity: boundaries.len(),
            cold_replay_pass_rate: predicate_hits as f64 / proposer.candidate_ids.len() as f64,
            best_candidate_id: best.candidate_id.clone(),
            best_score: best.score,
        });
    }
    rows.sort_by(|left, right| left.name.cmp(&right.name));
    let champion = leaderboard.first().ok_or(EvaluateError::EmptyLeaderboard)?;
    let physical_candidate_ticks = manifest
        .members
        .iter()
        .map(|member| member.frame_count * u64::from(config.repetitions))
        .sum();
    let physical_simulator_ticks = report
        .attempts
        .iter()
        .map(|attempt| {
            attempt
                .first_hit_tick
                .map(|tick| tick.saturating_add(1))
                .unwrap_or(frame_counts[attempt.candidate_id.as_str()])
        })
        .sum();
    let summary = ProposerTournamentSummary {
        schema: "dusklight-proposer-tournament/v1",
        segment,
        boot,
        budget_unit: definition.budget_unit,
        budget_per_proposer: definition.budget_per_proposer,
        repetitions: config.repetitions,
        physical_candidates: manifest.members.len(),
        physical_episodes: manifest.members.len() * config.repetitions as usize,
        physical_candidate_ticks,
        physical_simulator_ticks,
        evaluation_wall_millis,
        incumbent_score,
        rows,
        champion_id: champion.candidate_id.clone(),
        champion_score: champion.score,
        output_root: config.output_root.clone(),
    };
    write_json(
        &config.output_root.join("tournament.summary.json"),
        &summary,
    )?;
    Ok(summary)
}

fn tape_intervention(parent: &InputTape, child: &InputTape) -> Option<InterventionRange> {
    if parent.boot != child.boot
        || parent.tick_rate_numerator != child.tick_rate_numerator
        || parent.tick_rate_denominator != child.tick_rate_denominator
    {
        return None;
    }
    let shared = parent.frames.len().min(child.frames.len());
    let start = (0..shared)
        .find(|index| parent.frames[*index] != child.frames[*index])
        .or_else(|| (parent.frames.len() != child.frames.len()).then_some(shared))?;
    let mut parent_end = parent.frames.len();
    let mut child_end = child.frames.len();
    while parent_end > start
        && child_end > start
        && parent.frames[parent_end - 1] == child.frames[child_end - 1]
    {
        parent_end -= 1;
        child_end -= 1;
    }
    Some(InterventionRange {
        start_frame: start as u64,
        end_frame_exclusive: child_end as u64,
        parent_end_frame_exclusive: parent_end as u64,
    })
}

pub fn run_anchored_search(
    config: &AnchoredSearchRunConfig,
) -> Result<AnchoredSearchRunSummary, EvaluateError> {
    let search = &config.search;
    if !is_anchored_profile(search.segment) {
        return Err(EvaluateError::InvalidConfig(format!(
            "anchored search requires a movement segment, got {}",
            search.segment.as_str()
        )));
    }
    if config.objective.segment != search.segment {
        return Err(EvaluateError::InvalidConfig(
            "anchored search segment does not match its objective".into(),
        ));
    }
    if search.generations == 0
        || search.population_size == 0
        || search.elite_count == 0
        || search.elite_count > search.population_size
        || !search.game.is_file()
        || !search.dvd.is_file()
        || !search.working_directory.is_dir()
        || directory_is_nonempty(&search.output_root)?
    {
        return Err(EvaluateError::InvalidConfig(
            "valid execution paths, population limits, and a new/empty output root are required"
                .into(),
        ));
    }
    let seed = search.seed_candidate.clone().ok_or_else(|| {
        EvaluateError::InvalidConfig(
            "anchored search requires a losslessly imported observed suffix candidate; it has no synthetic baseline"
                .into(),
        )
    })?;
    if seed.segment != search.segment {
        return Err(EvaluateError::InvalidConfig(
            "anchored seed candidate has the wrong segment profile".into(),
        ));
    }
    seed.validate()?;
    validate_anchored_game_args(&search.game_args_prefix)?;
    validate_anchored_execution_paths(&config.objective, &search.game, &search.dvd)?;
    let prepared = prepare_anchored_objective(&config.objective, PathBuf::new())?;
    fs::create_dir_all(&search.output_root)?;
    let mut population_root = search.output_root.join("g000");
    let mut manifest = write_seed_population(
        &population_root,
        seed,
        search.population_size,
        search.rng_seed,
    )?;
    let mut final_results = None;
    let mut training_corpora = BTreeMap::<String, TransitionCorpus>::new();
    let mut previous_dataset_generation: Option<OnlineDatasetGeneration> = None;
    let mut previous_model_lineage: Option<OnlineModelLineage> = None;
    let mut behavior_archive = BehaviorArchive::default();
    let mut semantic_novelty_catalog = SemanticNoveltyCatalog::default();
    for generation in 0..search.generations {
        let manifest_path = population_root.join("manifest.json");
        let results_path = population_root.join("results.json");
        let (report, results) = evaluate_anchored_population_internal(
            &AnchoredEvaluateConfig {
                evaluation: EvaluateConfig {
                    population_path: manifest_path.clone(),
                    game: search.game.clone(),
                    dvd: search.dvd.clone(),
                    output_root: population_root.join("evaluations"),
                    results_path: results_path.clone(),
                    working_directory: search.working_directory.clone(),
                    game_args_prefix: search.game_args_prefix.clone(),
                    workers: search.workers,
                    repetitions: search.repetitions,
                    timeout: search.timeout,
                    harness: search.harness.clone(),
                },
                objective: config.objective.clone(),
            },
            Some(&prepared),
        )?;
        let leaderboard = rank_population(&manifest, &results.results)?;
        write_json(&population_root.join("leaderboard.json"), &leaderboard)?;
        let mut generation_corpora = BTreeMap::new();
        let mut generation_contexts = BTreeMap::new();
        let mut generation_semantics =
            BTreeMap::<String, (u32, SemanticNoveltyDescriptor, BehaviorContext)>::new();
        let mut generation_outcomes = BTreeMap::new();
        let mut evaluation_attempts = Vec::with_capacity(report.attempts.len());
        let mut quarantined_corpora = BTreeMap::<String, TransitionCorpus>::new();
        let mut quarantined_digests = BTreeSet::<ArtifactDigest>::new();
        for attempt in &report.attempts {
            if let Some(descriptor) = semantic_novelty_descriptor(attempt)? {
                let replace = generation_semantics
                    .get(&attempt.candidate_id)
                    .is_none_or(|(selected_attempt, _, _)| attempt.attempt < *selected_attempt);
                if replace {
                    let context = archive_behavior_context(attempt, &descriptor);
                    generation_semantics.insert(
                        attempt.candidate_id.clone(),
                        (attempt.attempt, descriptor, context),
                    );
                }
            }
            let transition_corpus_sha256 = if let Some(path) = attempt.transition_corpus.as_ref() {
                let corpus = TransitionCorpus::read_zstd_file(path)
                    .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
                let digest = corpus
                    .content_digest()
                    .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
                quarantined_digests.insert(digest);
                quarantined_corpora
                    .entry(digest.to_string())
                    .or_insert_with(|| corpus.clone());
                generation_corpora
                    .entry(attempt.candidate_id.clone())
                    .or_insert(corpus);
                generation_outcomes
                    .entry(attempt.candidate_id.clone())
                    .or_insert(attempt.outcome.class);
                Some(digest)
            } else {
                None
            };
            evaluation_attempts.push(EvaluationAttemptInput {
                candidate_id: attempt.candidate_id.clone(),
                attempt: attempt.attempt,
                worker_id: attempt.worker_id.clone(),
                transition_corpus_sha256,
            });
        }
        generation_contexts.extend(
            generation_semantics
                .iter()
                .map(|(candidate_id, (_, _, context))| (candidate_id.clone(), context.clone())),
        );
        let baseline_observed_episodes = semantic_novelty_catalog.observed_episodes();
        let candidates = generation_semantics
            .iter()
            .map(|(candidate_id, (_, descriptor, _))| {
                let assessment = semantic_novelty_catalog
                    .assess(descriptor, SemanticNoveltyCatalogConfig::default())
                    .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
                let proposal_signal = SemanticNoveltyProposalSignal::from_assessment(
                    assessment.clone(),
                    SemanticNoveltyProposalSignalConfig::default(),
                )
                .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
                Ok::<_, EvaluateError>(CandidateSemanticNoveltyAssessment {
                    candidate_id: candidate_id.clone(),
                    assessment,
                    proposal_signal,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let semantic_proposal_scores = candidates
            .iter()
            .map(|candidate| {
                (
                    candidate.candidate_id.clone(),
                    candidate.proposal_signal.proposal_ordering_score(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        write_json(
            &population_root.join("semantic-novelty.json"),
            &SemanticNoveltyGenerationReport {
                schema: "dusklight-semantic-novelty-generation/v1",
                generation,
                baseline_observed_episodes,
                candidates,
            },
        )?;
        for (_, descriptor, _) in generation_semantics.values() {
            semantic_novelty_catalog
                .record(descriptor)
                .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
        }
        write_json(
            &population_root.join("semantic-novelty-catalog.json"),
            &semantic_novelty_catalog.snapshot(),
        )?;
        let evaluation_seal = EvaluationGenerationSeal::build(
            generation,
            report.repetitions,
            report.planned_attempts,
            report.completed_attempts,
            report.infrastructure_faults,
            &evaluation_attempts,
        )
        .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
        write_json(
            &population_root.join("evaluation-generation-seal.json"),
            &evaluation_seal,
        )?;
        if generation + 1 < search.generations {
            evaluation_seal
                .admit_training_generation(generation + 1, &quarantined_digests)
                .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
            for (digest, corpus) in quarantined_corpora {
                training_corpora.entry(digest).or_insert(corpus);
            }
        }
        let member_by_id: BTreeMap<_, _> = manifest
            .members
            .iter()
            .map(|member| (member.candidate_id.as_str(), member))
            .collect();
        let mut evaluated_episodes = BTreeMap::new();
        for row in &leaderboard {
            let Some(corpus) = generation_corpora.get(&row.candidate_id) else {
                continue;
            };
            let member = member_by_id[row.candidate_id.as_str()];
            let candidate: Candidate =
                serde_json::from_slice(&fs::read(population_root.join(&member.candidate_file))?)?;
            let episode = QEpisode {
                candidate,
                corpus: corpus.clone(),
                outcome: generation_outcomes[&row.candidate_id],
            };
            let context = generation_contexts
                .get(&row.candidate_id)
                .cloned()
                .unwrap_or_default();
            behavior_archive
                .consider_with_context(episode.clone(), row.score, generation, &context)
                .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
            let descriptor = describe_behavior_with_context(corpus, &context)
                .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
            evaluated_episodes.insert(row.candidate_id.clone(), (episode, descriptor));
        }
        final_results = Some(results);
        if generation + 1 < search.generations {
            let next_root = search.output_root.join(format!("g{:03}", generation + 1));
            let mut q_episodes = Vec::new();
            let mut elite_ids = HashSet::new();
            let mut elite_descriptors = Vec::new();
            for row in leaderboard.iter().take(search.elite_count) {
                elite_ids.insert(row.candidate_id.clone());
                let Some((episode, descriptor)) = evaluated_episodes.get(&row.candidate_id) else {
                    continue;
                };
                elite_descriptors.push(descriptor.clone());
                q_episodes.push(episode.clone());
            }
            let non_elite_budget = search.population_size - search.elite_count;
            let archive_budget = if non_elite_budget >= 3 {
                (non_elite_budget / 4).max(1)
            } else {
                0
            };
            let archived = behavior_archive
                .select_diverse(&elite_ids, &elite_descriptors, archive_budget)
                .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
            let archive_summary = behavior_archive
                .summary(&archived)
                .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
            write_json(
                &population_root.join("behavior-archive.json"),
                &archive_summary,
            )?;
            let archived_candidates = archived
                .iter()
                .map(|entry| entry.episode.candidate.clone())
                .collect::<Vec<_>>();
            let corpora = training_corpora.values().cloned().collect::<Vec<_>>();
            let dataset_generation = if corpora.is_empty() {
                None
            } else {
                let dataset = OnlineDatasetGeneration::build(
                    previous_dataset_generation.as_ref(),
                    &evaluation_seal,
                    &corpora,
                )
                .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
                write_json(
                    &population_root.join("online-dataset-generation.json"),
                    &dataset,
                )?;
                Some(dataset)
            };
            let mut q_ids = elite_ids;
            for entry in &archived {
                let id = entry.episode.candidate.id()?;
                if q_ids.insert(id) {
                    q_episodes.push(entry.episode.clone());
                }
            }
            q_episodes.sort_by(|left, right| {
                let left_id = left
                    .candidate
                    .id()
                    .expect("proposal source candidates were validated before archiving");
                let right_id = right
                    .candidate
                    .id()
                    .expect("proposal source candidates were validated before archiving");
                semantic_proposal_scores
                    .get(&right_id)
                    .copied()
                    .unwrap_or_default()
                    .cmp(
                        &semantic_proposal_scores
                            .get(&left_id)
                            .copied()
                            .unwrap_or_default(),
                    )
                    .then_with(|| left_id.cmp(&right_id))
            });
            let q_budget = (non_elite_budget - archived_candidates.len()).div_ceil(2);
            let q_result = if q_budget == 0 || q_episodes.is_empty() || dataset_generation.is_none()
            {
                Err(
                    "no non-elite slots, aligned elite episodes, or sealed training generation is available"
                        .to_string(),
                )
            } else {
                propose_q_candidates_with_lineage(
                    &corpora,
                    &q_episodes,
                    QProposalConfig {
                        generation: generation + 1,
                        max_proposals: q_budget,
                        iterations: 12,
                        trees_per_action: 15,
                        seed: search.rng_seed + u64::from(generation) + 1,
                    },
                    dataset_generation.as_ref().expect("checked above"),
                    previous_model_lineage.as_ref(),
                )
                .map_err(|error| error.to_string())
            };
            let q_candidates = match q_result {
                Ok(batch) => {
                    if let Some(lineage) = batch.summary.model_lineage.as_ref() {
                        write_json(&population_root.join("online-model-lineage.json"), lineage)?;
                        previous_model_lineage = Some(lineage.clone());
                    }
                    let candidate_ids = batch
                        .candidates
                        .iter()
                        .map(Candidate::id)
                        .collect::<Result<Vec<_>, _>>()?;
                    write_json(
                        &population_root.join("q-proposals.json"),
                        &serde_json::json!({
                            "status": "ready",
                            "summary": batch.summary,
                            "candidate_ids": candidate_ids,
                        }),
                    )?;
                    batch.candidates
                }
                Err(error) => {
                    write_json(
                        &population_root.join("q-proposals.json"),
                        &serde_json::json!({
                            "status": "unavailable",
                            "error": error,
                            "training_corpora": training_corpora.len(),
                            "aligned_elite_episodes": q_episodes.len(),
                        }),
                    )?;
                    Vec::new()
                }
            };
            if let Some(dataset_generation) = dataset_generation {
                previous_dataset_generation = Some(dataset_generation);
            }
            manifest = evolve_population_with_retained_and_proposals(
                &manifest_path,
                &final_results.as_ref().unwrap().results,
                &next_root,
                EvolutionConfig {
                    population_size: search.population_size,
                    elite_count: search.elite_count,
                    rng_seed: search.rng_seed + u64::from(generation) + 1,
                },
                &archived_candidates,
                &q_candidates,
            )?;
            population_root = next_root;
        }
    }
    let results = final_results.expect("nonzero generations");
    if results.objective != prepared.identity {
        return Err(EvaluateError::InvalidResult(
            "final anchored results changed objective identity".into(),
        ));
    }
    let leaderboard = rank_population(&manifest, &results.results)?;
    let champion = leaderboard.first().ok_or(EvaluateError::EmptyLeaderboard)?;
    let member = manifest
        .members
        .iter()
        .find(|member| member.candidate_id == champion.candidate_id)
        .ok_or(EvaluateError::EmptyLeaderboard)?;
    let source = population_root.join(&member.tape_file);
    let suffix = InputTape::decode(&fs::read(&source)?)?.tape;
    let full = concatenate(vec![
        ChainSegment::all(prepared.prefix),
        ChainSegment::all(suffix),
    ])
    .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?
    .tape;
    let champion_suffix_tape = search.output_root.join("champion.suffix.tape");
    fs::copy(source, &champion_suffix_tape)?;
    let champion_tape = search.output_root.join("champion.tape");
    fs::write(&champion_tape, full.encode()?)?;
    let champion_candidate = search.output_root.join("champion.candidate.json");
    fs::copy(
        population_root.join(&member.candidate_file),
        &champion_candidate,
    )?;
    let summary = AnchoredSearchRunSummary {
        schema: ANCHORED_RUN_SCHEMA,
        segment: search.segment,
        objective: results.objective,
        generations: search.generations,
        population_size: search.population_size,
        repetitions: search.repetitions,
        rng_seed: search.rng_seed,
        champion_id: champion.candidate_id.clone(),
        champion_candidate,
        champion_suffix_tape,
        champion_tape,
        score: champion.score,
        output_root: search.output_root.clone(),
    };
    write_json(&search.output_root.join("run.summary.json"), &summary)?;
    Ok(summary)
}

#[derive(Clone)]
struct ProvenBootCandidate {
    candidate: Candidate,
    tape: InputTape,
    sim_tick: u64,
    tape_frame: u64,
    boundary_fingerprint: BoundaryFingerprint,
}

#[derive(Clone)]
struct BootReductionTarget {
    sim_tick: u64,
    tape_frame: u64,
    boundary_fingerprint: BoundaryFingerprint,
}

impl BootReductionTarget {
    fn accepts(&self, candidate: &ProvenBootCandidate) -> bool {
        candidate.sim_tick == self.sim_tick
            && candidate.tape_frame == self.tape_frame
            && candidate.boundary_fingerprint == self.boundary_fingerprint
    }
}

pub fn minimize_boot(config: &BootMinimizeConfig) -> Result<BootMinimizeSummary, EvaluateError> {
    if config.candidate.segment != SegmentProfile::BootToFsp103 {
        return Err(EvaluateError::InvalidConfig(
            "boot minimization requires a boot_to_fsp103 candidate".into(),
        ));
    }
    config.candidate.validate()?;
    if !config.game.is_file()
        || !config.dvd.is_file()
        || !config.working_directory.is_dir()
        || config.workers == 0
        || config.repetitions < 2
        || config.timeout.is_zero()
        || directory_is_nonempty(&config.output_root)?
    {
        return Err(EvaluateError::InvalidConfig(
            "game, DVD, working directory, at least two repetitions, positive execution limits, and a new/empty output root are required"
                .into(),
        ));
    }
    fs::create_dir_all(&config.output_root)?;
    let source_id = config.candidate.id()?;
    let source_tape = config.candidate.compile()?;
    let source_frames = config.candidate.frame_count();
    let source_pulses = pulse_frame_count(&source_tape);
    let mut round = 0_u32;
    let initial = evaluate_boot_batch(
        config,
        vec![config.candidate.clone()],
        &config
            .output_root
            .join("rounds")
            .join(format!("{round:04}")),
        round,
    )?
    .into_iter()
    .next()
    .ok_or_else(|| {
        EvaluateError::InvalidResult(
            "the starting candidate did not reach gameplay-ready-f-sp103".into(),
        )
    })?;
    let mut current = initial;
    let target = BootReductionTarget {
        sim_tick: current.sim_tick,
        tape_frame: current.tape_frame,
        boundary_fingerprint: current.boundary_fingerprint.clone(),
    };
    round += 1;

    // First partition the ordered active frames into contiguous chunks. This
    // splits even one dense 800-frame A/Start mash into removable regions. The
    // frames become neutral rather than disappearing, so surviving pulses keep
    // their exact absolute timestamps throughout ddmin.
    let mut granularity = 2_usize;
    loop {
        let pulse_frames: Vec<_> = current
            .tape
            .frames
            .iter()
            .enumerate()
            .filter_map(|(index, frame)| (frame.pads[0].buttons != 0).then_some(index))
            .collect();
        if pulse_frames.is_empty() {
            break;
        }
        let partitions = granularity.min(pulse_frames.len());
        let mut candidates = Vec::with_capacity(partitions);
        for partition in 0..partitions {
            let start = pulse_frames.len() * partition / partitions;
            let end = pulse_frames.len() * (partition + 1) / partitions;
            let ranges = coalesce_pulse_frames(&pulse_frames[start..end]);
            candidates.push(candidate_with_neutralized_ranges(
                &current,
                &ranges,
                round,
                "ddmin pulse chunk",
            )?);
        }
        let mut proven = evaluate_boot_batch(
            config,
            candidates,
            &config
                .output_root
                .join("rounds")
                .join(format!("{round:04}")),
            round,
        )?;
        proven.retain(|candidate| target.accepts(candidate));
        round += 1;
        if let Some(best) = best_boot_candidate(proven) {
            current = best;
            granularity = 2;
        } else if partitions == pulse_frames.len() {
            break;
        } else {
            granularity = (partitions * 2).min(pulse_frames.len());
        }
    }

    // A run can contain several held or mashed button frames. Finish at frame
    // granularity, repeatedly taking the deletion with the fewest remaining
    // pulse frames and then the earliest exact goal tick.
    loop {
        let pulse_frames: Vec<_> = current
            .tape
            .frames
            .iter()
            .enumerate()
            .filter_map(|(index, frame)| (frame.pads[0].buttons != 0).then_some(index))
            .collect();
        if pulse_frames.is_empty() {
            break;
        }
        let candidates = pulse_frames
            .iter()
            .map(|index| {
                candidate_with_neutralized_ranges(
                    &current,
                    &[(*index, *index + 1)],
                    round,
                    "minimize individual pulse",
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut proven = evaluate_boot_batch(
            config,
            candidates,
            &config
                .output_root
                .join("rounds")
                .join(format!("{round:04}")),
            round,
        )?;
        proven.retain(|candidate| target.accepts(candidate));
        round += 1;
        if let Some(best) = best_boot_candidate(proven) {
            current = best;
        } else {
            break;
        }
    }

    let required_frames = usize::try_from(current.tape_frame)
        .map_err(|_| EvaluateError::InvalidResult("goal tape frame is too large".into()))?
        .checked_add(1)
        .ok_or_else(|| EvaluateError::InvalidResult("goal tape frame overflowed".into()))?;
    if required_frames > current.tape.frames.len() {
        return Err(EvaluateError::InvalidResult(
            "goal tape frame lies outside the candidate tape".into(),
        ));
    }
    let mut trimmed_tape = current.tape.clone();
    trimmed_tape.frames.truncate(required_frames);
    let mut trimmed = Candidate::from_absolute_tape(SegmentProfile::BootToFsp103, &trimmed_tape)?;
    trimmed.ancestry = crate::search::Ancestry {
        generation: round,
        parent_id: Some(current.candidate.id()?),
        mutation: Some("trim after exact goal tape frame".into()),
        intervention: Some(crate::search::InterventionRange {
            start_frame: required_frames as u64,
            end_frame_exclusive: required_frames as u64,
            parent_end_frame_exclusive: current.tape.frames.len() as u64,
        }),
    };
    let proof_root = config.output_root.join("proof");
    let (mut proof_candidates, proof_report) =
        evaluate_boot_batch_with_report(config, vec![trimmed], &proof_root, round)?;
    proof_candidates.retain(|candidate| target.accepts(candidate));
    let minimized = proof_candidates.into_iter().next().ok_or_else(|| {
        EvaluateError::InvalidResult(
            "the tape trimmed to goal tape_frame + 1 did not reproduce the exact goal".into(),
        )
    })?;

    let candidate_path = config.output_root.join("minimized.candidate.json");
    let tape_path = config.output_root.join("minimized.tape");
    let proof_path = config.output_root.join("proof.json");
    fs::write(
        &candidate_path,
        serde_json::to_vec_pretty(&minimized.candidate)?,
    )?;
    fs::write(&tape_path, minimized.tape.encode()?)?;
    write_json(&proof_path, &proof_report)?;
    let summary = BootMinimizeSummary {
        schema: "dusklight-boot-minimization/v1",
        source_candidate_id: source_id,
        minimized_candidate_id: minimized.candidate.id()?,
        source_frames,
        minimized_frames: minimized.candidate.frame_count(),
        source_pulse_frames: source_pulses,
        minimized_pulse_frames: pulse_frame_count(&minimized.tape),
        goal_sim_tick: minimized.sim_tick,
        goal_tape_frame: minimized.tape_frame,
        goal_boundary_fingerprint: minimized.boundary_fingerprint.digest.clone(),
        candidate: candidate_path,
        tape: tape_path,
        proof: proof_path,
        output_root: config.output_root.clone(),
    };
    write_json(&config.output_root.join("minimize.summary.json"), &summary)?;
    Ok(summary)
}

/// Systematically moves the existing boot pulse sequence to earlier absolute
/// frames. This is coordinate descent over every legal earlier timestamp, not
/// a stochastic search: a move may be retained without improving the goal tick
/// when its earlier timestamp can expose a coordinated improvement on a later
/// pass.
pub fn golf_boot(config: &BootGolfConfig) -> Result<BootGolfSummary, EvaluateError> {
    if config.candidate.segment != SegmentProfile::BootToFsp103 {
        return Err(EvaluateError::InvalidConfig(
            "boot timing golf requires a boot_to_fsp103 candidate".into(),
        ));
    }
    config.candidate.validate()?;
    if !config.game.is_file()
        || !config.dvd.is_file()
        || !config.working_directory.is_dir()
        || config.workers == 0
        || config.repetitions < 2
        || config.timeout.is_zero()
        || directory_is_nonempty(&config.output_root)?
    {
        return Err(EvaluateError::InvalidConfig(
            "game, DVD, working directory, at least two repetitions, positive execution limits, and a new/empty output root are required"
                .into(),
        ));
    }
    fs::create_dir_all(&config.output_root)?;
    let evaluation = BootMinimizeConfig {
        candidate: config.candidate.clone(),
        game: config.game.clone(),
        dvd: config.dvd.clone(),
        output_root: config.output_root.clone(),
        working_directory: config.working_directory.clone(),
        game_args_prefix: config.game_args_prefix.clone(),
        workers: config.workers,
        repetitions: config.repetitions,
        timeout: config.timeout,
    };
    let source_id = config.candidate.id()?;
    let mut round = 0_u32;
    let mut evaluated_candidates = 1_usize;
    let initial = evaluate_boot_batch(
        &evaluation,
        vec![config.candidate.clone()],
        &config
            .output_root
            .join("rounds")
            .join(format!("{round:04}")),
        round,
    )?
    .into_iter()
    .next()
    .ok_or_else(|| {
        EvaluateError::InvalidResult(
            "the starting candidate did not reach gameplay-ready-f-sp103".into(),
        )
    })?;
    let source_goal_sim_tick = initial.sim_tick;
    let source_fingerprint = initial.boundary_fingerprint.clone();
    let source_pulse_timestamps = pulse_timestamps(&initial.tape)?;
    if source_pulse_timestamps.is_empty() {
        return Err(EvaluateError::InvalidConfig(
            "boot timing golf requires at least one pulse frame".into(),
        ));
    }
    let mut current = initial;
    let mut accepted_moves = 0_u32;
    round += 1;

    loop {
        let timestamps = pulse_timestamps(&current.tape)?;
        let mut candidates = Vec::new();
        // Last-to-first ordering makes the menu/cutscene pulses most likely to
        // occupy the first worker slots while retaining deterministic results.
        for pulse_index in (0..timestamps.len()).rev() {
            let earliest = if pulse_index == 0 {
                0
            } else {
                timestamps[pulse_index - 1]
                    .checked_add(1)
                    .ok_or_else(|| EvaluateError::InvalidResult("pulse frame overflowed".into()))?
            };
            for timestamp in (earliest..timestamps[pulse_index]).rev() {
                candidates.push(candidate_with_shifted_pulse(
                    &current,
                    pulse_index,
                    timestamp,
                    round,
                )?);
            }
        }
        if candidates.is_empty() {
            break;
        }
        evaluated_candidates = evaluated_candidates
            .checked_add(candidates.len())
            .ok_or_else(|| EvaluateError::InvalidResult("candidate count overflowed".into()))?;
        let mut proven = evaluate_boot_batch(
            &evaluation,
            candidates,
            &config
                .output_root
                .join("rounds")
                .join(format!("{round:04}")),
            round,
        )?;
        proven.retain(|candidate| {
            candidate.boundary_fingerprint == source_fingerprint
                && candidate.sim_tick <= current.sim_tick
                && boot_golf_cmp(candidate, &current).is_lt()
        });
        let Some(best) = proven.into_iter().min_by(boot_golf_cmp) else {
            break;
        };
        current = best;
        accepted_moves = accepted_moves
            .checked_add(1)
            .ok_or_else(|| EvaluateError::InvalidResult("accepted move count overflowed".into()))?;
        round += 1;
    }

    let exact_target = BootReductionTarget {
        sim_tick: current.sim_tick,
        tape_frame: current.tape_frame,
        boundary_fingerprint: source_fingerprint.clone(),
    };
    let required_frames = usize::try_from(current.tape_frame)
        .map_err(|_| EvaluateError::InvalidResult("goal tape frame is too large".into()))?
        .checked_add(1)
        .ok_or_else(|| EvaluateError::InvalidResult("goal tape frame overflowed".into()))?;
    if required_frames > current.tape.frames.len() {
        return Err(EvaluateError::InvalidResult(
            "goal tape frame lies outside the candidate tape".into(),
        ));
    }
    let mut trimmed_tape = current.tape.clone();
    trimmed_tape.frames.truncate(required_frames);
    let mut trimmed = Candidate::from_absolute_tape(SegmentProfile::BootToFsp103, &trimmed_tape)?;
    trimmed.ancestry = crate::search::Ancestry {
        generation: round,
        parent_id: Some(current.candidate.id()?),
        mutation: Some("trim after exact goal tape frame".into()),
        intervention: Some(crate::search::InterventionRange {
            start_frame: required_frames as u64,
            end_frame_exclusive: required_frames as u64,
            parent_end_frame_exclusive: current.tape.frames.len() as u64,
        }),
    };
    let proof_root = config.output_root.join("proof");
    let (mut proof_candidates, proof_report) =
        evaluate_boot_batch_with_report(&evaluation, vec![trimmed], &proof_root, round)?;
    evaluated_candidates = evaluated_candidates
        .checked_add(1)
        .ok_or_else(|| EvaluateError::InvalidResult("candidate count overflowed".into()))?;
    proof_candidates.retain(|candidate| exact_target.accepts(candidate));
    let golfed = proof_candidates.into_iter().next().ok_or_else(|| {
        EvaluateError::InvalidResult(
            "the final boot timing candidate did not reproduce its exact proof".into(),
        )
    })?;

    let candidate_path = config.output_root.join("golfed.candidate.json");
    let tape_path = config.output_root.join("golfed.tape");
    let proof_path = config.output_root.join("proof.json");
    fs::write(
        &candidate_path,
        serde_json::to_vec_pretty(&golfed.candidate)?,
    )?;
    fs::write(&tape_path, golfed.tape.encode()?)?;
    write_json(&proof_path, &proof_report)?;
    let summary = BootGolfSummary {
        schema: "dusklight-boot-timing-golf/v1",
        source_candidate_id: source_id,
        golfed_candidate_id: golfed.candidate.id()?,
        source_goal_sim_tick,
        goal_sim_tick: golfed.sim_tick,
        goal_tape_frame: golfed.tape_frame,
        goal_boundary_fingerprint: source_fingerprint.digest,
        source_pulse_timestamps,
        golfed_pulse_timestamps: pulse_timestamps(&golfed.tape)?,
        accepted_moves,
        evaluated_candidates,
        candidate: candidate_path,
        tape: tape_path,
        proof: proof_path,
        output_root: config.output_root.clone(),
    };
    write_json(&config.output_root.join("golf.summary.json"), &summary)?;
    Ok(summary)
}

fn pulse_timestamps(tape: &InputTape) -> Result<Vec<u64>, EvaluateError> {
    tape.frames
        .iter()
        .enumerate()
        .filter(|(_, frame)| frame.pads[0].buttons != 0)
        .map(|(index, _)| {
            u64::try_from(index).map_err(|_| {
                EvaluateError::InvalidResult("pulse timestamp does not fit in u64".into())
            })
        })
        .collect()
}

fn pulse_timestamp_sum(tape: &InputTape) -> Result<u64, EvaluateError> {
    pulse_timestamps(tape)?
        .into_iter()
        .try_fold(0_u64, |sum, timestamp| {
            sum.checked_add(timestamp).ok_or_else(|| {
                EvaluateError::InvalidResult("pulse timestamp sum overflowed".into())
            })
        })
}

fn boot_golf_cmp(left: &ProvenBootCandidate, right: &ProvenBootCandidate) -> std::cmp::Ordering {
    let left_timestamps = pulse_timestamps(&left.tape).expect("validated candidate timestamps");
    let right_timestamps = pulse_timestamps(&right.tape).expect("validated candidate timestamps");
    left.sim_tick
        .cmp(&right.sim_tick)
        .then_with(|| {
            pulse_timestamp_sum(&left.tape)
                .expect("validated candidate timestamp sum")
                .cmp(&pulse_timestamp_sum(&right.tape).expect("validated candidate timestamp sum"))
        })
        .then(left_timestamps.cmp(&right_timestamps))
        .then_with(|| {
            left.candidate
                .id()
                .unwrap()
                .cmp(&right.candidate.id().unwrap())
        })
}

fn candidate_with_shifted_pulse(
    parent: &ProvenBootCandidate,
    pulse_index: usize,
    new_timestamp: u64,
    generation: u32,
) -> Result<Candidate, EvaluateError> {
    let timestamps = pulse_timestamps(&parent.tape)?;
    let old_timestamp = *timestamps.get(pulse_index).ok_or_else(|| {
        EvaluateError::InvalidResult(format!("pulse index {pulse_index} is out of range"))
    })?;
    let new_index = usize::try_from(new_timestamp)
        .map_err(|_| EvaluateError::InvalidResult("new pulse timestamp is too large".into()))?;
    let old_index = usize::try_from(old_timestamp)
        .map_err(|_| EvaluateError::InvalidResult("old pulse timestamp is too large".into()))?;
    if new_timestamp >= old_timestamp
        || parent.tape.frames[new_index].pads[0].buttons != 0
        || (pulse_index > 0 && new_timestamp <= timestamps[pulse_index - 1])
    {
        return Err(EvaluateError::InvalidResult(
            "shifted pulse does not preserve strict input order".into(),
        ));
    }
    let mut tape = parent.tape.clone();
    let pad = tape.frames[old_index].pads[0];
    tape.frames[old_index].pads[0] = RawPadState::default();
    tape.frames[new_index].pads[0] = pad;
    let mut candidate = Candidate::from_absolute_tape(SegmentProfile::BootToFsp103, &tape)?;
    candidate.ancestry = crate::search::Ancestry {
        generation,
        parent_id: Some(parent.candidate.id()?),
        mutation: Some(format!(
            "move pulse {pulse_index} from frame {old_timestamp} to {new_timestamp}"
        )),
        intervention: Some(crate::search::InterventionRange {
            start_frame: old_timestamp.min(new_timestamp),
            end_frame_exclusive: old_timestamp.max(new_timestamp) + 1,
            parent_end_frame_exclusive: old_timestamp.max(new_timestamp) + 1,
        }),
    };
    Ok(candidate)
}

fn pulse_frame_count(tape: &InputTape) -> usize {
    tape.frames
        .iter()
        .filter(|frame| frame.pads[0].buttons != 0)
        .count()
}

fn coalesce_pulse_frames(frames: &[usize]) -> Vec<(usize, usize)> {
    let mut runs = Vec::new();
    for &frame in frames {
        if let Some((_, end)) = runs.last_mut()
            && *end == frame
        {
            *end += 1;
        } else {
            runs.push((frame, frame + 1));
        }
    }
    runs
}

fn candidate_with_neutralized_ranges(
    parent: &ProvenBootCandidate,
    ranges: &[(usize, usize)],
    generation: u32,
    mutation: &str,
) -> Result<Candidate, EvaluateError> {
    let mut tape = parent.tape.clone();
    for &(start, end) in ranges {
        for frame in &mut tape.frames[start..end] {
            frame.pads[0] = RawPadState::default();
        }
    }
    let mut candidate = Candidate::from_absolute_tape(SegmentProfile::BootToFsp103, &tape)?;
    candidate.ancestry = crate::search::Ancestry {
        generation,
        parent_id: Some(parent.candidate.id()?),
        mutation: Some(mutation.into()),
        intervention: Some(crate::search::InterventionRange {
            start_frame: ranges
                .iter()
                .map(|(start, _)| *start as u64)
                .min()
                .unwrap_or(0),
            end_frame_exclusive: ranges.iter().map(|(_, end)| *end as u64).max().unwrap_or(0),
            parent_end_frame_exclusive: ranges
                .iter()
                .map(|(_, end)| *end as u64)
                .max()
                .unwrap_or(0),
        }),
    };
    Ok(candidate)
}

fn best_boot_candidate(candidates: Vec<ProvenBootCandidate>) -> Option<ProvenBootCandidate> {
    candidates.into_iter().min_by(|left, right| {
        left.sim_tick
            .cmp(&right.sim_tick)
            .then(left.tape_frame.cmp(&right.tape_frame))
            .then(pulse_frame_count(&left.tape).cmp(&pulse_frame_count(&right.tape)))
            .then_with(|| {
                left.candidate
                    .id()
                    .unwrap()
                    .cmp(&right.candidate.id().unwrap())
            })
    })
}

fn evaluate_boot_batch(
    config: &BootMinimizeConfig,
    candidates: Vec<Candidate>,
    root: &Path,
    generation: u32,
) -> Result<Vec<ProvenBootCandidate>, EvaluateError> {
    Ok(evaluate_boot_batch_with_report(config, candidates, root, generation)?.0)
}

fn evaluate_boot_batch_with_report(
    config: &BootMinimizeConfig,
    candidates: Vec<Candidate>,
    root: &Path,
    generation: u32,
) -> Result<(Vec<ProvenBootCandidate>, EvaluationReport), EvaluateError> {
    let population_root = root.join("population");
    let manifest = write_explicit_population(
        &population_root,
        SegmentProfile::BootToFsp103,
        generation,
        candidates.clone(),
    )?;
    let report = evaluate_population(&EvaluateConfig {
        population_path: population_root.join("manifest.json"),
        game: config.game.clone(),
        dvd: config.dvd.clone(),
        output_root: root.join("evidence"),
        results_path: root.join("results.json"),
        working_directory: config.working_directory.clone(),
        game_args_prefix: config.game_args_prefix.clone(),
        workers: config.workers,
        repetitions: config.repetitions,
        timeout: config.timeout,
        harness: None,
    })?;
    let mut proven = Vec::new();
    for candidate in candidates {
        let id = candidate.id()?;
        let attempts: Vec<_> = report
            .attempts
            .iter()
            .filter(|attempt| attempt.candidate_id == id)
            .collect();
        if attempts.len() != config.repetitions as usize
            || !attempts.iter().all(|attempt| attempt.goal_reached)
        {
            continue;
        }
        let observation = attempts[0]
            .milestone_observations
            .get("gameplay-ready-f-sp103")
            .ok_or_else(|| {
                EvaluateError::InvalidResult(format!(
                    "successful boot candidate {id} has no goal observation"
                ))
            })?;
        let boundary_fingerprint = attempts[0]
            .boundary_fingerprints
            .get("gameplay-ready-f-sp103")
            .ok_or_else(|| {
                EvaluateError::InvalidResult(format!(
                    "successful boot candidate {id} has no goal boundary fingerprint"
                ))
            })?
            .clone();
        proven.push(ProvenBootCandidate {
            tape: candidate.compile()?,
            candidate,
            sim_tick: observation.sim_tick,
            tape_frame: observation.tape_frame,
            boundary_fingerprint,
        });
    }
    // Keep manifest live in this scope as a sanity assertion that every exact
    // caller-supplied candidate was represented once.
    debug_assert_eq!(
        manifest.members.len(),
        report.planned_attempts / config.repetitions as usize
    );
    Ok((proven, report))
}

#[derive(Clone, Debug)]
struct Trial {
    candidate_id: String,
    ancestry: crate::search::Ancestry,
    rng_seed: u64,
    attempt: u32,
    tape: PathBuf,
    logical_tick_budget: u64,
    boot: TapeBoot,
    suffix_tape: Option<PathBuf>,
    root: PathBuf,
    state: PathBuf,
    milestones: PathBuf,
    gameplay_trace: Option<PathBuf>,
    stdout: PathBuf,
    stderr: PathBuf,
}

fn build_trials(
    manifest: &PopulationManifest,
    population_root: &Path,
    output_root: &Path,
    repetitions: u32,
) -> Result<Vec<Trial>, EvaluateError> {
    let mut trials = Vec::with_capacity(manifest.members.len() * repetitions as usize);
    for member in &manifest.members {
        if member.candidate_id.is_empty()
            || !member
                .candidate_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(EvaluateError::InvalidManifest(format!(
                "candidate ID {:?} is unsafe",
                member.candidate_id
            )));
        }
        let tape = fs::canonicalize(population_root.join(&member.tape_file))?;
        if !tape.starts_with(population_root) {
            return Err(EvaluateError::InvalidManifest(format!(
                "candidate {} tape escapes the population directory",
                member.candidate_id
            )));
        }
        let candidate_path = fs::canonicalize(population_root.join(&member.candidate_file))?;
        if !candidate_path.starts_with(population_root) {
            return Err(EvaluateError::InvalidManifest(format!(
                "candidate {} source escapes the population directory",
                member.candidate_id
            )));
        }
        let candidate: Candidate = serde_json::from_slice(&fs::read(&candidate_path)?)?;
        candidate.validate()?;
        let tape_bytes = fs::read(&tape)?;
        let decoded = InputTape::decode(&tape_bytes)?;
        let compiled = candidate.compile()?;
        if candidate.segment != manifest.segment
            || candidate.boot != manifest.boot
            || candidate.id()? != member.candidate_id
            || candidate.ancestry != member.ancestry
            || compiled.frames.len() as u64 != member.frame_count
            || member.input_complexity != Some(tape_input_complexity(&compiled))
            || compiled.encode()? != tape_bytes
        {
            return Err(EvaluateError::InvalidManifest(format!(
                "candidate {} ID, ancestry, frame count, complexity, source, and tape are not one content identity",
                member.candidate_id
            )));
        }
        let expected_boot = match manifest.segment {
            SegmentProfile::BootToFsp103 => TapeBoot::Process,
            SegmentProfile::Fsp103ToFsp104 => TapeBoot::Stage {
                stage: "F_SP103".into(),
                room: 1,
                point: 1,
                layer: 3,
                save_slot: None,
                fixture: None,
            },
            SegmentProfile::LinkControlToTunnelCrawlStart => {
                return Err(EvaluateError::InvalidManifest(
                    "anchored movement candidates require an anchored objective".into(),
                ));
            }
        };
        if manifest.boot != expected_boot || decoded.tape.boot != manifest.boot {
            return Err(EvaluateError::InvalidManifest(format!(
                "candidate {} boot origin {:?} and manifest origin {:?} do not match direct profile origin {:?}",
                member.candidate_id, decoded.tape.boot, manifest.boot, expected_boot
            )));
        }
        for attempt in 1..=repetitions {
            let root = output_root
                .join("candidates")
                .join(&member.candidate_id)
                .join(format!("attempt-{attempt:03}"));
            trials.push(Trial {
                candidate_id: member.candidate_id.clone(),
                ancestry: candidate.ancestry.clone(),
                rng_seed: manifest.rng_seed,
                attempt,
                tape: tape.clone(),
                logical_tick_budget: member.frame_count,
                boot: decoded.tape.boot.clone(),
                suffix_tape: None,
                state: root.join("state"),
                milestones: root.join("milestones.json"),
                gameplay_trace: (attempt == 1).then(|| root.join("gameplay.trace")),
                stdout: root.join("stdout.txt"),
                stderr: root.join("stderr.txt"),
                root,
            });
        }
    }
    Ok(trials)
}

fn build_anchored_trials(
    manifest: &PopulationManifest,
    population_root: &Path,
    output_root: &Path,
    repetitions: u32,
    objective: &PreparedAnchoredObjective,
) -> Result<Vec<Trial>, EvaluateError> {
    let mut trials = Vec::with_capacity(manifest.members.len() * repetitions as usize);
    for member in &manifest.members {
        if member.candidate_id.is_empty()
            || !member
                .candidate_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(EvaluateError::InvalidManifest(format!(
                "candidate ID {:?} is unsafe",
                member.candidate_id
            )));
        }
        let suffix_path = fs::canonicalize(population_root.join(&member.tape_file))?;
        if !suffix_path.starts_with(population_root) {
            return Err(EvaluateError::InvalidManifest(format!(
                "candidate {} tape escapes the population directory",
                member.candidate_id
            )));
        }
        let candidate_path = fs::canonicalize(population_root.join(&member.candidate_file))?;
        if !candidate_path.starts_with(population_root) {
            return Err(EvaluateError::InvalidManifest(format!(
                "candidate {} source escapes the population directory",
                member.candidate_id
            )));
        }
        let candidate: Candidate = serde_json::from_slice(&fs::read(&candidate_path)?)?;
        candidate.validate()?;
        let suffix_bytes = fs::read(&suffix_path)?;
        let suffix = InputTape::decode(&suffix_bytes)?.tape;
        let compiled = candidate.compile()?;
        if candidate.segment != manifest.segment
            || candidate.boot != manifest.boot
            || candidate.id()? != member.candidate_id
            || candidate.ancestry != member.ancestry
            || compiled.frames.len() as u64 != member.frame_count
            || compiled.encode()? != suffix_bytes
        {
            return Err(EvaluateError::InvalidManifest(format!(
                "candidate {} ID, ancestry, frame count, source, and tape are not one content identity",
                member.candidate_id
            )));
        }
        let chained = concatenate(vec![
            ChainSegment::all(objective.prefix.clone()),
            ChainSegment::all(suffix),
        ])
        .map_err(|error| EvaluateError::InvalidManifest(error.to_string()))?;
        let logical_tick_budget = u64::try_from(chained.tape.frames.len()).map_err(|_| {
            EvaluateError::InvalidManifest(format!(
                "candidate {} chained tape length does not fit u64",
                member.candidate_id
            ))
        })?;
        for attempt in 1..=repetitions {
            let root = output_root
                .join("candidates")
                .join(&member.candidate_id)
                .join(format!("attempt-{attempt:03}"));
            fs::create_dir_all(&root)?;
            let full_tape = root.join("full.tape");
            fs::write(&full_tape, chained.tape.encode()?)?;
            trials.push(Trial {
                candidate_id: member.candidate_id.clone(),
                ancestry: candidate.ancestry.clone(),
                rng_seed: manifest.rng_seed,
                attempt,
                tape: full_tape,
                logical_tick_budget,
                boot: chained.tape.boot.clone(),
                suffix_tape: Some(suffix_path.clone()),
                state: root.join("state"),
                milestones: root.join("milestones.json"),
                gameplay_trace: (attempt == 1).then(|| root.join("gameplay.trace")),
                stdout: root.join("stdout.txt"),
                stderr: root.join("stderr.txt"),
                root,
            });
        }
    }
    Ok(trials)
}

fn run_trial(
    config: &EvaluateConfig,
    segment: SegmentProfile,
    trial: &Trial,
    worker_id: &str,
    global_cancel: &AtomicBool,
    anchored: Option<&PreparedAnchoredObjective>,
) -> AttemptEvidence {
    let started = Instant::now();
    let mut evidence = AttemptEvidence {
        schema: ATTEMPT_SCHEMA,
        candidate_id: trial.candidate_id.clone(),
        ancestry: trial.ancestry.clone(),
        attempt: trial.attempt,
        worker_id: worker_id.into(),
        segment,
        boot: trial.boot.clone(),
        tape: trial.tape.clone(),
        realized_tape: None,
        suffix_tape: trial.suffix_tape.clone(),
        artifact_root: trial.root.clone(),
        harness_request: None,
        harness_request_sha256: None,
        harness_result: None,
        harness_result_sha256: None,
        harness_terminal: None,
        state_root: trial.state.clone(),
        milestone_result: trial.milestones.clone(),
        gameplay_trace: None,
        gameplay_trace_blob: None,
        gameplay_trace_error: None,
        transition_corpus: None,
        transition_evidence: None,
        episode_manifest: None,
        immutable_episode: None,
        dataset_source: None,
        transition_count: None,
        transition_corpus_error: None,
        stdout: trial.stdout.clone(),
        stderr: trial.stderr.clone(),
        elapsed_millis: 0,
        exit_code: None,
        timed_out: false,
        cancelled: false,
        infrastructure_error: None,
        outcome: EpisodeOutcome {
            class: EpisodeOutcomeClass::Failed,
            reason: "trial has not completed".into(),
        },
        crash_artifacts: Vec::new(),
        milestone_depth: 0,
        deepest_milestone: "none".into(),
        first_hit_tick: None,
        goal_reached: false,
        milestone_observations: BTreeMap::new(),
        boundary_fingerprints: BTreeMap::new(),
        value_projections: BTreeMap::new(),
    };
    let mut run = || -> Result<TrialScore, EvaluateError> {
        if let Some(harness) = &config.harness {
            return run_harness_trial(
                harness,
                segment,
                trial,
                global_cancel,
                anchored,
                &mut evidence,
            );
        }
        fs::create_dir_all(&trial.state)?;
        let stdout = File::create(&trial.stdout)?;
        let stderr = File::create(&trial.stderr)?;
        let mut command = Command::new(&config.game);
        command
            .current_dir(&config.working_directory)
            .args(&config.game_args_prefix)
            .arg("--dvd")
            .arg(&config.dvd);
        let (milestone_list, goal) = if let Some(objective) = anchored {
            command
                .arg("--milestone-program")
                .arg(&objective.runtime_program);
            (
                format!(
                    "{},{}",
                    objective.identity.source_milestone, objective.identity.goal_milestone
                ),
                objective.identity.goal_milestone.clone(),
            )
        } else {
            match segment {
                SegmentProfile::BootToFsp103 => (
                    "gameplay-ready-f-sp103".into(),
                    "gameplay-ready-f-sp103".into(),
                ),
                SegmentProfile::Fsp103ToFsp104 => (
                    "gameplay-ready-f-sp103,exit-f-sp103-to-f-sp104,entered-f-sp104".into(),
                    "entered-f-sp104".into(),
                ),
                SegmentProfile::LinkControlToTunnelCrawlStart => unreachable!(
                    "anchored profiles are evaluated through evaluate_anchored_population"
                ),
            }
        };
        command
            .arg("--input-tape")
            .arg(&trial.tape)
            .arg("--input-tape-end")
            .arg("hold")
            .arg("--automation-tick-budget")
            .arg(trial.logical_tick_budget.to_string())
            .arg("--automation-data-root")
            .arg(&trial.state)
            .arg("--milestones")
            .arg(&milestone_list)
            .arg("--milestone-goal")
            .arg(&goal)
            .arg("--milestone-result")
            .arg(&trial.milestones);
        if let Some(gameplay_trace) = &trial.gameplay_trace {
            command.arg("--gameplay-trace").arg(gameplay_trace);
        }
        command
            .arg("--cvar")
            .arg("game.instantSaves=true")
            .arg("--cvar")
            .arg("backend.cardFileType=1")
            .arg("--cvar")
            .arg("backend.wasPresetChosen=true")
            .arg("--cvar")
            .arg("game.enableMenuPointer=false")
            .arg("--headless")
            .arg("--fixed-step")
            .arg("--exit-after-tape")
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        hide_window(&mut command);
        let mut child = command.spawn().map_err(EvaluateError::Launch)?;
        let status = loop {
            if global_cancel.load(Ordering::Acquire) {
                evidence.cancelled = true;
                let _ = child.kill();
                let _ = child.wait();
                return Err(EvaluateError::Cancelled);
            }
            if started.elapsed() >= config.timeout {
                evidence.timed_out = true;
                let _ = child.kill();
                let _ = child.wait();
                return Err(EvaluateError::Timeout(config.timeout));
            }
            match child.try_wait()? {
                Some(status) => {
                    evidence.exit_code = status.code();
                    break status;
                }
                None => thread::sleep(Duration::from_millis(10)),
            }
        };
        let score = if let Some(objective) = anchored {
            parse_anchored_milestones(&trial.milestones, objective, &trial.boot)
        } else {
            parse_native_milestones(&trial.milestones, segment, &trial.boot)
        }?;
        validate_native_exit(status, score.goal_reached)?;
        Ok(score)
    };
    match run() {
        Ok(score) => {
            evidence.milestone_depth = score.depth;
            evidence.deepest_milestone = score.deepest;
            evidence.first_hit_tick = score.score_tick;
            evidence.goal_reached = score.goal_reached;
            evidence.milestone_observations = score.milestone_observations;
            evidence.boundary_fingerprints = score.boundary_fingerprints;
            evidence.value_projections = score.value_projections;
        }
        Err(error) => evidence.infrastructure_error = Some(error.to_string()),
    }
    let gameplay_trace = evidence
        .gameplay_trace
        .clone()
        .or_else(|| trial.gameplay_trace.clone());
    evidence.gameplay_trace = None;
    if let Some(path) = gameplay_trace {
        match fs::read(&path)
            .map_err(|error| error.to_string())
            .and_then(|bytes| crate::trace::decode(&bytes).map_err(|error| error.to_string()))
            .and_then(|trace| {
                if trace.boot != trial.boot {
                    Err(format!(
                        "gameplay trace boot origin {:?} does not match tape origin {:?}",
                        trace.boot, trial.boot
                    ))
                } else if trace.capacity_exhausted {
                    Err("gameplay trace capacity was exhausted".into())
                } else if trace.records.is_empty() {
                    Err("gameplay trace contains no records".into())
                } else {
                    Ok(())
                }
            }) {
            Ok(()) => evidence.gameplay_trace = Some(path),
            Err(error) => evidence.gameplay_trace_error = Some(error),
        }
    }
    evidence.outcome = classify_attempt_outcome(&evidence);
    if trial.attempt == 1
        && evidence.infrastructure_error.is_none()
        && evidence.gameplay_trace.is_some()
        && let Some(objective) = anchored
    {
        match extract_trial_transition_corpus(trial, &evidence, objective) {
            Ok((
                path,
                evidence_path,
                episode_manifest,
                immutable_episode,
                dataset_source,
                count,
            )) => {
                evidence.transition_corpus = Some(path);
                evidence.transition_evidence = Some(evidence_path);
                evidence.episode_manifest = Some(episode_manifest);
                evidence.immutable_episode = immutable_episode;
                evidence.dataset_source = Some(dataset_source);
                evidence.transition_count = Some(count);
            }
            Err(error) => evidence.transition_corpus_error = Some(error),
        }
    }
    evidence.elapsed_millis = started.elapsed().as_millis();
    evidence
}

fn run_harness_trial(
    harness: &HarnessEvaluateConfig,
    segment: SegmentProfile,
    trial: &Trial,
    global_cancel: &AtomicBool,
    anchored: Option<&PreparedAnchoredObjective>,
    evidence: &mut AttemptEvidence,
) -> Result<TrialScore, EvaluateError> {
    if global_cancel.load(Ordering::Acquire) {
        evidence.cancelled = true;
        return Err(EvaluateError::Cancelled);
    }
    let repository_root = fs::canonicalize(&harness.repository_root)?;
    fs::create_dir_all(&trial.root)?;
    let trial_root = fs::canonicalize(&trial.root)?;
    let artifact_root = trial_root.join("harness");
    let destination = artifact_root
        .strip_prefix(&repository_root)
        .map_err(|_| {
            EvaluateError::InvalidConfig(format!(
                "search output must be beneath the harness repository root: {}",
                artifact_root.display()
            ))
        })?
        .to_str()
        .ok_or_else(|| EvaluateError::InvalidConfig("search output path is not UTF-8".into()))?
        .replace(std::path::MAIN_SEPARATOR, "/");
    let request = derive_candidate_request(
        &harness.request_template,
        &repository_root,
        &trial.tape,
        &destination,
        trial.rng_seed,
    )?;
    if let Some(objective) = anchored
        && (request.objective.goal != objective.identity.goal_milestone
            || request.objective.program_sha256.to_string()
                != objective.identity.milestone_program_sha256)
    {
        return Err(EvaluateError::InvalidConfig(
            "anchored search request template does not bind the prepared objective".into(),
        ));
    }

    let request_path = trial_root.join("request.json");
    write_json(&request_path, &request)?;
    let result = execute_request(&request, &repository_root, trial.attempt)
        .map_err(|error| EvaluateError::NativeResult(error.to_string()))?;
    let result_path = artifact_root.join("result.json");
    evidence.artifact_root = artifact_root.clone();
    evidence.harness_request = Some(request_path);
    evidence.harness_request_sha256 = Some(request.content_sha256);
    evidence.harness_result = Some(result_path);
    evidence.harness_result_sha256 = Some(result.content_sha256);
    evidence.harness_terminal = Some(result.terminal);
    evidence.state_root = artifact_root.join("state");
    evidence.milestone_result =
        harness_artifact_path(&artifact_root, result.artifacts.objective_result.as_ref())
            .unwrap_or_else(|| artifact_root.join("objective.json"));
    evidence.gameplay_trace =
        harness_artifact_path(&artifact_root, result.artifacts.gameplay_trace.as_ref());
    evidence.realized_tape =
        harness_artifact_path(&artifact_root, result.artifacts.realized_input.as_ref());
    evidence.stdout = harness_artifact_path(&artifact_root, result.artifacts.stdout.as_ref())
        .unwrap_or_else(|| artifact_root.join("stdout.txt"));
    evidence.stderr = harness_artifact_path(&artifact_root, result.artifacts.stderr.as_ref())
        .unwrap_or_else(|| artifact_root.join("stderr.txt"));
    evidence.elapsed_millis = u128::from(result.timing.host_elapsed_millis);
    evidence.exit_code = match result.terminal {
        HarnessTerminalReason::Reached => Some(0),
        HarnessTerminalReason::Exhausted | HarnessTerminalReason::TargetLost => {
            Some(NATIVE_GOAL_MISS_EXIT_CODE)
        }
        _ => None,
    };
    evidence.timed_out = result.terminal == HarnessTerminalReason::HostTimeout;
    evidence.cancelled = result.terminal == HarnessTerminalReason::Cancelled;

    match result.terminal {
        HarnessTerminalReason::Reached | HarnessTerminalReason::Exhausted => score_harness_result(
            &result,
            &request,
            &evidence.milestone_result,
            &trial.boot,
            segment,
            anchored,
        ),
        _ => Ok(empty_harness_score()),
    }
}

fn harness_artifact_path(
    artifact_root: &Path,
    reference: Option<&ArtifactReference>,
) -> Option<PathBuf> {
    reference.map(|reference| artifact_root.join(&reference.path))
}

fn empty_harness_score() -> TrialScore {
    TrialScore {
        depth: 0,
        deepest: "none".into(),
        score_tick: None,
        goal_reached: false,
        milestone_observations: BTreeMap::new(),
        boundary_fingerprints: BTreeMap::new(),
        value_projections: BTreeMap::new(),
    }
}

fn score_harness_result(
    result: &HarnessRunResult,
    request: &HarnessRunRequest,
    objective_result: &Path,
    expected_boot: &TapeBoot,
    segment: SegmentProfile,
    anchored: Option<&PreparedAnchoredObjective>,
) -> Result<TrialScore, EvaluateError> {
    let native: NativeMilestoneResult = serde_json::from_slice(&fs::read(objective_result)?)?;
    if native.schema.name != "dusklight.automation.milestones"
        || native.schema.version != 5
        || native.goal.as_deref() != Some(request.objective.goal.as_str())
        || native.program_digest.as_deref() != Some(&request.objective.program_sha256.to_string())
        || native.milestones.len() != 1
    {
        return Err(EvaluateError::NativeResult(
            "core-harness objective artifact does not match its request".into(),
        ));
    }
    validate_native_boot(&native, expected_boot)?;
    let milestone = &native.milestones[0];
    if milestone.id != request.objective.goal
        || milestone.hit != result.objective.reached
        || native.goal_reached != result.objective.reached
    {
        return Err(EvaluateError::NativeResult(
            "core-harness result contradicts its native objective artifact".into(),
        ));
    }
    if !milestone.hit {
        return Ok(if let Some(objective) = anchored {
            anchored_source_score(objective)
        } else {
            empty_harness_score()
        });
    }
    let (sim_tick, tape_frame, evidence) = match (
        milestone.sim_tick,
        milestone.tape_frame,
        milestone.evidence.as_ref(),
    ) {
        (Some(sim_tick), Some(tape_frame), Some(evidence)) => (sim_tick, tape_frame, evidence),
        _ => {
            return Err(EvaluateError::NativeResult(
                "reached harness objective omitted tick or boundary evidence".into(),
            ));
        }
    };
    validate_fingerprint(&evidence.boundary_fingerprint)?;
    let mut observations = BTreeMap::new();
    observations.insert(
        milestone.id.clone(),
        MilestoneObservation {
            sim_tick,
            tape_frame,
            boundary_index: milestone.boundary_index,
            phase: milestone.phase.clone(),
            stable_ticks: milestone.stable_ticks,
            definition_digest: milestone.definition_digest.clone(),
            program_digest: milestone.program_digest.clone(),
        },
    );
    let mut fingerprints = BTreeMap::new();
    fingerprints.insert(milestone.id.clone(), evidence.boundary_fingerprint.clone());
    let projections = validate_value_projections(milestone.projections.as_ref())?;
    let value_projections = if projections.is_empty() {
        BTreeMap::new()
    } else {
        BTreeMap::from([(milestone.id.clone(), projections)])
    };
    if let Some(objective) = anchored {
        let mut score = anchored_source_score(objective);
        score.depth = 2;
        score.deepest = milestone.id.clone();
        score.score_tick = Some(
            tape_frame
                .checked_sub(objective.identity.source_boundary_index)
                .ok_or_else(|| {
                    EvaluateError::NativeResult(
                        "anchored harness goal fired inside the immutable prefix".into(),
                    )
                })?,
        );
        score.goal_reached = true;
        score.milestone_observations.extend(observations);
        score.boundary_fingerprints.extend(fingerprints);
        score.value_projections = value_projections;
        return Ok(score);
    }
    let depth = match segment {
        SegmentProfile::BootToFsp103 => 2,
        SegmentProfile::Fsp103ToFsp104 => 4,
        SegmentProfile::LinkControlToTunnelCrawlStart => unreachable!("anchored profile"),
    };
    Ok(TrialScore {
        depth,
        deepest: milestone.id.clone(),
        score_tick: Some(sim_tick),
        goal_reached: true,
        milestone_observations: observations,
        boundary_fingerprints: fingerprints,
        value_projections,
    })
}

fn anchored_source_score(objective: &PreparedAnchoredObjective) -> TrialScore {
    TrialScore {
        depth: 1,
        deepest: objective.identity.source_milestone.clone(),
        score_tick: Some(0),
        goal_reached: false,
        milestone_observations: BTreeMap::from([(
            objective.identity.source_milestone.clone(),
            MilestoneObservation {
                sim_tick: objective.identity.source_tape_frame,
                tape_frame: objective.identity.source_tape_frame,
                boundary_index: Some(objective.identity.source_boundary_index),
                phase: Some(objective.source.phase.clone()),
                stable_ticks: Some(objective.source.stable_ticks),
                definition_digest: Some(objective.source.digest.clone()),
                program_digest: Some(objective.identity.milestone_program_sha256.clone()),
            },
        )]),
        boundary_fingerprints: BTreeMap::from([(
            objective.identity.source_milestone.clone(),
            BoundaryFingerprint {
                schema: "dusklight.milestone-boundary/v1".into(),
                algorithm: "xxh3-128".into(),
                canonical_encoding: "little-endian-fixed-v1".into(),
                digest: objective.identity.source_boundary_fingerprint.clone(),
            },
        )]),
        value_projections: BTreeMap::new(),
    }
}

fn classify_attempt_outcome(evidence: &AttemptEvidence) -> EpisodeOutcome {
    if let Some(terminal) = evidence.harness_terminal {
        let class = match terminal {
            HarnessTerminalReason::Reached => EpisodeOutcomeClass::Successful,
            HarnessTerminalReason::Exhausted
            | HarnessTerminalReason::Impossible
            | HarnessTerminalReason::TargetLost
            | HarnessTerminalReason::Rejected => EpisodeOutcomeClass::Failed,
            HarnessTerminalReason::Unsupported | HarnessTerminalReason::CapabilityMismatch => {
                EpisodeOutcomeClass::Unsupported
            }
            HarnessTerminalReason::HostTimeout | HarnessTerminalReason::Hung => {
                EpisodeOutcomeClass::TimedOut
            }
            HarnessTerminalReason::WorkerCrashed | HarnessTerminalReason::GameCrashed => {
                EpisodeOutcomeClass::Crashed
            }
            HarnessTerminalReason::IdentityMismatch
            | HarnessTerminalReason::ProtocolFailure
            | HarnessTerminalReason::Nondeterministic => EpisodeOutcomeClass::Desynced,
            HarnessTerminalReason::Cancelled => EpisodeOutcomeClass::Failed,
        };
        return EpisodeOutcome {
            class,
            reason: format!("core harness terminal: {}", terminal.name()),
        };
    }
    classify_outcome(
        evidence.timed_out,
        evidence.cancelled,
        evidence.gameplay_trace_error.as_deref(),
        evidence.infrastructure_error.as_deref(),
        evidence.goal_reached,
    )
}

fn classify_outcome(
    timed_out: bool,
    cancelled: bool,
    gameplay_trace_error: Option<&str>,
    infrastructure_error: Option<&str>,
    goal_reached: bool,
) -> EpisodeOutcome {
    if timed_out {
        return EpisodeOutcome {
            class: EpisodeOutcomeClass::TimedOut,
            reason: "evaluation timeout expired".into(),
        };
    }
    if gameplay_trace_error.is_some_and(|reason| reason.contains("capacity was exhausted")) {
        return EpisodeOutcome {
            class: EpisodeOutcomeClass::Truncated,
            reason: gameplay_trace_error.unwrap().into(),
        };
    }
    if let Some(reason) = infrastructure_error {
        let class = if reason.starts_with("could not launch Dusklight") {
            EpisodeOutcomeClass::Unsupported
        } else if reason.contains("worker exit") {
            EpisodeOutcomeClass::Crashed
        } else if reason.starts_with("invalid native milestone result")
            || reason.starts_with("invalid search result")
        {
            EpisodeOutcomeClass::Desynced
        } else if cancelled {
            EpisodeOutcomeClass::Failed
        } else {
            EpisodeOutcomeClass::Crashed
        };
        return EpisodeOutcome {
            class,
            reason: reason.into(),
        };
    }
    if goal_reached {
        EpisodeOutcome {
            class: EpisodeOutcomeClass::Successful,
            reason: "objective reached".into(),
        }
    } else {
        EpisodeOutcome {
            class: EpisodeOutcomeClass::Failed,
            reason: "objective not reached".into(),
        }
    }
}

fn extract_trial_transition_corpus(
    trial: &Trial,
    evidence: &AttemptEvidence,
    objective: &PreparedAnchoredObjective,
) -> Result<(PathBuf, PathBuf, PathBuf, Option<PathBuf>, PathBuf, u64), String> {
    let trace_path = evidence
        .gameplay_trace
        .as_ref()
        .ok_or_else(|| "validated gameplay trace is missing".to_string())?;
    let trace_bytes = fs::read(trace_path).map_err(|error| error.to_string())?;
    let decoded = crate::trace::decode(&trace_bytes).map_err(|error| error.to_string())?;
    let start_tape_frame = objective
        .identity
        .source_tape_frame
        .checked_add(1)
        .ok_or_else(|| "learning range start overflows".to_string())?;
    let end_tape_frame = if evidence.goal_reached {
        evidence
            .milestone_observations
            .get(&objective.identity.goal_milestone)
            .map(|observation| observation.tape_frame)
            .ok_or_else(|| "goal hit lacks a tape-frame observation".to_string())?
    } else {
        decoded
            .records
            .last()
            .and_then(|record| record.tape_frame)
            .ok_or_else(|| "goal miss trace lacks a final tape frame".to_string())?
    };
    if end_tape_frame < start_tape_frame {
        return Err(format!(
            "learning range {start_tape_frame}..={end_tape_frame} is empty"
        ));
    }
    let episode_tape_path = evidence.realized_tape.as_ref().unwrap_or(&trial.tape);
    let tape_bytes = fs::read(episode_tape_path).map_err(|error| error.to_string())?;
    let decoded_tape = InputTape::decode(&tape_bytes)
        .map_err(|error| error.to_string())?
        .tape;
    let start_reference = learning_boundary_reference(
        &objective.identity.digest,
        &objective.identity.source_milestone,
        &objective.identity.source_boundary_fingerprint,
    );
    let terminal_reference = if evidence.goal_reached {
        let boundary = evidence
            .boundary_fingerprints
            .get(&objective.identity.goal_milestone)
            .ok_or_else(|| "goal hit lacks a terminal boundary fingerprint".to_string())?;
        Some(learning_boundary_reference(
            &objective.identity.digest,
            &objective.identity.goal_milestone,
            &boundary.digest,
        ))
    } else {
        None
    };
    let episode_digest = learning_episode_digest(
        &objective.identity.digest,
        &trial.candidate_id,
        &trace_bytes,
    );
    let corpus = extract_exploratory_from_bytes(
        &trace_bytes,
        &tape_bytes,
        ExploratoryExtractConfig {
            episode_digest,
            start_tape_frame,
            end_tape_frame,
            start_reference: Some(start_reference),
            terminal_reference,
            end_is_terminal: evidence.goal_reached,
        },
    )
    .map_err(|error| error.to_string())?;
    let count = u64::try_from(corpus.transitions.len())
        .map_err(|_| "transition count does not fit u64".to_string())?;
    let path = trial.root.join("transitions.dtcz");
    let transition_evidence = TransitionEvidenceBundle::build(TransitionEvidenceBuild {
        corpus: &corpus,
        trace: &decoded,
        tape: &decoded_tape,
        trace_sha256: ArtifactDigest(Sha256::digest(&trace_bytes).into()),
        tape_sha256: ArtifactDigest(Sha256::digest(&tape_bytes).into()),
        start_tape_frame,
        end_tape_frame,
        terminal_reason: evidence
            .goal_reached
            .then_some(TerminalReasonEvidence::ObjectiveReached),
    })
    .map_err(|error| error.to_string())?;
    let transition_evidence_bytes =
        serde_json::to_vec_pretty(&transition_evidence).map_err(|error| error.to_string())?;
    let intervention_offset = objective.prefix.frames.len() as u64;
    let intervention = trial
        .ancestry
        .intervention
        .as_ref()
        .map(|value| EpisodeIntervention {
            start_frame: intervention_offset.saturating_add(value.start_frame),
            end_frame_exclusive: intervention_offset.saturating_add(value.end_frame_exclusive),
            parent_end_frame_exclusive: intervention_offset
                .saturating_add(value.parent_end_frame_exclusive),
            description: trial
                .ancestry
                .mutation
                .clone()
                .unwrap_or_else(|| "candidate intervention".into()),
        });
    let producer_kind = if let Some(mutation) = trial.ancestry.mutation.as_deref() {
        if mutation.starts_with("q_") {
            EpisodeProducerKind::FittedQ
        } else if mutation.starts_with("structured_counterfactual") {
            EpisodeProducerKind::StructuredCounterfactual
        } else if mutation.starts_with("archive_novelty") {
            EpisodeProducerKind::ArchiveNovelty
        } else if mutation.starts_with("blind_") {
            EpisodeProducerKind::BlindCoverage
        } else if mutation.starts_with("systematic_probe") {
            EpisodeProducerKind::SystematicProbe
        } else if mutation.starts_with("random_probe") {
            EpisodeProducerKind::RandomProbe
        } else if mutation.starts_with("latin_hypercube") {
            EpisodeProducerKind::LatinHypercube
        } else if trial.ancestry.generation == 0 && trial.ancestry.parent_id.is_none() {
            EpisodeProducerKind::Seed
        } else {
            EpisodeProducerKind::Evolution
        }
    } else if trial.ancestry.generation == 0 && trial.ancestry.parent_id.is_none() {
        EpisodeProducerKind::Seed
    } else {
        EpisodeProducerKind::Evolution
    };
    let context = EpisodeContext {
        schema: EPISODE_CONTEXT_SCHEMA_V1.into(),
        run_build: RunBuildIdentity {
            executable_sha256: objective
                .identity
                .game_sha256
                .parse()
                .map_err(|error| format!("invalid objective game digest: {error}"))?,
            dusklight_commit: None,
            aurora_commit: None,
            target: Some(format!(
                "{}-{}",
                std::env::consts::ARCH,
                std::env::consts::OS
            )),
            profile: None,
            feature_digest: None,
        },
        objective: EpisodeObjectiveIdentity {
            id: format!(
                "{}:{}",
                objective.identity.segment.as_str(),
                objective.identity.goal_milestone
            ),
            digest: objective
                .identity
                .digest
                .parse()
                .map_err(|error| format!("invalid objective digest: {error}"))?,
        },
        producer: EpisodeProducerIdentity {
            kind: producer_kind,
            name: "huntctl".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
        seed: EpisodeSeed::Deterministic {
            value: trial.rng_seed,
        },
        worker_id: evidence.worker_id.clone(),
        lineage: EpisodeLineage {
            candidate_id: Some(trial.candidate_id.clone()),
            parent_candidate_id: trial.ancestry.parent_id.clone(),
            generation: trial.ancestry.generation,
            intervention,
        },
        outcome: evidence.outcome.clone(),
    };
    let transition_evidence_sha256 =
        ArtifactDigest(Sha256::digest(&transition_evidence_bytes).into());
    let episode_manifest = EpisodeManifest::build(EpisodeManifestBuild {
        context: &context,
        boot: &decoded_tape.boot,
        corpus: &corpus,
        query_view_id: "movement-state/v1",
        tape_sha256: ArtifactDigest(Sha256::digest(&tape_bytes).into()),
        trace_sha256: ArtifactDigest(Sha256::digest(&trace_bytes).into()),
        transition_evidence_sha256,
    })
    .map_err(|error| error.to_string())?;
    let evidence_path = trial.root.join("transitions.dtcz.evidence.json");
    let episode_manifest_path = trial.root.join("episode.json");
    let immutable_episode_path = trial.root.join("immutable-episode.json");
    let dataset_source_path = trial.root.join("dataset-source.json");
    corpus
        .write_zstd_file(&path, 3)
        .map_err(|error| error.to_string())?;
    fs::write(&evidence_path, transition_evidence_bytes).map_err(|error| error.to_string())?;
    fs::write(
        &episode_manifest_path,
        serde_json::to_vec_pretty(&episode_manifest).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let immutable_episode_path = if let Some(terminal) = evidence.harness_terminal {
        let result_path = evidence
            .harness_result
            .as_ref()
            .ok_or_else(|| "harness episode is missing its sealed result path".to_string())?;
        let result: HarnessRunResult =
            serde_json::from_slice(&fs::read(result_path).map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?;
        if result.terminal != terminal
            || Some(result.content_sha256) != evidence.harness_result_sha256
        {
            return Err("harness episode terminal or result identity changed".into());
        }
        let immutable_episode = ImmutableEpisodeArtifact::build(ImmutableEpisodeBuild {
            manifest: &episode_manifest,
            corpus: &corpus,
            evidence: &transition_evidence,
            transition_evidence_sha256,
            terminal,
            terminal_detail: &result.detail.message,
        })
        .map_err(|error| error.to_string())?;
        fs::write(
            &immutable_episode_path,
            serde_json::to_vec_pretty(&immutable_episode).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        Some(immutable_episode_path)
    } else {
        None
    };
    fs::write(
        &dataset_source_path,
        serde_json::to_vec_pretty(&DatasetSourceDescriptor {
            schema: DATASET_SOURCE_SCHEMA_V1.into(),
            source_id: episode_manifest.episode_sha256.to_string(),
            episode_manifest: fs::canonicalize(&episode_manifest_path)
                .map_err(|error| error.to_string())?,
            transition_corpus: fs::canonicalize(&path).map_err(|error| error.to_string())?,
            absolute_tape: fs::canonicalize(episode_tape_path)
                .map_err(|error| error.to_string())?,
            transition_evidence: fs::canonicalize(&evidence_path)
                .map_err(|error| error.to_string())?,
            gameplay_trace: fs::canonicalize(trace_path).map_err(|error| error.to_string())?,
            route_family: episode_manifest.objective.id.clone(),
            screenshot_sha256: Vec::new(),
            checkpoint_sha256: Vec::new(),
        })
        .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok((
        path,
        evidence_path,
        episode_manifest_path,
        immutable_episode_path,
        dataset_source_path,
        count,
    ))
}

fn learning_episode_digest(
    objective_digest: &str,
    candidate_id: &str,
    trace_bytes: &[u8],
) -> ArtifactDigest {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.search-learning-episode/v1\0");
    for bytes in [
        objective_digest.as_bytes(),
        candidate_id.as_bytes(),
        trace_bytes,
    ] {
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    ArtifactDigest(hasher.finalize().into())
}

fn write_episode_ledger(
    output_root: &Path,
    attempts: &[AttemptEvidence],
) -> Result<Option<PathBuf>, EvaluateError> {
    let mut ledger = EpisodeLedger::new();
    let mut candidate_inputs = BTreeMap::new();
    for attempt in attempts {
        let (Some(manifest_path), Some(corpus_path)) =
            (&attempt.episode_manifest, &attempt.transition_corpus)
        else {
            continue;
        };
        let manifest: EpisodeManifest = serde_json::from_slice(&fs::read(manifest_path)?)?;
        let corpus = TransitionCorpus::read_zstd_file(corpus_path)
            .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
        manifest
            .validate(&corpus)
            .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
        if let Some(previous) =
            candidate_inputs.insert(attempt.candidate_id.clone(), manifest.input_identity_sha256)
            && previous != manifest.input_identity_sha256
        {
            return Err(EvaluateError::InvalidResult(format!(
                "candidate {} produced conflicting episode input identities",
                attempt.candidate_id
            )));
        }
        ledger.ingest_episode(&manifest, manifest_path.clone());
    }
    if ledger.groups.is_empty() {
        return Ok(None);
    }
    for attempt in attempts {
        let Some(input_identity) = candidate_inputs.get(&attempt.candidate_id).copied() else {
            continue;
        };
        let proof_path = attempt.artifact_root.join("attempt.json");
        let proof_bytes = fs::read(&proof_path)?;
        ledger
            .ingest_proof(
                input_identity,
                ArtifactDigest(Sha256::digest(&proof_bytes).into()),
                proof_path,
                attempt.worker_id.clone(),
                attempt.attempt,
                attempt.outcome.clone(),
            )
            .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
    }
    ledger
        .validate()
        .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
    let path = output_root.join("episodes.json");
    write_json(&path, &ledger)?;
    Ok(Some(path))
}

fn address_attempt_artifacts(
    output_root: &Path,
    attempts: &mut [AttemptEvidence],
) -> Result<(), EvaluateError> {
    let store = ContentStore::initialize(output_root.join("content"))
        .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
    for attempt in attempts {
        if let Some(path) = &attempt.gameplay_trace {
            attempt.gameplay_trace_blob = Some(
                store
                    .put_file(path, ContentKind::GameplayTrace)
                    .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?,
            );
        }
        if matches!(
            attempt.outcome.class,
            EpisodeOutcomeClass::Crashed
                | EpisodeOutcomeClass::TimedOut
                | EpisodeOutcomeClass::Desynced
                | EpisodeOutcomeClass::Unsupported
                | EpisodeOutcomeClass::Truncated
        ) {
            let mut paths = vec![
                attempt.stdout.clone(),
                attempt.stderr.clone(),
                attempt.milestone_result.clone(),
            ];
            if attempt.gameplay_trace.is_none() {
                paths.push(attempt.artifact_root.join("gameplay.trace"));
            }
            for path in paths {
                if fs::metadata(&path)
                    .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)
                {
                    let blob = store
                        .put_file(&path, ContentKind::CrashArtifact)
                        .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
                    if !attempt
                        .crash_artifacts
                        .iter()
                        .any(|existing| existing.sha256 == blob.sha256)
                    {
                        attempt.crash_artifacts.push(blob);
                    }
                }
            }
        }
        write_json(&attempt.artifact_root.join("attempt.json"), attempt)?;
    }
    Ok(())
}

fn semantic_novelty_descriptor(
    evidence: &AttemptEvidence,
) -> Result<Option<SemanticNoveltyDescriptor>, EvaluateError> {
    evidence
        .gameplay_trace
        .as_ref()
        .map(|path| {
            let bytes = fs::read(path)?;
            let trace = crate::trace::decode(&bytes)
                .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
            let boundaries = evidence
                .boundary_fingerprints
                .iter()
                .map(|(name, value)| BoundaryFingerprintFact {
                    name: name.clone(),
                    schema: value.schema.clone(),
                    algorithm: value.algorithm.clone(),
                    canonical_encoding: value.canonical_encoding.clone(),
                    digest: value.digest.clone(),
                })
                .collect();
            SemanticNoveltyDescriptor::from_trace(&trace, boundaries)
                .map_err(|error| EvaluateError::InvalidResult(error.to_string()))
        })
        .transpose()
}

fn archive_behavior_context(
    evidence: &AttemptEvidence,
    descriptor: &SemanticNoveltyDescriptor,
) -> BehaviorContext {
    let axes = descriptor.axis_identities();
    let mut context = archive_behavior_context_from_evidence(
        &evidence.boundary_fingerprints,
        &evidence.value_projections,
        axes.contacts,
    );
    context.procedure_sequence_identity = axes.procedure_sequence;
    context.event_sequence_identity = axes.event_sequence;
    context.state_transition_identity = axes.state_transitions;
    context.actor_relationship_identity = axes.actor_relationships;
    context.flag_state_identity = axes.flags;
    context.kinematic_extrema_identity = axes.kinematic_extrema;
    context
}

fn archive_behavior_context_from_evidence(
    boundaries: &BTreeMap<String, BoundaryFingerprint>,
    projections: &BTreeMap<String, BTreeMap<String, ValueProjectionEvidence>>,
    contact_behavior_identity: Option<String>,
) -> BehaviorContext {
    let mut rng = Vec::new();
    let mut actors = Vec::new();
    let mut downstream_boundaries = Vec::new();
    let mut downstream = Vec::new();
    for (milestone, milestone_projections) in projections {
        for (name, projection) in milestone_projections {
            let Some(fingerprint) = projection.value_fingerprint.as_ref() else {
                continue;
            };
            if !projection.available {
                continue;
            }
            let encoded = serde_json::to_vec(&(
                milestone,
                name,
                &projection.identity,
                &fingerprint.schema,
                &fingerprint.algorithm,
                &fingerprint.canonical_encoding,
                &fingerprint.digest,
            ))
            .expect("validated value-projection identity is serializable");
            if projection
                .values
                .iter()
                .any(|value| value.get("kind").and_then(serde_json::Value::as_str) == Some("rng"))
            {
                rng.push(encoded.clone());
            }
            if projection.values.iter().any(|value| {
                value.get("kind").and_then(serde_json::Value::as_str) == Some("actor_population")
            }) {
                actors.push(encoded.clone());
            }
            downstream.push(encoded);
        }
    }
    for (milestone, fingerprint) in boundaries {
        let encoded = serde_json::to_vec(&(
            milestone,
            &fingerprint.schema,
            &fingerprint.algorithm,
            &fingerprint.canonical_encoding,
            &fingerprint.digest,
        ))
        .expect("validated boundary identity is serializable");
        downstream_boundaries.push(encoded.clone());
        downstream.push(encoded);
    }
    BehaviorContext {
        procedure_sequence_identity: None,
        event_sequence_identity: None,
        state_transition_identity: None,
        actor_relationship_identity: None,
        flag_state_identity: None,
        kinematic_extrema_identity: None,
        objective_rng_identity: archive_axis_identity(b"rng/v1", &rng),
        actor_population_identity: archive_axis_identity(b"actors/v1", &actors),
        contact_behavior_identity,
        boundary_state_identity: archive_axis_identity(b"boundaries/v1", &downstream_boundaries),
        downstream_state_identity: archive_axis_identity(b"downstream/v1", &downstream),
    }
}

fn archive_axis_identity(domain: &[u8], entries: &[Vec<u8>]) -> Option<String> {
    if entries.is_empty() {
        return None;
    }
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight-behavior-archive-axis/v1\0");
    hasher.update((domain.len() as u64).to_le_bytes());
    hasher.update(domain);
    for entry in entries {
        hasher.update((entry.len() as u64).to_le_bytes());
        hasher.update(entry);
    }
    Some(format!("{:x}", hasher.finalize()))
}

fn learning_boundary_reference(
    objective_digest: &str,
    milestone: &str,
    boundary_fingerprint: &str,
) -> StateReference {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.search-learning-boundary/v1\0");
    for value in [objective_digest, milestone, boundary_fingerprint] {
        hasher.update((value.len() as u64).to_le_bytes());
        hasher.update(value.as_bytes());
    }
    StateReference {
        kind: StateReferenceKind::Boundary,
        digest: ArtifactDigest(hasher.finalize().into()),
    }
}

fn validate_native_exit(status: ExitStatus, goal_reached: bool) -> Result<(), EvaluateError> {
    match (status.code(), goal_reached) {
        (Some(0), true) | (Some(NATIVE_GOAL_MISS_EXIT_CODE), false) => Ok(()),
        (code, _) => Err(EvaluateError::NativeResult(format!(
            "worker exit {code:?} disagrees with goal_reached={goal_reached} (expected 0 for a hit or {NATIVE_GOAL_MISS_EXIT_CODE} for a valid miss)"
        ))),
    }
}

#[derive(Debug)]
struct TrialScore {
    depth: u16,
    deepest: String,
    score_tick: Option<u64>,
    goal_reached: bool,
    milestone_observations: BTreeMap<String, MilestoneObservation>,
    boundary_fingerprints: BTreeMap<String, BoundaryFingerprint>,
    value_projections: BTreeMap<String, BTreeMap<String, ValueProjectionEvidence>>,
}

#[derive(Deserialize)]
struct NativeMilestoneResult {
    schema: NativeSchema,
    boot: Option<TapeBoot>,
    boot_origin_established: Option<bool>,
    goal: Option<String>,
    goal_reached: bool,
    program_digest: Option<String>,
    milestones: Vec<NativeMilestone>,
}

#[derive(Deserialize)]
struct NativeSchema {
    name: String,
    version: u32,
}

#[derive(Deserialize)]
struct NativeMilestone {
    id: String,
    hit: bool,
    sim_tick: Option<u64>,
    tape_frame: Option<u64>,
    phase: Option<String>,
    stable_ticks: Option<u16>,
    definition_digest: Option<String>,
    program_digest: Option<String>,
    boundary_index: Option<u64>,
    evidence: Option<NativeEvidence>,
    projections: Option<Vec<ValueProjectionEvidence>>,
}

#[derive(Deserialize)]
struct NativeEvidence {
    boundary_fingerprint: BoundaryFingerprint,
    boot: Option<TapeBoot>,
    stage: Option<NativeStageEvidence>,
    next_stage: Option<NativeNextStageEvidence>,
    player: Option<NativePlayerEvidence>,
}

#[derive(Deserialize)]
struct NativeStageEvidence {
    name: String,
    room: i32,
    point: i32,
}

#[derive(Deserialize)]
struct NativeNextStageEvidence {
    enabled: bool,
    name: String,
    room: i32,
    point: i32,
}

#[derive(Deserialize)]
struct NativePlayerEvidence {
    present: bool,
    is_link: bool,
    procedure_id: u16,
}

fn parse_anchored_milestones(
    path: &Path,
    objective: &PreparedAnchoredObjective,
    expected_boot: &TapeBoot,
) -> Result<TrialScore, EvaluateError> {
    let native: NativeMilestoneResult =
        serde_json::from_slice(&fs::read(path).map_err(|error| {
            EvaluateError::NativeResult(format!(
                "worker produced no readable milestone result at {}: {error}",
                path.display()
            ))
        })?)?;
    if native.schema.name != "dusklight.automation.milestones"
        || !matches!(native.schema.version, 1 | 2 | 3 | 4 | 5)
    {
        return Err(EvaluateError::NativeResult(
            "unsupported native milestone schema".into(),
        ));
    }
    validate_native_boot(&native, expected_boot)?;
    if native.program_digest.as_deref()
        != Some(objective.identity.milestone_program_sha256.as_str())
    {
        return Err(EvaluateError::NativeResult(
            "native result milestone program digest does not match the anchored objective".into(),
        ));
    }
    if native.goal.as_deref() != Some(objective.identity.goal_milestone.as_str()) {
        return Err(EvaluateError::NativeResult(format!(
            "native result goal {:?} does not match anchored goal {}",
            native.goal, objective.identity.goal_milestone
        )));
    }
    let mut milestones = BTreeMap::new();
    for milestone in native.milestones {
        let id = milestone.id.clone();
        if milestones.insert(id.clone(), milestone).is_some() {
            return Err(EvaluateError::NativeResult(format!(
                "duplicate native milestone {id}"
            )));
        }
    }
    let requested = [
        objective.identity.source_milestone.as_str(),
        objective.identity.goal_milestone.as_str(),
    ];
    if milestones.len() != requested.len()
        || requested.iter().any(|id| !milestones.contains_key(*id))
    {
        return Err(EvaluateError::NativeResult(
            "native result does not contain the exact anchored milestone set".into(),
        ));
    }
    let expected = |id: &str| {
        if id == objective.identity.source_milestone {
            &objective.source
        } else {
            &objective.goal
        }
    };
    let mut observations = BTreeMap::new();
    let mut fingerprints = BTreeMap::new();
    let mut value_projections = BTreeMap::new();
    for (id, milestone) in &milestones {
        let definition = expected(id);
        if milestone.phase.as_deref() != Some(definition.phase.as_str())
            || milestone.stable_ticks != Some(definition.stable_ticks)
            || milestone.definition_digest.as_deref() != Some(definition.digest.as_str())
            || milestone.program_digest.as_deref()
                != Some(objective.identity.milestone_program_sha256.as_str())
        {
            return Err(EvaluateError::NativeResult(format!(
                "milestone {id} authored proof metadata does not match the anchored objective"
            )));
        }
        if milestone.hit {
            if native.schema.version >= 5 && milestone.projections.is_none() {
                return Err(EvaluateError::NativeResult(format!(
                    "milestone {id} omitted its value projection evidence"
                )));
            }
            let projections = validate_value_projections(milestone.projections.as_ref())?;
            if projections.len() != definition.projections.len()
                || definition.projections.iter().any(|(name, identity)| {
                    projections.get(name).map(|projection| &projection.identity) != Some(identity)
                })
            {
                return Err(EvaluateError::NativeResult(format!(
                    "milestone {id} value projection identities do not match the authored program"
                )));
            }
            if !projections.is_empty() {
                value_projections.insert(id.clone(), projections);
            }
        } else if milestone.projections.is_some() {
            return Err(EvaluateError::NativeResult(format!(
                "unhit milestone {id} contains value projection evidence"
            )));
        }
        match (
            milestone.hit,
            milestone.boundary_index,
            milestone.sim_tick,
            milestone.tape_frame,
            &milestone.evidence,
        ) {
            (true, Some(boundary_index), Some(sim_tick), Some(tape_frame), Some(evidence)) => {
                if boundary_index != tape_frame.saturating_add(1) || sim_tick != tape_frame {
                    return Err(EvaluateError::NativeResult(format!(
                        "milestone {id} tick, tape frame, and boundary index are not one absolute fixed-step boundary"
                    )));
                }
                validate_fingerprint(&evidence.boundary_fingerprint)?;
                validate_evidence_boot(evidence, native.schema.version, expected_boot)?;
                observations.insert(
                    id.clone(),
                    MilestoneObservation {
                        sim_tick,
                        tape_frame,
                        boundary_index: Some(boundary_index),
                        phase: milestone.phase.clone(),
                        stable_ticks: milestone.stable_ticks,
                        definition_digest: milestone.definition_digest.clone(),
                        program_digest: milestone.program_digest.clone(),
                    },
                );
                fingerprints.insert(id.clone(), evidence.boundary_fingerprint.clone());
            }
            (false, None, None, None, None) => {}
            _ => {
                return Err(EvaluateError::NativeResult(format!(
                    "milestone {id} has inconsistent authored hit evidence"
                )));
            }
        }
    }
    let source = &milestones[&objective.identity.source_milestone];
    if !source.hit {
        return Err(EvaluateError::NativeResult(
            "immutable prefix did not reproduce the anchored source milestone".into(),
        ));
    }
    if source.tape_frame != Some(objective.identity.source_tape_frame)
        || source.boundary_index != Some(objective.identity.source_boundary_index)
        || fingerprints[&objective.identity.source_milestone].digest
            != objective.identity.source_boundary_fingerprint
    {
        return Err(EvaluateError::NativeResult(
            "immutable prefix source frame, boundary index, or fingerprint changed".into(),
        ));
    }
    let goal = &milestones[&objective.identity.goal_milestone];
    if native.goal_reached != goal.hit {
        return Err(EvaluateError::NativeResult(
            "goal_reached disagrees with the authored anchored goal".into(),
        ));
    }
    let score_tick = if goal.hit {
        let goal_frame = goal.tape_frame.expect("hit tuple checked above");
        if goal_frame < objective.identity.prefix_frames {
            return Err(EvaluateError::NativeResult(
                "anchored goal fired inside the immutable prefix".into(),
            ));
        }
        let evidence = goal.evidence.as_ref().expect("hit tuple checked above");
        let stage = evidence.stage.as_ref().ok_or_else(|| {
            EvaluateError::NativeResult("anchored goal evidence has no stage object".into())
        })?;
        match objective.identity.segment {
            SegmentProfile::Fsp103ToFsp104 => {
                let next_stage = evidence.next_stage.as_ref().ok_or_else(|| {
                    EvaluateError::NativeResult(
                        "Ordon transition goal evidence has no next_stage object".into(),
                    )
                })?;
                if stage.name != "F_SP103"
                    || stage.room != 1
                    || !next_stage.enabled
                    || next_stage.name != "F_SP104"
                    || next_stage.room != 1
                    || next_stage.point != 0
                {
                    return Err(EvaluateError::NativeResult(
                        "anchored goal evidence is not the committed F_SP103 to F_SP104 room 1 spawn 0 transition"
                            .into(),
                    ));
                }
            }
            SegmentProfile::LinkControlToTunnelCrawlStart => {
                let player = evidence.player.as_ref().ok_or_else(|| {
                    EvaluateError::NativeResult("tunnel goal evidence has no player object".into())
                })?;
                if stage.name != "F_SP104"
                    || stage.room != 1
                    || stage.point != 0
                    || !player.present
                    || !player.is_link
                    || player.procedure_id != 53
                {
                    return Err(EvaluateError::NativeResult(
                        "anchored goal evidence is not F_SP104 room 1 spawn 0 crawl_start (53)"
                            .into(),
                    ));
                }
            }
            SegmentProfile::BootToFsp103 => unreachable!("validated anchored profile"),
        }
        Some(goal_frame - objective.identity.source_boundary_index)
    } else {
        Some(0)
    };
    Ok(TrialScore {
        depth: if goal.hit { 2 } else { 1 },
        deepest: if goal.hit {
            objective.identity.goal_milestone.clone()
        } else {
            objective.identity.source_milestone.clone()
        },
        score_tick,
        goal_reached: goal.hit,
        milestone_observations: observations,
        boundary_fingerprints: fingerprints,
        value_projections,
    })
}

fn parse_native_milestones(
    path: &Path,
    segment: SegmentProfile,
    expected_boot: &TapeBoot,
) -> Result<TrialScore, EvaluateError> {
    let native: NativeMilestoneResult =
        serde_json::from_slice(&fs::read(path).map_err(|error| {
            EvaluateError::NativeResult(format!(
                "worker produced no readable milestone result at {}: {error}",
                path.display()
            ))
        })?)?;
    if native.schema.name != "dusklight.automation.milestones"
        || !matches!(native.schema.version, 1 | 2 | 3 | 4 | 5)
    {
        return Err(EvaluateError::NativeResult(
            "unsupported native milestone schema".into(),
        ));
    }
    validate_native_boot(&native, expected_boot)?;
    let expected_goal = match segment {
        SegmentProfile::BootToFsp103 => "gameplay-ready-f-sp103",
        SegmentProfile::Fsp103ToFsp104 => "entered-f-sp104",
        SegmentProfile::LinkControlToTunnelCrawlStart => {
            unreachable!("anchored profiles are evaluated through evaluate_anchored_population")
        }
    };
    if native.goal.as_deref() != Some(expected_goal) {
        return Err(EvaluateError::NativeResult(format!(
            "native result goal {:?} does not match {expected_goal}",
            native.goal
        )));
    }
    let mut milestones = BTreeMap::new();
    for milestone in native.milestones {
        let id = milestone.id.clone();
        if milestones.insert(id.clone(), milestone).is_some() {
            return Err(EvaluateError::NativeResult(format!(
                "duplicate native milestone {id}"
            )));
        }
    }
    let requested: &[&str] = match segment {
        SegmentProfile::BootToFsp103 => &["gameplay-ready-f-sp103"],
        SegmentProfile::Fsp103ToFsp104 => &[
            "gameplay-ready-f-sp103",
            "exit-f-sp103-to-f-sp104",
            "entered-f-sp104",
        ],
        SegmentProfile::LinkControlToTunnelCrawlStart => {
            unreachable!("anchored profiles are evaluated through evaluate_anchored_population")
        }
    };
    if milestones.len() != requested.len()
        || requested.iter().any(|id| !milestones.contains_key(*id))
    {
        return Err(EvaluateError::NativeResult(
            "native result does not contain the exact requested milestone set".into(),
        ));
    }
    let mut fingerprints = BTreeMap::new();
    let mut observations = BTreeMap::new();
    let value_projections = BTreeMap::new();
    for (id, milestone) in &milestones {
        match (
            milestone.hit,
            milestone.sim_tick,
            milestone.tape_frame,
            &milestone.evidence,
        ) {
            (true, Some(sim_tick), Some(tape_frame), Some(evidence)) => {
                validate_fingerprint(&evidence.boundary_fingerprint)?;
                validate_evidence_boot(evidence, native.schema.version, expected_boot)?;
                observations.insert(
                    id.clone(),
                    MilestoneObservation {
                        sim_tick,
                        tape_frame,
                        boundary_index: None,
                        phase: None,
                        stable_ticks: None,
                        definition_digest: None,
                        program_digest: None,
                    },
                );
                fingerprints.insert(id.clone(), evidence.boundary_fingerprint.clone());
            }
            (false, None, None, None) => {}
            _ => {
                return Err(EvaluateError::NativeResult(format!(
                    "milestone {id} has inconsistent hit evidence"
                )));
            }
        }
    }
    let hit = |id: &str| milestones[id].hit;
    let tick = |id: &str| milestones[id].sim_tick;
    if native.goal_reached != hit(expected_goal) {
        return Err(EvaluateError::NativeResult(
            "goal_reached disagrees with the goal milestone".into(),
        ));
    }
    let (depth, deepest, score_tick) = match segment {
        SegmentProfile::BootToFsp103 if hit("gameplay-ready-f-sp103") => {
            (2, "gameplay-ready-f-sp103", tick("gameplay-ready-f-sp103"))
        }
        SegmentProfile::BootToFsp103 => (0, "none", None),
        SegmentProfile::Fsp103ToFsp104 if hit("entered-f-sp104") => {
            if !hit("exit-f-sp103-to-f-sp104") {
                return Err(EvaluateError::NativeResult(
                    "entered F_SP104 without the required source-exit milestone".into(),
                ));
            }
            (4, "entered-f-sp104", tick("exit-f-sp103-to-f-sp104"))
        }
        SegmentProfile::Fsp103ToFsp104 if hit("exit-f-sp103-to-f-sp104") => (
            3,
            "exit-f-sp103-to-f-sp104",
            tick("exit-f-sp103-to-f-sp104"),
        ),
        SegmentProfile::Fsp103ToFsp104 if hit("gameplay-ready-f-sp103") => {
            (2, "gameplay-ready-f-sp103", tick("gameplay-ready-f-sp103"))
        }
        SegmentProfile::Fsp103ToFsp104 => (0, "none", None),
        SegmentProfile::LinkControlToTunnelCrawlStart => {
            unreachable!("anchored profiles are evaluated through evaluate_anchored_population")
        }
    };
    if segment == SegmentProfile::Fsp103ToFsp104
        && hit("exit-f-sp103-to-f-sp104")
        && !hit("gameplay-ready-f-sp103")
    {
        return Err(EvaluateError::NativeResult(
            "source exit was hit without the gameplay-ready prerequisite".into(),
        ));
    }
    Ok(TrialScore {
        depth,
        deepest: deepest.into(),
        score_tick,
        goal_reached: native.goal_reached,
        milestone_observations: observations,
        boundary_fingerprints: fingerprints,
        value_projections,
    })
}

fn validate_native_boot(
    native: &NativeMilestoneResult,
    expected_boot: &TapeBoot,
) -> Result<(), EvaluateError> {
    if native.schema.version < 3 {
        if *expected_boot != TapeBoot::Process {
            return Err(EvaluateError::NativeResult(
                "legacy native milestone result cannot authenticate a stage boot origin".into(),
            ));
        }
        return Ok(());
    }
    if native.boot.as_ref() != Some(expected_boot) {
        return Err(EvaluateError::NativeResult(format!(
            "native milestone boot origin {:?} does not match tape origin {:?}",
            native.boot, expected_boot
        )));
    }
    if native.boot_origin_established != Some(true) {
        return Err(EvaluateError::NativeResult(
            "native milestone result did not establish its declared boot origin".into(),
        ));
    }
    Ok(())
}

fn validate_evidence_boot(
    evidence: &NativeEvidence,
    schema_version: u32,
    expected_boot: &TapeBoot,
) -> Result<(), EvaluateError> {
    if schema_version >= 3 && evidence.boot.as_ref() != Some(expected_boot) {
        return Err(EvaluateError::NativeResult(
            "native boundary evidence lost or changed its boot origin".into(),
        ));
    }
    Ok(())
}

fn validate_fingerprint(fingerprint: &BoundaryFingerprint) -> Result<(), EvaluateError> {
    let supported_contract = (fingerprint.schema == "dusklight.milestone-boundary/v1"
        && fingerprint.canonical_encoding == "little-endian-fixed-v1")
        || (fingerprint.schema == "dusklight.milestone-boundary/v2"
            && fingerprint.canonical_encoding == "little-endian-fixed-v2")
        || (fingerprint.schema == "dusklight.milestone-boundary/v3"
            && fingerprint.canonical_encoding == "little-endian-fixed-v3")
        || (fingerprint.schema == "dusklight.milestone-boundary/v4"
            && fingerprint.canonical_encoding == "little-endian-fixed-v4");
    if !supported_contract
        || fingerprint.algorithm != "xxh3-128"
        || fingerprint.digest.len() != 32
        || !fingerprint
            .digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(EvaluateError::NativeResult(
            "invalid native boundary fingerprint".into(),
        ));
    }
    Ok(())
}

fn validate_value_projections(
    projections: Option<&Vec<ValueProjectionEvidence>>,
) -> Result<BTreeMap<String, ValueProjectionEvidence>, EvaluateError> {
    let mut output = BTreeMap::new();
    for projection in projections.into_iter().flatten() {
        if projection.name.is_empty()
            || projection.name.len() > 96
            || projection.identity.len() != 64
            || !projection
                .identity
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            || projection.values.is_empty()
        {
            return Err(EvaluateError::NativeResult(
                "invalid native value projection identity".into(),
            ));
        }
        let all_items_available = projection
            .values
            .iter()
            .all(|value| value.get("available").and_then(serde_json::Value::as_bool) == Some(true));
        if projection.available != all_items_available {
            return Err(EvaluateError::NativeResult(format!(
                "value projection {:?} availability disagrees with its items",
                projection.name
            )));
        }
        match (&projection.value_fingerprint, projection.available) {
            (Some(fingerprint), true)
                if fingerprint.schema == "dusklight.value-projection/v1"
                    && fingerprint.algorithm == "xxh3-128"
                    && fingerprint.canonical_encoding == "little-endian-exact-v1"
                    && fingerprint.digest.len() == 32
                    && fingerprint
                        .digest
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()) => {}
            (None, false) => {}
            _ => {
                return Err(EvaluateError::NativeResult(format!(
                    "value projection {:?} has an invalid value fingerprint",
                    projection.name
                )));
            }
        }
        if output
            .insert(projection.name.clone(), projection.clone())
            .is_some()
        {
            return Err(EvaluateError::NativeResult(format!(
                "duplicate native value projection {:?}",
                projection.name
            )));
        }
    }
    Ok(output)
}

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
mod minimize_tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn attempt_outcomes_keep_all_terminal_classes_distinct() {
        let class = |timed_out, cancelled, trace, infrastructure, goal| {
            classify_outcome(timed_out, cancelled, trace, infrastructure, goal).class
        };
        assert_eq!(
            class(false, false, None, None, true),
            EpisodeOutcomeClass::Successful
        );
        assert_eq!(
            class(false, false, None, None, false),
            EpisodeOutcomeClass::Failed
        );
        assert_eq!(
            class(
                false,
                false,
                None,
                Some("invalid native milestone result: worker exit None disagrees"),
                false,
            ),
            EpisodeOutcomeClass::Crashed
        );
        assert_eq!(
            class(true, false, None, None, false),
            EpisodeOutcomeClass::TimedOut
        );
        assert_eq!(
            class(
                false,
                false,
                None,
                Some("invalid native milestone result: bad boundary digest"),
                false,
            ),
            EpisodeOutcomeClass::Desynced
        );
        assert_eq!(
            class(
                false,
                false,
                None,
                Some("could not launch Dusklight: unsupported executable"),
                false,
            ),
            EpisodeOutcomeClass::Unsupported
        );
        assert_eq!(
            class(
                false,
                false,
                Some("gameplay trace capacity was exhausted"),
                None,
                false,
            ),
            EpisodeOutcomeClass::Truncated
        );
    }

    fn proven(sim_tick: u64, tape_frame: u64, digest: &str) -> ProvenBootCandidate {
        let candidate = Candidate::baseline(SegmentProfile::BootToFsp103);
        ProvenBootCandidate {
            tape: candidate.compile().unwrap(),
            candidate,
            sim_tick,
            tape_frame,
            boundary_fingerprint: BoundaryFingerprint {
                schema: "dusklight.milestone-boundary/v1".into(),
                algorithm: "xxh3-128".into(),
                canonical_encoding: "little-endian-fixed-v1".into(),
                digest: digest.into(),
            },
        }
    }

    #[test]
    fn boot_reduction_target_rejects_later_or_different_proof() {
        let source = proven(439, 439, &"a".repeat(32));
        let target = BootReductionTarget {
            sim_tick: source.sim_tick,
            tape_frame: source.tape_frame,
            boundary_fingerprint: source.boundary_fingerprint.clone(),
        };
        assert!(target.accepts(&source));
        assert!(!target.accepts(&proven(440, 439, &"a".repeat(32))));
        assert!(!target.accepts(&proven(439, 440, &"a".repeat(32))));
        assert!(!target.accepts(&proven(439, 439, &"b".repeat(32))));
    }

    #[test]
    fn anchored_parser_requires_exact_program_source_and_crawl_evidence() {
        assert!(validate_anchored_game_args(&["--stage".into(), "F_SP103,1,1,3".into()]).is_err());
        assert!(validate_anchored_game_args(&["--stage=F_SP103,1,1,3".into()]).is_err());
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("huntctl-anchored-proof-{unique}"));
        fs::create_dir_all(&root).unwrap();
        let prefix_path = root.join("prefix.tape");
        let prefix = InputTape {
            tick_rate_numerator: 30,
            tick_rate_denominator: 1,
            frames: vec![crate::tape::InputFrame::default(); 2],
            ..InputTape::default()
        };
        fs::write(&prefix_path, prefix.encode().unwrap()).unwrap();
        let program = crate::milestone_dsl::compile_source(
            r#"milestones 1.0
milestone link_control {
  phase post_sim
  when stage.name == "F_SP103"
}
milestone tunnel_crawl_start {
  phase post_sim
  when stage.name == "F_SP104" && stage.room == 1 && stage.spawn == 0 && player.procedure == "crawl_start"
}
"#,
        )
        .unwrap();
        let program_path = root.join("objective.dmsp");
        fs::write(&program_path, &program.bytes).unwrap();
        let game_path = root.join("game.exe");
        let dvd_path = root.join("disc.iso");
        fs::write(&game_path, b"game-build").unwrap();
        fs::write(&dvd_path, b"disc-build").unwrap();
        let prepared = prepare_anchored_objective(
            &AnchoredObjectiveConfig {
                segment: SegmentProfile::LinkControlToTunnelCrawlStart,
                prefix_tape: prefix_path,
                milestone_program: program_path,
                game: game_path,
                dvd: dvd_path,
                source_milestone: "link_control".into(),
                source_boundary_fingerprint: "a".repeat(32),
                goal_milestone: "tunnel_crawl_start".into(),
            },
            root.join("runtime.dmsp"),
        )
        .unwrap();
        let fingerprint = |digest: String| {
            serde_json::json!({
                "schema": "dusklight.milestone-boundary/v1",
                "algorithm": "xxh3-128",
                "canonical_encoding": "little-endian-fixed-v1",
                "digest": digest,
            })
        };
        let authored = |id: &str,
                        definition: &AuthoredDefinitionExpectation,
                        sim_tick: u64,
                        tape_frame: u64,
                        boundary_index: u64,
                        digest: String,
                        goal: bool| {
            serde_json::json!({
                "id": id,
                "hit": true,
                "phase": definition.phase,
                "stable_ticks": definition.stable_ticks,
                "definition_digest": definition.digest,
                "program_digest": prepared.identity.milestone_program_sha256,
                "boundary_index": boundary_index,
                "sim_tick": sim_tick,
                "tape_frame": tape_frame,
                "evidence": {
                    "stage": {
                        "name": if goal { "F_SP104" } else { "F_SP103" },
                        "room": 1,
                        "point": if goal { 0 } else { 1 },
                    },
                    "player": {
                        "present": true,
                        "is_link": true,
                        "procedure_id": if goal { 53 } else { 3 },
                    },
                    "boundary_fingerprint": fingerprint(digest),
                }
            })
        };
        let result = serde_json::json!({
            "schema": {"name": "dusklight.automation.milestones", "version": 1},
            "goal": "tunnel_crawl_start",
            "goal_reached": true,
            "program_digest": prepared.identity.milestone_program_sha256,
            "milestones": [
                authored("link_control", &prepared.source, 1, 1, 2, "a".repeat(32), false),
                authored("tunnel_crawl_start", &prepared.goal, 2, 2, 3, "b".repeat(32), true),
            ],
        });
        let result_path = root.join("result.json");
        fs::write(&result_path, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
        let score = parse_anchored_milestones(&result_path, &prepared, &TapeBoot::Process).unwrap();
        assert!(score.goal_reached);
        assert_eq!(score.depth, 2);
        assert_eq!(score.score_tick, Some(0));

        let suffix_path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../routes/intro/segments/human420.tape");
        let suffix = InputTape::decode(&fs::read(suffix_path).unwrap())
            .unwrap()
            .tape;
        let candidate =
            Candidate::from_absolute_tape(SegmentProfile::LinkControlToTunnelCrawlStart, &suffix)
                .unwrap();
        let population_root = root.join("population");
        let manifest = write_explicit_population(
            &population_root,
            SegmentProfile::LinkControlToTunnelCrawlStart,
            0,
            vec![candidate],
        )
        .unwrap();
        let trials = build_anchored_trials(
            &manifest,
            &fs::canonicalize(&population_root).unwrap(),
            &root.join("attempts"),
            1,
            &prepared,
        )
        .unwrap();
        let full = InputTape::decode(&fs::read(&trials[0].tape).unwrap())
            .unwrap()
            .tape;
        assert_eq!(full.frames.len(), prefix.frames.len() + suffix.frames.len());
        assert_eq!(
            &full.frames[..prefix.frames.len()],
            prefix.frames.as_slice()
        );
        assert_eq!(
            &full.frames[prefix.frames.len()..],
            suffix.frames.as_slice()
        );
        bind_population_objective(&population_root, &prepared.identity).unwrap();
        bind_population_objective(&population_root, &prepared.identity).unwrap();
        let mut different_objective = prepared.identity.clone();
        different_objective.digest = "d".repeat(64);
        assert!(bind_population_objective(&population_root, &different_objective).is_err());

        let member_tape = population_root.join(&manifest.members[0].tape_file);
        let mut tampered = suffix.clone();
        tampered.frames[0].pads[0].buttons ^= 0x0100;
        fs::write(&member_tape, tampered.encode().unwrap()).unwrap();
        assert!(
            build_anchored_trials(
                &manifest,
                &fs::canonicalize(&population_root).unwrap(),
                &root.join("tampered-attempts"),
                1,
                &prepared,
            )
            .is_err()
        );

        let mut wrong = result;
        wrong["milestones"][0]["evidence"]["boundary_fingerprint"]["digest"] =
            serde_json::Value::String("c".repeat(32));
        fs::write(&result_path, serde_json::to_vec_pretty(&wrong).unwrap()).unwrap();
        assert!(parse_anchored_milestones(&result_path, &prepared, &TapeBoot::Process).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn named_value_parity_is_equal_different_or_incomparable_without_topology() {
        let projection = |identity: &str, digest: &str, available: bool| ValueProjectionEvidence {
            name: "handoff-state".into(),
            identity: identity.into(),
            available,
            value_fingerprint: available.then(|| BoundaryFingerprint {
                schema: "dusklight.value-projection/v1".into(),
                algorithm: "xxh3-128".into(),
                canonical_encoding: "little-endian-exact-v1".into(),
                digest: digest.into(),
            }),
            values: vec![serde_json::json!({"kind":"flag", "available":available})],
        };
        let reference = projection(&"a".repeat(64), &"1".repeat(32), true);
        assert_eq!(
            compare_value_projections(
                &reference,
                &projection(&"a".repeat(64), &"1".repeat(32), true)
            ),
            ValueParityComparison::Equal
        );
        assert_eq!(
            compare_value_projections(
                &reference,
                &projection(&"a".repeat(64), &"2".repeat(32), true)
            ),
            ValueParityComparison::Different
        );
        assert_eq!(
            compare_value_projections(
                &reference,
                &projection(&"b".repeat(64), &"1".repeat(32), true)
            ),
            ValueParityComparison::Incomparable
        );
        assert_eq!(
            compare_value_projections(
                &reference,
                &projection(&"a".repeat(64), &"1".repeat(32), false)
            ),
            ValueParityComparison::Incomparable
        );
    }

    #[test]
    fn archive_context_partitions_named_rng_actors_and_downstream_boundaries() {
        let boundary = |digest: &str| BoundaryFingerprint {
            schema: "dusklight.milestone-boundary/v4".into(),
            algorithm: "xxh3-128".into(),
            canonical_encoding: "little-endian-fixed-v4".into(),
            digest: digest.into(),
        };
        let projection = |digest: &str| ValueProjectionEvidence {
            name: "handoff-state".into(),
            identity: "a".repeat(64),
            available: true,
            value_fingerprint: Some(BoundaryFingerprint {
                schema: "dusklight.value-projection/v1".into(),
                algorithm: "xxh3-128".into(),
                canonical_encoding: "little-endian-exact-v1".into(),
                digest: digest.into(),
            }),
            values: vec![
                serde_json::json!({"kind":"rng", "available":true}),
                serde_json::json!({"kind":"actor_population", "available":true}),
            ],
        };
        let boundaries = BTreeMap::from([("goal".into(), boundary(&"1".repeat(32)))]);
        let projections = |digest: &str| {
            BTreeMap::from([(
                "goal".into(),
                BTreeMap::from([("handoff-state".into(), projection(digest))]),
            )])
        };
        let reference = archive_behavior_context_from_evidence(
            &boundaries,
            &projections(&"2".repeat(32)),
            Some("c".repeat(64)),
        );
        assert!(reference.objective_rng_identity.is_some());
        assert!(reference.actor_population_identity.is_some());
        assert_eq!(reference.contact_behavior_identity, Some("c".repeat(64)));
        assert!(reference.boundary_state_identity.is_some());
        assert!(reference.downstream_state_identity.is_some());

        let changed_projection = archive_behavior_context_from_evidence(
            &boundaries,
            &projections(&"3".repeat(32)),
            Some("d".repeat(64)),
        );
        assert_ne!(
            reference.objective_rng_identity,
            changed_projection.objective_rng_identity
        );
        assert_ne!(
            reference.actor_population_identity,
            changed_projection.actor_population_identity
        );
        assert_ne!(
            reference.downstream_state_identity,
            changed_projection.downstream_state_identity
        );
        assert_ne!(
            reference.contact_behavior_identity,
            changed_projection.contact_behavior_identity
        );

        let changed_boundary = BTreeMap::from([("goal".into(), boundary(&"4".repeat(32)))]);
        let changed_downstream = archive_behavior_context_from_evidence(
            &changed_boundary,
            &projections(&"2".repeat(32)),
            Some("c".repeat(64)),
        );
        assert_eq!(
            reference.objective_rng_identity,
            changed_downstream.objective_rng_identity
        );
        assert_eq!(
            reference.actor_population_identity,
            changed_downstream.actor_population_identity
        );
        assert_ne!(
            reference.boundary_state_identity,
            changed_downstream.boundary_state_identity
        );
        assert_ne!(
            reference.downstream_state_identity,
            changed_downstream.downstream_state_identity
        );
    }

    #[test]
    fn contact_behavior_identity_is_portable_and_run_deduplicated() {
        fn contact_trace(flags: u32, owner: u32, records: usize) -> crate::trace::DecodedTrace {
            let collision = crate::trace::TracePlayerBackgroundCollision {
                flags,
                ground_height: 12.0,
                roof_height: 100.0,
                water_height: -100.0,
                ground_bg_index: Some(2),
                ground_poly_index: Some(3),
                ground_owner_session_process_id: Some(owner),
                ground_plane: [0.0, 1.0, 0.0, -12.0],
                ground_identity_present: true,
                roof_bg_index: None,
                roof_poly_index: None,
                roof_owner_session_process_id: None,
                roof_identity_present: false,
                water_bg_index: None,
                water_poly_index: None,
                water_owner_session_process_id: None,
                water_identity_present: false,
                walls: std::array::from_fn(|_| crate::trace::TraceCollisionWall {
                    identity_present: false,
                    bg_index: None,
                    poly_index: None,
                    owner_session_process_id: Some(owner),
                    angle_y: 0,
                    flags: 0,
                }),
                old_position: [1.0, 2.0, 3.0],
                resolved_frame_displacement: [1.0, 0.0, 0.0],
                final_position: [2.0, 2.0, 3.0],
            };
            crate::trace::DecodedTrace {
                version: 5,
                boot: TapeBoot::Process,
                tick_rate_numerator: 30,
                tick_rate_denominator: 1,
                requested_channels: 0,
                capacity_exhausted: false,
                retention: None,
                channel_formats: BTreeMap::new(),
                records: (0..records)
                    .map(|_| crate::trace::TraceRecord {
                        player_background_collision: Some(collision.clone()),
                        ..crate::trace::TraceRecord::default()
                    })
                    .collect(),
            }
        }

        let contact_identity = |trace| {
            crate::semantic_novelty::SemanticNoveltyDescriptor::from_trace(&trace, Vec::new())
                .unwrap()
                .axis_identities()
                .contacts
                .unwrap()
        };
        let reference = contact_identity(contact_trace(1, 7, 1));
        assert_eq!(reference, contact_identity(contact_trace(1, 99, 3)));
        assert_ne!(reference, contact_identity(contact_trace(2, 7, 1)));
    }
}
