//! Deterministic, trainable complete-set and fixed-slot representation baselines.
//!
//! This module intentionally owns only an offline representation experiment. It
//! does not rank routes or declare gameplay success. Exact categorical values
//! remain categorical through a training-corpus vocabulary, optional values
//! carry explicit masks, and actor runtime IDs are used only to canonicalize
//! enumeration. The complete-set model learns a shared per-node transform and
//! permutation-invariant pooling before its prediction head.

use crate::artifact::Digest;
use crate::native_actor_features::NativeActorFeatureView;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const TRAINABLE_SET_COMPARISON_SCHEMA_V1: &str = "dusklight-trainable-set-comparison/v1";
const MAX_SAMPLES: usize = 100_000;
const MAX_NODES: usize = u16::MAX as usize;
const MAX_FEATURE_COLUMNS: usize = 256;
const MAX_HIDDEN_WIDTH: usize = 256;
const MAX_EPOCHS: usize = 2_048;
const MAX_FIXED_SLOTS: usize = 256;
const MAX_CATEGORY_VALUES: usize = 65_536;
const MAX_PARAMETERS: usize = 16_000_000;

#[derive(Clone, Debug)]
pub struct TypedSetNode {
    /// Structural identity only. It is never presented to either model.
    pub stable_id: u64,
    pub categorical: Vec<i64>,
    pub categorical_present: Vec<bool>,
    pub continuous: Vec<f32>,
    pub continuous_present: Vec<bool>,
    pub binary: Vec<bool>,
    pub binary_present: Vec<bool>,
}

#[derive(Clone, Debug)]
pub struct TypedSetSample {
    pub sample_sha256: Digest,
    pub actor_feature_schema_sha256: Digest,
    pub base: Vec<f32>,
    pub base_present: Vec<bool>,
    pub nodes: Vec<TypedSetNode>,
    pub target: f32,
}

impl TypedSetSample {
    /// Materialize one complete model sample from a sealed native actor-feature
    /// observation. Goal-relative vectors remain optional numeric channels and
    /// runtime generations remain structural ordering keys.
    pub fn from_native_actor_observation(
        view: &NativeActorFeatureView,
        observation_index: usize,
        sample_sha256: Digest,
        base: Vec<f32>,
        base_present: Vec<bool>,
        target: f32,
    ) -> Result<Self, TrainableSetError> {
        view.validate()
            .map_err(|error| TrainableSetError::new(error.to_string()))?;
        let observation = view
            .observations
            .get(observation_index)
            .ok_or_else(|| TrainableSetError::new("actor-feature observation index is invalid"))?;
        Ok(Self {
            sample_sha256,
            actor_feature_schema_sha256: view.feature_schema_sha256,
            base,
            base_present,
            nodes: observation.actors.iter().map(native_node).collect(),
            target,
        })
    }
}

