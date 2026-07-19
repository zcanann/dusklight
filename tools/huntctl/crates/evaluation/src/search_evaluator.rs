//! Native, cross-platform population evaluation and multi-generation search.

use crate::artifact::Digest as ArtifactDigest;
use crate::bayesian_search::BayesianSnapshot;
use crate::behavior_archive::{BehaviorArchive, BehaviorContext, describe_behavior_with_context};
use crate::candidate_envelope::{
    CandidateEnvelope, CandidateEnvelopeSet, NamedDigest, ProposerIdentity, ProposerKind,
};
use crate::compatibility::{CompatibilityMode, ensure_compatible};
use crate::content_store::{ContentBlob, ContentKind, ContentStore};
use crate::continuous_search::{
    ContinuousAxes, ContinuousMethod, ContinuousOptimizerSnapshot,
};
use crate::dataset::{DATASET_SOURCE_SCHEMA_V1, DatasetSourceDescriptor};
use crate::episode::{
    EPISODE_CONTEXT_SCHEMA_V1, EpisodeContext, EpisodeIntervention, EpisodeLedger, EpisodeLineage,
    EpisodeManifest, EpisodeManifestBuild, EpisodeObjectiveIdentity, EpisodeOutcome,
    EpisodeOutcomeClass, EpisodeProducerIdentity, EpisodeProducerKind, EpisodeSeed,
    RunBuildIdentity,
};
pub use crate::harness::evaluation::{AnchoredObjectiveIdentity, BoundaryFingerprint};
use crate::harness::execution::execute_request;
use crate::harness::objective_suite::{ArtifactReference, ObjectiveSeed};
use crate::harness::run_contract::{HarnessRunRequest, HarnessRunResult, HarnessTerminalReason};
use crate::learning::evaluation_isolation::{
    EvaluationAttemptInput, EvaluationGenerationSeal, EvaluationOutcomeCollection,
    EvaluationOutcomeInput,
};
use crate::learning::online_lineage::{OnlineDatasetGeneration, OnlineModelLineage};
use crate::learning::planning_priors::QBeamPriorTable;
use crate::offline_rl::{
    ExploratoryExtractConfig, extract_exploratory_v2_from_bytes, movement_action_schema_digest_v2,
};
use crate::q_search::{
    QEpisode, QProposalConfig, QProposalReadinessEvidence, propose_q_candidates_with_lineage,
};
use crate::search::{
    Ancestry, Candidate, CandidateResult, EvolutionConfig, LeaderboardEntry, LexicographicScore,
    MacroAction, POPULATION_SCHEMA, PopulationManifest, RESULTS_SCHEMA, SearchResults,
    SegmentProfile,
    evolve_population_with_retained_and_proposals, rank_population, tape_input_complexity,
    write_explicit_population, write_seed_population,
};
use crate::semantic_novelty::catalog::{
    SemanticNoveltyAssessment, SemanticNoveltyCatalog, SemanticNoveltyCatalogConfig,
};
use crate::semantic_novelty::proposal_signal::{
    SemanticNoveltyProposalSignal, SemanticNoveltyProposalSignalConfig,
};
use crate::semantic_novelty::{BoundaryFingerprintFact, SemanticNoveltyDescriptor};
use crate::tape::{InputTape, TapeBoot};
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

