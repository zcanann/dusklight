//! Authenticated contracts obeyed by every core-harness executor.

pub use dusklight_automation_contracts::{
    artifact, controller_program, observation_view, scenario_fixture, tape,
};
pub use dusklight_control::{tape_dsl, tape_program};
pub use dusklight_objectives::milestone_dsl;

pub mod objective_suite;
pub mod observation_contract;
pub mod run_contract;
