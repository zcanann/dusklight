//! Executable-facing composition of native runs, evaluators, campaigns, and proposers.

pub mod behavior_archive;
pub mod harness;
pub mod learning;
pub mod search_evaluator;

// Compatibility vocabulary for the adapters while callers migrate to direct domain crates.
// These remain one-way dependencies; no lower-level crate calls back into orchestration.
pub use dusklight_automation_contracts::{
    actor_identity, artifact, candidate_envelope, compatibility, controller_program,
    observation_view, scenario_fixture, tape,
};
pub use dusklight_control::{tape_chain, tape_dsl, tape_program};
pub use dusklight_evidence::{content_store, episode, transition_corpus, transition_evidence};
pub use dusklight_learning::{
    action_guidance, dataset, fqi, offline_rl, online_lineage, planning_priors,
};
pub use dusklight_objectives::milestone_dsl;
pub use dusklight_search::{bayesian_search, continuous_search, search};
pub use dusklight_semantic_novelty as semantic_novelty;
pub use dusklight_trace::trace;

pub use learning::q_search;
