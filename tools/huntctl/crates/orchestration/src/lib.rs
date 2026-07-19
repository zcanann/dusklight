//! Executable-facing composition of objective campaigns.

pub mod harness;
pub use dusklight_evaluation as search_evaluator;

// Campaign is the only implementation left here. These compatibility names
// serve that adapter without restoring ownership of the underlying domains.
pub use dusklight_automation_contracts::{artifact, compatibility};
pub use dusklight_search::search;
