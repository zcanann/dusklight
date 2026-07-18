//! Rust control plane primitives for persistent Dusklight simulation workers.

pub mod action_guidance;
pub mod artifact;
pub mod bayesian_search;
pub mod behavior_archive;
pub mod client;
pub mod comparison_oracle;
pub mod compatibility;
pub mod content_store;
pub mod continuous_search;
pub mod controller_compilation;
pub mod controller_program;
pub mod corpus;
pub mod corpus_ops;
pub mod episode;
pub mod game_tactic;
pub mod intervention;
pub mod learning;
pub mod milestone_dsl;
pub mod motion_path;
pub mod motion_path_golf;
pub mod observation_view;
pub mod option_diagnostics;
pub mod option_execution;
pub mod option_golf;
pub mod oracle_pipeline;
pub mod pool;
pub mod protocol;
pub mod roll_option;
pub mod route_store;
pub mod route_workbench;
pub mod scenario_fixture;
pub mod search;
pub mod search_evaluator;
pub mod semantic_novelty;
pub mod semantic_oracle;
pub mod tactic_tests;
pub mod tape;
pub mod tape_chain;
pub mod tape_dsl;
pub mod tape_edit;
pub mod tape_program;
pub mod timeline;
pub mod trace;
pub mod trace_diff;
pub mod transition_corpus;
pub mod transition_evidence;
pub mod transport;
pub mod world_geometry;
pub mod world_inventory;
pub mod world_spatial;

// Compatibility re-exports keep existing callers stable while the physical
// source tree migrates to domain folders.
pub use learning::{
    calibration, dataset, double_q, fqi, iql, low_data_baselines, offline_rl, q_search,
    reward_shaping,
};

pub use artifact::{ARTIFACT_SCHEMA_VERSION, ArtifactIdentity, BuildIdentity, Digest};
pub use client::{CONTROL_PROTOCOL_NAME, CONTROL_PROTOCOL_VERSION, ClientError, WorkerClient};
pub use compatibility::{CompatibilityDifference, CompatibilityMode, compatibility_differences};
pub use protocol::PROTOCOL_VERSION;
