//! Rust control plane primitives for persistent Dusklight simulation workers.

pub mod action_guidance;
pub mod behavior_archive;
pub mod benchmark;
pub mod corpus_ops;
pub mod harness;
pub mod learning;
pub mod motion_path_golf;
pub mod option_golf;
pub mod route_workbench;
pub mod search_evaluator;
pub mod tactic_tests;
pub mod trace_diff;

// Compatibility re-exports keep existing callers stable while the physical
// source tree migrates behind compiler-enforced crate boundaries.
pub use dusklight_automation_contracts::{
    actor_identity, artifact, candidate_envelope, compatibility, controller_program,
    observation_view, scenario_fixture, tape,
};
pub use dusklight_control::{
    controller_compilation, game_tactic, motion_path, option_diagnostics, option_execution,
    roll_option, tape_chain, tape_dsl, tape_edit, tape_program,
};
pub use dusklight_evidence::{
    content_store, corpus, episode, transition_corpus, transition_evidence,
};
pub use dusklight_interventions as intervention;
pub use dusklight_objectives::milestone_dsl;
pub use dusklight_oracles::{comparison_oracle, oracle_pipeline, semantic_oracle};
pub use dusklight_routes::{route_store, timeline};
pub use dusklight_search::{bayesian_search, continuous_search, search};
pub use dusklight_semantic_novelty as semantic_novelty;
pub use dusklight_trace::{trace, trace_typed_facts};
pub use dusklight_worker_protocol::{client, pool, protocol, transport};
pub use dusklight_world::{world_geometry, world_inventory, world_spatial};

pub use learning::{
    calibration, dataset, double_q, fqi, iql, low_data_baselines, offline_rl, q_search,
    reward_shaping,
};

pub use artifact::{
    ARTIFACT_SCHEMA_VERSION, ArtifactIdentity, ArtifactIdentityError, BuildIdentity, Digest,
};
pub use client::{CONTROL_PROTOCOL_NAME, CONTROL_PROTOCOL_VERSION, ClientError, WorkerClient};
pub use compatibility::{
    CompatibilityDifference, CompatibilityError, CompatibilityMode, compatibility_differences,
    ensure_compatible,
};
pub use protocol::PROTOCOL_VERSION;
