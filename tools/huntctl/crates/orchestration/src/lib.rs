//! Executable-facing composition of objective campaigns.

#![recursion_limit = "256"]

mod anchored_search;
mod campaign_replay;
mod compact_suffix_batch;
pub mod deterministic_fixture;
mod discovery_horizon;
pub mod generalized_tactic_evidence;
pub mod harness;
pub mod learner;
pub mod learning_value_comparison;
pub mod learning_value_evidence;
pub mod learning_value_matrix;
pub mod learning_value_report;
pub mod native_checkpoint_benchmark;
pub mod native_goal_learning_loop;
pub mod native_goal_learning_loop_runner;
pub mod native_residual_campaign;
pub mod native_residual_campaign_runner;
pub mod native_residual_winner_promotion;
pub mod native_residual_winner_selection;
mod native_scratch_deletion;
pub mod native_scratch_duration;
pub mod native_scratch_heading;
pub mod native_scratch_learner;
pub mod native_subsystem_parity;
pub mod native_suffix_result;
pub mod native_suffix_worker;
pub mod native_tactic_policy_runner;
pub mod native_tactic_route_runner;
pub mod native_tactic_worker;
pub mod optimization_request;
pub mod optimization_resume;
pub mod persistence;
pub mod reporting;
pub mod residual_campaign;
pub mod residual_campaign_audit;
pub mod residual_campaign_runner;
pub mod residual_critic_ranking;
pub mod residual_horizon_tightening;
pub mod residual_reverse_curriculum;
pub mod residual_winner_cold_replay;
pub mod residual_winner_minimization;
pub mod scheduler;
pub mod stage_actor_coverage;
pub mod stage_observation_coverage;
pub mod stage_survey;
mod stage_survey_artifact;
pub mod state_graph;
mod tactic_macro_store;
pub mod tactic_q_campaign;
mod tactic_q_checkpoint_store;
pub mod tactic_replay_control_plane;
pub mod tactics;
pub mod worker_pool;

pub use anchored_search::{
    ANCHORED_RUN_SCHEMA, AnchoredSearchRunConfig, AnchoredSearchRunSummary, run_anchored_search,
};
pub use dusklight_bounded_search::{
    BayesianSearchRunConfig, BayesianSearchRunSummary, BeamSearchConfig, BeamSearchSummary,
    ContinuousSearchRunConfig, ContinuousSearchRunSummary, SEARCH_RUN_SCHEMA, SearchRunConfig,
    SearchRunSummary, run_bayesian_search, run_beam_search, run_continuous_search, run_search,
};
pub use dusklight_evaluation::{
    ANCHORED_RESULTS_SCHEMA, ATTEMPT_SCHEMA, AnchoredEvaluateConfig, AnchoredObjectiveConfig,
    AnchoredObjectiveIdentity, AnchoredSearchResults, AttemptEvidence, BoundaryFingerprint,
    EVALUATION_SCHEMA, EvaluateConfig, EvaluateError, EvaluationReport, EvaluationWorkerSchedule,
    HarnessEvaluateConfig, MilestoneObservation, PlannedWorkerAssignment,
    PreparedAnchoredEvaluator, ValueParityComparison, ValueProjectionEvidence,
    WORKER_SCHEDULE_SCHEMA, anchored_objective_identity, attempt_behavior_context,
    attempt_semantic_novelty_descriptor, attempts_support_required_native_facts,
    compare_value_projections, derive_candidate_request, evaluate_anchored_population,
    evaluate_population, evaluate_prepared_anchored_population, learned_proposals_pass_holdout,
    prepare_anchored_evaluator, search_evaluator,
};
pub use dusklight_finalist_reduction::{
    AnchoredInputGolfConfig, AnchoredInputGolfRound, AnchoredInputGolfSummary,
    AnchoredRouteMinimizeConfig, AnchoredRouteMinimizeRound, AnchoredRouteMinimizeSummary,
    BootGolfConfig, BootGolfSummary, BootMinimizeConfig, BootMinimizeSummary, golf_anchored_inputs,
    golf_boot, minimize_anchored_route, minimize_boot,
};
pub use dusklight_proposer_tournament::{
    ProposerReplayVerdict, ProposerTournamentConfig, ProposerTournamentRow,
    ProposerTournamentSummary, TournamentBudgetUnit, TournamentDefinition, TournamentProposer,
    TournamentProposerKind, run_proposer_tournament,
};

// Compatibility names serve executable-facing composition without restoring
// ownership of the underlying portable domains.
pub use dusklight_automation_contracts::{artifact, compatibility};
pub use dusklight_search::search;
