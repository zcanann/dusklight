use super::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BootGolfRunIdentity {
    pub(super) schema: String,
    pub(super) strategy: String,
    pub(super) source_candidate_id: String,
    pub(super) source_goal_sim_tick: u64,
    pub(super) source_goal_tape_frame: u64,
    pub(super) source_boundary_fingerprint: BoundaryFingerprint,
    pub(super) game_sha256: ArtifactDigest,
    pub(super) dvd_sha256: ArtifactDigest,
    pub(super) working_directory: PathBuf,
    pub(super) game_args_prefix: Vec<String>,
    pub(super) repetitions: u32,
    pub(super) timeout_millis: u64,
    pub(super) harness_request_sha256: Option<ArtifactDigest>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BootGolfCachedProof {
    pub(super) candidate_id: String,
    pub(super) sim_tick: u64,
    pub(super) tape_frame: u64,
    pub(super) boundary_fingerprint: BoundaryFingerprint,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BootGolfBatchCache {
    pub(super) schema: String,
    pub(super) content_sha256: ArtifactDigest,
    pub(super) run: BootGolfRunIdentity,
    pub(super) round: u32,
    pub(super) batch_index: usize,
    pub(super) candidate_ids: Vec<String>,
    pub(super) proven: Vec<BootGolfCachedProof>,
    pub(super) evaluation: PathBuf,
    pub(super) evaluation_sha256: ArtifactDigest,
    pub(super) results: PathBuf,
    pub(super) results_sha256: ArtifactDigest,
}

#[derive(Clone)]
pub(super) struct ProvenBootCandidate {
    pub(super) candidate: Candidate,
    pub(super) tape: InputTape,
    pub(super) sim_tick: u64,
    pub(super) tape_frame: u64,
    pub(super) boundary_fingerprint: BoundaryFingerprint,
}

#[derive(Clone)]
pub(super) struct BootReductionTarget {
    pub(super) sim_tick: u64,
    pub(super) tape_frame: u64,
    pub(super) boundary_fingerprint: BoundaryFingerprint,
}

impl BootReductionTarget {
    pub(super) fn accepts(&self, candidate: &ProvenBootCandidate) -> bool {
        candidate.sim_tick == self.sim_tick
            && candidate.tape_frame == self.tape_frame
            && candidate.boundary_fingerprint == self.boundary_fingerprint
    }
}

pub(super) fn alternate_menu_buttons(buttons: u16) -> Option<u16> {
    match buttons {
        BUTTON_A => Some(BUTTON_START),
        BUTTON_START => Some(BUTTON_A),
        _ => None,
    }
}

pub(super) fn pulse_frame_count(tape: &InputTape) -> usize {
    tape.frames
        .iter()
        .filter(|frame| frame.pads[0].buttons != 0)
        .count()
}
