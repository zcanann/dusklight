use super::*;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticQFinalResult {
    pub schema: String,
    pub content_sha256: Digest,
    #[serde(default, skip_serializing_if = "digest_is_zero")]
    pub execution_authority_sha256: Digest,
    pub objective_sha256: Digest,
    pub root_checkpoint_sha256: Digest,
    pub route_tape_sha256: Digest,
    pub replay_sha256: Digest,
    pub terminal_state_sha256: Digest,
    pub route_tape: InputTape,
    pub replay: Vec<OptionTransitionSample>,
    pub replay_routes: Vec<InputTape>,
    pub terminal: FactSnapshot,
}

impl TacticQFinalResult {
    pub fn write(&self, path: &Path) -> Result<(), TacticQCampaignError> {
        tactic_q_checkpoint_store::write_final_result(self, path)
    }

    pub fn read(path: &Path) -> Result<Self, TacticQCampaignError> {
        tactic_q_checkpoint_store::read_final_result(path)
    }
}
