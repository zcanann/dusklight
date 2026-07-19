//! Candidate proposal policies over immutable evidence and learned models.
//!
//! This is the only lower-level crate where learned model outputs may become
//! ordinary search candidates. It cannot execute or score those candidates.

pub use dusklight_automation_contracts::{artifact, candidate_envelope, tape};
pub use dusklight_evidence::{episode, transition_corpus};
pub use dusklight_learning::{
    action_guidance, evaluation_isolation, fqi, offline_rl, online_lineage, training_guard,
};
pub use dusklight_search::search;

pub mod behavior_archive;
pub mod q_search;
