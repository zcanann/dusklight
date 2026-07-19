//! Immutable evidence primitives for Dusklight harness runs and learning.
//!
//! Search and learning may consume this crate. This crate cannot depend on
//! either domain, so episode truth and storage cannot acquire proposer logic.

// Preserve the existing internal paths while declaring their external owner.
pub use dusklight_automation_contracts::{artifact, run_terminal, tape};
pub use dusklight_control::option_execution;
pub use dusklight_trace::trace;

pub mod content_store;
pub mod corpus;
pub mod episode;
pub mod episode_store;
pub mod semantic_state_hash;
pub mod trace_diff;
pub mod transition_corpus;
pub mod transition_evidence;
