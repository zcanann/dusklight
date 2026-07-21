//! Bounded, deterministic learning primitives for Dusklight automation.
//!
//! This crate owns immutable datasets, model fitting, calibration, readiness
//! gates, and proposal-model artifacts. It cannot depend on search ranking,
//! native process execution, route state, workbench code, or CLI parsing.

// Preserve the former internal paths while enforcing their external owners.
pub use dusklight_automation_contracts::{artifact, observation_view, tape};
pub use dusklight_control::{game_tactic, option_diagnostics, option_execution};
pub use dusklight_evidence::{episode, transition_corpus, transition_evidence};
pub use dusklight_objectives::milestone_dsl;
pub use dusklight_trace::trace;
pub use dusklight_world::world_spatial;

pub mod action_guidance;
pub mod actor_set_representation;
pub mod batch;
pub mod calibration;
pub mod compiled_goal_graph;
pub mod dataset;
pub mod double_q;
pub mod dyna_mixture;
pub mod ensemble_q;
pub mod evaluation_isolation;
pub mod factorized_actions;
pub mod factorized_pad_action;
pub mod fqi;
pub mod frozen_inference;
pub mod goal_conditioning;
pub mod graph_representation;
pub mod hindsight;
pub mod history_critics;
pub mod inference_conformance;
pub mod inference_placement;
pub mod iql;
pub mod latent_model_admission;
pub mod local_dynamics;
pub mod low_data_baselines;
pub mod model_ownership;
pub mod model_representation;
pub mod native_actor_features;
pub mod native_actor_view;
pub mod native_collision_history;
pub mod native_collision_view;
pub mod native_geometry_view;
pub mod offline_rl;
pub mod online_lineage;
pub mod option_policy;
pub mod option_values;
pub mod planning_priors;
pub mod prioritized_replay;
pub mod rainbow;
pub mod reward_shaping;
pub mod rl_readiness;
pub mod semantic_goal_input;
pub mod trainable_set_encoder;
pub mod training_guard;
pub mod transfer_learning;
