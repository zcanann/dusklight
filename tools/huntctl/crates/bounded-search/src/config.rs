use dusklight_automation_contracts::artifact::Digest as ArtifactDigest;
use dusklight_evaluation::{
    AnchoredObjectiveConfig, AnchoredObjectiveIdentity, HarnessEvaluateConfig,
};
use dusklight_learning::planning_priors::QBeamPriorTable;
use dusklight_search::bayesian_search::BayesianSnapshot;
use dusklight_search::continuous_search::{
    ContinuousAxes, ContinuousMethod, ContinuousOptimizerSnapshot,
};
use dusklight_search::search::{Candidate, LexicographicScore, MacroAction, SegmentProfile};
use serde::Serialize;
use std::path::PathBuf;
use std::time::Duration;

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
    pub score: LexicographicScore,
    pub output_root: PathBuf,
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
    pub harness: Option<HarnessEvaluateConfig>,
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
    pub harness: Option<HarnessEvaluateConfig>,
    /// When present, evaluate every semantic candidate as a suffix after this
    /// immutable prefix and rank it against the authenticated route goal.
    pub objective: Option<AnchoredObjectiveConfig>,
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
    pub objective: Option<AnchoredObjectiveIdentity>,
    pub final_optimizer: ContinuousOptimizerSnapshot,
    pub champion_id: String,
    pub champion_score: LexicographicScore,
    pub champion_values: Vec<f64>,
    pub champion_candidate: PathBuf,
    pub champion_suffix_tape: Option<PathBuf>,
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
    pub harness: Option<HarnessEvaluateConfig>,
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
