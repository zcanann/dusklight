//! Bounded candidate, ranking, and optimizer primitives for Dusklight.
//!
//! This crate owns candidate representation and pure proposal/ranking policy.
//! It cannot execute the game, inspect native evidence, train models, mutate
//! routes, or parse CLI commands.

pub mod bayesian_search;
pub mod continuous_search;
pub mod search;

// Keep module-local paths stable while their owners remain explicit external
// crates in this manifest.
pub use dusklight_automation_contracts::tape;
pub use dusklight_control::{game_tactic, motion_path, option_execution, roll_option};
