//! Trainable shared complete-set encoder with masked auxiliary heads.
//!
//! One actor transform and state latent are updated by every supported target.
//! Missing targets are masked, target normalization is fitted on training rows
//! only, and held-out results are compared with training-mean predictors.

use crate::artifact::Digest;
use crate::native_actor_features::NativeActorFeatureView;
use crate::native_auxiliary_dataset::{
    AuxiliarySplit, NativeAuxiliaryDataset, NativeAuxiliaryExample,
};
use crate::trainable_set_encoder::{
    DeterministicRng, Dimensions, FeatureLayout, TrainableSetConfig, TrainableSetError,
    TypedSetNode, TypedSetSample, clip, dense_tanh, dot, initialized_weights, ordered_nodes,
    validate_sample_dimensions,
};
use dusklight_evidence::native_episode_shard::{
    NativeActorObservation, NativeChannelStatus, NativeEpisodeShard, NativeLearningObservation,
};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const MULTITASK_SET_ENCODER_REPORT_SCHEMA_V1: &str =
    "dusklight-multitask-set-encoder-report/v1";
pub const SHUFFLED_AUXILIARY_CONTROL_SCHEMA_V1: &str = "dusklight-shuffled-auxiliary-control/v1";
const MAX_TARGETS: usize = 64;
const MAX_SAMPLES: usize = 100_000;
const MAX_HIDDEN_WIDTH: usize = 256;
const MAX_EPOCHS: usize = 2_048;
const MAX_PARAMETERS: usize = 16_000_000;
type TargetNormalization = (Vec<f64>, Vec<f64>, Vec<usize>);

#[derive(Clone, Debug)]
pub struct MultiTaskSetSample {
    pub input: TypedSetSample,
    pub targets: Vec<f32>,
    pub target_present: Vec<bool>,
}

