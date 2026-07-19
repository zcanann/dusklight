//! Shared identities emitted by native objective evaluation.

use dusklight_search::search::SegmentProfile;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AnchoredObjectiveIdentity {
    pub schema: String,
    pub segment: SegmentProfile,
    pub digest: String,
    pub prefix_sha256: String,
    pub prefix_frames: u64,
    pub milestone_program_sha256: String,
    pub game_sha256: String,
    pub dvd_sha256: String,
    pub source_milestone: String,
    pub source_definition_sha256: String,
    pub source_boundary_fingerprint: String,
    pub source_tape_frame: u64,
    pub source_boundary_index: u64,
    pub goal_milestone: String,
    pub goal_definition_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BoundaryFingerprint {
    pub schema: String,
    pub algorithm: String,
    pub canonical_encoding: String,
    pub digest: String,
}
