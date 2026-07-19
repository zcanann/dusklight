//! Executable-facing composition of objective campaigns.

mod anchored_search;
pub mod harness;
mod tournament;

pub use dusklight_evaluation::*;
pub use anchored_search::{
    ANCHORED_RUN_SCHEMA, AnchoredSearchRunConfig, AnchoredSearchRunSummary, run_anchored_search,
};
pub use dusklight_finalist_reduction::*;
pub use dusklight_bounded_search::*;
pub use tournament::{
    ProposerReplayVerdict, ProposerTournamentConfig, ProposerTournamentRow,
    ProposerTournamentSummary, TournamentBudgetUnit, TournamentDefinition, TournamentProposer,
    TournamentProposerKind, run_proposer_tournament,
};

// Compatibility names serve executable-facing composition without restoring
// ownership of the underlying portable domains.
pub use dusklight_automation_contracts::{artifact, compatibility};
pub use dusklight_search::search;
