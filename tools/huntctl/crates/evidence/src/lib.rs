//! Immutable evidence primitives for Dusklight harness runs and learning.
//!
//! Search and learning may consume this crate. This crate cannot depend on
//! either domain, so episode truth and storage cannot acquire proposer logic.

// Preserve the existing internal paths while declaring their external owner.
pub use dusklight_automation_contracts::{artifact, tape};

pub mod content_store;
pub mod corpus;
pub mod episode;
pub mod transition_corpus;
