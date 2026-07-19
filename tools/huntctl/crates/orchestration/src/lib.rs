//! Executable-facing composition of objective campaigns.

mod anchored_search;
pub mod harness;
pub mod finalist_reduction;
mod search_drivers;
mod tournament;

pub use dusklight_evaluation::*;
pub use anchored_search::{
    ANCHORED_RUN_SCHEMA, AnchoredSearchRunConfig, AnchoredSearchRunSummary, run_anchored_search,
};
pub use finalist_reduction::{
    AnchoredRouteMinimizeConfig, AnchoredRouteMinimizeRound, AnchoredRouteMinimizeSummary,
    BootGolfConfig, BootGolfSummary, BootMinimizeConfig, BootMinimizeSummary, golf_boot,
    minimize_anchored_route, minimize_boot,
};
pub use search_drivers::{
    BayesianSearchRunConfig, BayesianSearchRunSummary, BeamSearchConfig, BeamSearchSummary,
    ContinuousSearchRunConfig, ContinuousSearchRunSummary, SearchRunConfig, SearchRunSummary,
    SEARCH_RUN_SCHEMA, run_bayesian_search, run_beam_search, run_continuous_search, run_search,
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
