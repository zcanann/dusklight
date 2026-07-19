//! Rust control plane primitives for persistent Dusklight simulation workers.

pub mod benchmark;
pub mod corpus_ops;

// Compatibility re-exports keep existing callers stable while the physical
// source tree migrates behind compiler-enforced crate boundaries.
pub use dusklight_automation_contracts::{
    actor_identity, artifact, candidate_envelope, compatibility, controller_program,
    observation_view, scenario_fixture, tape,
};
pub use dusklight_control::{
    controller_compilation, game_tactic, motion_path, option_diagnostics, option_execution,
    roll_option, tactic_tests, tape_chain, tape_dsl, tape_edit, tape_program,
};
pub use dusklight_evidence::{
    content_store, corpus, episode, trace_diff, transition_corpus, transition_evidence,
};
pub use dusklight_interventions as intervention;
pub use dusklight_objectives::milestone_dsl;
pub use dusklight_oracles::{comparison_oracle, oracle_pipeline, semantic_oracle};
pub use dusklight_orchestration::{behavior_archive, harness, learning, search_evaluator};
pub use dusklight_route_workbench as route_workbench;
pub use dusklight_routes::{route_store, timeline};
pub use dusklight_search::{
    bayesian_search, continuous_search, motion_path_golf, option_golf, search,
};
pub use dusklight_semantic_novelty as semantic_novelty;
pub use dusklight_trace::{trace, trace_typed_facts};
pub use dusklight_worker_protocol::{client, pool, protocol, transport};
pub use dusklight_world::{world_geometry, world_inventory, world_spatial};

pub use dusklight_orchestration::learning::{
    action_guidance, calibration, dataset, double_q, fqi, iql, low_data_baselines, offline_rl,
    q_search, reward_shaping,
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
