//! Rust control plane primitives for persistent Dusklight simulation workers.

pub mod benchmark;
pub mod corpus_ops;

// Compatibility re-exports keep existing callers stable while the physical
// source tree migrates behind compiler-enforced crate boundaries.
pub use dusklight_automation_contracts::{
    actor_identity, artifact, candidate_envelope, compatibility, controller_program,
    native_fidelity, observation_view, scenario_fixture, tape,
};
pub use dusklight_control::{
    controller_compilation, game_tactic, motion_path, option_diagnostics, option_execution,
    roll_option, tactic_tests, tape_chain, tape_dsl, tape_edit, tape_program,
};
pub use dusklight_evidence::{
    content_store, corpus, episode, episode_store, native_corpus_inspection, native_episode_shard,
    observation_parity, semantic_state_hash, trace_diff, transition_corpus, transition_evidence,
};
pub use dusklight_interventions as intervention;
pub use dusklight_learning as learning;
pub use dusklight_objectives::milestone_dsl;
pub use dusklight_oracles::{comparison_oracle, oracle_pipeline, semantic_oracle};
pub use dusklight_orchestration as search_evaluator;
pub use dusklight_orchestration::harness;
pub use dusklight_orchestration::stage_actor_coverage;
pub use dusklight_orchestration::stage_observation_coverage;
pub use dusklight_orchestration::stage_survey;
pub use dusklight_proposals::behavior_archive;
pub use dusklight_route_workbench as route_workbench;
pub use dusklight_routes::{route_store, timeline};
pub use dusklight_search::{
    bayesian_search, continuous_search, motion_path_golf, option_golf, search, suffix_batch,
};
pub use dusklight_semantic_novelty as semantic_novelty;
pub use dusklight_throughput_benchmark as throughput_benchmark;
pub use dusklight_trace::{route_diagnostics, trace, trace_typed_facts};
pub use dusklight_worker_protocol::{client, pool, protocol, transport};
pub use dusklight_world::{
    actor_profile_catalog, stage_boot_catalog, world_context, world_geometry, world_inventory,
    world_spatial, world_surface_graph,
};

pub use dusklight_learning::{
    action_guidance, calibration, dataset, double_q, fqi, iql, low_data_baselines,
    native_actor_view, native_collision_history, native_episode_history, native_geometry_view,
    native_resource_load_view, native_room_load_view, native_surface_graph_view, offline_rl,
    reward_shaping,
};
pub use dusklight_proposals::q_search;

pub use artifact::{
    ARTIFACT_SCHEMA_VERSION, ArtifactIdentity, ArtifactIdentityError, BuildIdentity, Digest,
};
pub use client::{CONTROL_PROTOCOL_NAME, CONTROL_PROTOCOL_VERSION, ClientError, WorkerClient};
pub use compatibility::{
    CompatibilityDifference, CompatibilityError, CompatibilityMode, compatibility_differences,
    ensure_compatible,
};
pub use protocol::PROTOCOL_VERSION;
