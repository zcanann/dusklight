//! Worker communication and local scheduling for Dusklight automation.
//!
//! This crate may depend on portable automation contracts, but cannot depend
//! on search, learning, route, workbench, or CLI orchestration code.

// Preserve the internal module path used by the binary frame codec while
// keeping the dependency direction explicit in Cargo metadata.
pub use dusklight_automation_contracts::artifact;

pub mod client;
pub mod pool;
pub mod protocol;
pub mod transport;
