//! Executable-facing composition of objective campaigns.

pub mod harness;
pub mod finalist_reduction;

pub use dusklight_evaluation::*;
pub use finalist_reduction::{golf_boot, minimize_anchored_route, minimize_boot};

// Compatibility names serve executable-facing composition without restoring
// ownership of the underlying portable domains.
pub use dusklight_automation_contracts::{artifact, compatibility};
pub use dusklight_search::search;
