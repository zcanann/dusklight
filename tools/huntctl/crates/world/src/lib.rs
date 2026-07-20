//! Read-only world data extraction and spatial queries.
//!
//! This crate depends only on portable contracts. It cannot acquire runtime
//! control, search, learning, route, or UI behavior.

pub use dusklight_automation_contracts::artifact;

pub mod actor_profile_catalog;
pub mod stage_boot_catalog;
pub mod world_context;
pub mod world_geometry;
pub mod world_inventory;
pub mod world_spatial;
