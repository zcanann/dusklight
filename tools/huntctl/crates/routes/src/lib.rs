//! Authored route models, validation, lineage resolution, and immutable storage.
//!
//! Workbench interaction and native execution remain root adapters. This crate
//! owns the route truth those adapters consume and mutate through typed APIs.

pub mod route_store;
pub mod timeline;

// Preserve concise module-local paths while making every owner a declared,
// one-way dependency rather than reaching back into root orchestration.
pub use dusklight_automation_contracts::tape;
pub use dusklight_control::{tape_dsl, tape_program};
pub use dusklight_objectives::milestone_dsl;
pub use dusklight_search::search;
