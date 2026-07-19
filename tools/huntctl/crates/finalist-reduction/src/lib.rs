//! Finalist-reduction policy layered over authenticated native evaluation.

mod boot;
mod route;

pub use boot::{golf_boot, minimize_boot};
pub use route::minimize_anchored_route;

use dusklight_automation_contracts::artifact::Digest as ArtifactDigest;
#[cfg(test)]
use dusklight_automation_contracts::tape::TapeBoot;
use dusklight_automation_contracts::tape::{InputTape, RawPadState};
use dusklight_control::tape_chain::{ChainSegment, concatenate};
use dusklight_evaluation::{
    AnchoredEvaluateConfig, AnchoredObjectiveConfig, AnchoredObjectiveIdentity,
    BoundaryFingerprint, EvaluateConfig, EvaluateError, EvaluationReport, HarnessEvaluateConfig,
    PreparedAnchoredEvaluator, evaluate_population, evaluate_prepared_anchored_population,
    prepare_anchored_evaluator,
};
use dusklight_search::search::{
    Ancestry, Candidate, InterventionRange, MacroAction, SegmentProfile, tape_input_complexity,
    write_explicit_population,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

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
    pub resume: bool,
    pub timeout: Duration,
    pub harness: Option<HarnessEvaluateConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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
    pub harness: Option<HarnessEvaluateConfig>,
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
    pub harness: Option<HarnessEvaluateConfig>,
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

fn is_anchored_profile(profile: SegmentProfile) -> bool {
    matches!(
        profile,
        SegmentProfile::Fsp103ToFsp104 | SegmentProfile::LinkControlToTunnelCrawlStart
    )
}

fn directory_is_nonempty(path: &Path) -> Result<bool, EvaluateError> {
    Ok(path.is_dir() && fs::read_dir(path)?.next().transpose()?.is_some())
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), EvaluateError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