#[derive(Clone, Debug)]
pub struct NativeMultiTaskActorCorpus {
    pub actor_feature_schema_sha256: Digest,
    pub target_names: Vec<String>,
    pub training_dataset_sha256: Digest,
    pub validation_dataset_sha256: Digest,
    pub test_dataset_sha256: Digest,
    pub training: Vec<MultiTaskSetSample>,
    pub validation: Vec<MultiTaskSetSample>,
    pub test: Vec<MultiTaskSetSample>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ShuffledAuxiliaryControl {
    pub schema: &'static str,
    pub shuffled_training_dataset_sha256: Digest,
    pub report: MultiTaskSetEncoderReport,
    pub test_evaluation: MultiTaskSetEvaluation,
}

impl NativeMultiTaskActorCorpus {
    pub fn build(
        dataset: &NativeAuxiliaryDataset,
        shard: &NativeEpisodeShard,
    ) -> Result<Self, TrainableSetError> {
        dataset
            .validate()
            .map_err(|error| TrainableSetError::new(error.to_string()))?;
        if dataset.observation_schema != shard.metadata.observation_schema
            || dataset.action_schema != shard.metadata.action_schema
            || dataset
                .examples
                .iter()
                .any(|example| example.shard_sha256 != shard.content_sha256)
        {
            return Err(TrainableSetError::new(
                "native multitask sources are detached or span unsupported shards",
            ));
        }
        let actor_feature_schema_sha256 = native_actor_feature_schema()?;
        let episodes = shard
            .episodes
            .iter()
            .map(|episode| (episode.id.as_str(), episode))
            .collect::<BTreeMap<_, _>>();
        let target_names = native_target_names();
        let mut training = Vec::new();
        let mut validation = Vec::new();
        let mut test = Vec::new();
        for example in &dataset.examples {
            let episode = episodes.get(example.episode_id.as_str()).ok_or_else(|| {
                TrainableSetError::new("native multitask episode is absent from shard")
            })?;
            let step = episode
                .steps
                .get(example.step_index as usize)
                .ok_or_else(|| {
                    TrainableSetError::new("native multitask step is absent from episode")
                })?;
            if hex_128(step.pre_input.state_identity) != example.pre_input_state_xxh3_128 {
                return Err(TrainableSetError::new(
                    "native multitask pre-input state identity is detached",
                ));
            }
            if step.pre_input.actors_truncated {
                return Err(TrainableSetError::new(
                    "native multitask actor observations must be complete",
                ));
            }
            let (base, base_present) = broad_base(&step.pre_input);
            let (targets, target_present) = native_targets(example);
            let sample = MultiTaskSetSample {
                input: TypedSetSample {
                    sample_sha256: example.example_sha256,
                    actor_feature_schema_sha256,
                    base,
                    base_present,
                    nodes: native_actor_nodes(&step.pre_input),
                    target: 0.0,
                },
                targets,
                target_present,
            };
            match example.split {
                AuxiliarySplit::Training => training.push(sample),
                AuxiliarySplit::Validation => validation.push(sample),
                AuxiliarySplit::Test => test.push(sample),
            }
        }
        if training.is_empty() || validation.is_empty() || test.is_empty() {
            return Err(TrainableSetError::new(
                "native multitask corpus requires all three episode splits",
            ));
        }
        Ok(Self {
            actor_feature_schema_sha256,
            target_names,
            training_dataset_sha256: sample_manifest_digest(&training)?,
            validation_dataset_sha256: sample_manifest_digest(&validation)?,
            test_dataset_sha256: sample_manifest_digest(&test)?,
            training,
            validation,
            test,
        })
    }
}

impl MultiTaskSetSample {
    #[allow(clippy::too_many_arguments)]
    pub fn from_native_actor_observation(
        view: &NativeActorFeatureView,
        observation_index: usize,
        sample_sha256: Digest,
        base: Vec<f32>,
        base_present: Vec<bool>,
        targets: Vec<f32>,
        target_present: Vec<bool>,
    ) -> Result<Self, TrainableSetError> {
        Ok(Self {
            input: TypedSetSample::from_native_actor_observation(
                view,
                observation_index,
                sample_sha256,
                base,
                base_present,
                0.0,
            )?,
            targets,
            target_present,
        })
    }
}

pub fn fit_shuffled_auxiliary_control(
    actor_feature_schema_sha256: Digest,
    target_names: Vec<String>,
    mut training: Vec<MultiTaskSetSample>,
    validation_dataset_sha256: Digest,
    validation: &[MultiTaskSetSample],
    test: &[MultiTaskSetSample],
    config: TrainableSetConfig,
) -> Result<ShuffledAuxiliaryControl, TrainableSetError> {
    if training.is_empty() || target_names.is_empty() {
        return Err(TrainableSetError::new(
            "shuffled auxiliary control requires training rows and targets",
        ));
    }
    let mut rng = DeterministicRng::new(config.seed ^ 0x5a11_f1ed_c017_0001);
    for target in 0..target_names.len() {
        let rows = training
            .iter()
            .enumerate()
            .filter_map(|(row, sample)| sample.target_present[target].then_some(row))
            .collect::<Vec<_>>();
        let mut shuffled_rows = rows.clone();
        rng.shuffle(&mut shuffled_rows);
        let values = shuffled_rows
            .iter()
            .map(|row| training[*row].targets[target])
            .collect::<Vec<_>>();
        for (row, value) in rows.into_iter().zip(values) {
            training[row].targets[target] = value;
        }
    }
    let shuffled_training_dataset_sha256 = sample_manifest_digest(&training)?;
    let (report, model) = CompleteSetMultiTaskEncoder::fit(
        actor_feature_schema_sha256,
        shuffled_training_dataset_sha256,
        validation_dataset_sha256,
        target_names,
        &training,
        validation,
        config,
    )?;
    let test_evaluation = model.evaluate(test)?;
    Ok(ShuffledAuxiliaryControl {
        schema: SHUFFLED_AUXILIARY_CONTROL_SCHEMA_V1,
        shuffled_training_dataset_sha256,
        report,
        test_evaluation,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MultiTaskEncoderDecision {
    RetainTrainingMeanBaseline,
    SharedEncoderCandidate,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AuxiliaryHeadMetrics {
    pub name: String,
    pub training_support: usize,
    pub held_out_support: usize,
    pub training_mse: f64,
    pub held_out_mse: f64,
    pub held_out_training_mean_mse: f64,
    pub relative_held_out_improvement: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AuxiliaryHeadEvaluation {
    pub name: String,
    pub support: usize,
    pub mse: f64,
    pub training_mean_mse: f64,
    pub relative_improvement: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MultiTaskSetEvaluation {
    pub samples: usize,
    pub normalized_mse: f64,
    pub training_mean_normalized_mse: f64,
    pub relative_improvement: f64,
    pub heads: Vec<AuxiliaryHeadEvaluation>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MultiTaskSetEncoderReport {
    pub schema: &'static str,
    pub actor_feature_schema_sha256: Digest,
    pub training_dataset_sha256: Digest,
    pub held_out_dataset_sha256: Digest,
    pub config: TrainableSetConfig,
    pub target_names: Vec<String>,
    pub target_support_training: Vec<usize>,
    pub target_support_held_out: Vec<usize>,
    pub maximum_training_nodes: usize,
    pub maximum_held_out_nodes: usize,
    pub parameter_count: usize,
    pub optimizer_steps: u64,
    pub training_normalized_mse: f64,
    pub held_out_normalized_mse: f64,
    pub held_out_training_mean_normalized_mse: f64,
    pub relative_held_out_improvement: f64,
    pub heads: Vec<AuxiliaryHeadMetrics>,
    pub decision: MultiTaskEncoderDecision,
    pub model_sha256: Digest,
    pub promotion_authority: bool,
    pub report_sha256: Digest,
}

#[derive(Clone, Debug, Serialize)]
pub struct CompleteSetMultiTaskEncoder {
    actor_feature_schema_sha256: Digest,
    layout: FeatureLayout,
    config: TrainableSetConfig,
    target_names: Vec<String>,
    target_mean: Vec<f64>,
    target_inverse_stddev: Vec<f64>,
    node_weights: Vec<f64>,
    node_bias: Vec<f64>,
    state_weights: Vec<f64>,
    state_bias: Vec<f64>,
    output_weights: Vec<f64>,
    output_bias: Vec<f64>,
    optimizer_steps: u64,
}

struct MultiTaskForward {
    node_inputs: Vec<Vec<f64>>,
    node_hidden: Vec<Vec<f64>>,
    max_indices: Vec<Option<usize>>,
    state_input: Vec<f64>,
    state_hidden: Vec<f64>,
    predictions: Vec<f64>,
}

impl CompleteSetMultiTaskEncoder {
    #[allow(clippy::too_many_arguments)]
    pub fn fit(
        actor_feature_schema_sha256: Digest,
        training_dataset_sha256: Digest,
        held_out_dataset_sha256: Digest,
        target_names: Vec<String>,
        training: &[MultiTaskSetSample],
        held_out: &[MultiTaskSetSample],
        config: TrainableSetConfig,
    ) -> Result<(MultiTaskSetEncoderReport, Self), TrainableSetError> {
        let dimensions = validate_samples(
            actor_feature_schema_sha256,
            training_dataset_sha256,
            held_out_dataset_sha256,
            &target_names,
            training,
            held_out,
            config,
        )?;
        let layout = FeatureLayout::fit(training.iter().map(|sample| &sample.input), dimensions)?;
        let (target_mean, target_inverse_stddev, target_support_training) =
            target_normalization(training, target_names.len())?;
        let target_support_held_out = target_support(held_out, target_names.len());
        if target_support_held_out.contains(&0) {
            return Err(TrainableSetError::new(
                "each auxiliary target requires held-out support",
            ));
        }
        let mut model = Self::initialized(
            actor_feature_schema_sha256,
            layout,
            config,
            target_names.clone(),
            target_mean,
            target_inverse_stddev,
        )?;
        let mut order = (0..training.len()).collect::<Vec<_>>();
        let mut rng = DeterministicRng::new(config.seed ^ 0x4d55_4c54_4954_4153);
        for _ in 0..config.epochs {
            rng.shuffle(&mut order);
            for &index in &order {
                model.train_one(&training[index])?;
            }
        }
        let model_sha256 = model.model_sha256()?;
        let training_normalized_mse = model.normalized_mse(training)?;
        let held_out_normalized_mse = model.normalized_mse(held_out)?;
        let held_out_training_mean_normalized_mse = model.training_mean_normalized_mse(held_out)?;
        let relative_held_out_improvement = relative_improvement(
            held_out_training_mean_normalized_mse,
            held_out_normalized_mse,
        );
        let heads = model.head_metrics(training, held_out)?;
        let decision = if relative_held_out_improvement >= config.minimum_relative_improvement {
            MultiTaskEncoderDecision::SharedEncoderCandidate
        } else {
            MultiTaskEncoderDecision::RetainTrainingMeanBaseline
        };
        let mut report = MultiTaskSetEncoderReport {
            schema: MULTITASK_SET_ENCODER_REPORT_SCHEMA_V1,
            actor_feature_schema_sha256,
            training_dataset_sha256,
            held_out_dataset_sha256,
            config,
            target_names,
            target_support_training,
            target_support_held_out,
            maximum_training_nodes: training
                .iter()
                .map(|sample| sample.input.nodes.len())
                .max()
                .unwrap_or(0),
            maximum_held_out_nodes: held_out
                .iter()
                .map(|sample| sample.input.nodes.len())
                .max()
                .unwrap_or(0),
            parameter_count: model.parameter_count(),
            optimizer_steps: model.optimizer_steps,
            training_normalized_mse,
            held_out_normalized_mse,
            held_out_training_mean_normalized_mse,
            relative_held_out_improvement,
            heads,
            decision,
            model_sha256,
            promotion_authority: false,
            report_sha256: Digest::ZERO,
        };
        report.report_sha256 = report_digest(&report)?;
        Ok((report, model))
    }

    fn initialized(
        actor_feature_schema_sha256: Digest,
        layout: FeatureLayout,
        config: TrainableSetConfig,
        target_names: Vec<String>,
        target_mean: Vec<f64>,
        target_inverse_stddev: Vec<f64>,
    ) -> Result<Self, TrainableSetError> {
        let state_input_width = layout.base_input_width + 2 + config.node_hidden_width * 2;
        let target_width = target_names.len();
        let parameter_count = config
            .node_hidden_width
            .checked_mul(layout.node_input_width + 1)
            .and_then(|value| value.checked_add(config.head_hidden_width * (state_input_width + 1)))
            .and_then(|value| value.checked_add(target_width * (config.head_hidden_width + 1)))
            .ok_or_else(|| TrainableSetError::new("multitask parameter count overflowed"))?;
        if parameter_count > MAX_PARAMETERS {
            return Err(TrainableSetError::new(
                "multitask set encoder exceeds its parameter budget",
            ));
        }
        let mut rng = DeterministicRng::new(config.seed ^ 0x5348_4152_4544_0001);
        Ok(Self {
            actor_feature_schema_sha256,
            node_weights: initialized_weights(
                config.node_hidden_width,
                layout.node_input_width,
                &mut rng,
            ),
            node_bias: vec![0.0; config.node_hidden_width],
            state_weights: initialized_weights(
                config.head_hidden_width,
                state_input_width,
                &mut rng,
            ),
            state_bias: vec![0.0; config.head_hidden_width],
            output_weights: initialized_weights(target_width, config.head_hidden_width, &mut rng),
            output_bias: vec![0.0; target_width],
            layout,
            config,
            target_names,
            target_mean,
            target_inverse_stddev,
            optimizer_steps: 0,
        })
    }

    pub fn encode(&self, sample: &TypedSetSample) -> Result<Vec<f32>, TrainableSetError> {
        self.validate_input(sample)?;
        Ok(self
            .forward(sample)
            .state_hidden
            .into_iter()
            .map(|value| value as f32)
            .collect())
    }

    pub fn predict(&self, sample: &TypedSetSample) -> Result<Vec<f32>, TrainableSetError> {
        self.validate_input(sample)?;
        Ok(self
            .forward(sample)
            .predictions
            .iter()
            .enumerate()
            .map(|(target, prediction)| {
                (prediction / self.target_inverse_stddev[target] + self.target_mean[target]) as f32
            })
            .collect())
    }

    pub fn model_sha256(&self) -> Result<Digest, TrainableSetError> {
        canonical_digest(b"dusklight.complete-set-multitask-encoder/v1\0", self)
    }

    pub fn parameter_count(&self) -> usize {
        self.node_weights.len()
            + self.node_bias.len()
            + self.state_weights.len()
            + self.state_bias.len()
            + self.output_weights.len()
            + self.output_bias.len()
    }

    pub fn evaluate(
        &self,
        samples: &[MultiTaskSetSample],
    ) -> Result<MultiTaskSetEvaluation, TrainableSetError> {
        if samples.is_empty() {
            return Err(TrainableSetError::new(
                "multitask evaluation requires samples",
            ));
        }
        let normalized_mse = self.normalized_mse(samples)?;
        let training_mean_normalized_mse = self.training_mean_normalized_mse(samples)?;
        let mut squared_error = vec![0.0; self.target_names.len()];
        let mut baseline_error = vec![0.0; self.target_names.len()];
        let mut support = vec![0_usize; self.target_names.len()];
        for sample in samples {
            let predictions = self.predict(&sample.input)?;
            for target in 0..self.target_names.len() {
                if sample.target_present[target] {
                    let prediction = f64::from(predictions[target]);
                    let expected = f64::from(sample.targets[target]);
                    squared_error[target] += (prediction - expected).powi(2);
                    baseline_error[target] += (self.target_mean[target] - expected).powi(2);
                    support[target] += 1;
                }
            }
        }
        let heads = (0..self.target_names.len())
            .map(|target| {
                if support[target] == 0 {
                    return Err(TrainableSetError::new(
                        "multitask evaluation target has no support",
                    ));
                }
                let mse = squared_error[target] / support[target] as f64;
                let training_mean_mse = baseline_error[target] / support[target] as f64;
                Ok(AuxiliaryHeadEvaluation {
                    name: self.target_names[target].clone(),
                    support: support[target],
                    mse,
                    training_mean_mse,
                    relative_improvement: relative_improvement(training_mean_mse, mse),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(MultiTaskSetEvaluation {
            samples: samples.len(),
            normalized_mse,
            training_mean_normalized_mse,
            relative_improvement: relative_improvement(
                training_mean_normalized_mse,
                normalized_mse,
            ),
            heads,
        })
    }

    fn validate_input(&self, sample: &TypedSetSample) -> Result<(), TrainableSetError> {
        if sample.actor_feature_schema_sha256 != self.actor_feature_schema_sha256 {
            return Err(TrainableSetError::new(
                "multitask sample actor schema does not match model",
            ));
        }
        validate_sample_dimensions(sample, self.layout.dimensions())
    }

    fn forward(&self, sample: &TypedSetSample) -> MultiTaskForward {
        let nodes = ordered_nodes(&sample.nodes);
        let node_inputs = nodes
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
                for feature in 0..hidden.len() {
                    mean_pool[feature] += hidden[feature];
                    if hidden[feature] > max_pool[feature] {
                        max_pool[feature] = hidden[feature];
                        max_indices[feature] = Some(node_index);
                    }
                }
            }
            for value in &mut mean_pool {
                *value /= node_hidden.len() as f64;
            }
        }
        let mut state_input = self.layout.base_input(sample);
        state_input.push(f64::from(!sample.nodes.is_empty()));
        state_input.push((sample.nodes.len() as f64).ln_1p() / (u16::MAX as f64).ln_1p());
        state_input.extend(mean_pool);
        state_input.extend(max_pool);
        let state_hidden = dense_tanh(
            &state_input,
            &self.state_weights,
            &self.state_bias,
            self.config.head_hidden_width,
        );
        let predictions = (0..self.target_names.len())
            .map(|target| {
                dot(
                    &state_hidden,
                    &self.output_weights[target * self.config.head_hidden_width
                        ..(target + 1) * self.config.head_hidden_width],
                ) + self.output_bias[target]
            })
            .collect();
        MultiTaskForward {
            node_inputs,
            node_hidden,
            max_indices,
            state_input,
            state_hidden,
            predictions,
        }
    }

    fn train_one(&mut self, sample: &MultiTaskSetSample) -> Result<(), TrainableSetError> {
        self.validate_input(&sample.input)?;
        let forward = self.forward(&sample.input);
        let output_before = self.output_weights.clone();
        let state_before = self.state_weights.clone();
        let present_count = sample
            .target_present
            .iter()
            .filter(|present| **present)
            .count();
        let mut d_outputs = vec![0.0; self.target_names.len()];
        for (target, d_output) in d_outputs.iter_mut().enumerate() {
            if !sample.target_present[target] {
                continue;
            }
            let expected = (f64::from(sample.targets[target]) - self.target_mean[target])
                * self.target_inverse_stddev[target];
            *d_output = clip(
                2.0 * (forward.predictions[target] - expected) / present_count as f64,
                self.config.gradient_clip,
            );
            for hidden in 0..self.config.head_hidden_width {
                let parameter = target * self.config.head_hidden_width + hidden;
                let gradient = *d_output * forward.state_hidden[hidden]
                    + self.config.l2_penalty * self.output_weights[parameter];
                self.output_weights[parameter] -=
                    self.config.learning_rate * clip(gradient, self.config.gradient_clip);
            }
            self.output_bias[target] -= self.config.learning_rate * *d_output;
        }
        let mut d_state_pre = vec![0.0; self.config.head_hidden_width];
        for hidden in 0..self.config.head_hidden_width {
            let gradient = (0..self.target_names.len())
                .map(|target| {
                    d_outputs[target]
                        * output_before[target * self.config.head_hidden_width + hidden]
                })
                .sum::<f64>();
            d_state_pre[hidden] = gradient * (1.0 - forward.state_hidden[hidden].powi(2));
        }
        let mut d_state_input = vec![0.0; forward.state_input.len()];
        for (output, delta) in d_state_pre.iter().copied().enumerate() {
            for (input, d_input) in d_state_input.iter_mut().enumerate() {
                let parameter = output * forward.state_input.len() + input;
                *d_input += state_before[parameter] * delta;
                let gradient = delta * forward.state_input[input]
                    + self.config.l2_penalty * self.state_weights[parameter];
                self.state_weights[parameter] -=
                    self.config.learning_rate * clip(gradient, self.config.gradient_clip);
            }
            self.state_bias[output] -=
                self.config.learning_rate * clip(delta, self.config.gradient_clip);
        }
        let pool_offset = self.layout.base_input_width + 2;
        let d_mean = &d_state_input[pool_offset..pool_offset + self.config.node_hidden_width];
        let d_max = &d_state_input[pool_offset + self.config.node_hidden_width..];
        let node_count = forward.node_hidden.len();
        let mut node_weight_gradient = vec![0.0; self.node_weights.len()];
        let mut node_bias_gradient = vec![0.0; self.node_bias.len()];
        for node_index in 0..node_count {
            for hidden in 0..self.config.node_hidden_width {
                let mut gradient = d_mean[hidden] / node_count as f64;
                if forward.max_indices[hidden] == Some(node_index) {
                    gradient += d_max[hidden];
                }
                let delta = gradient * (1.0 - forward.node_hidden[node_index][hidden].powi(2));
                for input in 0..self.layout.node_input_width {
                    node_weight_gradient[hidden * self.layout.node_input_width + input] +=
                        delta * forward.node_inputs[node_index][input];
                }
                node_bias_gradient[hidden] += delta;
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
        if self
            .node_weights
            .iter()
            .chain(&self.node_bias)
            .chain(&self.state_weights)
            .chain(&self.state_bias)
            .chain(&self.output_weights)
            .chain(&self.output_bias)
            .any(|value| !value.is_finite())
        {
            return Err(TrainableSetError::new(
                "multitask set encoder parameters became non-finite",
            ));
        }
        Ok(())
    }

    fn normalized_mse(&self, samples: &[MultiTaskSetSample]) -> Result<f64, TrainableSetError> {
        let mut squared_error = 0.0;
        let mut count = 0_usize;
        for sample in samples {
            self.validate_input(&sample.input)?;
            let prediction = self.forward(&sample.input).predictions;
            for (target, predicted) in prediction.iter().enumerate() {
                if sample.target_present[target] {
                    let expected = (f64::from(sample.targets[target]) - self.target_mean[target])
                        * self.target_inverse_stddev[target];
                    squared_error += (*predicted - expected).powi(2);
                    count += 1;
                }
            }
        }
        Ok(squared_error / count as f64)
    }

    fn training_mean_normalized_mse(
        &self,
        samples: &[MultiTaskSetSample],
    ) -> Result<f64, TrainableSetError> {
        let mut squared_error = 0.0;
        let mut count = 0_usize;
        for sample in samples {
            for target in 0..self.target_names.len() {
                if sample.target_present[target] {
                    let expected = (f64::from(sample.targets[target]) - self.target_mean[target])
                        * self.target_inverse_stddev[target];
                    squared_error += expected.powi(2);
                    count += 1;
                }
            }
        }
        if count == 0 {
            return Err(TrainableSetError::new(
                "multitask baseline has no supported targets",
            ));
        }
        Ok(squared_error / count as f64)
    }

    fn head_metrics(
        &self,
        training: &[MultiTaskSetSample],
        held_out: &[MultiTaskSetSample],
    ) -> Result<Vec<AuxiliaryHeadMetrics>, TrainableSetError> {
        let collect = |samples: &[MultiTaskSetSample]| {
            let mut squared_error = vec![0.0; self.target_names.len()];
            let mut baseline_error = vec![0.0; self.target_names.len()];
            let mut support = vec![0_usize; self.target_names.len()];
            for sample in samples {
                let predictions = self.predict(&sample.input)?;
                for target in 0..self.target_names.len() {
                    if sample.target_present[target] {
                        let expected = f64::from(sample.targets[target]);
                        squared_error[target] +=
                            (f64::from(predictions[target]) - expected).powi(2);
                        baseline_error[target] += (self.target_mean[target] - expected).powi(2);
                        support[target] += 1;
                    }
                }
            }
            Ok::<_, TrainableSetError>((support, squared_error, baseline_error))
        };
        let (training_support, training_error, _) = collect(training)?;
        let (held_out_support, held_out_error, held_out_baseline_error) = collect(held_out)?;
        Ok((0..self.target_names.len())
            .map(|target| {
                let training_mse = training_error[target] / training_support[target] as f64;
                let held_out_mse = held_out_error[target] / held_out_support[target] as f64;
                let held_out_training_mean_mse =
                    held_out_baseline_error[target] / held_out_support[target] as f64;
                AuxiliaryHeadMetrics {
                    name: self.target_names[target].clone(),
                    training_support: training_support[target],
                    held_out_support: held_out_support[target],
                    training_mse,
                    held_out_mse,
                    held_out_training_mean_mse,
                    relative_held_out_improvement: relative_improvement(
                        held_out_training_mean_mse,
                        held_out_mse,
                    ),
                }
            })
            .collect())
    }
}

fn native_actor_feature_schema() -> Result<Digest, TrainableSetError> {
    canonical_digest(
        b"dusklight.native-direct-actor-features/v1\0",
        &(
            native_actor_categorical_names(),
            native_actor_continuous_names(),
            native_actor_binary_names(),
        ),
    )
}

fn native_actor_categorical_names() -> Vec<String> {
    [
        "parameters",
        "status",
        "actor_name",
        "profile_name",
        "set_id",
        "home_room",
        "current_room",
        "group",
        "argument",
        "health",
        "actor_type",
        "process_subtype",
        "condition",
        "old_room",
        "pause_flag",
        "process_init_state",
        "process_create_phase",
        "cull_type",
        "demo_actor_id",
        "carry_type",
        "attention_flags",
        "attention_distance_0",
        "attention_distance_1",
        "attention_distance_2",
        "attention_distance_3",
        "attention_distance_4",
        "attention_distance_5",
        "attention_distance_6",
        "attention_distance_7",
        "attention_distance_8",
        "attention_auxiliary",
        "event_command",
        "event_condition",
        "event_id",
        "event_map_tool_id",
        "event_index",
        "return_save_room",
        "return_save_point",
        "return_switch_room",
        "return_required_event_set",
        "return_required_event_unset",
        "return_required_switch_set",
        "return_required_switch_unset",
        "enemy_flags",
        "enemy_throw_mode",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn native_actor_continuous_names() -> Vec<String> {
    let mut names = Vec::new();
    for prefix in [
        "absolute_position",
        "absolute_home_position",
        "absolute_old_position",
        "absolute_velocity",
    ] {
        extend_vec3_feature_names(&mut names, prefix);
    }
    names.push("forward_speed".into());
    extend_vec3_feature_names(&mut names, "scale");
    names.extend(["gravity".into(), "max_fall_speed".into()]);
    extend_vec3_feature_names(&mut names, "absolute_eye_position");
    for prefix in [
        "home_angle_s16",
        "old_angle_s16",
        "current_angle_s16",
        "shape_angle_s16",
        "link_relative_position",
        "link_relative_home_position",
        "link_relative_velocity",
    ] {
        extend_vec3_feature_names(&mut names, prefix);
    }
    names.push("link_distance".into());
    extend_vec3_feature_names(&mut names, "parent_relative_position");
    extend_vec3_feature_names(&mut names, "parent_relative_velocity");
    extend_vec3_feature_names(&mut names, "attention_absolute_position");
    extend_vec3_feature_names(&mut names, "attention_link_relative_position");
    extend_vec3_feature_names(&mut names, "enemy_absolute_down_position");
    extend_vec3_feature_names(&mut names, "enemy_absolute_head_lock_position");
    names
}

fn native_actor_binary_names() -> Vec<String> {
    let mut names = [
        "base_state_available",
        "heap_present",
        "model_present",
        "joint_collision_present",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    names.extend((0..32).map(|bit| format!("status_bit_{bit}")));
    names.extend(
        [
            "attention_present",
            "event_participation_present",
            "return_place_writer_present",
            "enemy_base_present",
            "return_no_telop_clear",
            "return_event_set_satisfied",
            "return_event_unset_satisfied",
            "return_switch_set_satisfied",
            "return_switch_unset_satisfied",
            "return_eligible",
            "player_targeted_actor",
            "player_ride_actor",
            "player_held_item_actor",
            "player_grabbed_actor",
            "player_thrown_boomerang_actor",
            "player_copy_rod_actor",
            "player_hookshot_roof_wait_actor",
            "player_chain_grab_actor",
            "player_attention_hint_actor",
            "player_attention_catch_actor",
            "player_attention_look_actor",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    names
}

fn extend_vec3_feature_names(names: &mut Vec<String>, prefix: &str) {
    names.extend(["x", "y", "z"].map(|axis| format!("{prefix}_{axis}")));
}

fn native_actor_nodes(observation: &NativeLearningObservation) -> Vec<TypedSetNode> {
    let actors_by_generation = observation
        .actors
        .iter()
        .map(|actor| (actor.runtime_generation, actor))
        .collect::<BTreeMap<_, _>>();
    observation
        .actors
        .iter()
        .map(|actor| native_actor_node(observation, actor, &actors_by_generation))
        .collect()
}

fn native_actor_node(
    observation: &NativeLearningObservation,
    actor: &NativeActorObservation,
    actors_by_generation: &BTreeMap<u64, &NativeActorObservation>,
) -> TypedSetNode {
    let mut categorical = Vec::new();
    let mut categorical_present = Vec::new();
    let mut category = |value: i64, available: bool| {
        categorical.push(if available { value } else { 0 });
        categorical_present.push(available);
    };
    for value in [
        i64::from(actor.parameters),
        i64::from(actor.status),
        i64::from(actor.actor_name),
        i64::from(actor.profile_name),
        i64::from(actor.set_id),
        i64::from(actor.home_room),
        i64::from(actor.current_room),
        i64::from(actor.group),
        i64::from(actor.argument),
        i64::from(actor.health),
    ] {
        category(value, true);
    }
    for value in [
        i64::from(actor.actor_type),
        i64::from(actor.process_subtype),
        i64::from(actor.condition),
        i64::from(actor.old_room),
        i64::from(actor.pause_flag),
        i64::from(actor.process_init_state),
        i64::from(actor.process_create_phase),
        i64::from(actor.cull_type),
        i64::from(actor.demo_actor_id),
        i64::from(actor.carry_type),
    ] {
        category(value, actor.base_state_available);
    }
    if let Some(attention) = &actor.attention {
        category(i64::from(attention.flags), true);
        for value in attention.distance_indices {
            category(i64::from(value), true);
        }
        category(i64::from(attention.auxiliary), true);
    } else {
        for _ in 0..11 {
            category(0, false);
        }
    }
    if let Some(event) = &actor.event_participation {
        for value in [
            i64::from(event.command),
            i64::from(event.condition),
            i64::from(event.event_id),
            i64::from(event.map_tool_id),
            i64::from(event.index),
        ] {
            category(value, true);
        }
    } else {
        for _ in 0..5 {
            category(0, false);
        }
    }
    if let Some(writer) = &actor.return_place_writer {
        for value in [
            i64::from(writer.save_room),
            i64::from(writer.save_point),
            i64::from(writer.switch_room),
            i64::from(writer.required_event_set),
            i64::from(writer.required_event_unset),
            i64::from(writer.required_switch_set),
            i64::from(writer.required_switch_unset),
        ] {
            category(value, true);
        }
    } else {
        for _ in 0..7 {
            category(0, false);
        }
    }
    if let Some(enemy) = &actor.enemy_base {
        category(i64::from(enemy.flags), true);
        category(i64::from(enemy.throw_mode), true);
    } else {
        category(0, false);
        category(0, false);
    }

    let mut continuous = Vec::new();
    let mut continuous_present = Vec::new();
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor.position,
        true,
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor.home_position,
        true,
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor.old_position,
        actor.base_state_available,
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor.velocity,
        true,
    );
    push_continuous(
        &mut continuous,
        &mut continuous_present,
        actor.forward_speed,
        true,
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor.scale,
        actor.base_state_available,
    );
    push_continuous(
        &mut continuous,
        &mut continuous_present,
        actor.gravity,
        actor.base_state_available,
    );
    push_continuous(
        &mut continuous,
        &mut continuous_present,
        actor.max_fall_speed,
        actor.base_state_available,
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor.eye_position,
        actor.base_state_available,
    );
    for (index, angles) in [
        actor.home_angle,
        actor.old_angle,
        actor.current_angle,
        actor.shape_angle,
    ]
    .into_iter()
    .enumerate()
    {
        let available = actor.base_state_available || index >= 2;
        push_continuous3(
            &mut continuous,
            &mut continuous_present,
            angles.map(f32::from),
            available,
        );
    }
    let player_available = observation.player_present;
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        subtract3(actor.position, observation.player_position),
        player_available,
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        subtract3(actor.home_position, observation.player_position),
        player_available,
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        subtract3(actor.velocity, observation.player_velocity),
        player_available,
    );
    push_continuous(
        &mut continuous,
        &mut continuous_present,
        length3(subtract3(actor.position, observation.player_position)),
        player_available,
    );
    let parent = actors_by_generation.get(&u64::from(actor.parent_runtime_generation));
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        parent.map_or([0.0; 3], |parent| {
            subtract3(actor.position, parent.position)
        }),
        parent.is_some(),
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        parent.map_or([0.0; 3], |parent| {
            subtract3(actor.velocity, parent.velocity)
        }),
        parent.is_some(),
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor
            .attention
            .as_ref()
            .map_or([0.0; 3], |value| value.position),
        actor.attention.is_some(),
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor.attention.as_ref().map_or([0.0; 3], |value| {
            subtract3(value.position, observation.player_position)
        }),
        actor.attention.is_some() && player_available,
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor
            .enemy_base
            .as_ref()
            .map_or([0.0; 3], |value| value.down_position),
        actor.enemy_base.is_some(),
    );
    push_continuous3(
        &mut continuous,
        &mut continuous_present,
        actor
            .enemy_base
            .as_ref()
            .map_or([0.0; 3], |value| value.head_lock_position),
        actor.enemy_base.is_some(),
    );

    let mut binary = Vec::new();
    let mut binary_present = Vec::new();
    let mut boolean = |value: bool, available: bool| {
        binary.push(value && available);
        binary_present.push(available);
    };
    boolean(actor.base_state_available, true);
    boolean(actor.heap_present, actor.base_state_available);
    boolean(actor.model_present, actor.base_state_available);
    boolean(actor.joint_collision_present, actor.base_state_available);
    for bit in 0..32 {
        boolean(actor.status & (1_u32 << bit) != 0, true);
    }
    boolean(actor.attention.is_some(), true);
    boolean(actor.event_participation.is_some(), true);
    boolean(actor.return_place_writer.is_some(), true);
    boolean(actor.enemy_base.is_some(), true);
    if let Some(writer) = &actor.return_place_writer {
        for value in [
            writer.no_telop_clear,
            writer.event_set_satisfied,
            writer.event_unset_satisfied,
            writer.switch_set_satisfied,
            writer.switch_unset_satisfied,
            writer.eligible,
        ] {
            boolean(value, true);
        }
    } else {
        for _ in 0..6 {
            boolean(false, false);
        }
    }
    let relationships_available =
        observation.player_relationships_status == NativeChannelStatus::Present;
    let relationships = observation.player_relationships.as_ref();
    let related = |identity: Option<
        &dusklight_evidence::native_episode_shard::NativeActorIdentity,
    >| {
        identity.is_some_and(|identity| {
            identity.present && u64::from(identity.runtime_generation) == actor.runtime_generation
        })
    };
    for value in [
        relationships.and_then(|value| value.targeted_actor.as_ref()),
        relationships.and_then(|value| value.ride_actor.as_ref()),
        relationships.and_then(|value| value.held_item_actor.as_ref()),
        relationships.and_then(|value| value.grabbed_actor.as_ref()),
        relationships.and_then(|value| value.thrown_boomerang_actor.as_ref()),
        relationships.and_then(|value| value.copy_rod_actor.as_ref()),
        relationships.and_then(|value| value.hookshot_roof_wait_actor.as_ref()),
        relationships.and_then(|value| value.chain_grab_actor.as_ref()),
        relationships.and_then(|value| value.attention_hint_actor.as_ref()),
        relationships.and_then(|value| value.attention_catch_actor.as_ref()),
        relationships.and_then(|value| value.attention_look_actor.as_ref()),
    ] {
        boolean(related(value), relationships_available);
    }
    debug_assert_eq!(categorical.len(), native_actor_categorical_names().len());
    debug_assert_eq!(continuous.len(), native_actor_continuous_names().len());
    debug_assert_eq!(binary.len(), native_actor_binary_names().len());
    TypedSetNode {
        stable_id: actor.runtime_generation,
        categorical,
        categorical_present,
        continuous,
        continuous_present,
        binary,
        binary_present,
    }
}

fn subtract3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn push_continuous(values: &mut Vec<f32>, present: &mut Vec<bool>, value: f32, available: bool) {
    values.push(if available { value } else { 0.0 });
    present.push(available);
}

fn push_continuous3(
    values: &mut Vec<f32>,
    present: &mut Vec<bool>,
    value: [f32; 3],
    available: bool,
) {
    for component in value {
        push_continuous(values, present, component, available);
    }
}

fn length3(value: [f32; 3]) -> f32 {
    value
        .iter()
        .map(|component| component * component)
        .sum::<f32>()
        .sqrt()
}

fn native_target_names() -> Vec<String> {
    [
        "player_position_delta_x",
        "player_position_delta_y",
        "player_position_delta_z",
        "player_velocity_delta_x",
        "player_velocity_delta_y",
        "player_velocity_delta_z",
        "player_forward_speed_delta",
        "contact_changed",
        "procedure_changed",
        "mode_flags_changed",
        "actor_disappearance_count",
        "consumed_stick_x",
        "consumed_stick_y",
        "consumed_button_0x0100",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn native_targets(example: &NativeAuxiliaryExample) -> (Vec<f32>, Vec<bool>) {
    let mut targets = vec![0.0; 14];
    let mut present = vec![false; 14];
    if let Some(dynamics) = &example.targets.player_dynamics {
        targets[..3].copy_from_slice(&dynamics.position_delta);
        targets[3..6].copy_from_slice(&dynamics.velocity_delta);
        targets[6] = dynamics.forward_speed_delta;
        present[..7].fill(true);
    }
    if let Some(contacts) = &example.targets.contacts {
        targets[7] = f32::from(contacts.activated != 0 || contacts.cleared != 0);
        present[7] = true;
    }
    if let Some(action) = &example.targets.action_phase {
        targets[8] = f32::from(action.procedure_before != action.procedure_after);
        targets[9] = f32::from(action.mode_flags_activated != 0 || action.mode_flags_cleared != 0);
        present[8..10].fill(true);
    }
    targets[10] = example
        .targets
        .actor_lifecycle
        .disappeared_runtime_generations
        .len() as f32;
    targets[11] = f32::from(example.targets.inverse_action.stick_x);
    targets[12] = f32::from(example.targets.inverse_action.stick_y);
    targets[13] = f32::from(example.targets.inverse_action.buttons & 0x0100 != 0);
    present[10..].fill(true);
    (targets, present)
}

fn broad_base(observation: &NativeLearningObservation) -> (Vec<f32>, Vec<bool>) {
    let mut values = Vec::new();
    let mut present = Vec::new();
    let mut push = |value: f32, available: bool| {
        values.push(if available { value } else { 0.0 });
        present.push(available);
    };
    for value in observation.player_position {
        push(value, observation.player_present);
    }
    for value in observation.player_velocity {
        push(value, observation.player_present);
    }
    push(observation.player_forward_speed, observation.player_present);
    for angle in observation
        .player_current_angle
        .into_iter()
        .chain(observation.player_shape_angle)
    {
        push(f32::from(angle), observation.player_present);
    }
    push(
        f32::from(observation.player_procedure),
        observation.player_present,
    );
    for bit in 0..32 {
        push(
            f32::from(observation.player_mode_flags & (1_u32 << bit) != 0),
            observation.player_present,
        );
    }
    for bit in 0..8 {
        push(
            f32::from(observation.player_contacts & (1_u8 << bit) != 0),
            observation.player_present,
        );
    }
    push(f32::from(observation.event_running), true);
    push(f32::from(observation.event_id), true);
    push(f32::from(observation.event_mode), true);
    push(f32::from(observation.event_status), true);
    push(f32::from(observation.event_map_tool_id), true);
    push(f32::from(observation.room), true);
    push(f32::from(observation.layer), true);
    push(f32::from(observation.point), true);
    for value in [
        observation.previous_input.stick_x,
        observation.previous_input.stick_y,
        observation.previous_input.substick_x,
        observation.previous_input.substick_y,
    ] {
        push(f32::from(value), true);
    }
    for value in [
        observation.previous_input.trigger_left,
        observation.previous_input.trigger_right,
        observation.previous_input.analog_a,
        observation.previous_input.analog_b,
    ] {
        push(f32::from(value), true);
    }
    for bit in 0..16 {
        push(
            f32::from(observation.previous_input.buttons & (1_u16 << bit) != 0),
            true,
        );
    }
    push(
        observation.camera_yaw_radians.unwrap_or(0.0),
        observation.camera_yaw_radians.is_some(),
    );
    for index in 0..9 {
        let camera_value = observation.camera.as_ref().map(|camera| match index {
            0 => f32::from(camera.view_yaw),
            1 => f32::from(camera.controlled_yaw),
            2 => f32::from(camera.bank),
            3..=5 => camera.eye[index - 3],
            _ => camera.center[index - 6],
        });
        push(camera_value.unwrap_or(0.0), camera_value.is_some());
    }
    push(
        observation.player_ground_height.unwrap_or(0.0),
        observation.player_ground_height.is_some(),
    );
    push(
        observation.player_roof_height.unwrap_or(0.0),
        observation.player_roof_height.is_some(),
    );
    let water_height = observation
        .player_background_collision
        .as_ref()
        .map(|collision| collision.water_height);
    push(water_height.unwrap_or(0.0), water_height.is_some());
    for index in 0..2 {
        let correction = observation.collision_correction.map(|value| value[index]);
        push(correction.unwrap_or(0.0), correction.is_some());
    }
    for index in 0..7 {
        let scene = observation.scene_exit.as_ref().map(|exit| match index {
            0 => exit.signed_distance_to_volume,
            1..=3 => exit.player_local_position[index - 1],
            _ => exit.volume_extent[index - 4],
        });
        push(scene.unwrap_or(0.0), scene.is_some());
    }
    for stream_index in 0..2 {
        let stream = observation.rng_streams.get(stream_index);
        push(
            stream.map_or(0.0, |value| f32::from(value.id)),
            stream.is_some(),
        );
        for state_index in 0..3 {
            push(
                stream.map_or(0.0, |value| value.state[state_index] as f32),
                stream.is_some(),
            );
        }
        push(
            stream.map_or(0.0, |value| value.call_count as f32),
            stream.is_some(),
        );
    }
    push(observation.rng_streams.len() as f32, true);
    push(observation.rng_streams.len().saturating_sub(2) as f32, true);
    for value in [
        observation.goal.requested_count,
        observation.goal.hit_count,
        observation.goal.stable_ticks,
        observation.goal.consecutive_ticks,
        u16::from(observation.goal.sequence_steps),
        u16::from(observation.goal.sequence_next_step),
        observation.goal.sequence_within_ticks,
        observation.goal.sequence_elapsed_ticks,
    ] {
        push(f32::from(value), observation.goal.configured);
    }
    push(
        f32::from(observation.goal.reached),
        observation.goal.configured,
    );
    (values, present)
}

fn sample_manifest_digest(samples: &[MultiTaskSetSample]) -> Result<Digest, TrainableSetError> {
    canonical_digest(
        b"dusklight.native-multitask-sample-dataset/v1\0",
        &samples
            .iter()
            .map(|sample| {
                (
                    sample.input.sample_sha256,
                    &sample.targets,
                    &sample.target_present,
                )
            })
            .collect::<Vec<_>>(),
    )
}

fn hex_128(value: [u8; 16]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[allow(clippy::too_many_arguments)]
fn validate_samples(
    actor_feature_schema_sha256: Digest,
    training_dataset_sha256: Digest,
    held_out_dataset_sha256: Digest,
    target_names: &[String],
    training: &[MultiTaskSetSample],
    held_out: &[MultiTaskSetSample],
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
        || target_names.is_empty()
        || target_names.len() > MAX_TARGETS
        || target_names.iter().any(|name| name.is_empty())
        || target_names.iter().collect::<BTreeSet<_>>().len() != target_names.len()
        || config.epochs == 0
        || config.epochs > MAX_EPOCHS
        || config.node_hidden_width == 0
        || config.node_hidden_width > MAX_HIDDEN_WIDTH
        || config.head_hidden_width == 0
        || config.head_hidden_width > MAX_HIDDEN_WIDTH
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
            "multitask set encoder configuration is invalid",
        ));
    }
    let first_node = training
        .iter()
        .chain(held_out)
        .find_map(|sample| sample.input.nodes.first())
        .ok_or_else(|| TrainableSetError::new("multitask corpus has no actor nodes"))?;
    let dimensions = Dimensions {
        categorical: first_node.categorical.len(),
        continuous: first_node.continuous.len(),
        binary: first_node.binary.len(),
        base: training[0].input.base.len(),
    };
    let mut identities = BTreeSet::new();
    for sample in training.iter().chain(held_out) {
        if sample.input.sample_sha256 == Digest::ZERO
            || !identities.insert(sample.input.sample_sha256)
            || sample.input.actor_feature_schema_sha256 != actor_feature_schema_sha256
            || sample.targets.len() != target_names.len()
            || sample.target_present.len() != target_names.len()
            || sample
                .targets
                .iter()
                .zip(&sample.target_present)
                .any(|(target, present)| !target.is_finite() || (!present && *target != 0.0))
        {
            return Err(TrainableSetError::new(
                "multitask sample identity, schema, target, or mask is invalid",
            ));
        }
        validate_sample_dimensions(&sample.input, dimensions)?;
    }
    if target_support(training, target_names.len()).contains(&0) {
        return Err(TrainableSetError::new(
            "each auxiliary target requires training support",
        ));
    }
    Ok(dimensions)
}

fn target_support(samples: &[MultiTaskSetSample], width: usize) -> Vec<usize> {
    (0..width)
        .map(|target| {
            samples
                .iter()
                .filter(|sample| sample.target_present[target])
                .count()
        })
        .collect()
}

fn target_normalization(
    training: &[MultiTaskSetSample],
    width: usize,
) -> Result<TargetNormalization, TrainableSetError> {
    let support = target_support(training, width);
    let mut means = Vec::with_capacity(width);
    let mut inverse_stddevs = Vec::with_capacity(width);
    for target in 0..width {
        let values = training
            .iter()
            .filter(|sample| sample.target_present[target])
            .map(|sample| f64::from(sample.targets[target]))
            .collect::<Vec<_>>();
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
    if means
        .iter()
        .chain(&inverse_stddevs)
        .any(|value| !value.is_finite())
    {
        return Err(TrainableSetError::new(
            "multitask target normalization is non-finite",
        ));
    }
    Ok((means, inverse_stddevs, support))
}

fn relative_improvement(baseline: f64, model: f64) -> f64 {
    if baseline > f64::EPSILON {
        (baseline - model) / baseline
    } else {
        0.0
    }
}

fn report_digest(report: &MultiTaskSetEncoderReport) -> Result<Digest, TrainableSetError> {
    let mut canonical = report.clone();
    canonical.report_sha256 = Digest::ZERO;
    canonical_digest(b"dusklight.multitask-set-encoder-report/v1\0", &canonical)
}

fn canonical_digest<T: Serialize>(domain: &[u8], value: &T) -> Result<Digest, TrainableSetError> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| TrainableSetError::new(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trainable_set_encoder::TypedSetNode;

    fn sample(identity: u8, first: f32, second: f32, reverse: bool) -> MultiTaskSetSample {
        let mut nodes = vec![
            TypedSetNode {
                stable_id: 1,
                categorical: vec![10],
                categorical_present: vec![true],
                continuous: vec![first],
                continuous_present: vec![true],
                binary: vec![first > 0.0],
                binary_present: vec![true],
            },
            TypedSetNode {
                stable_id: 2,
                categorical: vec![20],
                categorical_present: vec![true],
                continuous: vec![second],
                continuous_present: vec![true],
                binary: vec![second > 0.0],
                binary_present: vec![true],
            },
        ];
        if reverse {
            nodes.reverse();
        }
        let second_present = !identity.is_multiple_of(5);
        MultiTaskSetSample {
            input: TypedSetSample {
                sample_sha256: Digest([identity; 32]),
                actor_feature_schema_sha256: Digest([7; 32]),
                base: vec![first - second],
                base_present: vec![true],
                nodes,
                target: 0.0,
            },
            targets: vec![
                first + second,
                if second_present { first - second } else { 0.0 },
            ],
            target_present: vec![true, second_present],
        }
    }

    fn corpus(start: u8, count: usize) -> Vec<MultiTaskSetSample> {
        (0..count)
            .map(|index| {
                let first = ((index * 17 % 41) as f32 - 20.0) / 10.0;
                let second = ((index * 29 % 37) as f32 - 18.0) / 10.0;
                sample(start + index as u8, first, second, index % 2 == 0)
            })
            .collect()
    }

    #[test]
    fn direct_native_adapter_keeps_the_complete_typed_actor_population() {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v15.dseps"
        ))
        .unwrap();
        let observation = &shard.episodes[0].steps[0].pre_input;
        assert!(!observation.actors_truncated);
        let nodes = native_actor_nodes(observation);
        assert_eq!(nodes.len(), observation.actors.len());
        assert_eq!(
            nodes.iter().map(|node| node.stable_id).collect::<Vec<_>>(),
            observation
                .actors
                .iter()
                .map(|actor| actor.runtime_generation)
                .collect::<Vec<_>>()
        );
        assert!(nodes.iter().all(|node| {
            node.categorical.len() == native_actor_categorical_names().len()
                && node.categorical.len() == node.categorical_present.len()
                && node.continuous.len() == native_actor_continuous_names().len()
                && node.continuous.len() == node.continuous_present.len()
                && node.binary.len() == native_actor_binary_names().len()
                && node.binary.len() == node.binary_present.len()
        }));
        let (base, present) = broad_base(observation);
        assert_eq!(base.len(), 129);
        assert_eq!(present.len(), 129);
        assert_ne!(native_actor_feature_schema().unwrap(), Digest::ZERO);
    }

    #[test]
    fn shuffled_control_rebinds_targets_without_changing_support() {
        let training = corpus(1, 96);
        let validation = corpus(130, 32);
        let test = corpus(170, 32);
        let original_digest = sample_manifest_digest(&training).unwrap();
        let config = TrainableSetConfig {
            epochs: 2,
            node_hidden_width: 8,
            head_hidden_width: 8,
            minimum_relative_improvement: 1.0,
            ..TrainableSetConfig::default()
        };
        let control = fit_shuffled_auxiliary_control(
            Digest([7; 32]),
            vec!["sum".into(), "difference".into()],
            training,
            sample_manifest_digest(&validation).unwrap(),
            &validation,
            &test,
            config,
        )
        .unwrap();
        assert_eq!(control.schema, SHUFFLED_AUXILIARY_CONTROL_SCHEMA_V1);
        assert_ne!(control.shuffled_training_dataset_sha256, original_digest);
        assert_eq!(control.report.target_support_training, vec![96, 77]);
        assert_eq!(control.test_evaluation.samples, 32);
        assert_eq!(
            control.report.decision,
            MultiTaskEncoderDecision::RetainTrainingMeanBaseline
        );
    }

    #[test]
    fn shared_complete_set_encoder_learns_masked_heads_on_held_out_rows() {
        let training = corpus(1, 96);
        let held_out = corpus(130, 32);
        let config = TrainableSetConfig {
            epochs: 180,
            node_hidden_width: 12,
            head_hidden_width: 16,
            learning_rate: 0.003,
            minimum_relative_improvement: 0.25,
            ..TrainableSetConfig::default()
        };
        let (report, model) = CompleteSetMultiTaskEncoder::fit(
            Digest([7; 32]),
            Digest([8; 32]),
            Digest([9; 32]),
            vec!["sum".into(), "difference".into()],
            &training,
            &held_out,
            config,
        )
        .unwrap();
        assert_eq!(report.target_support_training, vec![96, 77]);
        assert_eq!(report.target_support_held_out, vec![32, 25]);
        assert!(report.relative_held_out_improvement > 0.25);
        assert_eq!(
            report.decision,
            MultiTaskEncoderDecision::SharedEncoderCandidate
        );
        assert_eq!(model.encode(&held_out[0].input).unwrap().len(), 16);
        assert_eq!(model.predict(&held_out[0].input).unwrap().len(), 2);
        let evaluation = model.evaluate(&held_out).unwrap();
        assert_eq!(evaluation.samples, 32);
        assert!(evaluation.relative_improvement > 0.25);
        assert!(!report.promotion_authority);
        assert_ne!(report.report_sha256, Digest::ZERO);
    }

    #[test]
    fn rejects_cross_split_identity_and_unsupported_target() {
        let training = corpus(1, 8);
        let mut held_out = corpus(40, 4);
        held_out[0].input.sample_sha256 = training[0].input.sample_sha256;
        assert!(
            CompleteSetMultiTaskEncoder::fit(
                Digest([7; 32]),
                Digest([8; 32]),
                Digest([9; 32]),
                vec!["sum".into(), "difference".into()],
                &training,
                &held_out,
                TrainableSetConfig::default(),
            )
            .is_err()
        );
        let mut unsupported = corpus(40, 4);
        for sample in &mut unsupported {
            sample.target_present[1] = false;
            sample.targets[1] = 0.0;
        }
        assert!(
            CompleteSetMultiTaskEncoder::fit(
                Digest([7; 32]),
                Digest([8; 32]),
                Digest([9; 32]),
                vec!["sum".into(), "difference".into()],
                &training,
                &unsupported,
                TrainableSetConfig::default(),
            )
            .is_err()
        );
    }
}
