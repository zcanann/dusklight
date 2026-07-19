//! Bounded objective authoring, compilation, and offline trace evaluation.

// These are lower-level domain dependencies, not root-crate callbacks.
pub use dusklight_automation_contracts::{actor_identity, tape};
pub use dusklight_trace::{trace, trace_typed_facts};

pub mod milestone_dsl;
