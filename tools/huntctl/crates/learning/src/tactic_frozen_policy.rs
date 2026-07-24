//! Immutable, independently reloadable greedy policy for tactic-level Q learning.
//!
//! The existing option-value batch and fitted-Q implementation remain the sole
//! training authority. A frozen policy seals the exact batch and configuration
//! needed to deterministically reconstruct that model, plus the full executable
//! tactic-universe identity against which it may be run.

use crate::artifact::Digest;
use crate::option_values::{
    OptionActionDescriptor, OptionValueBatch, OptionValueConfig, OptionValueError,
    OptionValueModel,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

pub const TACTIC_FROZEN_POLICY_SCHEMA_V2: &str = "dusklight-tactic-frozen-policy/v2";

/// The exact tabular-Q decision for a state observed during training.
///
/// Fitted Q remains the fallback for novel states. Retaining the observed
/// greedy action prevents a sparse regression forest from overriding a known
/// successful branch with an extrapolated value at the same authenticated
/// state.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedGreedyTactic {
    pub state_sha256: Digest,
    pub action: OptionActionDescriptor,
    pub q_value: f64,
}

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
    pub observed_greedy: Vec<ObservedGreedyTactic>,
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
        let observed_greedy = derive_observed_greedy(&training_batch, &config)?;
        let mut policy = Self {
            schema: TACTIC_FROZEN_POLICY_SCHEMA_V2.into(),
            content_sha256: Digest::ZERO,
            source_campaign_sha256,
            root_checkpoint_sha256,
            root_state_sha256,
            feature_schema_sha256,
            action_universe_sha256,
            objective_sha256,
            training_batch_sha256,
            model_artifact_sha256,
            observed_greedy,
            training_batch,
            config,
        };
        policy.content_sha256 = policy.compute_identity()?;
        policy.validate()?;
        Ok(policy)
    }

    pub fn validate(&self) -> Result<(), TacticFrozenPolicyError> {
        self.training_batch.validate()?;
        if self.schema != TACTIC_FROZEN_POLICY_SCHEMA_V2
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
            || self.observed_greedy != derive_observed_greedy(&self.training_batch, &self.config)?
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

    pub fn observed_greedy_tactic(&self, state: Digest) -> Option<&ObservedGreedyTactic> {
        self.observed_greedy
            .binary_search_by_key(&state, |entry| entry.state_sha256)
            .ok()
            .map(|index| &self.observed_greedy[index])
    }

    fn compute_identity(&self) -> Result<Digest, TacticFrozenPolicyError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        digest_json(&canonical)
    }
}

fn derive_observed_greedy(
    batch: &OptionValueBatch,
    config: &OptionValueConfig,
) -> Result<Vec<ObservedGreedyTactic>, TacticFrozenPolicyError> {
    batch.validate()?;
    let mut samples = batch.samples.iter().collect::<Vec<_>>();
    samples.sort_by(|left, right| {
        right
            .realized_tape_range
            .start_frame
            .cmp(&left.realized_tape_range.start_frame)
            .then_with(|| left.before_state_sha256.cmp(&right.before_state_sha256))
            .then_with(|| left.action.option_id.cmp(&right.action.option_id))
    });

    let mut values = BTreeMap::<Digest, f64>::new();
    let mut choices = BTreeMap::<Digest, (f64, Vec<u8>, OptionActionDescriptor)>::new();
    for sample in samples {
        let continuation = if sample.terminal {
            0.0
        } else {
            values
                .get(&sample.after_state_sha256)
                .copied()
                .unwrap_or(0.0)
        };
        let discount =
            f64::from(config.fitted_q.discount).powi(sample.duration_ticks as i32);
        let q_value = f64::from(sample.reward) + discount * continuation;
        if !q_value.is_finite() {
            return Err(TacticFrozenPolicyError::Invalid(
                "observed tabular-Q value is not finite",
            ));
        }
        let action_key = serde_json::to_vec(&sample.action)?;
        let replace = choices
            .get(&sample.before_state_sha256)
            .is_none_or(|(prior_q, prior_key, _)| {
                q_value > *prior_q || (q_value == *prior_q && action_key < *prior_key)
            });
        if replace {
            choices.insert(
                sample.before_state_sha256,
                (q_value, action_key, sample.action.clone()),
            );
            values.insert(sample.before_state_sha256, q_value);
        }
    }
    Ok(choices
        .into_iter()
        .map(
            |(state_sha256, (q_value, _, action))| ObservedGreedyTactic {
                state_sha256,
                action,
                q_value,
            },
        )
        .collect())
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
        let observed = decoded.observed_greedy_tactic(Digest([7; 32])).unwrap();
        assert_eq!(observed.action.option_id, "wait");
        assert_eq!(observed.q_value, 1.0);
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

    #[test]
    fn observed_tabular_q_retains_the_successful_multi_step_branch() {
        let mut advance = sample();
        advance.action.option_id = "advance".into();
        advance.reward = 0.0;
        advance.terminal = false;
        advance.after_state_sha256 = Digest([8; 32]);

        let mut finish = sample();
        finish.action.option_id = "finish".into();
        finish.state = vec![1.0];
        finish.next_state = vec![2.0];
        finish.reward = 10.0;
        finish.before_state_sha256 = Digest([8; 32]);
        finish.after_state_sha256 = Digest([9; 32]);
        finish.source_checkpoint_sha256 = Digest([10; 32]);
        finish.next_checkpoint_sha256 = Digest([12; 32]);
        finish.realized_tape_range = TapeRange {
            start_frame: 1,
            end_frame_exclusive: 2,
        };
        finish.realized_tape_sha256 = Digest([13; 32]);

        let mut distractor = sample();
        distractor.action.option_id = "distractor".into();
        distractor.reward = 2.0;
        distractor.after_state_sha256 = Digest([14; 32]);
        distractor.next_checkpoint_sha256 = Digest([15; 32]);
        distractor.realized_tape_sha256 = Digest([16; 32]);

        let batch = OptionValueBatch::new(
            Digest([1; 32]),
            Digest([2; 32]),
            1,
            vec![advance, finish, distractor],
            vec![0, 0, 1],
        )
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
                    discount: 0.9,
                    bootstrap: false,
                    ..FqiConfig::default()
                },
            },
        )
        .unwrap();

        let root = policy.observed_greedy_tactic(Digest([7; 32])).unwrap();
        assert_eq!(root.action.option_id, "advance");
        assert!((root.q_value - 9.0).abs() < 1.0e-6);
        assert_eq!(
            policy
                .observed_greedy_tactic(Digest([8; 32]))
                .unwrap()
                .action
                .option_id,
            "finish"
        );
    }
}
