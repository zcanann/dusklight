//! Native execution of authenticated harness requests.
//!
//! This crate owns process launch, artifact capture, and human-readable run
//! inspection. It does not schedule campaigns, rank candidates, propose
//! actions, or train models.

pub use dusklight_automation_contracts::{artifact, controller_program, scenario_fixture, tape};
pub use dusklight_control::tape_dsl;
pub use dusklight_harness_contracts::{
    evaluation, native_evidence, objective_suite, observation_contract, run_contract,
};
pub use dusklight_objectives::milestone_dsl;
pub use dusklight_trace::trace;

pub mod execution;
pub mod inspection;
pub mod request_materialization;
