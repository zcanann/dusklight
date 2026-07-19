//! Input authoring and bounded control realization for Dusklight automation.
//!
//! This crate converts authored control intent into portable contract values.
//! It cannot depend on objectives, evidence, search, learning, routes, workers,
//! native process execution, or CLI code.

pub use dusklight_automation_contracts::{artifact, controller_program, tape};

pub mod controller_compilation;
pub mod game_tactic;
pub mod motion_path;
pub mod option_diagnostics;
pub mod option_execution;
pub mod roll_option;
pub mod tape_chain;
pub mod tape_dsl;
pub mod tape_edit;
pub mod tape_program;
