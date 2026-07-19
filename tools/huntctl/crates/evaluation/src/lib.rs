//! Authenticated population evaluation and evidence interpretation.
//!
//! This crate may execute already-materialized candidates through the harness
//! runtime and attach objective evidence. It does not define harness request
//! truth, learner implementations, proposal policies, or top-level campaigns.

pub use dusklight_automation_contracts::{artifact, candidate_envelope, compatibility, tape};
pub use dusklight_control::tape_chain;
pub use dusklight_evidence::{content_store, episode, transition_corpus, transition_evidence};
pub use dusklight_harness_runtime as harness;
pub use dusklight_learning as learning;
pub use dusklight_learning::{
    dataset, evaluation_isolation, offline_rl, online_lineage, planning_priors,
};
pub use dusklight_objectives::milestone_dsl;
pub use dusklight_proposals::{behavior_archive, q_search};
pub use dusklight_search::{bayesian_search, continuous_search, search};
pub use dusklight_semantic_novelty as semantic_novelty;
pub use dusklight_trace::trace;

pub mod search_evaluator;
pub mod harness_authority;

pub use search_evaluator::*;