#[derive(Clone, Debug)]
pub struct AnchoredSearchRunConfig {
    pub search: SearchRunConfig,
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

impl TournamentProposerKind {
    const fn envelope_kind(self) -> ProposerKind {
        match self {
            Self::IncumbentMutation => ProposerKind::Scripted,
            Self::BlindExploration => ProposerKind::Random,
            Self::Structured => ProposerKind::StructuredSearch,
            Self::Learned => ProposerKind::Learned,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TournamentProposer {
    pub name: String,
    pub kind: TournamentProposerKind,
    pub population: PathBuf,
    pub proposal_envelopes: PathBuf,
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
    pub harness: Option<HarnessEvaluateConfig>,
    /// Optional clean-boot prefix objective. This is mutually exclusive with
    /// the core-harness request mode and keeps route suffixes on the same fair
    /// tournament boundary as directly bootable candidates.
    pub anchored: Option<AnchoredObjectiveConfig>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProposerTournamentRow {
    pub name: String,
    pub kind: TournamentProposerKind,
    pub proposer: ProposerIdentity,
    pub proposal_envelope_set_sha256: ArtifactDigest,
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
    pub boundary_fingerprints: Vec<String>,
    pub cold_replay_pass_rate: f64,
    pub replay_verdict: ProposerReplayVerdict,
    pub best_candidate_id: String,
    pub best_score: LexicographicScore,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best_proved_tape: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best_proved_tape_sha256: Option<ArtifactDigest>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposerReplayVerdict {
    Proved,
    ObjectiveMiss,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProposerTournamentSummary {
    pub schema: &'static str,
    pub segment: SegmentProfile,
    pub boot: TapeBoot,
    pub objective: NamedDigest,
    pub action_schema: NamedDigest,
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
pub struct AnchoredRouteMinimizeConfig {
    pub candidate: Candidate,
    pub objective: AnchoredObjectiveConfig,
    pub output_root: PathBuf,
    pub working_directory: PathBuf,
    pub game_args_prefix: Vec<String>,
    pub workers: usize,
    pub repetitions: u32,
    pub candidate_budget: usize,
    pub timeout: Duration,
}

#[derive(Clone, Debug, Serialize)]
pub struct AnchoredRouteMinimizeRound {
    pub round: u32,
    pub operation: String,
    pub evaluated_candidates: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepted_candidate_id: Option<String>,
    pub retained_frames: u64,
    pub retained_actions: usize,
    pub retained_input_complexity: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct AnchoredRouteMinimizeSummary {
    pub schema: &'static str,
    pub objective: AnchoredObjectiveIdentity,
    pub source_candidate_id: String,
    pub minimized_candidate_id: String,
    pub source_frames: u64,
    pub minimized_frames: u64,
    pub source_actions: usize,
    pub minimized_actions: usize,
    pub source_input_complexity: u64,
    pub minimized_input_complexity: u64,
    pub goal_first_hit_tick: u64,
    pub goal_sim_tick: u64,
    pub goal_tape_frame: u64,
    pub goal_boundary_fingerprint: String,
    pub evaluated_candidates: usize,
    pub accepted_reductions: usize,
    pub candidate: PathBuf,
    pub suffix_tape: PathBuf,
    pub realized_tape: PathBuf,
    pub source_proof: PathBuf,
    pub final_proof: PathBuf,
    pub reduction_history: PathBuf,
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
}

fn prepare_anchored_objective(
    config: &AnchoredObjectiveConfig,
    runtime_program: PathBuf,
) -> Result<PreparedAnchoredEvaluator, EvaluateError> {
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

mod tournament;
pub use tournament::run_proposer_tournament;
#[cfg(test)]
use tournament::{learned_holdout_scores_adequate, native_terminals_support_required_facts};
use tournament::{
    learned_proposal_held_out_performance, required_native_facts_supported,
};
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
    write_initial_proposal_envelopes(
        &manifest,
        &population_root,
        NamedDigest::new(
            prepared.identity.goal_milestone.clone(),
            prepared.identity.digest.parse().map_err(|error| {
                EvaluateError::InvalidResult(format!("invalid anchored objective digest: {error}"))
            })?,
        ),
        search.population_size,
    )?;
    let mut final_results = None;
    let mut training_corpora = BTreeMap::<String, TransitionCorpus>::new();
    let mut previous_dataset_generation: Option<OnlineDatasetGeneration> = None;
    let mut previous_model_lineage: Option<OnlineModelLineage> = None;
    let mut initial_learned_trial_consumed = false;
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
        let mut evaluation_outcomes = Vec::with_capacity(report.attempts.len());
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
            evaluation_outcomes.push(EvaluationOutcomeInput {
                candidate_id: attempt.candidate_id.clone(),
                attempt: attempt.attempt,
                outcome: attempt.outcome.class,
                // The immutable anchored source is a prerequisite, not route
                // progress. Source-only misses are ordinary failures; authored
                // milestones between source and goal define near misses.
                milestone_depth: attempt.milestone_depth.saturating_sub(1),
                goal_reached: attempt.goal_reached,
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
        let outcome_collection =
            EvaluationOutcomeCollection::build(&evaluation_seal, &evaluation_outcomes)
                .map_err(|error| EvaluateError::InvalidResult(error.to_string()))?;
        write_json(
            &population_root.join("evaluation-outcomes.json"),
            &outcome_collection,
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
                objective: NamedDigest::new(
                    prepared.identity.goal_milestone.clone(),
                    prepared.identity.digest.parse().map_err(|error| {
                        EvaluateError::InvalidResult(format!(
                            "invalid anchored objective digest: {error}"
                        ))
                    })?,
                ),
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
            let initial_bounded_trial = !initial_learned_trial_consumed;
            let readiness = QProposalReadinessEvidence {
                required_facts_supported: required_native_facts_supported(&report.attempts),
                determinism_proved: report.repetitions >= 2
                    && report.attempts.iter().all(|attempt| {
                        attempt.harness_terminal != Some(HarnessTerminalReason::Nondeterministic)
                    }),
                held_out_performance_adequate: !initial_bounded_trial
                    && learned_proposal_held_out_performance(&manifest, &leaderboard),
                initial_bounded_trial,
            };
            let q_result = match dataset_generation.as_ref() {
                Some(dataset_generation)
                    if outcome_collection.required_mix_complete
                        && q_budget > 0
                        && !q_episodes.is_empty() =>
                {
                    propose_q_candidates_with_lineage(
                        &corpora,
                        &q_episodes,
                        QProposalConfig {
                            generation: generation + 1,
                            max_proposals: q_budget,
                            iterations: 12,
                            trees_per_action: 15,
                            seed: search.rng_seed + u64::from(generation) + 1,
                            readiness,
                        },
                        dataset_generation,
                        previous_model_lineage.as_ref(),
                    )
                    .map_err(|error| error.to_string())
                }
                _ if !outcome_collection.required_mix_complete => Err(
                    "sealed evaluation generation has no complete success/near-miss/ordinary-failure mix"
                        .to_string(),
                ),
                _ => Err(
                    "no non-elite slots, aligned elite episodes, or sealed training generation is available"
                        .to_string(),
                ),
            };
            let q_candidates = match q_result {
                Ok(batch) => {
                    if batch.summary.proposal_gate.initial_bounded_trial
                        && batch.summary.proposal_gate.learned_policy_enabled
                        && batch
                            .summary
                            .collection_schedule
                            .iter()
                            .any(|lane| matches!(*lane, "guided_exploit" | "ensemble_disagreement"))
                    {
                        initial_learned_trial_consumed = true;
                    }
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
                            "envelopes": batch.envelopes,
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

fn write_initial_proposal_envelopes(
    manifest: &PopulationManifest,
    population_root: &Path,
    objective: NamedDigest,
    population_size: usize,
) -> Result<(), EvaluateError> {
    let configuration = serde_json::to_vec(&(
        "dusklight-anchored-seed-population/v1",
        manifest.segment,
        &manifest.boot,
        manifest.rng_seed,
        population_size,
    ))?;
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.anchored-seed-population/v1\0");
    hasher.update((configuration.len() as u64).to_le_bytes());
    hasher.update(configuration);
    let configuration_sha256 = ArtifactDigest(hasher.finalize().into());
    let mut envelopes = Vec::with_capacity(manifest.members.len());
    for member in &manifest.members {
        let candidate_sha256 = member.candidate_id.parse().map_err(|error| {
            EvaluateError::InvalidManifest(format!("invalid candidate digest: {error}"))
        })?;
        let parent_candidate_sha256 = member
            .ancestry
            .parent_id
            .as_deref()
            .map(str::parse)
            .transpose()
            .map_err(|error| {
                EvaluateError::InvalidManifest(format!("invalid parent candidate digest: {error}"))
            })?;
        let (kind, id) = if parent_candidate_sha256.is_some() {
            (ProposerKind::StructuredSearch, "search.seed-mutation")
        } else {
            (ProposerKind::Scripted, "scripted.observed-seed")
        };
        envelopes.push(
            CandidateEnvelope::build(
                candidate_sha256,
                parent_candidate_sha256,
                member.ancestry.generation,
                objective.clone(),
                NamedDigest::new("movement-action/v2", movement_action_schema_digest_v2()),
                manifest.rng_seed,
                ProposerIdentity {
                    kind,
                    id: id.into(),
                    version: env!("CARGO_PKG_VERSION").into(),
                    configuration_sha256,
                },
            )
            .map_err(|error| EvaluateError::InvalidManifest(error.to_string()))?,
        );
    }
    let set = CandidateEnvelopeSet::build(envelopes)
        .map_err(|error| EvaluateError::InvalidManifest(error.to_string()))?;
    write_json(&population_root.join("proposal-envelopes.json"), &set)
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
