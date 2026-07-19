//! Executable-facing composition of objective campaigns.

pub mod harness;
pub mod finalist_reduction;
mod search_drivers;
mod tournament;

pub use dusklight_evaluation::*;
pub use finalist_reduction::{
    AnchoredRouteMinimizeConfig, AnchoredRouteMinimizeRound, AnchoredRouteMinimizeSummary,
    BootGolfConfig, BootGolfSummary, BootMinimizeConfig, BootMinimizeSummary, golf_boot,
    minimize_anchored_route, minimize_boot,
};
pub use search_drivers::{
    BayesianSearchRunConfig, BayesianSearchRunSummary, BeamSearchConfig, BeamSearchSummary,
    ContinuousSearchRunConfig, ContinuousSearchRunSummary, run_bayesian_search, run_beam_search,
    run_continuous_search, run_search,
};
pub use tournament::{
    ProposerReplayVerdict, ProposerTournamentConfig, ProposerTournamentRow,
    ProposerTournamentSummary, TournamentBudgetUnit, TournamentDefinition, TournamentProposer,
    TournamentProposerKind, run_proposer_tournament,
};

// Compatibility names serve executable-facing composition without restoring
// ownership of the underlying portable domains.
pub use dusklight_automation_contracts::{artifact, compatibility};
pub use dusklight_search::search;