fn native_node(row: &crate::native_actor_features::NativeActorFeatureRow) -> TypedSetNode {
    let mut continuous = row.continuous.clone();
    let mut continuous_present = row.continuous_present.clone();
    for position in &row.goal_relative_positions {
        continuous.extend(position.unwrap_or([0.0; 3]));
        continuous_present.extend([position.is_some(); 3]);
    }
    TypedSetNode {
        stable_id: row.runtime_generation,
        categorical: row.categorical.clone(),
        categorical_present: row.categorical_present.clone(),
        continuous,
        continuous_present,
        binary: row.binary.clone(),
        binary_present: row.binary_present.clone(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct TrainableSetConfig {
    pub epochs: usize,
    pub node_hidden_width: usize,
    pub head_hidden_width: usize,
    pub fixed_slot_count: usize,
    pub learning_rate: f64,
    pub l2_penalty: f64,
    pub gradient_clip: f64,
    pub minimum_relative_improvement: f64,
    pub seed: u64,
}

impl Default for TrainableSetConfig {
    fn default() -> Self {
        Self {
            epochs: 128,
            node_hidden_width: 32,
            head_hidden_width: 32,
            fixed_slot_count: 16,
            learning_rate: 0.003,
            l2_penalty: 1.0e-5,
            gradient_clip: 5.0,
            minimum_relative_improvement: 0.05,
            seed: 0x5e71_c0de_d33f_0001,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetEncoderDecision {
    RetainFixedSlots,
    CompleteSetCandidate,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SetRegressorMetrics {
    pub name: &'static str,
    pub parameter_count: usize,
    pub training_rows: usize,
    pub held_out_rows: usize,
    pub optimizer_steps: u64,
    pub training_mse: f64,
    pub held_out_mse: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TrainableSetComparison {
    pub schema: &'static str,
    pub actor_feature_schema_sha256: Digest,
    pub training_dataset_sha256: Digest,
    pub held_out_dataset_sha256: Digest,
    pub config: TrainableSetConfig,
    pub categorical_width: usize,
    pub continuous_width: usize,
    pub binary_width: usize,
    pub base_width: usize,
    pub maximum_training_nodes: usize,
    pub maximum_held_out_nodes: usize,
    pub complete_set_model_sha256: Digest,
    pub fixed_slot_model_sha256: Digest,
    pub complete_set: SetRegressorMetrics,
    pub fixed_slots: SetRegressorMetrics,
    pub equal_training_row_budget: bool,
    pub equal_held_out_row_budget: bool,
    pub equal_optimizer_step_budget: bool,
    pub relative_held_out_improvement: f64,
    pub decision: SetEncoderDecision,
    pub promotion_authority: bool,
    pub comparison_sha256: Digest,
}

#[derive(Clone, Debug, Serialize)]
pub struct CompleteSetRegressor {
    actor_feature_schema_sha256: Digest,
    layout: FeatureLayout,
    config: TrainableSetConfig,
    node_weights: Vec<f64>,
    node_bias: Vec<f64>,
    head_weights: Vec<f64>,
    head_bias: Vec<f64>,
    output_weights: Vec<f64>,
    output_bias: f64,
    optimizer_steps: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct FixedSlotRegressor {
    actor_feature_schema_sha256: Digest,
    layout: FeatureLayout,
    config: TrainableSetConfig,
    head_weights: Vec<f64>,
    head_bias: Vec<f64>,
    output_weights: Vec<f64>,
    output_bias: f64,
    optimizer_steps: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct FeatureLayout {
    pub(crate) categorical_values: Vec<Vec<i64>>,
    pub(crate) continuous_mean: Vec<f64>,
    pub(crate) continuous_inverse_stddev: Vec<f64>,
    pub(crate) base_mean: Vec<f64>,
    pub(crate) base_inverse_stddev: Vec<f64>,
    pub(crate) categorical_width: usize,
    pub(crate) continuous_width: usize,
    pub(crate) binary_width: usize,
    pub(crate) base_width: usize,
    pub(crate) node_input_width: usize,
    pub(crate) base_input_width: usize,
}

impl TrainableSetComparison {
    #[allow(clippy::too_many_arguments)]
    pub fn fit(
        actor_feature_schema_sha256: Digest,
        training_dataset_sha256: Digest,
        held_out_dataset_sha256: Digest,
        training: &[TypedSetSample],
        held_out: &[TypedSetSample],
        config: TrainableSetConfig,
    ) -> Result<(Self, CompleteSetRegressor, FixedSlotRegressor), TrainableSetError> {
        let dimensions = validate_samples(
            actor_feature_schema_sha256,
            training_dataset_sha256,
            held_out_dataset_sha256,
            training,
            held_out,
            config,
        )?;
        let layout = FeatureLayout::fit(training, dimensions)?;
        let mut complete_set = CompleteSetRegressor::initialized(
            actor_feature_schema_sha256,
            layout.clone(),
            config,
            config.seed,
        )?;
        let mut fixed_slots = FixedSlotRegressor::initialized(
            actor_feature_schema_sha256,
            layout,
            config,
            config.seed ^ 0xf17e_d510_7bad_0001,
        )?;
        let mut order = (0..training.len()).collect::<Vec<_>>();
        let mut rng = DeterministicRng::new(config.seed ^ 0x7a11_5eed_0000_0001);
        for _ in 0..config.epochs {
            rng.shuffle(&mut order);
            for &index in &order {
                complete_set.train_one(&training[index])?;
                fixed_slots.train_one(&training[index])?;
            }
        }
        let complete_metrics = complete_set.metrics(training, held_out)?;
        let fixed_metrics = fixed_slots.metrics(training, held_out)?;
        let complete_set_model_sha256 = complete_set.model_sha256()?;
        let fixed_slot_model_sha256 = fixed_slots.model_sha256()?;
        let equal_training_row_budget =
            complete_metrics.training_rows == fixed_metrics.training_rows;
        let equal_held_out_row_budget =
            complete_metrics.held_out_rows == fixed_metrics.held_out_rows;
        let equal_optimizer_step_budget =
            complete_metrics.optimizer_steps == fixed_metrics.optimizer_steps;
        if !equal_training_row_budget || !equal_held_out_row_budget || !equal_optimizer_step_budget
        {
            return Err(TrainableSetError::new(
                "set comparison did not preserve equal data and optimizer-step budgets",
            ));
        }
        let relative_held_out_improvement = if fixed_metrics.held_out_mse > f64::EPSILON {
            (fixed_metrics.held_out_mse - complete_metrics.held_out_mse)
                / fixed_metrics.held_out_mse
        } else {
            0.0
        };
        let decision = if relative_held_out_improvement >= config.minimum_relative_improvement {
            SetEncoderDecision::CompleteSetCandidate
        } else {
            SetEncoderDecision::RetainFixedSlots
        };
        let mut comparison = Self {
            schema: TRAINABLE_SET_COMPARISON_SCHEMA_V1,
            actor_feature_schema_sha256,
            training_dataset_sha256,
            held_out_dataset_sha256,
            config,
            categorical_width: dimensions.categorical,
            continuous_width: dimensions.continuous,
            binary_width: dimensions.binary,
            base_width: dimensions.base,
            maximum_training_nodes: training
                .iter()
                .map(|sample| sample.nodes.len())
                .max()
                .unwrap(),
            maximum_held_out_nodes: held_out
                .iter()
                .map(|sample| sample.nodes.len())
                .max()
                .unwrap(),
            complete_set_model_sha256,
            fixed_slot_model_sha256,
            complete_set: complete_metrics,
            fixed_slots: fixed_metrics,
            equal_training_row_budget,
            equal_held_out_row_budget,
            equal_optimizer_step_budget,
            relative_held_out_improvement,
            decision,
            promotion_authority: false,
            comparison_sha256: Digest::ZERO,
        };
        comparison.comparison_sha256 = comparison.digest()?;
        Ok((comparison, complete_set, fixed_slots))
    }

    fn digest(&self) -> Result<Digest, TrainableSetError> {
        let mut canonical = self.clone();
        canonical.comparison_sha256 = Digest::ZERO;
        canonical_digest(b"dusklight.trainable-set-comparison/v1\0", &canonical)
    }
}

impl CompleteSetRegressor {
    fn initialized(
        actor_feature_schema_sha256: Digest,
        layout: FeatureLayout,
        config: TrainableSetConfig,
        seed: u64,
    ) -> Result<Self, TrainableSetError> {
        let head_input_width = layout.base_input_width + 2 + config.node_hidden_width * 2;
        let parameter_count = config
            .node_hidden_width
            .checked_mul(layout.node_input_width + 1)
            .and_then(|value| value.checked_add(config.head_hidden_width * (head_input_width + 1)))
            .and_then(|value| value.checked_add(config.head_hidden_width + 1))
            .ok_or_else(|| TrainableSetError::new("complete-set parameter count overflowed"))?;
        if parameter_count > MAX_PARAMETERS {
            return Err(TrainableSetError::new(
                "complete-set model exceeds its parameter budget",
            ));
        }
        let mut rng = DeterministicRng::new(seed);
        Ok(Self {
            actor_feature_schema_sha256,
            node_weights: initialized_weights(
                config.node_hidden_width,
                layout.node_input_width,
                &mut rng,
            ),
            node_bias: vec![0.0; config.node_hidden_width],
            head_weights: initialized_weights(config.head_hidden_width, head_input_width, &mut rng),
            head_bias: vec![0.0; config.head_hidden_width],
            output_weights: initialized_weights(1, config.head_hidden_width, &mut rng),
            output_bias: 0.0,
            layout,
            config,
            optimizer_steps: 0,
        })
    }

    pub fn encode(&self, sample: &TypedSetSample) -> Result<Vec<f32>, TrainableSetError> {
        self.validate_sample(sample)?;
        let forward = self.forward(sample);
        Ok(forward
            .mean_pool
            .iter()
            .chain(&forward.max_pool)
            .map(|value| *value as f32)
            .collect())
    }

    pub fn predict(&self, sample: &TypedSetSample) -> Result<f32, TrainableSetError> {
        self.validate_sample(sample)?;
        Ok(self.forward(sample).prediction as f32)
    }

    pub fn parameter_count(&self) -> usize {
        self.node_weights.len()
            + self.node_bias.len()
            + self.head_weights.len()
            + self.head_bias.len()
            + self.output_weights.len()
            + 1
    }

    pub fn model_sha256(&self) -> Result<Digest, TrainableSetError> {
        canonical_digest(b"dusklight.complete-set-regressor/v1\0", self)
    }

    fn validate_sample(&self, sample: &TypedSetSample) -> Result<(), TrainableSetError> {
        if sample.actor_feature_schema_sha256 != self.actor_feature_schema_sha256 {
            return Err(TrainableSetError::new(
                "complete-set sample feature schema does not match the model",
            ));
        }
        validate_sample_dimensions(sample, self.layout.dimensions())
    }

    fn forward(&self, sample: &TypedSetSample) -> SetForward {
        let ordered = ordered_nodes(&sample.nodes);
        let node_inputs = ordered
            .iter()
            .map(|node| self.layout.node_input(node))
            .collect::<Vec<_>>();
        let node_hidden = node_inputs
            .iter()
            .map(|input| {
                dense_tanh(
                    input,
                    &self.node_weights,
                    &self.node_bias,
                    self.config.node_hidden_width,
                )
            })
            .collect::<Vec<_>>();
        let mut mean_pool = vec![0.0; self.config.node_hidden_width];
        let mut max_pool = vec![0.0; self.config.node_hidden_width];
        let mut max_indices = vec![None; self.config.node_hidden_width];
        if !node_hidden.is_empty() {
            max_pool.fill(f64::NEG_INFINITY);
            for (node_index, hidden) in node_hidden.iter().enumerate() {
                for index in 0..hidden.len() {
                    mean_pool[index] += hidden[index];
                    if hidden[index] > max_pool[index] {
                        max_pool[index] = hidden[index];
                        max_indices[index] = Some(node_index);
                    }
                }
            }
            for value in &mut mean_pool {
                *value /= node_hidden.len() as f64;
            }
        }
        let mut head_input = self.layout.base_input(sample);
        head_input.push(f64::from(!sample.nodes.is_empty()));
        head_input.push((sample.nodes.len() as f64).ln_1p() / (MAX_NODES as f64).ln_1p());
        head_input.extend(&mean_pool);
        head_input.extend(&max_pool);
        let head_hidden = dense_tanh(
            &head_input,
            &self.head_weights,
            &self.head_bias,
            self.config.head_hidden_width,
        );
        let prediction = dot(&head_hidden, &self.output_weights) + self.output_bias;
        SetForward {
            node_inputs,
            node_hidden,
            mean_pool,
            max_pool,
            max_indices,
            head_input,
            head_hidden,
            prediction,
        }
    }

    fn train_one(&mut self, sample: &TypedSetSample) -> Result<(), TrainableSetError> {
        self.validate_sample(sample)?;
        let forward = self.forward(sample);
        let error = forward.prediction - f64::from(sample.target);
        if !error.is_finite() {
            return Err(TrainableSetError::new(
                "complete-set prediction became non-finite",
            ));
        }
        let output_weights_before = self.output_weights.clone();
        let head_weights_before = self.head_weights.clone();
        let mut d_prediction = 2.0 * error;
        d_prediction = clip(d_prediction, self.config.gradient_clip);

        for (weight, hidden) in self.output_weights.iter_mut().zip(&forward.head_hidden) {
            let gradient = d_prediction * hidden + self.config.l2_penalty * *weight;
            *weight -= self.config.learning_rate * clip(gradient, self.config.gradient_clip);
        }
        self.output_bias -= self.config.learning_rate * d_prediction;

        let mut d_head_pre = vec![0.0; self.config.head_hidden_width];
        for index in 0..d_head_pre.len() {
            d_head_pre[index] = d_prediction
                * output_weights_before[index]
                * (1.0 - forward.head_hidden[index].powi(2));
        }
        let mut d_head_input = vec![0.0; forward.head_input.len()];
        for (output, delta) in d_head_pre.iter().copied().enumerate() {
            for (input, d_input) in d_head_input.iter_mut().enumerate() {
                let parameter = output * forward.head_input.len() + input;
                *d_input += head_weights_before[parameter] * delta;
                let gradient = delta * forward.head_input[input]
                    + self.config.l2_penalty * self.head_weights[parameter];
                self.head_weights[parameter] -=
                    self.config.learning_rate * clip(gradient, self.config.gradient_clip);
            }
            self.head_bias[output] -=
                self.config.learning_rate * clip(delta, self.config.gradient_clip);
        }

        let pool_offset = self.layout.base_input_width + 2;
        let d_mean = &d_head_input[pool_offset..pool_offset + self.config.node_hidden_width];
        let d_max = &d_head_input[pool_offset + self.config.node_hidden_width..];
        let node_count = forward.node_hidden.len();
        let mut node_weight_gradient = vec![0.0; self.node_weights.len()];
        let mut node_bias_gradient = vec![0.0; self.node_bias.len()];
        for node_index in 0..node_count {
            let mut d_node_pre = vec![0.0; self.config.node_hidden_width];
            for hidden in 0..self.config.node_hidden_width {
                let mut gradient = d_mean[hidden] / node_count as f64;
                if forward.max_indices[hidden] == Some(node_index) {
                    gradient += d_max[hidden];
                }
                d_node_pre[hidden] =
                    gradient * (1.0 - forward.node_hidden[node_index][hidden].powi(2));
            }
            for (output, delta) in d_node_pre.iter().copied().enumerate() {
                for input in 0..self.layout.node_input_width {
                    let parameter = output * self.layout.node_input_width + input;
                    node_weight_gradient[parameter] +=
                        delta * forward.node_inputs[node_index][input];
                }
                node_bias_gradient[output] += delta;
            }
        }
        for (weight, gradient) in self.node_weights.iter_mut().zip(node_weight_gradient) {
            let gradient = gradient + self.config.l2_penalty * *weight;
            *weight -= self.config.learning_rate * clip(gradient, self.config.gradient_clip);
        }
        for (bias, gradient) in self.node_bias.iter_mut().zip(node_bias_gradient) {
            *bias -= self.config.learning_rate * clip(gradient, self.config.gradient_clip);
        }
        self.optimizer_steps += 1;
        self.validate_parameters()
    }

    fn validate_parameters(&self) -> Result<(), TrainableSetError> {
        if self
            .node_weights
            .iter()
            .chain(&self.node_bias)
            .chain(&self.head_weights)
            .chain(&self.head_bias)
            .chain(&self.output_weights)
            .chain(std::iter::once(&self.output_bias))
            .any(|value| !value.is_finite())
        {
            return Err(TrainableSetError::new(
                "complete-set parameters became non-finite",
            ));
        }
        Ok(())
    }

    fn metrics(
        &self,
        training: &[TypedSetSample],
        held_out: &[TypedSetSample],
    ) -> Result<SetRegressorMetrics, TrainableSetError> {
        Ok(SetRegressorMetrics {
            name: "trainable_complete_deepsets",
            parameter_count: self.parameter_count(),
            training_rows: training.len(),
            held_out_rows: held_out.len(),
            optimizer_steps: self.optimizer_steps,
            training_mse: mse(training, |sample| self.predict(sample))?,
            held_out_mse: mse(held_out, |sample| self.predict(sample))?,
        })
    }
}

impl FixedSlotRegressor {
    fn initialized(
        actor_feature_schema_sha256: Digest,
        layout: FeatureLayout,
        config: TrainableSetConfig,
        seed: u64,
    ) -> Result<Self, TrainableSetError> {
        let head_input_width = layout
            .base_input_width
            .checked_add(2)
            .and_then(|value| {
                value.checked_add(config.fixed_slot_count * (layout.node_input_width + 1))
            })
            .ok_or_else(|| TrainableSetError::new("fixed-slot input width overflowed"))?;
        let parameter_count = config
            .head_hidden_width
            .checked_mul(head_input_width + 1)
            .and_then(|value| value.checked_add(config.head_hidden_width + 1))
            .ok_or_else(|| TrainableSetError::new("fixed-slot parameter count overflowed"))?;
        if parameter_count > MAX_PARAMETERS {
            return Err(TrainableSetError::new(
                "fixed-slot model exceeds its parameter budget",
            ));
        }
        let mut rng = DeterministicRng::new(seed);
        Ok(Self {
            actor_feature_schema_sha256,
            head_weights: initialized_weights(config.head_hidden_width, head_input_width, &mut rng),
            head_bias: vec![0.0; config.head_hidden_width],
            output_weights: initialized_weights(1, config.head_hidden_width, &mut rng),
            output_bias: 0.0,
            layout,
            config,
            optimizer_steps: 0,
        })
    }

    pub fn predict(&self, sample: &TypedSetSample) -> Result<f32, TrainableSetError> {
        if sample.actor_feature_schema_sha256 != self.actor_feature_schema_sha256 {
            return Err(TrainableSetError::new(
                "fixed-slot sample feature schema does not match the model",
            ));
        }
        validate_sample_dimensions(sample, self.layout.dimensions())?;
        Ok(self.forward(sample).2 as f32)
    }

    pub fn parameter_count(&self) -> usize {
        self.head_weights.len() + self.head_bias.len() + self.output_weights.len() + 1
    }

    pub fn model_sha256(&self) -> Result<Digest, TrainableSetError> {
        canonical_digest(b"dusklight.fixed-slot-regressor/v1\0", self)
    }

    fn input(&self, sample: &TypedSetSample) -> Vec<f64> {
        let ordered = ordered_nodes(&sample.nodes);
        let mut input = self.layout.base_input(sample);
        input.push(f64::from(!sample.nodes.is_empty()));
        input.push((sample.nodes.len() as f64).ln_1p() / (MAX_NODES as f64).ln_1p());
        for slot in 0..self.config.fixed_slot_count {
            if let Some(node) = ordered.get(slot) {
                input.push(1.0);
                input.extend(self.layout.node_input(node));
            } else {
                input.push(0.0);
                input.resize(input.len() + self.layout.node_input_width, 0.0);
            }
        }
        input
    }

    fn forward(&self, sample: &TypedSetSample) -> (Vec<f64>, Vec<f64>, f64) {
        let input = self.input(sample);
        let hidden = dense_tanh(
            &input,
            &self.head_weights,
            &self.head_bias,
            self.config.head_hidden_width,
        );
        let prediction = dot(&hidden, &self.output_weights) + self.output_bias;
        (input, hidden, prediction)
    }

    fn train_one(&mut self, sample: &TypedSetSample) -> Result<(), TrainableSetError> {
        if sample.actor_feature_schema_sha256 != self.actor_feature_schema_sha256 {
            return Err(TrainableSetError::new(
                "fixed-slot sample feature schema does not match the model",
            ));
        }
        validate_sample_dimensions(sample, self.layout.dimensions())?;
        let (input, hidden, prediction) = self.forward(sample);
        let error = prediction - f64::from(sample.target);
        if !error.is_finite() {
            return Err(TrainableSetError::new(
                "fixed-slot prediction became non-finite",
            ));
        }
        let output_weights_before = self.output_weights.clone();
        let mut d_prediction = 2.0 * error;
        d_prediction = clip(d_prediction, self.config.gradient_clip);
        for (weight, value) in self.output_weights.iter_mut().zip(&hidden) {
            let gradient = d_prediction * value + self.config.l2_penalty * *weight;
            *weight -= self.config.learning_rate * clip(gradient, self.config.gradient_clip);
        }
        self.output_bias -= self.config.learning_rate * d_prediction;
        for output in 0..self.config.head_hidden_width {
            let delta =
                d_prediction * output_weights_before[output] * (1.0 - hidden[output].powi(2));
            for (input_index, value) in input.iter().enumerate() {
                let parameter = output * input.len() + input_index;
                let gradient =
                    delta * value + self.config.l2_penalty * self.head_weights[parameter];
                self.head_weights[parameter] -=
                    self.config.learning_rate * clip(gradient, self.config.gradient_clip);
            }
            self.head_bias[output] -=
                self.config.learning_rate * clip(delta, self.config.gradient_clip);
        }
        self.optimizer_steps += 1;
        if self
            .head_weights
            .iter()
            .chain(&self.head_bias)
            .chain(&self.output_weights)
            .chain(std::iter::once(&self.output_bias))
            .any(|value| !value.is_finite())
        {
            return Err(TrainableSetError::new(
                "fixed-slot parameters became non-finite",
            ));
        }
        Ok(())
    }

    fn metrics(
        &self,
        training: &[TypedSetSample],
        held_out: &[TypedSetSample],
    ) -> Result<SetRegressorMetrics, TrainableSetError> {
        Ok(SetRegressorMetrics {
            name: "fixed_runtime_id_slots",
            parameter_count: self.parameter_count(),
            training_rows: training.len(),
            held_out_rows: held_out.len(),
            optimizer_steps: self.optimizer_steps,
            training_mse: mse(training, |sample| self.predict(sample))?,
            held_out_mse: mse(held_out, |sample| self.predict(sample))?,
        })
    }
}

impl FeatureLayout {
    pub(crate) fn fit<'a>(
        training: impl IntoIterator<Item = &'a TypedSetSample>,
        dimensions: Dimensions,
    ) -> Result<Self, TrainableSetError> {
        let mut categorical_values = vec![BTreeSet::new(); dimensions.categorical];
        let mut continuous_values = vec![Vec::new(); dimensions.continuous];
        let mut base_values = vec![Vec::new(); dimensions.base];
        for sample in training {
            for node in &sample.nodes {
                for (index, values) in categorical_values.iter_mut().enumerate() {
                    if node.categorical_present[index] {
                        values.insert(node.categorical[index]);
                    }
                }
                for (index, values) in continuous_values.iter_mut().enumerate() {
                    if node.continuous_present[index] {
                        values.push(f64::from(node.continuous[index]));
                    }
                }
            }
            for (index, values) in base_values.iter_mut().enumerate() {
                if sample.base_present[index] {
                    values.push(f64::from(sample.base[index]));
                }
            }
        }
        let category_count = categorical_values.iter().map(BTreeSet::len).sum::<usize>();
        if category_count > MAX_CATEGORY_VALUES {
            return Err(TrainableSetError::new(
                "categorical vocabulary exceeds its bounded exact-value budget",
            ));
        }
        let categorical_values = categorical_values
            .into_iter()
            .map(BTreeSet::into_iter)
            .map(Iterator::collect)
            .collect::<Vec<Vec<i64>>>();
        let (continuous_mean, continuous_inverse_stddev) = normalization(&continuous_values);
        let (base_mean, base_inverse_stddev) = normalization(&base_values);
        // Each category has presence and OOV channels plus exact training-value
        // one-hot channels. Optional numeric/binary fields retain value + mask.
        let node_input_width = categorical_values
            .iter()
            .map(|values| values.len() + 2)
            .sum::<usize>()
            + dimensions.continuous * 2
            + dimensions.binary * 2;
        let base_input_width = dimensions.base * 2;
        Ok(Self {
            categorical_values,
            continuous_mean,
            continuous_inverse_stddev,
            base_mean,
            base_inverse_stddev,
            categorical_width: dimensions.categorical,
            continuous_width: dimensions.continuous,
            binary_width: dimensions.binary,
            base_width: dimensions.base,
            node_input_width,
            base_input_width,
        })
    }

    pub(crate) fn dimensions(&self) -> Dimensions {
        Dimensions {
            categorical: self.categorical_width,
            continuous: self.continuous_width,
            binary: self.binary_width,
            base: self.base_width,
        }
    }

    pub(crate) fn node_input(&self, node: &TypedSetNode) -> Vec<f64> {
        let mut output = Vec::with_capacity(self.node_input_width);
        for index in 0..self.categorical_width {
            let present = node.categorical_present[index];
            output.push(f64::from(present));
            let values = &self.categorical_values[index];
            let known = present
                .then(|| values.binary_search(&node.categorical[index]).ok())
                .flatten();
            output.push(f64::from(present && known.is_none()));
            for value_index in 0..values.len() {
                output.push(f64::from(known == Some(value_index)));
            }
        }
        for index in 0..self.continuous_width {
            let present = node.continuous_present[index];
            output.push(if present {
                (f64::from(node.continuous[index]) - self.continuous_mean[index])
                    * self.continuous_inverse_stddev[index]
            } else {
                0.0
            });
            output.push(f64::from(present));
        }
        for index in 0..self.binary_width {
            let present = node.binary_present[index];
            output.push(f64::from(present && node.binary[index]));
            output.push(f64::from(present));
        }
        output
    }

    pub(crate) fn base_input(&self, sample: &TypedSetSample) -> Vec<f64> {
        let mut output = Vec::with_capacity(self.base_input_width);
        for index in 0..self.base_width {
            let present = sample.base_present[index];
            output.push(if present {
                (f64::from(sample.base[index]) - self.base_mean[index])
                    * self.base_inverse_stddev[index]
            } else {
                0.0
            });
            output.push(f64::from(present));
        }
        output
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Dimensions {
    pub(crate) categorical: usize,
    pub(crate) continuous: usize,
    pub(crate) binary: usize,
    pub(crate) base: usize,
}

struct SetForward {
    node_inputs: Vec<Vec<f64>>,
    node_hidden: Vec<Vec<f64>>,
    mean_pool: Vec<f64>,
    max_pool: Vec<f64>,
    max_indices: Vec<Option<usize>>,
    head_input: Vec<f64>,
    head_hidden: Vec<f64>,
    prediction: f64,
}

fn validate_samples(
    actor_feature_schema_sha256: Digest,
    training_dataset_sha256: Digest,
    held_out_dataset_sha256: Digest,
    training: &[TypedSetSample],
    held_out: &[TypedSetSample],
    config: TrainableSetConfig,
) -> Result<Dimensions, TrainableSetError> {
    if actor_feature_schema_sha256 == Digest::ZERO
        || training_dataset_sha256 == Digest::ZERO
        || held_out_dataset_sha256 == Digest::ZERO
        || training_dataset_sha256 == held_out_dataset_sha256
        || training.is_empty()
        || held_out.is_empty()
        || training.len() > MAX_SAMPLES
        || held_out.len() > MAX_SAMPLES
        || config.epochs == 0
        || config.epochs > MAX_EPOCHS
        || config.node_hidden_width == 0
        || config.node_hidden_width > MAX_HIDDEN_WIDTH
        || config.head_hidden_width == 0
        || config.head_hidden_width > MAX_HIDDEN_WIDTH
        || config.fixed_slot_count == 0
        || config.fixed_slot_count > MAX_FIXED_SLOTS
        || !config.learning_rate.is_finite()
        || config.learning_rate <= 0.0
        || !config.l2_penalty.is_finite()
        || config.l2_penalty < 0.0
        || !config.gradient_clip.is_finite()
        || config.gradient_clip <= 0.0
        || !config.minimum_relative_improvement.is_finite()
        || !(0.0..=1.0).contains(&config.minimum_relative_improvement)
    {
        return Err(TrainableSetError::new(
            "trainable-set configuration or dataset identity is invalid",
        ));
    }
    let first_node = training
        .iter()
        .chain(held_out)
        .find_map(|sample| sample.nodes.first())
        .ok_or_else(|| TrainableSetError::new("set corpus contains no nodes"))?;
    let dimensions = Dimensions {
        categorical: first_node.categorical.len(),
        continuous: first_node.continuous.len(),
        binary: first_node.binary.len(),
        base: training[0].base.len(),
    };
    if dimensions.categorical + dimensions.continuous + dimensions.binary == 0
        || dimensions.categorical > MAX_FEATURE_COLUMNS
        || dimensions.continuous > MAX_FEATURE_COLUMNS
        || dimensions.binary > MAX_FEATURE_COLUMNS
        || dimensions.base > MAX_FEATURE_COLUMNS
    {
        return Err(TrainableSetError::new(
            "trainable-set feature dimensions are invalid",
        ));
    }
    let mut identities = BTreeSet::new();
    for sample in training.iter().chain(held_out) {
        if sample.sample_sha256 == Digest::ZERO || !identities.insert(sample.sample_sha256) {
            return Err(TrainableSetError::new(
                "trainable-set sample identities are missing or cross-split duplicated",
            ));
        }
        if sample.actor_feature_schema_sha256 != actor_feature_schema_sha256 {
            return Err(TrainableSetError::new(
                "trainable-set sample feature schema does not match the comparison",
            ));
        }
        validate_sample_dimensions(sample, dimensions)?;
    }
    Ok(dimensions)
}

pub(crate) fn validate_sample_dimensions(
    sample: &TypedSetSample,
    dimensions: Dimensions,
) -> Result<(), TrainableSetError> {
    let ids = sample
        .nodes
        .iter()
        .map(|node| node.stable_id)
        .collect::<BTreeSet<_>>();
    if sample.base.len() != dimensions.base
        || sample.base_present.len() != dimensions.base
        || sample
            .base
            .iter()
            .zip(&sample.base_present)
            .any(|(value, present)| !present && *value != 0.0)
        || sample.nodes.len() > MAX_NODES
        || ids.len() != sample.nodes.len()
        || !sample.target.is_finite()
        || sample.base.iter().any(|value| !value.is_finite())
        || sample.nodes.iter().any(|node| {
            node.categorical.len() != dimensions.categorical
                || node.categorical_present.len() != dimensions.categorical
                || node.continuous.len() != dimensions.continuous
                || node.continuous_present.len() != dimensions.continuous
                || node.binary.len() != dimensions.binary
                || node.binary_present.len() != dimensions.binary
                || node.continuous.iter().any(|value| !value.is_finite())
                || node
                    .categorical
                    .iter()
                    .zip(&node.categorical_present)
                    .any(|(value, present)| !present && *value != 0)
                || node
                    .continuous
                    .iter()
                    .zip(&node.continuous_present)
                    .any(|(value, present)| !present && *value != 0.0)
                || node
                    .binary
                    .iter()
                    .zip(&node.binary_present)
                    .any(|(value, present)| !present && *value)
        })
    {
        return Err(TrainableSetError::new(
            "trainable-set sample shape, identity, mask, or value is invalid",
        ));
    }
    Ok(())
}

pub(crate) fn ordered_nodes(nodes: &[TypedSetNode]) -> Vec<&TypedSetNode> {
    let mut ordered = nodes.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|node| node.stable_id);
    ordered
}

fn normalization(columns: &[Vec<f64>]) -> (Vec<f64>, Vec<f64>) {
    let mut means = Vec::with_capacity(columns.len());
    let mut inverse_stddevs = Vec::with_capacity(columns.len());
    for values in columns {
        if values.is_empty() {
            means.push(0.0);
            inverse_stddevs.push(1.0);
            continue;
        }
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        let variance = values
            .iter()
            .map(|value| (value - mean).powi(2))
            .sum::<f64>()
            / values.len() as f64;
        means.push(mean);
        inverse_stddevs.push(if variance > 1.0e-12 {
            1.0 / variance.sqrt()
        } else {
            1.0
        });
    }
    (means, inverse_stddevs)
}

pub(crate) fn initialized_weights(
    outputs: usize,
    inputs: usize,
    rng: &mut DeterministicRng,
) -> Vec<f64> {
    let scale = (6.0 / (inputs + outputs) as f64).sqrt();
    (0..outputs * inputs)
        .map(|_| (rng.unit() * 2.0 - 1.0) * scale)
        .collect()
}

pub(crate) fn dense_tanh(input: &[f64], weights: &[f64], bias: &[f64], outputs: usize) -> Vec<f64> {
    (0..outputs)
        .map(|output| {
            (dot(
                input,
                &weights[output * input.len()..(output + 1) * input.len()],
            ) + bias[output])
                .tanh()
        })
        .collect()
}

pub(crate) fn dot(left: &[f64], right: &[f64]) -> f64 {
    left.iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum()
}

pub(crate) fn clip(value: f64, limit: f64) -> f64 {
    value.clamp(-limit, limit)
}

fn mse(
    samples: &[TypedSetSample],
    predict: impl Fn(&TypedSetSample) -> Result<f32, TrainableSetError>,
) -> Result<f64, TrainableSetError> {
    let mut total = 0.0;
    for sample in samples {
        let error = f64::from(predict(sample)?) - f64::from(sample.target);
        total += error * error;
    }
    Ok(total / samples.len() as f64)
}

fn canonical_digest<T: Serialize>(domain: &[u8], value: &T) -> Result<Digest, TrainableSetError> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| TrainableSetError::new(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

pub(crate) struct DeterministicRng(u64);

impl DeterministicRng {
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0 = self.0.wrapping_mul(0x2545_f491_4f6c_dd1d);
        self.0
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / ((1_u64 << 53) as f64)
    }

    pub(crate) fn shuffle<T>(&mut self, values: &mut [T]) {
        for index in (1..values.len()).rev() {
            values.swap(index, self.next() as usize % (index + 1));
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrainableSetError(String);

impl TrainableSetError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for TrainableSetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for TrainableSetError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(identity: u16, signal: f32, reverse: bool) -> TypedSetSample {
        let mut nodes = (0..8)
            .map(|index| TypedSetNode {
                stable_id: index,
                categorical: vec![if index == 7 { 99 } else { 1 }],
                categorical_present: vec![true],
                continuous: vec![if index == 7 { signal } else { 0.0 }],
                continuous_present: vec![true],
                binary: vec![index == 7 && signal > 0.0],
                binary_present: vec![true],
            })
            .collect::<Vec<_>>();
        if reverse {
            nodes.reverse();
        }
        TypedSetSample {
            sample_sha256: Digest(
                (identity as u8..identity as u8 + 32)
                    .collect::<Vec<_>>()
                    .try_into()
                    .unwrap(),
            ),
            actor_feature_schema_sha256: Digest([1; 32]),
            base: vec![0.0],
            base_present: vec![true],
            nodes,
            target: signal,
        }
    }

    fn corpus(start: u16, count: usize) -> Vec<TypedSetSample> {
        (0..count)
            .map(|index| {
                let signal = ((index * 37 % 101) as f32 / 50.0) - 1.0;
                sample(start + index as u16, signal, index % 2 == 0)
            })
            .collect()
    }

    #[test]
    fn complete_set_learns_held_out_overflow_signal_under_equal_budgets() {
        let training = corpus(1, 96);
        let held_out = corpus(150, 32);
        let config = TrainableSetConfig {
            epochs: 240,
            node_hidden_width: 12,
            head_hidden_width: 12,
            fixed_slot_count: 4,
            learning_rate: 0.004,
            minimum_relative_improvement: 0.5,
            ..TrainableSetConfig::default()
        };
        let (report, complete, fixed) = TrainableSetComparison::fit(
            Digest([1; 32]),
            Digest([2; 32]),
            Digest([3; 32]),
            &training,
            &held_out,
            config,
        )
        .unwrap();
        assert!(report.equal_training_row_budget);
        assert!(report.equal_held_out_row_budget);
        assert!(report.equal_optimizer_step_budget);
        assert_eq!(report.maximum_training_nodes, 8);
        assert!(report.complete_set.held_out_mse < report.fixed_slots.held_out_mse * 0.25);
        assert_eq!(report.decision, SetEncoderDecision::CompleteSetCandidate);
        assert!(!report.promotion_authority);
        assert_ne!(report.comparison_sha256, Digest::ZERO);
        assert_eq!(
            report.complete_set_model_sha256,
            complete.model_sha256().unwrap()
        );
        assert_eq!(
            report.fixed_slot_model_sha256,
            fixed.model_sha256().unwrap()
        );
        assert_eq!(complete.actor_feature_schema_sha256, Digest([1; 32]));
        assert_eq!(fixed.actor_feature_schema_sha256, Digest([1; 32]));
    }

    #[test]
    fn learned_embedding_and_prediction_are_permutation_invariant() {
        let training = corpus(1, 32);
        let held_out = corpus(100, 8);
        let (_, model, _) = TrainableSetComparison::fit(
            Digest([1; 32]),
            Digest([2; 32]),
            Digest([3; 32]),
            &training,
            &held_out,
            TrainableSetConfig {
                epochs: 8,
                fixed_slot_count: 4,
                ..TrainableSetConfig::default()
            },
        )
        .unwrap();
        let first = sample(200, 0.75, false);
        let second = sample(200, 0.75, true);
        assert_eq!(
            model.encode(&first).unwrap(),
            model.encode(&second).unwrap()
        );
        assert_eq!(
            model.predict(&first).unwrap(),
            model.predict(&second).unwrap()
        );
    }

    #[test]
    fn masks_and_unknown_categories_are_explicit_model_inputs() {
        let training = corpus(1, 16);
        let held_out = corpus(100, 4);
        let (_, model, _) = TrainableSetComparison::fit(
            Digest([1; 32]),
            Digest([2; 32]),
            Digest([3; 32]),
            &training,
            &held_out,
            TrainableSetConfig {
                epochs: 2,
                fixed_slot_count: 4,
                ..TrainableSetConfig::default()
            },
        )
        .unwrap();
        let mut absent = sample(200, 0.0, false);
        absent.nodes[7].continuous_present[0] = false;
        absent.nodes[7].categorical_present[0] = false;
        absent.nodes[7].categorical[0] = 0;
        absent.nodes[7].binary_present[0] = false;
        let mut present_zero = absent.clone();
        present_zero.nodes[7].continuous_present[0] = true;
        present_zero.nodes[7].categorical_present[0] = true;
        present_zero.nodes[7].categorical[0] = 123_456; // held-out OOV
        present_zero.nodes[7].binary_present[0] = true;
        assert_ne!(
            model.encode(&absent).unwrap(),
            model.encode(&present_zero).unwrap()
        );
    }

    #[test]
    fn rejects_duplicate_identity_bad_masks_and_nonfinite_values() {
        let training = corpus(1, 8);
        let mut held_out = corpus(100, 4);
        held_out[0].sample_sha256 = training[0].sample_sha256;
        assert!(
            TrainableSetComparison::fit(
                Digest([1; 32]),
                Digest([2; 32]),
                Digest([3; 32]),
                &training,
                &held_out,
                TrainableSetConfig {
                    epochs: 1,
                    fixed_slot_count: 4,
                    ..TrainableSetConfig::default()
                },
            )
            .is_err()
        );

        let mut malformed = corpus(1, 8);
        malformed[0].nodes[0].continuous_present.clear();
        assert!(
            TrainableSetComparison::fit(
                Digest([1; 32]),
                Digest([2; 32]),
                Digest([3; 32]),
                &malformed,
                &corpus(100, 4),
                TrainableSetConfig {
                    epochs: 1,
                    fixed_slot_count: 4,
                    ..TrainableSetConfig::default()
                },
            )
            .is_err()
        );

        let mut nonfinite = corpus(1, 8);
        nonfinite[0].nodes[0].continuous[0] = f32::NAN;
        assert!(
            TrainableSetComparison::fit(
                Digest([1; 32]),
                Digest([2; 32]),
                Digest([3; 32]),
                &nonfinite,
                &corpus(100, 4),
                TrainableSetConfig {
                    epochs: 1,
                    fixed_slot_count: 4,
                    ..TrainableSetConfig::default()
                },
            )
            .is_err()
        );

        let mut wrong_schema_training = corpus(1, 8);
        wrong_schema_training[0].actor_feature_schema_sha256 = Digest([9; 32]);
        assert!(
            TrainableSetComparison::fit(
                Digest([1; 32]),
                Digest([2; 32]),
                Digest([3; 32]),
                &wrong_schema_training,
                &corpus(100, 4),
                TrainableSetConfig {
                    epochs: 1,
                    fixed_slot_count: 4,
                    ..TrainableSetConfig::default()
                },
            )
            .is_err()
        );
    }

    #[test]
    fn trainable_complete_set_accepts_more_than_controller_capacity() {
        let large = |identity: u8, reverse: bool| {
            let mut nodes = (0..257_u64)
                .map(|stable_id| TypedSetNode {
                    stable_id,
                    categorical: Vec::new(),
                    categorical_present: Vec::new(),
                    continuous: vec![stable_id as f32 / 257.0],
                    continuous_present: vec![true],
                    binary: Vec::new(),
                    binary_present: Vec::new(),
                })
                .collect::<Vec<_>>();
            if reverse {
                nodes.reverse();
            }
            TypedSetSample {
                sample_sha256: Digest([identity; 32]),
                actor_feature_schema_sha256: Digest([1; 32]),
                base: Vec::new(),
                base_present: Vec::new(),
                nodes,
                target: 0.5,
            }
        };
        let training = vec![large(4, false), large(5, true)];
        let held_out = vec![large(6, true)];
        let (report, model, _) = TrainableSetComparison::fit(
            Digest([1; 32]),
            Digest([2; 32]),
            Digest([3; 32]),
            &training,
            &held_out,
            TrainableSetConfig {
                epochs: 1,
                node_hidden_width: 4,
                head_hidden_width: 4,
                fixed_slot_count: 4,
                ..TrainableSetConfig::default()
            },
        )
        .unwrap();
        assert_eq!(report.maximum_training_nodes, 257);
        assert_eq!(
            model.encode(&large(7, false)).unwrap(),
            model.encode(&large(7, true)).unwrap()
        );
    }

    #[test]
    fn seeded_refit_is_byte_deterministic() {
        let training = corpus(1, 24);
        let held_out = corpus(100, 8);
        let fit = || {
            TrainableSetComparison::fit(
                Digest([1; 32]),
                Digest([2; 32]),
                Digest([3; 32]),
                &training,
                &held_out,
                TrainableSetConfig {
                    epochs: 4,
                    fixed_slot_count: 4,
                    ..TrainableSetConfig::default()
                },
            )
            .unwrap()
        };
        let first = fit();
        let second = fit();
        assert_eq!(
            serde_json::to_vec(&first.0).unwrap(),
            serde_json::to_vec(&second.0).unwrap()
        );
        assert_eq!(
            serde_json::to_vec(&first.1).unwrap(),
            serde_json::to_vec(&second.1).unwrap()
        );
        assert_eq!(
            serde_json::to_vec(&first.2).unwrap(),
            serde_json::to_vec(&second.2).unwrap()
        );
    }
}
