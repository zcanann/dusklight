//! Rust control plane primitives for persistent Dusklight simulation workers.

pub mod artifact;
pub mod client;
pub mod corpus;
pub mod pool;
pub mod protocol;
pub mod route_store;
pub mod route_workbench;
pub mod search;
pub mod search_evaluator;
pub mod tape;
pub mod tape_chain;
pub mod tape_dsl;
pub mod tape_program;
pub mod timeline;
pub mod trace;
pub mod transport;

pub use artifact::{ARTIFACT_SCHEMA_VERSION, ArtifactIdentity, BuildIdentity, Digest};
pub use client::{CONTROL_PROTOCOL_NAME, CONTROL_PROTOCOL_VERSION, ClientError, WorkerClient};
pub use protocol::PROTOCOL_VERSION;
