//! Direct training and native frozen export for one authenticated semantic goal.
//!
//! The policy imitates actions from real successful trajectories and applies a
//! bounded contrastive update away from actions in authenticated policy failures.
//! Physical episodes remain isolated across training, validation, and test. A separately
//! admitted reachability model is required as lineage and a fail-closed signal
//! that the same corpus supports useful goal-conditioned prediction. Training-
//! only normalization is folded into the first dense layer, so the emitted
//! `.dsfrozen` bytes consume the native 120-wide pre-input feature contract with
//! no manual conversion or runtime preprocessing.

use crate::artifact::Digest;
use crate::factorized_pad_action::{FACTORIZED_PAD_POLICY_HEAD_WIDTH, FactorizedPadPolicyHead};
use crate::frozen_inference::{FrozenActivation, FrozenDenseLayer, FrozenInferenceModel};
use crate::native_auxiliary_dataset::{AuxiliaryPadTarget, AuxiliarySplit};
use crate::native_goal_reachability::{
    NativeGoalReachabilityAdmission, NativeGoalReachabilityModel,
};
use crate::native_goal_trajectory::{NativeGoalTrajectoryDataset, NativeGoalTrajectoryRow};
use crate::native_policy_features::{
    NATIVE_POLICY_FEATURE_SCHEMA_SHA256, NATIVE_POLICY_FEATURE_WIDTH,
    encode_native_policy_observation,
};
use crate::native_replay_corpus::{DemonstrationMode, ReplayExperienceRole};
use dusklight_evidence::native_episode_shard::{NativeEpisodeShard, NativeRawPad};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const NATIVE_GOAL_FROZEN_POLICY_MANIFEST_SCHEMA_V3: &str =
    "dusklight-native-goal-frozen-policy-manifest/v3";
const MAX_ROWS: usize = 1_000_000;
const MAX_HIDDEN_WIDTH: usize = 256;
const MAX_EPOCHS: usize = 2_048;
const MAX_GRADIENT_UPDATES: usize = 100_000_000;
const CONTINUOUS_HEADS: usize = 8;
const BUTTON_HEAD_START: usize = 8;
const BUTTON_HEAD_END: usize = 24;
const FAILURE_CONTRAST_STRENGTH: f64 = 0.01;
const FAILURE_CONTINUOUS_MARGIN: f64 = 0.10;
const FAILURE_BUTTON_PROBABILITY_MARGIN: f64 = 0.10;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeGoalFrozenPolicyConfig {
    pub epochs: u16,
    pub hidden_width: u16,
    pub learning_rate: f64,
    pub l2_penalty: f64,
    pub gradient_clip: f64,
    pub minimum_validation_joint_improvement: f64,
    pub seed: u64,
}

impl Default for NativeGoalFrozenPolicyConfig {
    fn default() -> Self {
        Self {
            epochs: 96,
            hidden_width: 64,
            learning_rate: 0.002,
            l2_penalty: 1.0e-5,
            gradient_clip: 5.0,
            minimum_validation_joint_improvement: 0.02,
            seed: 0x474f_414c_504f_4c01,
        }
    }
}

