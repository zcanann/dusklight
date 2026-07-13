//! Rust control plane primitives for persistent Dusklight simulation workers.

pub mod artifact;
pub mod client;
pub mod protocol;
pub mod tape;
pub mod tape_program;
pub mod transport;

pub use artifact::{ARTIFACT_SCHEMA_VERSION, ArtifactIdentity, BuildIdentity, Digest};
pub use client::{CONTROL_PROTOCOL_NAME, CONTROL_PROTOCOL_VERSION, ClientError, WorkerClient};
pub use protocol::PROTOCOL_VERSION;
