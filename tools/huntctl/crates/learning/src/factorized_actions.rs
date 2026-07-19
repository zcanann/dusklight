//! Additive option-action factors for sharing evidence across sparse combinations.
//!
//! The encoder gives tactic, heading, magnitude, duration, target, and button
//! overlay independent feature blocks. A small ridge critic proves the intended
//! sharing behavior: it can score a combination that was never observed as a
//! whole, provided its constituent factors were represented in training.

use crate::artifact::Digest;
use crate::option_diagnostics::IntendedTarget;
use crate::option_execution::{MAX_OPTION_TICKS, OptionParameter, OptionType};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

pub const FACTORIZED_ACTION_SCHEMA_V1: &str = "dusklight-factorized-option-action/v1";
pub const FACTORIZED_VALUE_MODEL_SCHEMA_V1: &str = "dusklight-factorized-option-value-model/v1";
pub const MAX_FACTOR_CATALOG: usize = 128;
pub const MAX_FACTOR_SAMPLES: usize = 100_000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FactorizedOptionAction {
    pub schema: String,
    pub tactic: OptionType,
    /// Exact finite binary32 heading in radians. `None` means heading-free.
    pub heading_radians_f32_bits: Option<u32>,
    pub magnitude: Option<u8>,
    pub duration_ticks: u32,
    pub target: IntendedTarget,
    /// Main-controller button mask composed over the tactic realization.
    pub button_overlay: u16,
}

impl FactorizedOptionAction {
    pub fn new(tactic: OptionType, duration_ticks: u32) -> Self {
        Self {
            schema: FACTORIZED_ACTION_SCHEMA_V1.into(),
            tactic,
            heading_radians_f32_bits: None,
            magnitude: None,
            duration_ticks,
            target: IntendedTarget::None,
            button_overlay: 0,
        }
    }

