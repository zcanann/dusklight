//! Versioned gameplay-trace decoding and typed-fact projection.

// These compatibility names let the extracted modules retain their concise
// internal paths without granting trace a dependency on the root CLI crate.
pub use dusklight_automation_contracts::{scenario_fixture, tape};

pub mod trace;
pub mod trace_typed_facts;
