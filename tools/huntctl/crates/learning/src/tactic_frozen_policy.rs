//! Immutable, independently reloadable greedy policy for tactic-level Q learning.
//!
//! The existing option-value batch and fitted-Q implementation remain the sole
//! training authority. A frozen policy seals the exact batch and configuration
//! needed to deterministically reconstruct that model, plus the full executable
//! tactic-universe identity against which it may be run.

use crate::artifact::Digest;
use crate::option_values::{
    OptionValueBatch, OptionValueConfig, OptionValueError, OptionValueModel,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const TACTIC_FROZEN_POLICY_SCHEMA_V1: &str = "dusklight-tactic-frozen-policy/v1";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticFrozenPolicy {
    pub schema: String,
    pub content_sha256: Digest,
    pub source_campaign_sha256: Digest,
    pub root_checkpoint_sha256: Digest,
    pub root_state_sha256: Digest,
    pub feature_schema_sha256: Digest,
    pub action_universe_sha256: Digest,
    pub objective_sha256: Digest,
    pub training_batch_sha256: Digest,
    pub model_artifact_sha256: Digest,
    pub training_batch: OptionValueBatch,
    pub config: OptionValueConfig,
}

impl TacticFrozenPolicy {
    #[allow(clippy::too_many_arguments)]
    pub fn freeze(
        source_campaign_sha256: Digest,
        root_checkpoint_sha256: Digest,
        root_state_sha256: Digest,
        feature_schema_sha256: Digest,
        action_universe_sha256: Digest,
        objective_sha256: Digest,
        training_batch: OptionValueBatch,
        config: OptionValueConfig,
    ) -> Result<Self, TacticFrozenPolicyError> {
        training_batch.validate()?;
        let training_batch_sha256 = digest_json(&training_batch)?;
        let model = OptionValueModel::fit_batch(&training_batch, &config)?;
        let model_artifact_sha256 = digest_bytes(&model.artifact_bytes(&training_batch, &config)?);
        let mut policy = Self {
            schema: TACTIC_FROZEN_POLICY_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            source_campaign_sha256,
            root_checkpoint_sha256,
            root_state_sha256,
            feature_schema_sha256,
            action_universe_sha256,
            objective_sha256,
            training_batch_sha256,
            model_artifact_sha256,
            training_batch,
            config,
        };
        policy.content_sha256 = policy.compute_identity()?;
        policy.validate()?;
        Ok(policy)
    }

    pub fn validate(&self) -> Result<(), TacticFrozenPolicyError> {
        self.training_batch.validate()?;
        if self.schema != TACTIC_FROZEN_POLICY_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.source_campaign_sha256 == Digest::ZERO
            || self.root_checkpoint_sha256 == Digest::ZERO
            || self.root_state_sha256 == Digest::ZERO
            || self.feature_schema_sha256 == Digest::ZERO
            || self.action_universe_sha256 == Digest::ZERO
            || self.objective_sha256 == Digest::ZERO
            || self.training_batch.feature_schema != self.feature_schema_sha256
            || self.training_batch.objective_sha256 != self.objective_sha256
            || self.training_batch_sha256 != digest_json(&self.training_batch)?
            || self.content_sha256 != self.compute_identity()?
        {
            return Err(TacticFrozenPolicyError::Invalid(
                "frozen tactic policy envelope or training identity is invalid",
            ));
        }
        let model = OptionValueModel::fit_batch(&self.training_batch, &self.config)?;
        let model_bytes = model.artifact_bytes(&self.training_batch, &self.config)?;
        if self.model_artifact_sha256 != digest_bytes(&model_bytes) {
            return Err(TacticFrozenPolicyError::Invalid(
                "frozen tactic policy does not reconstruct its sealed fitted-Q model",
            ));
        }
        Ok(())
    }

    pub fn reconstruct_model(&self) -> Result<OptionValueModel, TacticFrozenPolicyError> {
        self.validate()?;
        OptionValueModel::fit_batch(&self.training_batch, &self.config).map_err(Into::into)
    }

    fn compute_identity(&self) -> Result<Digest, TacticFrozenPolicyError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        digest_json(&canonical)
    }
}

