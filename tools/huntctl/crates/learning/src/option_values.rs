//! Semi-Markov option-value learning before raw per-frame control.
//!
//! The model ranks authenticated, realized option descriptors. It deliberately
//! has no raw PAD-action output: frame-level actions remain a later tape-golf
//! surface after a high-level option has been selected and realized.

use crate::artifact::Digest;
use crate::fqi::{FittedQ, FqiConfig, FqiError, QEstimate, Transition};
use crate::option_execution::{OptionParameter, OptionType};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const OPTION_VALUE_BATCH_SCHEMA_V1: &str = "dusklight-option-value-batch/v1";
pub const OPTION_VALUE_MODEL_SCHEMA_V1: &str = "dusklight-option-value-model/v1";
pub const MAX_OPTION_ACTIONS: usize = 128;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptionActionDescriptor {
    pub option_id: String,
    #[serde(rename = "type")]
    pub option_type: OptionType,
    pub parameters: BTreeMap<String, OptionParameter>,
}

impl OptionActionDescriptor {
    pub fn validate(&self) -> Result<(), OptionValueError> {
        if !valid_name(&self.option_id, 96)
            || matches!(
                &self.option_type,
                OptionType::Custom(name) if !valid_name(name, 96)
            )
            || self.parameters.len() > 64
            || self
                .parameters
                .keys()
                .any(|name| !valid_name(name, 64))
            || self.parameters.values().any(invalid_parameter)
        {
            return Err(OptionValueError::Invalid(
                "invalid option action descriptor",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptionValueSample {
    pub action: OptionActionDescriptor,
    pub state: Vec<f32>,
    pub duration_ticks: u32,
    pub reward: f32,
    pub next_state: Vec<f32>,
    pub terminal: bool,
    /// Digest of the exact raw tape emitted by this realized option.
    pub realized_tape_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptionValueBatch {
    pub schema: String,
    pub feature_schema: Digest,
    pub objective_sha256: Digest,
    pub option_action_schema: Digest,
    pub feature_width: usize,
    pub samples: Vec<OptionValueSample>,
    pub episode_groups: Vec<u64>,
}

impl OptionValueBatch {
    pub fn new(
        feature_schema: Digest,
        objective_sha256: Digest,
        feature_width: usize,
        samples: Vec<OptionValueSample>,
        episode_groups: Vec<u64>,
    ) -> Result<Self, OptionValueError> {
        let option_action_schema = option_action_schema(&samples)?;
        let batch = Self {
            schema: OPTION_VALUE_BATCH_SCHEMA_V1.into(),
            feature_schema,
            objective_sha256,
            option_action_schema,
            feature_width,
            samples,
            episode_groups,
        };
        batch.validate()?;
        Ok(batch)
    }

    pub fn validate(&self) -> Result<(), OptionValueError> {
        if self.schema != OPTION_VALUE_BATCH_SCHEMA_V1
            || self.feature_schema == Digest::ZERO
            || self.objective_sha256 == Digest::ZERO
            || self.feature_width == 0
            || self.samples.is_empty()
            || self.samples.len() != self.episode_groups.len()
            || option_action_schema(&self.samples)? != self.option_action_schema
        {
            return Err(OptionValueError::Invalid(
                "option-value batch identity or shape is invalid",
            ));
        }
        for sample in &self.samples {
            validate_sample(self.feature_width, sample)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OptionValueConfig {
    pub fitted_q: FqiConfig,
}

impl Default for OptionValueConfig {
    fn default() -> Self {
        Self {
            fitted_q: FqiConfig {
                backup_steps: 1,
                ..FqiConfig::default()
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RankedOption {
    pub action_id: u32,
    pub descriptor: OptionActionDescriptor,
    pub mean_q: f64,
    pub ensemble_variance: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct OptionValueModel {
    feature_width: usize,
    actions: Vec<OptionActionDescriptor>,
    action_identity_sha256: Vec<Digest>,
    realized_tape_sha256: Vec<Digest>,
    critic: FittedQ,
    /// Documents that the learned policy stops at the option boundary.
    control_hierarchy: &'static str,
    raw_frame_policy: &'static str,
}

impl OptionValueModel {
    pub fn fit_batch(
        batch: &OptionValueBatch,
        config: &OptionValueConfig,
    ) -> Result<Self, OptionValueError> {
        batch.validate()?;
        Self::fit(
            batch.feature_width,
            &batch.samples,
            &batch.episode_groups,
            config,
        )
    }

    pub fn fit(
        feature_width: usize,
        samples: &[OptionValueSample],
        episode_groups: &[u64],
        config: &OptionValueConfig,
    ) -> Result<Self, OptionValueError> {
        if feature_width == 0 || samples.is_empty() || samples.len() != episode_groups.len() {
            return Err(OptionValueError::Invalid(
                "option training requires features, samples, and one episode group per sample",
            ));
        }
        if config.fitted_q.backup_steps != 1 {
            return Err(OptionValueError::Invalid(
                "option samples already encode semi-Markov durations; nested n-step backup is unsupported",
            ));
        }
        let mut keyed = BTreeMap::<Vec<u8>, OptionActionDescriptor>::new();
        for sample in samples {
            validate_sample(feature_width, sample)?;
            let key = serde_json::to_vec(&sample.action)
                .map_err(|error| OptionValueError::Serialization(error.to_string()))?;
            keyed.entry(key).or_insert_with(|| sample.action.clone());
        }
        if keyed.len() > MAX_OPTION_ACTIONS {
            return Err(OptionValueError::Invalid(
                "option action catalog exceeds 128 descriptors",
            ));
        }
        let actions = keyed.into_values().collect::<Vec<_>>();
        let action_keys = actions
            .iter()
            .map(serde_json::to_vec)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| OptionValueError::Serialization(error.to_string()))?;
        let action_ids = (0..actions.len()).map(|id| id as u32).collect::<Vec<_>>();
        let mut realized_by_action = vec![BTreeSet::<Digest>::new(); actions.len()];
        let transitions = samples
            .iter()
            .map(|sample| {
                let key = serde_json::to_vec(&sample.action)
                    .map_err(|error| OptionValueError::Serialization(error.to_string()))?;
                let action = action_keys.binary_search(&key).map_err(|_| {
                    OptionValueError::Invalid("option descriptor map is inconsistent")
                })?;
                realized_by_action[action].insert(sample.realized_tape_sha256);
                Ok(Transition {
                    state: sample.state.clone(),
                    action: action as u32,
                    duration: sample.duration_ticks,
                    reward: sample.reward,
                    next_state: sample.next_state.clone(),
                    terminal: sample.terminal,
                })
            })
            .collect::<Result<Vec<_>, OptionValueError>>()?;
        let critic = FittedQ::fit_with_episode_groups(
            feature_width,
            &action_ids,
            &transitions,
            episode_groups,
            &config.fitted_q,
        )?;
        let action_identity_sha256 = action_keys.iter().map(|bytes| sha256(bytes)).collect();
        let realized_tape_sha256 = realized_by_action
            .into_iter()
            .flat_map(BTreeSet::into_iter)
            .collect();
        Ok(Self {
            feature_width,
            actions,
            action_identity_sha256,
            realized_tape_sha256,
            critic,
            control_hierarchy: "option_value_then_deterministic_realization",
            raw_frame_policy: "last_mile_tape_golf_only",
        })
    }

    pub fn rank_options(&self, state: &[f32]) -> Result<Vec<RankedOption>, OptionValueError> {
        Ok(self
            .critic
            .rank_actions(state)?
            .into_iter()
            .map(|estimate| self.rank(estimate))
            .collect())
    }

    pub fn actions(&self) -> &[OptionActionDescriptor] {
        &self.actions
    }

    pub fn action_identity_sha256(&self) -> &[Digest] {
        &self.action_identity_sha256
    }

    pub fn realized_tape_sha256(&self) -> &[Digest] {
        &self.realized_tape_sha256
    }

    pub fn feature_width(&self) -> usize {
        self.feature_width
    }

    pub fn artifact_bytes(
        &self,
        batch: &OptionValueBatch,
        config: &OptionValueConfig,
    ) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(&serde_json::json!({
            "schema": OPTION_VALUE_MODEL_SCHEMA_V1,
            "feature_schema": batch.feature_schema,
            "objective_sha256": batch.objective_sha256,
            "option_action_schema": batch.option_action_schema,
            "config": config,
            "model": self,
        }))
    }

    fn rank(&self, estimate: QEstimate) -> RankedOption {
        RankedOption {
            action_id: estimate.action,
            descriptor: self.actions[estimate.action as usize].clone(),
            mean_q: estimate.mean,
            ensemble_variance: estimate.variance,
        }
    }
}

fn validate_sample(
    feature_width: usize,
    sample: &OptionValueSample,
) -> Result<(), OptionValueError> {
    sample.action.validate()?;
    if sample.state.len() != feature_width
        || sample.next_state.len() != feature_width
        || sample
            .state
            .iter()
            .chain(&sample.next_state)
            .any(|value| !value.is_finite())
        || !sample.reward.is_finite()
        || sample.duration_ticks == 0
        || sample.realized_tape_sha256 == Digest::ZERO
    {
        return Err(OptionValueError::Invalid("invalid realized option sample"));
    }
    Ok(())
}

fn invalid_parameter(parameter: &OptionParameter) -> bool {
    match parameter {
        OptionParameter::F32Bits(bits) => !f32::from_bits(*bits).is_finite(),
        OptionParameter::Vec3F32Bits(bits) => bits
            .iter()
            .any(|component| !f32::from_bits(*component).is_finite()),
        OptionParameter::Text(value) => {
            value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control)
        }
        OptionParameter::Digest(value) => *value == Digest::ZERO,
        _ => false,
    }
}

fn valid_name(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':' | b'/')
        })
}

fn sha256(bytes: &[u8]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Digest(hasher.finalize().into())
}

fn option_action_schema(samples: &[OptionValueSample]) -> Result<Digest, OptionValueError> {
    let mut descriptors = samples
        .iter()
        .map(|sample| serde_json::to_vec(&sample.action))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| OptionValueError::Serialization(error.to_string()))?;
    descriptors.sort();
    descriptors.dedup();
    let mut hasher = Sha256::new();
    hasher.update(OPTION_VALUE_BATCH_SCHEMA_V1.as_bytes());
    for descriptor in descriptors {
        hasher.update((descriptor.len() as u64).to_le_bytes());
        hasher.update(descriptor);
    }
    Ok(Digest(hasher.finalize().into()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OptionValueError {
    Invalid(&'static str),
    Serialization(String),
    FittedQ(String),
}

impl fmt::Display for OptionValueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => write!(formatter, "invalid option-value input: {message}"),
            Self::Serialization(message) => write!(formatter, "option identity failed: {message}"),
            Self::FittedQ(message) => write!(formatter, "option-value fit failed: {message}"),
        }
    }
}

impl Error for OptionValueError {}

impl From<FqiError> for OptionValueError {
    fn from(error: FqiError) -> Self {
        Self::FittedQ(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(name: &str, option_type: OptionType) -> OptionActionDescriptor {
        OptionActionDescriptor {
            option_id: name.into(),
            option_type,
            parameters: BTreeMap::new(),
        }
    }

    fn sample(
        state: f32,
        action: OptionActionDescriptor,
        duration: u32,
        reward: f32,
        terminal: bool,
        digest: u8,
    ) -> OptionValueSample {
        OptionValueSample {
            action,
            state: vec![state],
            duration_ticks: duration,
            reward,
            next_state: vec![state + 1.0],
            terminal,
            realized_tape_sha256: Digest([digest; 32]),
        }
    }

    #[test]
    fn learns_semi_markov_options_and_retains_realized_tape_proof() {
        let wait = action("wait", OptionType::Neutral);
        let roll = action("roll_forward", OptionType::Roll);
        let samples = vec![
            sample(0.0, wait.clone(), 4, -1.0, true, 1),
            sample(0.0, roll.clone(), 12, 6.0, true, 2),
            sample(1.0, wait, 4, -1.0, true, 3),
            sample(1.0, roll, 12, 6.0, true, 4),
        ];
        let config = OptionValueConfig {
            fitted_q: FqiConfig {
                iterations: 12,
                trees_per_action: 7,
                max_tree_depth: 3,
                bootstrap: false,
                seed: 7,
                ..FqiConfig::default()
            },
        };
        let first = OptionValueModel::fit(1, &samples, &[1, 2, 3, 4], &config).unwrap();
        let second = OptionValueModel::fit(1, &samples, &[1, 2, 3, 4], &config).unwrap();
        assert_eq!(
            serde_json::to_vec(&first).unwrap(),
            serde_json::to_vec(&second).unwrap()
        );
        assert_eq!(
            first.rank_options(&[0.0]).unwrap()[0].descriptor.option_id,
            "roll_forward"
        );
        assert_eq!(first.realized_tape_sha256().len(), 4);
        let encoded = serde_json::to_value(&first).unwrap();
        assert_eq!(
            encoded["control_hierarchy"],
            "option_value_then_deterministic_realization"
        );
        assert_eq!(encoded["raw_frame_policy"], "last_mile_tape_golf_only");
    }

    #[test]
    fn rejects_unproved_tape_identity_and_nested_n_step_backups() {
        let mut invalid = sample(0.0, action("roll", OptionType::Roll), 2, 1.0, true, 1);
        invalid.realized_tape_sha256 = Digest::ZERO;
        assert!(OptionValueModel::fit(1, &[invalid], &[1], &OptionValueConfig::default()).is_err());
        let valid = sample(0.0, action("roll", OptionType::Roll), 2, 1.0, true, 1);
        let mut config = OptionValueConfig::default();
        config.fitted_q.backup_steps = 2;
        assert!(OptionValueModel::fit(1, &[valid], &[1], &config).is_err());
    }

    #[test]
    fn rejects_descriptors_that_cannot_be_executed() {
        let invalid_parameters = [
            OptionParameter::F32Bits(f32::NAN.to_bits()),
            OptionParameter::Vec3F32Bits([0.0_f32.to_bits(), f32::INFINITY.to_bits(), 0]),
            OptionParameter::Text("bad\ntext".into()),
            OptionParameter::Digest(Digest::ZERO),
        ];
        for parameter in invalid_parameters {
            let mut invalid = sample(0.0, action("roll", OptionType::Roll), 2, 1.0, true, 1);
            invalid.action.parameters.insert("value".into(), parameter);
            assert!(
                OptionValueModel::fit(1, &[invalid], &[1], &OptionValueConfig::default()).is_err()
            );
        }
        let invalid = sample(
            0.0,
            action("roll", OptionType::Custom("spaces are invalid".into())),
            2,
            1.0,
            true,
            1,
        );
        assert!(OptionValueModel::fit(1, &[invalid], &[1], &OptionValueConfig::default()).is_err());
    }

    #[test]
    fn batch_authenticates_feature_objective_and_option_catalog() {
        let samples = vec![sample(
            0.0,
            action("roll", OptionType::Roll),
            2,
            1.0,
            true,
            1,
        )];
        let batch =
            OptionValueBatch::new(Digest([7; 32]), Digest([8; 32]), 1, samples, vec![1]).unwrap();
        let model = OptionValueModel::fit_batch(&batch, &OptionValueConfig::default()).unwrap();
        let artifact: serde_json::Value = serde_json::from_slice(
            &model
                .artifact_bytes(&batch, &OptionValueConfig::default())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(artifact["schema"], OPTION_VALUE_MODEL_SCHEMA_V1);
        let mut tampered = batch;
        tampered.samples[0].action.option_id = "different".into();
        assert!(tampered.validate().is_err());
    }
}
