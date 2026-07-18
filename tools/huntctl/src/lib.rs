//! Rust control plane primitives for persistent Dusklight simulation workers.

pub mod action_guidance;
pub mod bayesian_search;
pub mod behavior_archive;
pub mod benchmark;
pub mod comparison_oracle;
pub mod continuous_search;
pub mod corpus_ops;
pub mod harness;
pub mod intervention;
pub mod learning;
pub mod milestone_dsl;
pub mod motion_path_golf;
pub mod option_diagnostics;
pub mod option_golf;
pub mod oracle_pipeline;
pub mod route_store;
pub mod route_workbench;
pub mod search;
pub mod search_evaluator;
pub mod semantic_novelty;
pub mod semantic_oracle;
pub mod tactic_tests;
pub mod timeline;
pub mod trace;
pub mod trace_diff;
pub mod transition_evidence;

// Compatibility re-exports keep existing callers stable while the physical
// source tree migrates behind compiler-enforced crate boundaries.
pub use dusklight_automation_contracts::{
    actor_identity, artifact, compatibility, controller_program, observation_view,
    scenario_fixture, tape,
};
pub use dusklight_control::{
    controller_compilation, game_tactic, motion_path, option_execution, roll_option, tape_chain,
    tape_dsl, tape_edit, tape_program,
};
pub use dusklight_evidence::{content_store, corpus, episode, transition_corpus};
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
