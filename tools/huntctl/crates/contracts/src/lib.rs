//! Portable automation contracts shared by every Dusklight harness domain.
//!
//! This crate deliberately has no worker, search, learning, route, UI, or
//! process-execution dependencies. Keeping those dependencies impossible is
//! the reason this is a crate rather than another folder in `huntctl`.

pub mod actor_identity;
pub mod artifact;
pub mod compatibility;
pub mod controller_program;
pub mod observation_view;
pub mod scenario_fixture;
pub mod tape;