impl NativeGoalFrozenPolicyConfig {
    /// Validates the bounded policy-training configuration before export.
    pub fn validate(self) -> Result<(), NativeGoalFrozenPolicyError> {
        if self.epochs == 0
            || usize::from(self.epochs) > MAX_EPOCHS
            || self.hidden_width == 0
            || usize::from(self.hidden_width) > MAX_HIDDEN_WIDTH
            || !self.learning_rate.is_finite()
            || self.learning_rate <= 0.0
            || !self.l2_penalty.is_finite()
            || self.l2_penalty < 0.0
            || !self.gradient_clip.is_finite()
            || self.gradient_clip <= 0.0
            || !self.minimum_validation_joint_improvement.is_finite()
            || !(0.0..=1.0).contains(&self.minimum_validation_joint_improvement)
        {
            return Err(NativeGoalFrozenPolicyError::new(
                "goal frozen policy configuration is outside its bounded domain",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeGoalFrozenPolicyAdmission {
    RetainSuccessfulActionMean,
    FrozenPolicyCandidate,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeGoalFrozenPolicyMetrics {
    pub rows: usize,
    pub episodes: usize,
    pub continuous_mae: f64,
    pub baseline_continuous_mae: f64,
    pub button_bit_error_rate: f64,
    pub baseline_button_bit_error_rate: f64,
    pub joint_error: f64,
    pub baseline_joint_error: f64,
    pub joint_relative_improvement: f64,
    pub decoded_pad_exact_rate: f64,
    pub baseline_decoded_pad_exact_rate: f64,
}

impl NativeGoalFrozenPolicyMetrics {
    fn validate(&self) -> bool {
        self.rows > 0
            && self.episodes > 0
            && [
                self.continuous_mae,
                self.baseline_continuous_mae,
                self.joint_error,
                self.baseline_joint_error,
            ]
            .iter()
            .all(|value| value.is_finite() && *value >= 0.0 && *value <= 2.0)
            && [
                self.button_bit_error_rate,
                self.baseline_button_bit_error_rate,
                self.joint_relative_improvement,
                self.decoded_pad_exact_rate,
                self.baseline_decoded_pad_exact_rate,
            ]
            .iter()
            .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeGoalFrozenPolicyManifest {
    pub schema: String,
    pub source_dataset_sha256: Digest,
    pub source_replay_corpus_sha256: Digest,
    pub source_reachability_model_sha256: Digest,
    pub goal_input_sha256: Digest,
    pub goal_program_sha256: Digest,
    pub objective_sha256: Digest,
    pub goal_objective_identity: String,
    pub demonstration_mode: DemonstrationMode,
    pub observation_schema: String,
    pub action_schema: String,
    pub feature_schema_sha256: Digest,
    pub factorized_action_schema_sha256: Digest,
    pub config: NativeGoalFrozenPolicyConfig,
    pub feature_mean: Vec<f64>,
    pub feature_inverse_stddev: Vec<f64>,
    pub training_successful_episode_sha256: Vec<Digest>,
    pub training_successful_rows: usize,
    pub training_failed_rows: usize,
    pub failure_contrast_strength: f64,
    pub failure_continuous_margin: f64,
    pub failure_button_probability_margin: f64,
    pub gradient_updates: u64,
    pub training: NativeGoalFrozenPolicyMetrics,
    pub validation: NativeGoalFrozenPolicyMetrics,
    pub test: NativeGoalFrozenPolicyMetrics,
    pub admission: NativeGoalFrozenPolicyAdmission,
    pub frozen_artifact_sha256: Digest,
    pub frozen_model_xxh3_128: String,
    pub frozen_byte_count: usize,
    pub promotion_authority: bool,
    pub manifest_sha256: Digest,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeGoalFrozenPolicyExport {
    pub manifest: NativeGoalFrozenPolicyManifest,
    pub model_bytes: Vec<u8>,
}

#[derive(Clone)]
struct MaterializedPolicyRow {
    episode_sha256: Digest,
    split: AuxiliarySplit,
    success: bool,
    features: Vec<f64>,
    target: [f64; FACTORIZED_PAD_POLICY_HEAD_WIDTH],
    pad: AuxiliaryPadTarget,
}

struct PolicyNetwork {
    hidden_weights: Vec<f64>,
    hidden_bias: Vec<f64>,
    output_weights: Vec<f64>,
    output_bias: Vec<f64>,
    gradient_updates: u64,
}

struct Forward {
    hidden: Vec<f64>,
    output: [f64; FACTORIZED_PAD_POLICY_HEAD_WIDTH],
}

impl NativeGoalFrozenPolicyExport {
    pub fn fit(
        dataset: &NativeGoalTrajectoryDataset,
        shards: &[NativeEpisodeShard],
        reachability: &NativeGoalReachabilityModel,
        config: NativeGoalFrozenPolicyConfig,
    ) -> Result<Self, NativeGoalFrozenPolicyError> {
        config.validate()?;
        dataset
            .validate()
            .map_err(|error| NativeGoalFrozenPolicyError::new(error.to_string()))?;
        reachability
            .validate()
            .map_err(|error| NativeGoalFrozenPolicyError::new(error.to_string()))?;
        if reachability.admission != NativeGoalReachabilityAdmission::GoalConditionedCandidate
            || reachability.demonstration_mode != dataset.config.demonstration_mode
            || !reachability
                .source_dataset_sha256
                .contains(&dataset.dataset_sha256)
            || !reachability
                .source_goal_input_sha256
                .contains(&dataset.goal.input_sha256)
            || !reachability
                .source_goal_objective_identity
                .contains(&dataset.goal_objective_identity)
        {
            return Err(NativeGoalFrozenPolicyError::new(
                "goal frozen policy requires an admitted reachability model bound to its dataset and goal",
            ));
        }
        let rows = materialize(dataset, shards)?;
        validate_split_support(&rows)?;
        let training_indices = split_indices(&rows, AuxiliarySplit::Training);
        let work = training_indices
            .len()
            .checked_mul(usize::from(config.epochs))
            .filter(|work| *work <= MAX_GRADIENT_UPDATES)
            .ok_or_else(|| {
                NativeGoalFrozenPolicyError::new("goal policy training work exceeds its bound")
            })?;
        let (feature_mean, feature_inverse_stddev) = fit_normalization(&rows, &training_indices)?;
        let normalized = rows
            .iter()
            .map(|row| normalize(&row.features, &feature_mean, &feature_inverse_stddev))
            .collect::<Vec<_>>();
        let baseline_output = training_mean_output(&rows, &training_indices);
        let mut rng = Rng::new(config.seed);
        let mut network = PolicyNetwork::initialized(
            NATIVE_POLICY_FEATURE_WIDTH,
            usize::from(config.hidden_width),
            baseline_output,
            &mut rng,
        );
        let mut order = training_indices;
        for _ in 0..config.epochs {
            rng.shuffle(&mut order);
            for index in order.iter().copied() {
                network.update(
                    &normalized[index],
                    &rows[index].target,
                    rows[index].success,
                    config,
                )?;
            }
        }
        if usize::try_from(network.gradient_updates).ok() != Some(work) {
            return Err(NativeGoalFrozenPolicyError::new(
                "goal policy gradient accounting differs from bounded work",
            ));
        }
        let frozen_model = network.freeze(
            &feature_mean,
            &feature_inverse_stddev,
            dataset.goal.definition_sha256,
            usize::from(config.hidden_width),
        )?;
        let model_bytes = frozen_model
            .to_bytes()
            .map_err(|error| NativeGoalFrozenPolicyError::new(error.to_string()))?;
        let training = evaluate(
            &rows,
            AuxiliarySplit::Training,
            &frozen_model,
            baseline_output,
        )?;
        let validation = evaluate(
            &rows,
            AuxiliarySplit::Validation,
            &frozen_model,
            baseline_output,
        )?;
        let test = evaluate(&rows, AuxiliarySplit::Test, &frozen_model, baseline_output)?;
        let admission = admission(&validation, config);
        let action_schema_sha256 = Digest(
            FactorizedPadPolicyHead::default()
                .schema_sha256()
                .map_err(|error| NativeGoalFrozenPolicyError::new(error.to_string()))?,
        );
        let mut training_successful_episode_sha256 = rows
            .iter()
            .filter(|row| row.split == AuxiliarySplit::Training && row.success)
            .map(|row| row.episode_sha256)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        training_successful_episode_sha256.sort_unstable();
        let training_successful_rows = rows
            .iter()
            .filter(|row| row.split == AuxiliarySplit::Training && row.success)
            .count();
        let training_failed_rows = rows
            .iter()
            .filter(|row| row.split == AuxiliarySplit::Training && !row.success)
            .count();
        let mut manifest = NativeGoalFrozenPolicyManifest {
            schema: NATIVE_GOAL_FROZEN_POLICY_MANIFEST_SCHEMA_V3.into(),
            source_dataset_sha256: dataset.dataset_sha256,
            source_replay_corpus_sha256: dataset.replay_corpus_sha256,
            source_reachability_model_sha256: reachability.model_sha256,
            goal_input_sha256: dataset.goal.input_sha256,
            goal_program_sha256: dataset.goal_program_sha256,
            objective_sha256: dataset.goal.definition_sha256,
            goal_objective_identity: dataset.goal_objective_identity.clone(),
            demonstration_mode: dataset.config.demonstration_mode,
            observation_schema: dataset.observation_schema.clone(),
            action_schema: dataset.action_schema.clone(),
            feature_schema_sha256: Digest(NATIVE_POLICY_FEATURE_SCHEMA_SHA256),
            factorized_action_schema_sha256: action_schema_sha256,
            config,
            feature_mean,
            feature_inverse_stddev,
            training_successful_episode_sha256,
            training_successful_rows,
            training_failed_rows,
            failure_contrast_strength: FAILURE_CONTRAST_STRENGTH,
            failure_continuous_margin: FAILURE_CONTINUOUS_MARGIN,
            failure_button_probability_margin: FAILURE_BUTTON_PROBABILITY_MARGIN,
            gradient_updates: network.gradient_updates,
            training,
            validation,
            test,
            admission,
            frozen_artifact_sha256: frozen_model
                .artifact_sha256()
                .map_err(|error| NativeGoalFrozenPolicyError::new(error.to_string()))?,
            frozen_model_xxh3_128: format!("{:032x}", xxhash_rust::xxh3::xxh3_128(&model_bytes)),
            frozen_byte_count: model_bytes.len(),
            promotion_authority: false,
            manifest_sha256: Digest::ZERO,
        };
        // Seal the same floating-point representation that JSON artifacts will
        // load, avoiding adjacent f64 changes after a serialize/parse boundary.
        let canonical = serde_json::to_vec(&manifest)
            .map_err(|error| NativeGoalFrozenPolicyError::new(error.to_string()))?;
        manifest = serde_json::from_slice(&canonical)
            .map_err(|error| NativeGoalFrozenPolicyError::new(error.to_string()))?;
        manifest.manifest_sha256 = manifest.digest()?;
        let export = Self {
            manifest,
            model_bytes,
        };
        export.validate()?;
        Ok(export)
    }

    pub fn validate(&self) -> Result<(), NativeGoalFrozenPolicyError> {
        self.manifest.validate(&self.model_bytes)
    }
}

impl NativeGoalFrozenPolicyManifest {
    pub fn validate(&self, model_bytes: &[u8]) -> Result<(), NativeGoalFrozenPolicyError> {
        self.config.validate()?;
        let model = FrozenInferenceModel::from_bytes(model_bytes)
            .map_err(|error| NativeGoalFrozenPolicyError::new(error.to_string()))?;
        let action_schema_sha256 = Digest(
            FactorizedPadPolicyHead::default()
                .schema_sha256()
                .map_err(|error| NativeGoalFrozenPolicyError::new(error.to_string()))?,
        );
        let sources_valid = [
            self.source_dataset_sha256,
            self.source_replay_corpus_sha256,
            self.source_reachability_model_sha256,
            self.goal_input_sha256,
            self.goal_program_sha256,
            self.objective_sha256,
            self.feature_schema_sha256,
            self.factorized_action_schema_sha256,
            self.frozen_artifact_sha256,
        ]
        .iter()
        .all(|digest| *digest != Digest::ZERO);
        if self.schema != NATIVE_GOAL_FROZEN_POLICY_MANIFEST_SCHEMA_V3
            || !sources_valid
            || !is_lower_hex(&self.goal_objective_identity, 32)
            || self.observation_schema.is_empty()
            || self.action_schema.is_empty()
            || self.feature_schema_sha256 != Digest(NATIVE_POLICY_FEATURE_SCHEMA_SHA256)
            || self.factorized_action_schema_sha256 != action_schema_sha256
            || self.feature_mean.len() != NATIVE_POLICY_FEATURE_WIDTH
            || self.feature_inverse_stddev.len() != NATIVE_POLICY_FEATURE_WIDTH
            || self
                .feature_mean
                .iter()
                .chain(&self.feature_inverse_stddev)
                .any(|value| !value.is_finite())
            || self
                .feature_inverse_stddev
                .iter()
                .any(|value| *value <= 0.0)
            || self.training_successful_episode_sha256.is_empty()
            || self
                .training_successful_episode_sha256
                .contains(&Digest::ZERO)
            || self
                .training_successful_episode_sha256
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || !self.training.validate()
            || !self.validation.validate()
            || !self.test.validate()
            || self.training_successful_episode_sha256.len() != self.training.episodes
            || self.training_successful_rows != self.training.rows
            || self.failure_contrast_strength != FAILURE_CONTRAST_STRENGTH
            || self.failure_continuous_margin != FAILURE_CONTINUOUS_MARGIN
            || self.failure_button_probability_margin != FAILURE_BUTTON_PROBABILITY_MARGIN
            || self.gradient_updates
                != u64::try_from(
                    self.training_successful_rows
                        .checked_add(self.training_failed_rows)
                        .unwrap_or(0),
                )
                .ok()
                .and_then(|rows| rows.checked_mul(u64::from(self.config.epochs)))
                .unwrap_or(0)
            || self.admission != admission(&self.validation, self.config)
            || self.frozen_byte_count != model_bytes.len()
            || self.frozen_model_xxh3_128
                != format!("{:032x}", xxhash_rust::xxh3::xxh3_128(model_bytes))
            || self.frozen_artifact_sha256
                != model
                    .artifact_sha256()
                    .map_err(|error| NativeGoalFrozenPolicyError::new(error.to_string()))?
            || model.feature_schema_sha256 != self.feature_schema_sha256
            || model.action_schema_sha256 != self.factorized_action_schema_sha256
            || model.objective_sha256 != self.objective_sha256
            || model.input_width != NATIVE_POLICY_FEATURE_WIDTH
            || model.actions != (0..FACTORIZED_PAD_POLICY_HEAD_WIDTH as u32).collect::<Vec<_>>()
            || model.layers.len() != 2
            || model.layers[0].output_width != usize::from(self.config.hidden_width)
            || model.layers[0].activation != FrozenActivation::Relu
            || model.layers[1].output_width != FACTORIZED_PAD_POLICY_HEAD_WIDTH
            || model.layers[1].activation != FrozenActivation::Linear
            || self.promotion_authority
            || self.manifest_sha256 != self.digest()?
        {
            return Err(NativeGoalFrozenPolicyError::new(
                "native goal frozen policy manifest or model is invalid or detached",
            ));
        }
        Ok(())
    }

    fn digest(&self) -> Result<Digest, NativeGoalFrozenPolicyError> {
        let mut canonical = self.clone();
        canonical.manifest_sha256 = Digest::ZERO;
        canonical_digest(
            b"dusklight.native-goal-frozen-policy-manifest/v3\0",
            &canonical,
        )
    }
}

impl PolicyNetwork {
    fn initialized(
        input_width: usize,
        hidden_width: usize,
        baseline_output: [f64; FACTORIZED_PAD_POLICY_HEAD_WIDTH],
        rng: &mut Rng,
    ) -> Self {
        Self {
            hidden_weights: initialized_weights(hidden_width, input_width, rng),
            hidden_bias: vec![0.0; hidden_width],
            output_weights: initialized_weights(
                FACTORIZED_PAD_POLICY_HEAD_WIDTH,
                hidden_width,
                rng,
            ),
            output_bias: baseline_output.to_vec(),
            gradient_updates: 0,
        }
    }

    fn forward(&self, features: &[f64], hidden_width: usize) -> Forward {
        let input_width = features.len();
        let hidden = (0..hidden_width)
            .map(|index| {
                (dot(
                    &self.hidden_weights[index * input_width..(index + 1) * input_width],
                    features,
                ) + self.hidden_bias[index])
                    .max(0.0)
            })
            .collect::<Vec<_>>();
        let output = std::array::from_fn(|head| {
            dot(
                &self.output_weights[head * hidden_width..(head + 1) * hidden_width],
                &hidden,
            ) + self.output_bias[head]
        });
        Forward { hidden, output }
    }

    fn update(
        &mut self,
        features: &[f64],
        target: &[f64; FACTORIZED_PAD_POLICY_HEAD_WIDTH],
        successful_sample: bool,
        config: NativeGoalFrozenPolicyConfig,
    ) -> Result<(), NativeGoalFrozenPolicyError> {
        let hidden_width = usize::from(config.hidden_width);
        let input_width = features.len();
        let forward = self.forward(features, hidden_width);
        let output_weights_before = self.output_weights.clone();
        let mut d_output = [0.0; FACTORIZED_PAD_POLICY_HEAD_WIDTH];
        for head in 0..FACTORIZED_PAD_POLICY_HEAD_WIDTH {
            let gradient = if successful_sample {
                if (BUTTON_HEAD_START..BUTTON_HEAD_END).contains(&head) {
                    (logistic(forward.output[head]) - target[head]) / 16.0
                } else if head < CONTINUOUS_HEADS {
                    2.0 * (forward.output[head] - target[head]) / CONTINUOUS_HEADS as f64
                } else {
                    forward.output[head] - target[head]
                }
            } else if (BUTTON_HEAD_START..BUTTON_HEAD_END).contains(&head) {
                bounded_button_contrast_gradient(forward.output[head], target[head])
            } else if head < CONTINUOUS_HEADS {
                bounded_continuous_contrast_gradient(forward.output[head], target[head])
            } else {
                0.0
            };
            d_output[head] = clip(gradient, config.gradient_clip);
            for (hidden, value) in forward.hidden.iter().copied().enumerate() {
                let parameter = head * hidden_width + hidden;
                let parameter_gradient =
                    d_output[head] * value + config.l2_penalty * self.output_weights[parameter];
                self.output_weights[parameter] -=
                    config.learning_rate * clip(parameter_gradient, config.gradient_clip);
            }
            self.output_bias[head] -= config.learning_rate * d_output[head];
        }
        for hidden in 0..hidden_width {
            if forward.hidden[hidden] <= 0.0 {
                continue;
            }
            let hidden_gradient = (0..FACTORIZED_PAD_POLICY_HEAD_WIDTH)
                .map(|head| d_output[head] * output_weights_before[head * hidden_width + hidden])
                .sum::<f64>();
            for (input, feature) in features.iter().copied().enumerate() {
                let parameter = hidden * input_width + input;
                let parameter_gradient =
                    hidden_gradient * feature + config.l2_penalty * self.hidden_weights[parameter];
                self.hidden_weights[parameter] -=
                    config.learning_rate * clip(parameter_gradient, config.gradient_clip);
            }
            self.hidden_bias[hidden] -=
                config.learning_rate * clip(hidden_gradient, config.gradient_clip);
        }
        if self
            .hidden_weights
            .iter()
            .chain(&self.hidden_bias)
            .chain(&self.output_weights)
            .chain(&self.output_bias)
            .any(|value| !value.is_finite())
        {
            return Err(NativeGoalFrozenPolicyError::new(
                "goal policy update became non-finite",
            ));
        }
        self.gradient_updates = self
            .gradient_updates
            .checked_add(1)
            .ok_or_else(|| NativeGoalFrozenPolicyError::new("gradient update count overflowed"))?;
        Ok(())
    }

    fn freeze(
        &self,
        mean: &[f64],
        inverse_stddev: &[f64],
        objective_sha256: Digest,
        hidden_width: usize,
    ) -> Result<FrozenInferenceModel, NativeGoalFrozenPolicyError> {
        let input_width = mean.len();
        let mut first_weights = vec![0.0_f32; hidden_width * input_width];
        let mut first_bias = vec![0.0_f32; hidden_width];
        for hidden in 0..hidden_width {
            let mut bias = self.hidden_bias[hidden];
            for input in 0..input_width {
                let weight = self.hidden_weights[hidden * input_width + input];
                let folded = weight * inverse_stddev[input];
                first_weights[hidden * input_width + input] = finite_f32(folded)?;
                bias -= folded * mean[input];
            }
            first_bias[hidden] = finite_f32(bias)?;
        }
        let second_weights = self
            .output_weights
            .iter()
            .map(|value| finite_f32(*value))
            .collect::<Result<Vec<_>, _>>()?;
        let second_bias = self
            .output_bias
            .iter()
            .map(|value| finite_f32(*value))
            .collect::<Result<Vec<_>, _>>()?;
        let action_schema_sha256 = Digest(
            FactorizedPadPolicyHead::default()
                .schema_sha256()
                .map_err(|error| NativeGoalFrozenPolicyError::new(error.to_string()))?,
        );
        FrozenInferenceModel::new(
            Digest(NATIVE_POLICY_FEATURE_SCHEMA_SHA256),
            action_schema_sha256,
            objective_sha256,
            NATIVE_POLICY_FEATURE_WIDTH,
            (0..FACTORIZED_PAD_POLICY_HEAD_WIDTH as u32).collect(),
            vec![
                FrozenDenseLayer {
                    output_width: hidden_width,
                    activation: FrozenActivation::Relu,
                    weights: first_weights,
                    biases: first_bias,
                },
                FrozenDenseLayer {
                    output_width: FACTORIZED_PAD_POLICY_HEAD_WIDTH,
                    activation: FrozenActivation::Linear,
                    weights: second_weights,
                    biases: second_bias,
                },
            ],
        )
        .map_err(|error| NativeGoalFrozenPolicyError::new(error.to_string()))
    }
}

fn bounded_continuous_contrast_gradient(output: f64, target: f64) -> f64 {
    let difference = output - target;
    let distance = difference.abs();
    if distance >= FAILURE_CONTINUOUS_MARGIN {
        return 0.0;
    }
    let away = if difference > 0.0 {
        1.0
    } else if difference < 0.0 {
        -1.0
    } else if target >= 0.0 {
        -1.0
    } else {
        1.0
    };
    let remaining = (FAILURE_CONTINUOUS_MARGIN - distance) / FAILURE_CONTINUOUS_MARGIN;
    -FAILURE_CONTRAST_STRENGTH * 2.0 * away * remaining / CONTINUOUS_HEADS as f64
}

fn bounded_button_contrast_gradient(logit: f64, target: f64) -> f64 {
    let difference = logistic(logit) - target;
    let distance = difference.abs();
    if distance >= FAILURE_BUTTON_PROBABILITY_MARGIN {
        return 0.0;
    }
    let remaining =
        (FAILURE_BUTTON_PROBABILITY_MARGIN - distance) / FAILURE_BUTTON_PROBABILITY_MARGIN;
    -FAILURE_CONTRAST_STRENGTH * difference * remaining / 16.0
}

fn materialize(
    dataset: &NativeGoalTrajectoryDataset,
    shards: &[NativeEpisodeShard],
) -> Result<Vec<MaterializedPolicyRow>, NativeGoalFrozenPolicyError> {
    if shards.is_empty() {
        return Err(NativeGoalFrozenPolicyError::new(
            "goal policy requires native episode shards",
        ));
    }
    let mut shard_by_digest = BTreeMap::new();
    for shard in shards {
        if shard_by_digest
            .insert(shard.content_sha256, shard)
            .is_some()
        {
            return Err(NativeGoalFrozenPolicyError::new(
                "goal policy received a duplicate native shard",
            ));
        }
    }
    let mut row_ids = BTreeSet::new();
    let mut episode_splits = BTreeMap::new();
    let mut rows = Vec::new();
    for row in &dataset.rows {
        if !row_ids.insert(row.row_sha256) {
            return Err(NativeGoalFrozenPolicyError::new(
                "goal policy dataset duplicates a trajectory row",
            ));
        }
        let episode_sha256 = episode_identity(row);
        if episode_splits
            .insert(episode_sha256, row.split)
            .is_some_and(|prior| prior != row.split)
        {
            return Err(NativeGoalFrozenPolicyError::new(
                "one physical episode crosses goal policy splits",
            ));
        }
        let shard = shard_by_digest.get(&row.shard_sha256).ok_or_else(|| {
            NativeGoalFrozenPolicyError::new("goal policy is missing a referenced native shard")
        })?;
        if shard.metadata.observation_schema != dataset.observation_schema
            || shard.metadata.action_schema != dataset.action_schema
            || shard.metadata.objective != dataset.goal_definition_name
            || shard.metadata.objective_identity != dataset.goal_objective_identity
        {
            return Err(NativeGoalFrozenPolicyError::new(
                "goal policy shard metadata differs from the dataset",
            ));
        }
        let episode = shard
            .episodes
            .iter()
            .find(|episode| episode.id == row.episode_id)
            .ok_or_else(|| NativeGoalFrozenPolicyError::new("trajectory episode is absent"))?;
        if hex_128(episode.payload_xxh3_128) != row.episode_payload_xxh3_128
            || episode.ticks_executed != row.episode_ticks
            || episode.success != row.success
        {
            return Err(NativeGoalFrozenPolicyError::new(
                "trajectory episode identity or outcome differs from its policy row",
            ));
        }
        let step = episode
            .steps
            .get(row.step_index as usize)
            .ok_or_else(|| NativeGoalFrozenPolicyError::new("trajectory step is absent"))?;
        if hex_128(step.pre_input.state_identity) != row.pre_input_state_xxh3_128
            || pad_target(step.consumed_pad) != row.consumed_pad
        {
            return Err(NativeGoalFrozenPolicyError::new(
                "trajectory state or consumed PAD differs from its policy source",
            ));
        }
        let imitates_success = row.success
            && (row.role != ReplayExperienceRole::Demonstration
                || dataset
                    .config
                    .demonstration_mode
                    .imitates_demonstration_actions());
        let contrasts_failure = !row.success && row.role == ReplayExperienceRole::PolicyRollout;
        if imitates_success || contrasts_failure {
            let features = encode_native_policy_observation(&step.pre_input)
                .map_err(|error| NativeGoalFrozenPolicyError::new(error.to_string()))?
                .into_iter()
                .map(f64::from)
                .collect::<Vec<_>>();
            rows.push(MaterializedPolicyRow {
                episode_sha256,
                split: row.split,
                success: row.success,
                features,
                target: policy_target(row.consumed_pad),
                pad: row.consumed_pad,
            });
            if rows.len() > MAX_ROWS {
                return Err(NativeGoalFrozenPolicyError::new(
                    "goal policy trajectory-row limit exceeded",
                ));
            }
        }
    }
    if !rows.iter().any(|row| row.success) {
        return Err(NativeGoalFrozenPolicyError::new(
            "goal policy has no authenticated successful trajectory actions",
        ));
    }
    Ok(rows)
}

fn validate_split_support(
    rows: &[MaterializedPolicyRow],
) -> Result<(), NativeGoalFrozenPolicyError> {
    for split in [
        AuxiliarySplit::Training,
        AuxiliarySplit::Validation,
        AuxiliarySplit::Test,
    ] {
        if !rows.iter().any(|row| row.split == split && row.success) {
            return Err(NativeGoalFrozenPolicyError::new(
                "goal policy requires successful actions in training, validation, and test",
            ));
        }
    }
    Ok(())
}

fn split_indices(rows: &[MaterializedPolicyRow], split: AuxiliarySplit) -> Vec<usize> {
    rows.iter()
        .enumerate()
        .filter_map(|(index, row)| (row.split == split).then_some(index))
        .collect()
}

fn fit_normalization(
    rows: &[MaterializedPolicyRow],
    training: &[usize],
) -> Result<(Vec<f64>, Vec<f64>), NativeGoalFrozenPolicyError> {
    let mut mean = vec![0.0; NATIVE_POLICY_FEATURE_WIDTH];
    for index in training {
        for (mean, value) in mean.iter_mut().zip(&rows[*index].features) {
            *mean += value;
        }
    }
    for value in &mut mean {
        *value /= training.len() as f64;
    }
    let mut variance = vec![0.0; NATIVE_POLICY_FEATURE_WIDTH];
    for index in training {
        for ((variance, value), mean) in variance.iter_mut().zip(&rows[*index].features).zip(&mean)
        {
            *variance += (value - mean).powi(2);
        }
    }
    let inverse_stddev = variance
        .into_iter()
        .map(|value| {
            let stddev = (value / training.len() as f64).sqrt();
            if stddev > 1.0e-8 { 1.0 / stddev } else { 1.0 }
        })
        .collect::<Vec<_>>();
    if mean
        .iter()
        .chain(&inverse_stddev)
        .any(|value| !value.is_finite())
    {
        return Err(NativeGoalFrozenPolicyError::new(
            "goal policy training normalization became non-finite",
        ));
    }
    Ok((mean, inverse_stddev))
}

fn normalize(features: &[f64], mean: &[f64], inverse_stddev: &[f64]) -> Vec<f64> {
    features
        .iter()
        .zip(mean)
        .zip(inverse_stddev)
        .map(|((value, mean), inverse)| (value - mean) * inverse)
        .collect()
}

fn training_mean_output(
    rows: &[MaterializedPolicyRow],
    training: &[usize],
) -> [f64; FACTORIZED_PAD_POLICY_HEAD_WIDTH] {
    let mut output = [0.0; FACTORIZED_PAD_POLICY_HEAD_WIDTH];
    let successful = training
        .iter()
        .filter(|index| rows[**index].success)
        .copied()
        .collect::<Vec<_>>();
    for index in &successful {
        for (output, target) in output.iter_mut().zip(rows[*index].target) {
            *output += target;
        }
    }
    for (head, output) in output.iter_mut().enumerate() {
        *output /= successful.len() as f64;
        if (BUTTON_HEAD_START..BUTTON_HEAD_END).contains(&head) {
            *output = logit(output.clamp(0.01, 0.99));
        }
    }
    output[24] = 0.0;
    output
}

fn evaluate(
    rows: &[MaterializedPolicyRow],
    split: AuxiliarySplit,
    model: &FrozenInferenceModel,
    baseline_output: [f64; FACTORIZED_PAD_POLICY_HEAD_WIDTH],
) -> Result<NativeGoalFrozenPolicyMetrics, NativeGoalFrozenPolicyError> {
    let selected = rows
        .iter()
        .filter(|row| row.split == split && row.success)
        .collect::<Vec<_>>();
    let inputs = selected
        .iter()
        .map(|row| row.features.iter().map(|value| *value as f32).collect())
        .collect::<Vec<Vec<f32>>>();
    let mut outputs = Vec::with_capacity(inputs.len());
    for chunk in inputs.chunks(8_192) {
        outputs.extend(
            model
                .infer_batch(chunk)
                .map_err(|error| NativeGoalFrozenPolicyError::new(error.to_string()))?,
        );
    }
    let baseline = baseline_output.map(|value| value as f32);
    let head = FactorizedPadPolicyHead::default();
    let baseline_pad = head
        .decode(&baseline)
        .and_then(|action| action.realized_pad())
        .map_err(|error| NativeGoalFrozenPolicyError::new(error.to_string()))?;
    let mut continuous_error = 0.0;
    let mut baseline_continuous_error = 0.0;
    let mut button_errors = 0_usize;
    let mut baseline_button_errors = 0_usize;
    let mut exact = 0_usize;
    let mut baseline_exact = 0_usize;
    for ((row, output), input) in selected.iter().zip(outputs).zip(inputs) {
        if output.len() != FACTORIZED_PAD_POLICY_HEAD_WIDTH
            || input.len() != NATIVE_POLICY_FEATURE_WIDTH
        {
            return Err(NativeGoalFrozenPolicyError::new(
                "frozen policy evaluation width drifted",
            ));
        }
        for head_index in 0..CONTINUOUS_HEADS {
            let range = if head_index < 4 {
                -1.0..=1.0
            } else {
                0.0..=1.0
            };
            continuous_error += (f64::from(output[head_index]).clamp(*range.start(), *range.end())
                - row.target[head_index])
                .abs();
            baseline_continuous_error += (baseline_output[head_index]
                .clamp(*range.start(), *range.end())
                - row.target[head_index])
                .abs();
        }
        let predicted_pad = head
            .decode(&output)
            .and_then(|action| action.realized_pad())
            .map_err(|error| NativeGoalFrozenPolicyError::new(error.to_string()))?;
        button_errors += (predicted_pad.buttons ^ row.pad.buttons).count_ones() as usize;
        baseline_button_errors += (baseline_pad.buttons ^ row.pad.buttons).count_ones() as usize;
        exact += usize::from(pad_target(predicted_pad) == row.pad);
        baseline_exact += usize::from(pad_target(baseline_pad) == row.pad);
    }
    let row_count = selected.len();
    let continuous_mae = continuous_error / (row_count * CONTINUOUS_HEADS) as f64;
    let baseline_continuous_mae = baseline_continuous_error / (row_count * CONTINUOUS_HEADS) as f64;
    let button_bit_error_rate = button_errors as f64 / (row_count * 16) as f64;
    let baseline_button_bit_error_rate = baseline_button_errors as f64 / (row_count * 16) as f64;
    let joint_error = (continuous_mae + button_bit_error_rate) / 2.0;
    let baseline_joint_error = (baseline_continuous_mae + baseline_button_bit_error_rate) / 2.0;
    let metrics = NativeGoalFrozenPolicyMetrics {
        rows: row_count,
        episodes: selected
            .iter()
            .map(|row| row.episode_sha256)
            .collect::<BTreeSet<_>>()
            .len(),
        continuous_mae,
        baseline_continuous_mae,
        button_bit_error_rate,
        baseline_button_bit_error_rate,
        joint_error,
        baseline_joint_error,
        joint_relative_improvement: relative_improvement(baseline_joint_error, joint_error),
        decoded_pad_exact_rate: exact as f64 / row_count as f64,
        baseline_decoded_pad_exact_rate: baseline_exact as f64 / row_count as f64,
    };
    if !metrics.validate() {
        return Err(NativeGoalFrozenPolicyError::new(
            "goal frozen policy metrics are invalid",
        ));
    }
    Ok(metrics)
}

fn admission(
    validation: &NativeGoalFrozenPolicyMetrics,
    config: NativeGoalFrozenPolicyConfig,
) -> NativeGoalFrozenPolicyAdmission {
    if validation.joint_relative_improvement >= config.minimum_validation_joint_improvement
        && validation.continuous_mae <= validation.baseline_continuous_mae
        && validation.button_bit_error_rate <= validation.baseline_button_bit_error_rate
        && validation.decoded_pad_exact_rate >= validation.baseline_decoded_pad_exact_rate
    {
        NativeGoalFrozenPolicyAdmission::FrozenPolicyCandidate
    } else {
        NativeGoalFrozenPolicyAdmission::RetainSuccessfulActionMean
    }
}

fn policy_target(pad: AuxiliaryPadTarget) -> [f64; FACTORIZED_PAD_POLICY_HEAD_WIDTH] {
    let mut target = [0.0; FACTORIZED_PAD_POLICY_HEAD_WIDTH];
    for (index, value) in [pad.stick_x, pad.stick_y, pad.substick_x, pad.substick_y]
        .into_iter()
        .enumerate()
    {
        target[index] = signed_byte_unit(value);
    }
    for (index, value) in [
        pad.trigger_left,
        pad.trigger_right,
        pad.analog_a,
        pad.analog_b,
    ]
    .into_iter()
    .enumerate()
    {
        target[4 + index] = f64::from(value) / 255.0;
    }
    for bit in 0..16 {
        target[8 + bit] = f64::from(pad.buttons & (1 << bit) != 0);
    }
    target
}

fn signed_byte_unit(value: i8) -> f64 {
    if value < 0 {
        f64::from(value) / 128.0
    } else {
        f64::from(value) / 127.0
    }
}

fn pad_target(pad: NativeRawPad) -> AuxiliaryPadTarget {
    AuxiliaryPadTarget {
        buttons: pad.buttons,
        stick_x: pad.stick_x,
        stick_y: pad.stick_y,
        substick_x: pad.substick_x,
        substick_y: pad.substick_y,
        trigger_left: pad.trigger_left,
        trigger_right: pad.trigger_right,
        analog_a: pad.analog_a,
        analog_b: pad.analog_b,
    }
}

fn episode_identity(row: &NativeGoalTrajectoryRow) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.physical-native-episode/v1\0");
    hasher.update(row.shard_sha256.0);
    hasher.update(row.episode_payload_xxh3_128.as_bytes());
    Digest(hasher.finalize().into())
}

fn relative_improvement(baseline: f64, candidate: f64) -> f64 {
    if baseline > f64::EPSILON {
        ((baseline - candidate) / baseline).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn initialized_weights(rows: usize, columns: usize, rng: &mut Rng) -> Vec<f64> {
    let scale = (6.0 / (rows + columns) as f64).sqrt();
    (0..rows * columns)
        .map(|_| (rng.unit() * 2.0 - 1.0) * scale)
        .collect()
}

fn dot(left: &[f64], right: &[f64]) -> f64 {
    left.iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum()
}

fn logistic(value: f64) -> f64 {
    if value >= 0.0 {
        1.0 / (1.0 + (-value).exp())
    } else {
        let exponential = value.exp();
        exponential / (1.0 + exponential)
    }
}

fn logit(probability: f64) -> f64 {
    (probability / (1.0 - probability)).ln()
}

fn clip(value: f64, maximum: f64) -> f64 {
    value.clamp(-maximum, maximum)
}

fn finite_f32(value: f64) -> Result<f32, NativeGoalFrozenPolicyError> {
    let value = value as f32;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(NativeGoalFrozenPolicyError::new(
            "goal policy parameter cannot be represented in frozen f32",
        ))
    }
}

fn hex_128(value: [u8; 16]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn canonical_digest<T: Serialize>(
    domain: &[u8],
    value: &T,
) -> Result<Digest, NativeGoalFrozenPolicyError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| NativeGoalFrozenPolicyError::new(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

struct Rng {
    state: u64,
}

impl Rng {
    fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0x9e37_79b9_7f4a_7c15
            } else {
                seed
            },
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut value = self.state;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.state = value;
        value
    }

    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1_u64 << 53) as f64
    }

    fn shuffle<T>(&mut self, values: &mut [T]) {
        for index in (1..values.len()).rev() {
            values.swap(index, (self.next_u64() % (index as u64 + 1)) as usize);
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeGoalFrozenPolicyError(String);

impl NativeGoalFrozenPolicyError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for NativeGoalFrozenPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeGoalFrozenPolicyError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiled_goal_graph::CompiledGoalGraph;
    use crate::factorized_policy_suffix_batch::NativeFactorizedPolicyBatchConfig;
    use crate::milestone_dsl::compile_source;
    use crate::native_frozen_policy_suffix_batch::NativeFrozenPolicySuffixBatch;
    use crate::native_goal_reachability::{
        NativeGoalReachabilityConfig, NativeGoalReachabilityModel,
    };
    use crate::native_goal_trajectory::NativeGoalTrajectoryConfig;
    use crate::native_replay_corpus::{
        NativeReplayCorpus, ReplayEpisodeSource, ReplayExperienceRole,
    };
    use dusklight_evidence::native_episode_shard::authored_milestone_objective_identity;

    const GOAL_SOURCE: &str = r#"milestones 1.8
milestone reach_goal {
  phase post_sim
  when stage.room == 1
}
"#;

    fn graph() -> CompiledGoalGraph {
        let compiled = compile_source(GOAL_SOURCE).unwrap();
        CompiledGoalGraph::from_compiled(&compiled, 0).unwrap()
    }

    fn sources() -> (
        NativeEpisodeShard,
        NativeGoalTrajectoryDataset,
        NativeGoalReachabilityModel,
    ) {
        sources_with_failure_role(ReplayExperienceRole::RandomizedCoverage)
    }

    fn sources_with_failure_role(
        failure_role: ReplayExperienceRole,
    ) -> (
        NativeEpisodeShard,
        NativeGoalTrajectoryDataset,
        NativeGoalReachabilityModel,
    ) {
        let mut shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v14.dseps"
        ))
        .unwrap();
        let graph = graph();
        shard.metadata.objective = graph.definition_name.clone();
        shard.metadata.objective_identity = authored_milestone_objective_identity(
            &graph.program_sha256.to_string(),
            &graph.definition_sha256.to_string(),
        )
        .unwrap();
        let success_template = shard
            .episodes
            .iter()
            .find(|episode| episode.success)
            .unwrap()
            .clone();
        let failure_template = shard
            .episodes
            .iter()
            .find(|episode| !episode.success)
            .unwrap()
            .clone();
        shard.episodes = (0..120_u32)
            .map(|episode_index| {
                let success = episode_index % 2 == 0;
                let mut episode = if success {
                    success_template.clone()
                } else {
                    failure_template.clone()
                };
                episode.id = format!("policy-episode-{episode_index:04}");
                let digest = Sha256::digest(episode.id.as_bytes());
                episode.payload_xxh3_128.copy_from_slice(&digest[..16]);
                episode.success = success;
                episode.ticks_executed = 5;
                episode.first_hit_tick = success.then_some(4);
                let template = episode.steps[0].clone();
                episode.steps = (0..5_u32)
                    .map(|step_index| {
                        let positive = step_index % 2 == 0;
                        let mut step = template.clone();
                        step.pre_input.player_position[0] = if positive { 20.0 } else { -20.0 };
                        step.pre_input.player_position[1] = if success { 20.0 } else { -20.0 };
                        step.pre_input.remaining_ticks = 5 - step_index;
                        step.pre_input.state_identity =
                            state_identity(episode_index, step_index, 0);
                        step.post_simulation.state_identity =
                            state_identity(episode_index, step_index, 1);
                        let pad = NativeRawPad {
                            buttons: if positive { 1 } else { 0 },
                            stick_x: if positive { 64 } else { -64 },
                            stick_y: if positive { 32 } else { -32 },
                            substick_x: 0,
                            substick_y: 0,
                            trigger_left: if positive { 200 } else { 10 },
                            trigger_right: 0,
                            analog_a: 0,
                            analog_b: 0,
                            connected: true,
                            error: 0,
                        };
                        step.chosen_pad = pad;
                        step.consumed_pad = pad;
                        step
                    })
                    .collect();
                episode
            })
            .collect();
        let replay_sources = shard
            .episodes
            .iter()
            .enumerate()
            .map(|(episode_index, episode)| ReplayEpisodeSource {
                shard: &shard,
                episode_index,
                role: if episode.success {
                    ReplayExperienceRole::RandomizedCoverage
                } else {
                    failure_role
                },
                policy_lineage_sha256: (!episode.success
                    && failure_role == ReplayExperienceRole::PolicyRollout)
                    .then_some(Digest([9; 32])),
                parent_entry_sha256: None,
            })
            .collect::<Vec<_>>();
        let corpus = NativeReplayCorpus::build(None, &replay_sources).unwrap();
        let dataset = (0..10_000_u64)
            .find_map(|split_seed| {
                let dataset = NativeGoalTrajectoryDataset::build(
                    &corpus,
                    std::slice::from_ref(&shard),
                    &graph,
                    NativeGoalTrajectoryConfig {
                        demonstration_mode: DemonstrationMode::BehaviorCloningWarmStart,
                        n_step: 2,
                        discount_millionths: 900_000,
                        training_basis_points: 6_000,
                        validation_basis_points: 2_000,
                        split_seed,
                    },
                )
                .unwrap();
                split_has_both(&dataset, AuxiliarySplit::Training)
                    .then_some(dataset)
                    .filter(|dataset| split_has_both(dataset, AuxiliarySplit::Validation))
                    .filter(|dataset| split_has_both(dataset, AuxiliarySplit::Test))
            })
            .expect("a balanced deterministic episode split");
        let reachability = NativeGoalReachabilityModel::fit(
            std::slice::from_ref(&dataset),
            std::slice::from_ref(&shard),
            NativeGoalReachabilityConfig {
                members: 3,
                epochs: 18,
                hidden_width: 4,
                learning_rate: 0.02,
                l2_penalty: 1.0e-6,
                gradient_clip: 5.0,
                minimum_validation_improvement: 0.02,
                maximum_validation_reachability_stddev: 0.5,
                seed: 0x1234_5678_9abc_def0,
            },
        )
        .unwrap();
        assert_eq!(
            reachability.admission,
            NativeGoalReachabilityAdmission::GoalConditionedCandidate
        );
        (shard, dataset, reachability)
    }

    fn state_identity(episode: u32, step: u32, phase: u8) -> [u8; 16] {
        let mut hasher = Sha256::new();
        hasher.update(episode.to_le_bytes());
        hasher.update(step.to_le_bytes());
        hasher.update([phase]);
        hasher.finalize()[..16].try_into().unwrap()
    }

    fn split_has_both(dataset: &NativeGoalTrajectoryDataset, split: AuxiliarySplit) -> bool {
        dataset
            .rows
            .iter()
            .any(|row| row.split == split && row.success)
            && dataset
                .rows
                .iter()
                .any(|row| row.split == split && !row.success)
    }

    fn config() -> NativeGoalFrozenPolicyConfig {
        NativeGoalFrozenPolicyConfig {
            epochs: 80,
            hidden_width: 8,
            learning_rate: 0.01,
            l2_penalty: 1.0e-6,
            gradient_clip: 5.0,
            minimum_validation_joint_improvement: 0.02,
            seed: 0x9988_7766_5544_3322,
        }
    }

    #[test]
    fn trains_and_directly_exports_a_deterministic_native_policy() {
        let (shard, dataset, reachability) = sources();
        let first = NativeGoalFrozenPolicyExport::fit(
            &dataset,
            std::slice::from_ref(&shard),
            &reachability,
            config(),
        )
        .unwrap();
        first.validate().unwrap();
        assert_eq!(
            first.manifest.admission,
            NativeGoalFrozenPolicyAdmission::FrozenPolicyCandidate,
            "{:?}",
            first.manifest.validation
        );
        assert!(
            first.manifest.validation.joint_error < first.manifest.validation.baseline_joint_error
        );
        assert!(first.manifest.test.joint_error < first.manifest.test.baseline_joint_error);
        let decoded: NativeGoalFrozenPolicyManifest =
            serde_json::from_slice(&serde_json::to_vec(&first.manifest).unwrap()).unwrap();
        decoded.validate(&first.model_bytes).unwrap();

        let model = FrozenInferenceModel::from_bytes(&first.model_bytes).unwrap();
        let batch = NativeFrozenPolicySuffixBatch::build(
            &first.model_bytes,
            "policy.dsfrozen".into(),
            first.manifest.objective_sha256,
            "trained-goal-policy".into(),
            NativeFactorizedPolicyBatchConfig {
                source_frame: 440,
                source_boundary_fingerprint: "1f849e432274771426236d60fbf7d72f".into(),
                checkpoint_validation_ticks: 2,
                maximum_ticks: 5,
                verify_state_hashes: true,
            },
        )
        .unwrap();
        assert_eq!(
            batch.frozen_policy.model_xxh3_128,
            first.manifest.frozen_model_xxh3_128
        );
        let success = shard
            .episodes
            .iter()
            .find(|episode| episode.success)
            .unwrap();
        let inputs = [0_usize, 1]
            .into_iter()
            .map(|index| {
                encode_native_policy_observation(&success.steps[index].pre_input)
                    .unwrap()
                    .to_vec()
            })
            .collect::<Vec<_>>();
        let outputs = model.infer_batch(&inputs).unwrap();
        let head = FactorizedPadPolicyHead::default();
        let positive = head.decode(&outputs[0]).unwrap().realized_pad().unwrap();
        let negative = head.decode(&outputs[1]).unwrap().realized_pad().unwrap();
        assert!(positive.stick_x > 0);
        assert!(negative.stick_x < 0);
        assert_eq!(positive.buttons & 1, 1);
        assert_eq!(negative.buttons & 1, 0);

        let second = NativeGoalFrozenPolicyExport::fit(
            &dataset,
            std::slice::from_ref(&shard),
            &reachability,
            config(),
        )
        .unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn authenticated_policy_failures_change_the_next_frozen_objective() {
        let (baseline_shard, baseline_dataset, baseline_reachability) = sources();
        let baseline = NativeGoalFrozenPolicyExport::fit(
            &baseline_dataset,
            std::slice::from_ref(&baseline_shard),
            &baseline_reachability,
            config(),
        )
        .unwrap();
        assert_eq!(baseline.manifest.training_failed_rows, 0);

        let (contrast_shard, contrast_dataset, contrast_reachability) =
            sources_with_failure_role(ReplayExperienceRole::PolicyRollout);
        let contrast = NativeGoalFrozenPolicyExport::fit(
            &contrast_dataset,
            std::slice::from_ref(&contrast_shard),
            &contrast_reachability,
            config(),
        )
        .unwrap();
        contrast.validate().unwrap();
        assert!(contrast.manifest.training_failed_rows > 0);
        assert_eq!(contrast.manifest.failure_contrast_strength, 0.01);
        assert_eq!(contrast.manifest.failure_continuous_margin, 0.10);
        assert_eq!(contrast.manifest.failure_button_probability_margin, 0.10);
        assert_eq!(
            contrast.manifest.admission,
            NativeGoalFrozenPolicyAdmission::FrozenPolicyCandidate,
            "{:?}",
            contrast.manifest.validation
        );
        assert_ne!(baseline.model_bytes, contrast.model_bytes);
        assert_ne!(
            baseline.manifest.frozen_artifact_sha256,
            contrast.manifest.frozen_artifact_sha256
        );
    }

    #[test]
    fn failed_action_contrast_is_directional_and_stops_at_its_margin() {
        assert!(bounded_continuous_contrast_gradient(0.5, 0.5) > 0.0);
        assert!(bounded_continuous_contrast_gradient(-0.25, -0.25) < 0.0);
        assert_eq!(
            bounded_continuous_contrast_gradient(0.7, 0.5),
            0.0
        );

        let near_pressed = logit(0.95);
        let near_released = logit(0.05);
        assert!(bounded_button_contrast_gradient(near_pressed, 1.0) > 0.0);
        assert!(bounded_button_contrast_gradient(near_released, 0.0) < 0.0);
        assert_eq!(bounded_button_contrast_gradient(logit(0.8), 1.0), 0.0);
        assert_eq!(bounded_button_contrast_gradient(logit(0.2), 0.0), 0.0);
    }

    #[test]
    fn replay_only_demonstrations_train_no_policy_action_targets() {
        let (shard, mut dataset, _) = sources();
        for row in &mut dataset.rows {
            if row.success {
                row.role = ReplayExperienceRole::Demonstration;
            }
        }
        dataset.config.demonstration_mode = DemonstrationMode::ReplayOnly;
        assert!(materialize(&dataset, std::slice::from_ref(&shard)).is_err());

        dataset.config.demonstration_mode = DemonstrationMode::BehaviorCloningWarmStart;
        assert!(!materialize(&dataset, &[shard]).unwrap().is_empty());
    }

    #[test]
    fn source_manifest_and_frozen_byte_tampering_fail_closed() {
        let (shard, dataset, reachability) = sources();
        let export = NativeGoalFrozenPolicyExport::fit(
            &dataset,
            std::slice::from_ref(&shard),
            &reachability,
            config(),
        )
        .unwrap();

        let mut detached = shard.clone();
        detached.episodes[0].steps[0].pre_input.state_identity[0] ^= 1;
        assert!(
            NativeGoalFrozenPolicyExport::fit(
                &dataset,
                std::slice::from_ref(&detached),
                &reachability,
                config(),
            )
            .is_err()
        );

        let mut bytes = export.model_bytes.clone();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        assert!(export.manifest.validate(&bytes).is_err());

        let mut manifest = export.manifest;
        manifest.promotion_authority = true;
        manifest.manifest_sha256 = manifest.digest().unwrap();
        assert!(manifest.validate(&export.model_bytes).is_err());
    }
}
