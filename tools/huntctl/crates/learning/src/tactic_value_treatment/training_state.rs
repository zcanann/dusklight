//! Binary state of the deployed Bellman regressor, not another copy of replay.
use super::*;

const SCHEMA: &str = "dusklight-bellman-training-state/v1";
pub const MAX_BELLMAN_STATE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TrainingState {
    schema: String,
    model: ContinuousTacticValueModel,
}

impl ContinuousTacticValueModel {
    pub fn bellman_training(&self) -> Option<&crate::fqi::ParameterizedTraining> {
        self.forest.parameterized_training()
    }

    pub fn training_state_bytes(&self) -> Result<Vec<u8>, GeneralizedTacticValueError> {
        if self.bellman_training().is_none() || self.bellman_stats.is_none() {
            return Err(GeneralizedTacticValueError::InvalidConfig);
        }
        let bytes = serde_cbor::to_vec(&TrainingState {
            schema: SCHEMA.into(),
            model: self.clone(),
        })
        .map_err(invalid)?;
        if bytes.len() > MAX_BELLMAN_STATE_BYTES {
            return Err(invalid("Bellman training state exceeds 16 MiB"));
        }
        Ok(bytes)
    }

    pub fn from_training_state_bytes(bytes: &[u8]) -> Result<Self, GeneralizedTacticValueError> {
        if bytes.len() > MAX_BELLMAN_STATE_BYTES {
            return Err(invalid("Bellman training state exceeds 16 MiB"));
        }
        let state: TrainingState = serde_cbor::from_slice(bytes).map_err(invalid)?;
        if state.schema != SCHEMA
            || state.model.bellman_training().is_none()
            || state
                .model
                .bellman_stats
                .as_ref()
                .is_none_or(|stats| stats.native_rows < 2)
        {
            return Err(invalid("invalid Bellman training state"));
        }
        Ok(state.model)
    }
}

fn invalid(error: impl std::fmt::Display) -> GeneralizedTacticValueError {
    GeneralizedTacticValueError::InvalidTransition(error.to_string())
}