    pub fn validate(&self) -> Result<(), FactorizedActionError> {
        if self.schema != FACTORIZED_ACTION_SCHEMA_V1
            || self.duration_ticks == 0
            || self.duration_ticks > MAX_OPTION_TICKS
            || self
                .heading_radians_f32_bits
                .is_some_and(|bits| !f32::from_bits(bits).is_finite())
            || self
                .magnitude
                .is_some_and(|value| !(1..=127).contains(&value))
            || invalid_option_type(&self.tactic)
            || invalid_target(&self.target)
        {
            return Err(FactorizedActionError::InvalidAction);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FactorizedValueSample {
    pub action: FactorizedOptionAction,
    /// Observed return or a separately authenticated Bellman target.
    pub value_target: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FactorizedValueConfig {
    pub ridge_penalty: f64,
}

impl Default for FactorizedValueConfig {
    fn default() -> Self {
        Self {
            ridge_penalty: 1.0e-6,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FactorizedActionEncoder {
    tactics: Vec<OptionType>,
    targets: Vec<IntendedTarget>,
    feature_width: usize,
}

impl FactorizedActionEncoder {
    pub fn fit<'a>(
        actions: impl IntoIterator<Item = &'a FactorizedOptionAction>,
    ) -> Result<Self, FactorizedActionError> {
        let mut tactics = BTreeMap::<Vec<u8>, OptionType>::new();
        let mut targets = BTreeMap::<Vec<u8>, IntendedTarget>::new();
        let mut count = 0_usize;
        for action in actions {
            action.validate()?;
            count += 1;
            tactics
                .entry(canonical(&action.tactic)?)
                .or_insert_with(|| action.tactic.clone());
            targets
                .entry(canonical(&action.target)?)
                .or_insert_with(|| action.target.clone());
        }
        if count == 0 || tactics.len() > MAX_FACTOR_CATALOG || targets.len() > MAX_FACTOR_CATALOG {
            return Err(FactorizedActionError::InvalidCatalog);
        }
        let tactics = tactics.into_values().collect::<Vec<_>>();
        let targets = targets.into_values().collect::<Vec<_>>();
        let feature_width = 1 + tactics.len() + 3 + 2 + 1 + targets.len() + 16;
        Ok(Self {
            tactics,
            targets,
            feature_width,
        })
    }

    pub fn feature_width(&self) -> usize {
        self.feature_width
    }

    pub fn encode(
        &self,
        action: &FactorizedOptionAction,
    ) -> Result<Vec<f64>, FactorizedActionError> {
        action.validate()?;
        let tactic = catalog_index(&self.tactics, &action.tactic)?;
        let target = catalog_index(&self.targets, &action.target)?;
        let mut features = vec![0.0; self.feature_width];
        features[0] = 1.0;
        let tactic_start = 1;
        features[tactic_start + tactic] = 1.0;
        let heading_start = tactic_start + self.tactics.len();
        if let Some(bits) = action.heading_radians_f32_bits {
            let heading = f64::from(f32::from_bits(bits));
            features[heading_start] = 1.0;
            features[heading_start + 1] = heading.sin();
            features[heading_start + 2] = heading.cos();
        }
        let magnitude_start = heading_start + 3;
        if let Some(magnitude) = action.magnitude {
            features[magnitude_start] = 1.0;
            features[magnitude_start + 1] = f64::from(magnitude) / 127.0;
        }
        let duration_index = magnitude_start + 2;
        features[duration_index] =
            f64::from(action.duration_ticks).ln_1p() / f64::from(MAX_OPTION_TICKS).ln_1p();
        let target_start = duration_index + 1;
        features[target_start + target] = 1.0;
        let buttons_start = target_start + self.targets.len();
        for bit in 0..16 {
            features[buttons_start + bit] = f64::from((action.button_overlay >> bit) & 1);
        }
        Ok(features)
    }

    pub fn schema_sha256(&self) -> Result<Digest, FactorizedActionError> {
        Ok(sha256(&canonical(self)?))
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FactorizedValueModel {
    schema: &'static str,
    factor_schema_sha256: Digest,
    encoder: FactorizedActionEncoder,
    weights: Vec<f64>,
    samples: usize,
    ridge_penalty: f64,
    sharing_contract: &'static str,
}

impl FactorizedValueModel {
    pub fn fit(
        samples: &[FactorizedValueSample],
        config: &FactorizedValueConfig,
    ) -> Result<Self, FactorizedActionError> {
        if samples.is_empty()
            || samples.len() > MAX_FACTOR_SAMPLES
            || !config.ridge_penalty.is_finite()
            || config.ridge_penalty <= 0.0
            || samples
                .iter()
                .any(|sample| !sample.value_target.is_finite())
        {
            return Err(FactorizedActionError::InvalidTraining);
        }
        let encoder = FactorizedActionEncoder::fit(samples.iter().map(|sample| &sample.action))?;
        let width = encoder.feature_width();
        let mut normal = vec![vec![0.0_f64; width]; width];
        let mut target = vec![0.0_f64; width];
        for sample in samples {
            let features = encoder.encode(&sample.action)?;
            let active = features
                .iter()
                .enumerate()
                .filter(|(_, value)| **value != 0.0)
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            for &row in &active {
                target[row] += features[row] * sample.value_target;
                for &column in &active {
                    normal[row][column] += features[row] * features[column];
                }
            }
        }
        for (index, row) in normal.iter_mut().enumerate() {
            row[index] += config.ridge_penalty;
        }
        let weights = solve(normal, target)?;
        let factor_schema_sha256 = encoder.schema_sha256()?;
        Ok(Self {
            schema: FACTORIZED_VALUE_MODEL_SCHEMA_V1,
            factor_schema_sha256,
            encoder,
            weights,
            samples: samples.len(),
            ridge_penalty: config.ridge_penalty,
            sharing_contract: "independent_tactic_heading_magnitude_duration_target_button_blocks",
        })
    }

    pub fn predict(&self, action: &FactorizedOptionAction) -> Result<f64, FactorizedActionError> {
        let features = self.encoder.encode(action)?;
        Ok(dot(&features, &self.weights))
    }

    pub fn rank(
        &self,
        candidates: &[FactorizedOptionAction],
    ) -> Result<Vec<(FactorizedOptionAction, f64)>, FactorizedActionError> {
        if candidates.is_empty() || candidates.len() > MAX_FACTOR_CATALOG {
            return Err(FactorizedActionError::InvalidCatalog);
        }
        let mut ranked = candidates
            .iter()
            .map(|action| self.predict(action).map(|value| (action.clone(), value)))
            .collect::<Result<Vec<_>, _>>()?;
        ranked.sort_by(|left, right| {
            right.1.total_cmp(&left.1).then_with(|| {
                canonical(&left.0)
                    .unwrap()
                    .cmp(&canonical(&right.0).unwrap())
            })
        });
        Ok(ranked)
    }

    pub fn factor_schema_sha256(&self) -> Digest {
        self.factor_schema_sha256
    }
}

fn solve(
    mut matrix: Vec<Vec<f64>>,
    mut target: Vec<f64>,
) -> Result<Vec<f64>, FactorizedActionError> {
    let width = target.len();
    for column in 0..width {
        let pivot = (column..width)
            .max_by(|left, right| {
                matrix[*left][column]
                    .abs()
                    .total_cmp(&matrix[*right][column].abs())
            })
            .ok_or(FactorizedActionError::SingularModel)?;
        if matrix[pivot][column].abs() < f64::EPSILON {
            return Err(FactorizedActionError::SingularModel);
        }
        matrix.swap(column, pivot);
        target.swap(column, pivot);
        let scale = matrix[column][column];
        for value in &mut matrix[column][column..] {
            *value /= scale;
        }
        target[column] /= scale;
        let pivot_row = matrix[column].clone();
        for row in 0..width {
            if row == column {
                continue;
            }
            let scale = matrix[row][column];
            for (value, pivot) in matrix[row][column..].iter_mut().zip(&pivot_row[column..]) {
                *value -= scale * pivot;
            }
            target[row] -= scale * target[column];
        }
    }
    target
        .iter()
        .all(|value| value.is_finite())
        .then_some(target)
        .ok_or(FactorizedActionError::SingularModel)
}

fn dot(left: &[f64], right: &[f64]) -> f64 {
    left.iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum()
}

fn catalog_index<T: Serialize>(catalog: &[T], value: &T) -> Result<usize, FactorizedActionError> {
    let sought = canonical(value)?;
    catalog
        .iter()
        .position(|candidate| canonical(candidate).is_ok_and(|key| key == sought))
        .ok_or(FactorizedActionError::UnknownFactor)
}

fn canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, FactorizedActionError> {
    serde_json::to_vec(value)
        .map_err(|error| FactorizedActionError::Serialization(error.to_string()))
}

fn sha256(bytes: &[u8]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Digest(hasher.finalize().into())
}

fn invalid_option_type(option_type: &OptionType) -> bool {
    matches!(option_type, OptionType::Custom(name) if !valid_name(name, 96))
}

fn invalid_target(target: &IntendedTarget) -> bool {
    match target {
        IntendedTarget::None => false,
        IntendedTarget::Coordinate { value_f32_bits } => value_f32_bits
            .iter()
            .any(|bits| !f32::from_bits(*bits).is_finite()),
        IntendedTarget::Heading { radians_f32_bits } => {
            !f32::from_bits(*radians_f32_bits).is_finite()
        }
        IntendedTarget::Actor {
            selector,
            runtime_process_id,
        } => !valid_name(selector, 256) || runtime_process_id.is_some_and(|id| id == 0),
        IntendedTarget::Semantic { name, parameters } => {
            !valid_name(name, 96)
                || parameters.len() > 64
                || parameters.keys().any(|key| !valid_name(key, 64))
                || parameters.values().any(invalid_parameter)
        }
    }
}

fn invalid_parameter(parameter: &OptionParameter) -> bool {
    match parameter {
        OptionParameter::F32Bits(bits) => !f32::from_bits(*bits).is_finite(),
        OptionParameter::Vec3F32Bits(bits) => bits
            .iter()
            .any(|component| !f32::from_bits(*component).is_finite()),
        OptionParameter::Text(value) => !valid_name(value, 1024),
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FactorizedActionError {
    InvalidAction,
    InvalidCatalog,
    InvalidTraining,
    UnknownFactor,
    SingularModel,
    Serialization(String),
}

impl fmt::Display for FactorizedActionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAction => formatter.write_str("invalid factorized option action"),
            Self::InvalidCatalog => formatter.write_str("invalid factor catalog"),
            Self::InvalidTraining => formatter.write_str("invalid factorized value training data"),
            Self::UnknownFactor => {
                formatter.write_str("candidate uses a factor absent from training")
            }
            Self::SingularModel => formatter.write_str("factorized value system is singular"),
            Self::Serialization(message) => {
                write!(formatter, "factor serialization failed: {message}")
            }
        }
    }
}

impl Error for FactorizedActionError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(tactic: OptionType) -> FactorizedOptionAction {
        FactorizedOptionAction::new(tactic, 1)
    }

    fn sample(action: FactorizedOptionAction, value_target: f64) -> FactorizedValueSample {
        FactorizedValueSample {
            action,
            value_target,
        }
    }

    #[test]
    fn factor_blocks_change_independently() {
        let base = action(OptionType::Neutral);
        let mut heading = base.clone();
        heading.heading_radians_f32_bits = Some(1.0_f32.to_bits());
        let mut buttons = base.clone();
        buttons.button_overlay = 1 << 8;
        let encoder = FactorizedActionEncoder::fit([&base, &heading, &buttons]).unwrap();
        let base_features = encoder.encode(&base).unwrap();
        let heading_features = encoder.encode(&heading).unwrap();
        let button_features = encoder.encode(&buttons).unwrap();
        let heading_changes = base_features
            .iter()
            .zip(&heading_features)
            .filter(|(left, right)| left != right)
            .count();
        let button_changes = base_features
            .iter()
            .zip(&button_features)
            .filter(|(left, right)| left != right)
            .count();
        assert_eq!(heading_changes, 3);
        assert_eq!(button_changes, 1);
    }

    #[test]
    fn unseen_whole_combination_shares_observed_factor_strength() {
        let base = action(OptionType::Neutral);
        let mut roll = base.clone();
        roll.tactic = OptionType::Roll;
        let mut heading = base.clone();
        heading.heading_radians_f32_bits = Some(1.0_f32.to_bits());
        let mut magnitude = base.clone();
        magnitude.magnitude = Some(127);
        let mut duration = base.clone();
        duration.duration_ticks = 120;
        let mut targeted = base.clone();
        targeted.target = IntendedTarget::Actor {
            selector: "enemy_nearest".into(),
            runtime_process_id: None,
        };
        let mut buttons = base.clone();
        buttons.button_overlay = 1 << 8;
        let samples = vec![
            sample(base.clone(), 0.0),
            sample(roll.clone(), 5.0),
            sample(heading.clone(), 2.0),
            sample(magnitude.clone(), 3.0),
            sample(duration.clone(), 1.0),
            sample(targeted.clone(), 4.0),
            sample(buttons.clone(), 2.0),
        ];
        let model = FactorizedValueModel::fit(&samples, &FactorizedValueConfig::default()).unwrap();
        let mut combined = roll;
        combined.heading_radians_f32_bits = heading.heading_radians_f32_bits;
        combined.magnitude = magnitude.magnitude;
        combined.duration_ticks = duration.duration_ticks;
        combined.target = targeted.target;
        combined.button_overlay = buttons.button_overlay;
        assert!(!samples.iter().any(|sample| sample.action == combined));
        assert!(model.predict(&combined).unwrap() > 15.0);
        assert_eq!(
            model.rank(&[base, combined.clone()]).unwrap()[0].0,
            combined
        );
        assert_ne!(model.factor_schema_sha256(), Digest::ZERO);
    }

    #[test]
    fn rejects_invalid_or_unrepresented_factors() {
        let base = action(OptionType::Neutral);
        let model = FactorizedValueModel::fit(
            &[sample(base.clone(), 0.0)],
            &FactorizedValueConfig::default(),
        )
        .unwrap();
        let mut unknown = base.clone();
        unknown.tactic = OptionType::Roll;
        assert_eq!(
            model.predict(&unknown),
            Err(FactorizedActionError::UnknownFactor)
        );
        let mut invalid = base;
        invalid.magnitude = Some(0);
        assert_eq!(
            invalid.validate(),
            Err(FactorizedActionError::InvalidAction)
        );
    }
}