fn digest_json(value: &impl Serialize) -> Result<Digest, TacticFrozenPolicyError> {
    Ok(digest_bytes(&serde_json::to_vec(value)?))
}

fn digest_bytes(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

#[derive(Debug)]
pub enum TacticFrozenPolicyError {
    Invalid(&'static str),
    Values(OptionValueError),
    Serialization(serde_json::Error),
}

impl fmt::Display for TacticFrozenPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => formatter.write_str(message),
            Self::Values(error) => write!(formatter, "frozen tactic policy values failed: {error}"),
            Self::Serialization(error) => {
                write!(
                    formatter,
                    "frozen tactic policy serialization failed: {error}"
                )
            }
        }
    }
}

impl Error for TacticFrozenPolicyError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Values(error) => Some(error),
            Self::Serialization(error) => Some(error),
            Self::Invalid(_) => None,
        }
    }
}

impl From<OptionValueError> for TacticFrozenPolicyError {
    fn from(value: OptionValueError) -> Self {
        Self::Values(value)
    }
}

impl From<serde_json::Error> for TacticFrozenPolicyError {
    fn from(value: serde_json::Error) -> Self {
        Self::Serialization(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fqi::FqiConfig;
    use crate::option_execution::{OptionType, TapeRange};
    use crate::option_values::{OptionActionDescriptor, OptionValueSample};
    use std::collections::BTreeMap;

    fn sample() -> OptionValueSample {
        OptionValueSample {
            action: OptionActionDescriptor {
                option_id: "wait".into(),
                option_type: OptionType::Neutral,
                parameters: BTreeMap::new(),
            },
            state: vec![0.0],
            duration_ticks: 1,
            reward: 1.0,
            next_state: vec![1.0],
            terminal: true,
            before_state_sha256: Digest([7; 32]),
            after_state_sha256: Digest([8; 32]),
            source_checkpoint_sha256: Digest([9; 32]),
            next_checkpoint_sha256: Digest([10; 32]),
            realized_tape_range: TapeRange {
                start_frame: 0,
                end_frame_exclusive: 1,
            },
            realized_tape_sha256: Digest([11; 32]),
        }
    }

    #[test]
    fn frozen_policy_round_trips_and_reconstructs_the_exact_model() {
        let batch =
            OptionValueBatch::new(Digest([1; 32]), Digest([2; 32]), 1, vec![sample()], vec![0])
                .unwrap();
        let policy = TacticFrozenPolicy::freeze(
            Digest([3; 32]),
            Digest([4; 32]),
            Digest([7; 32]),
            Digest([1; 32]),
            Digest([5; 32]),
            Digest([2; 32]),
            batch,
            OptionValueConfig {
                fitted_q: FqiConfig {
                    iterations: 1,
                    trees_per_action: 1,
                    bootstrap: false,
                    ..FqiConfig::default()
                },
            },
        )
        .unwrap();
        let bytes = serde_json::to_vec(&policy).unwrap();
        let decoded: TacticFrozenPolicy = serde_json::from_slice(&bytes).unwrap();
        decoded.validate().unwrap();
        let model = decoded.reconstruct_model().unwrap();
        assert_eq!(model.feature_width(), 1);
        assert_eq!(model.actions()[0].option_id, "wait");
    }

    #[test]
    fn altered_training_data_cannot_reseal_itself_accidentally() {
        let batch =
            OptionValueBatch::new(Digest([1; 32]), Digest([2; 32]), 1, vec![sample()], vec![0])
                .unwrap();
        let mut policy = TacticFrozenPolicy::freeze(
            Digest([3; 32]),
            Digest([4; 32]),
            Digest([7; 32]),
            Digest([1; 32]),
            Digest([5; 32]),
            Digest([2; 32]),
            batch,
            OptionValueConfig {
                fitted_q: FqiConfig {
                    iterations: 1,
                    trees_per_action: 1,
                    bootstrap: false,
                    ..FqiConfig::default()
                },
            },
        )
        .unwrap();
        policy.training_batch.samples[0].reward = 2.0;
        assert!(policy.validate().is_err());
    }
}
