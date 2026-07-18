//! Deterministic discrete Implicit Q-Learning with advantage-weighted cloning.

use crate::artifact::Digest;
use crate::fqi::{MAX_FQI_ACTIONS, MAX_FQI_TRANSITIONS, Transition};
use serde::Serialize;
use std::error::Error;
use std::fmt;

pub const IQL_MODEL_SCHEMA_V1: &str = "dusklight-discrete-iql-model/v1";
pub const MAX_IQL_EPOCHS: usize = 256;
pub const MAX_IQL_HIDDEN_WIDTH: usize = 128;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct IqlConfig {
    pub epochs: usize,
    pub hidden_width: usize,
    pub learning_rate: f64,
    pub discount: f64,
    pub expectile: f64,
    pub advantage_inverse_temperature: f64,
    pub max_advantage_weight: f64,
    pub target_sync_steps: usize,
    pub gradient_clip: f64,
    pub seed: u64,
}

impl Default for IqlConfig {
    fn default() -> Self {
        Self {
            epochs: 64,
            hidden_width: 32,
            learning_rate: 0.001,
            discount: 0.995,
            expectile: 0.7,
            advantage_inverse_temperature: 3.0,
            max_advantage_weight: 100.0,
            target_sync_steps: 256,
            gradient_clip: 10.0,
            seed: 0x1a1a_5eed_0000_0001,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct IqlActionEstimate {
    pub action: u32,
    pub policy_probability: f64,
    pub mean_q: f64,
    pub value: f64,
    pub advantage: f64,
    pub critic_disagreement: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ImplicitQ {
    feature_width: usize,
    actions: Vec<u32>,
    feature_mean: Vec<f64>,
    feature_inverse_stddev: Vec<f64>,
    critic_a: Network,
    critic_b: Network,
    value: Network,
    policy: Network,
    gradient_updates: u64,
    target_synchronizations: u64,
    mean_advantage_weight: f64,
    clipped_advantage_weights: u64,
}

impl ImplicitQ {
    pub fn fit(
        feature_width: usize,
        actions: &[u32],
        transitions: &[Transition],
        config: &IqlConfig,
    ) -> Result<Self, IqlError> {
        validate(feature_width, actions, transitions, config)?;
        let mut actions = actions.to_vec();
        actions.sort_unstable();
        let (feature_mean, feature_inverse_stddev) = normalization(feature_width, transitions);
        let states = transitions
            .iter()
            .map(|transition| normalize(&transition.state, &feature_mean, &feature_inverse_stddev))
            .collect::<Vec<_>>();
        let next_states = transitions
            .iter()
            .map(|transition| {
                normalize(
                    &transition.next_state,
                    &feature_mean,
                    &feature_inverse_stddev,
                )
            })
            .collect::<Vec<_>>();
        let mut rng = Rng::new(config.seed);
        let mut critic_a =
            Network::initialized(feature_width, config.hidden_width, actions.len(), &mut rng);
        let mut critic_b =
            Network::initialized(feature_width, config.hidden_width, actions.len(), &mut rng);
        let mut target_a = critic_a.clone();
        let mut target_b = critic_b.clone();
        let mut value = Network::initialized(feature_width, config.hidden_width, 1, &mut rng);
        let mut policy =
            Network::initialized(feature_width, config.hidden_width, actions.len(), &mut rng);
        let mut order = (0..transitions.len()).collect::<Vec<_>>();
        let mut gradient_updates = 0_u64;
        let mut target_synchronizations = 0_u64;
        let mut advantage_weight_sum = 0.0;
        let mut clipped_advantage_weights = 0_u64;

        for epoch in 0..config.epochs {
            rng.shuffle(&mut order);
            for row in order.iter().copied() {
                let transition = &transitions[row];
                let action = actions
                    .binary_search(&transition.action)
                    .expect("transition actions were validated");

                let target_q = target_a
                    .value(&states[row], action)
                    .min(target_b.value(&states[row], action));
                let value_prediction = value.value(&states[row], 0);
                let residual = target_q - value_prediction;
                let expectile_weight = if residual >= 0.0 {
                    config.expectile
                } else {
                    1.0 - config.expectile
                };
                value.update(
                    &states[row],
                    &[expectile_weight * (value_prediction - target_q)],
                    config.learning_rate,
                    config.gradient_clip,
                )?;

                let q_target = if transition.terminal {
                    f64::from(transition.reward)
                } else {
                    f64::from(transition.reward)
                        + config.discount.powf(f64::from(transition.duration))
                            * value.value(&next_states[row], 0)
                };
                for critic in [&mut critic_a, &mut critic_b] {
                    let mut gradient = vec![0.0; actions.len()];
                    gradient[action] = critic.value(&states[row], action) - q_target;
                    critic.update(
                        &states[row],
                        &gradient,
                        config.learning_rate,
                        config.gradient_clip,
                    )?;
                }

                let q_observed = critic_a
                    .value(&states[row], action)
                    .min(critic_b.value(&states[row], action));
                let advantage = q_observed - value.value(&states[row], 0);
                let raw_weight = (config.advantage_inverse_temperature * advantage).exp();
                let advantage_weight = raw_weight.min(config.max_advantage_weight);
                if raw_weight > config.max_advantage_weight {
                    clipped_advantage_weights += 1;
                }
                advantage_weight_sum += advantage_weight;
                let probabilities = softmax(&policy.values(&states[row]));
                let policy_gradient = probabilities
                    .iter()
                    .enumerate()
                    .map(|(candidate, probability)| {
                        advantage_weight * (probability - f64::from(candidate == action))
                    })
                    .collect::<Vec<_>>();
                policy.update(
                    &states[row],
                    &policy_gradient,
                    config.learning_rate,
                    config.gradient_clip,
                )?;

                gradient_updates += 1;
                if gradient_updates % config.target_sync_steps as u64 == 0 {
                    target_a = critic_a.clone();
                    target_b = critic_b.clone();
                    target_synchronizations += 1;
                }
            }
            if !value.all_finite()
                || !critic_a.all_finite()
                || !critic_b.all_finite()
                || !policy.all_finite()
            {
                return Err(IqlError::Diverged { epoch });
            }
        }

        Ok(Self {
            feature_width,
            actions,
            feature_mean,
            feature_inverse_stddev,
            critic_a,
            critic_b,
            value,
            policy,
            gradient_updates,
            target_synchronizations,
            mean_advantage_weight: advantage_weight_sum / gradient_updates as f64,
            clipped_advantage_weights,
        })
    }

    pub fn rank_actions(&self, state: &[f32]) -> Result<Vec<IqlActionEstimate>, IqlError> {
        let state = self.normalized_state(state)?;
        let probabilities = softmax(&self.policy.values(&state));
        let state_value = self.value.value(&state, 0);
        let mut ranking = self
            .actions
            .iter()
            .enumerate()
            .map(|(index, action)| {
                let critic_a = self.critic_a.value(&state, index);
                let critic_b = self.critic_b.value(&state, index);
                let mean_q = (critic_a + critic_b) * 0.5;
                IqlActionEstimate {
                    action: *action,
                    policy_probability: probabilities[index],
                    mean_q,
                    value: state_value,
                    advantage: critic_a.min(critic_b) - state_value,
                    critic_disagreement: (critic_a - critic_b).abs(),
                }
            })
            .collect::<Vec<_>>();
        ranking.sort_by(|left, right| {
            right
                .policy_probability
                .total_cmp(&left.policy_probability)
                .then_with(|| right.mean_q.total_cmp(&left.mean_q))
                .then_with(|| left.action.cmp(&right.action))
        });
        Ok(ranking)
    }

    pub fn gradient_updates(&self) -> u64 {
        self.gradient_updates
    }

    pub fn target_synchronizations(&self) -> u64 {
        self.target_synchronizations
    }

    pub fn mean_advantage_weight(&self) -> f64 {
        self.mean_advantage_weight
    }

    pub fn clipped_advantage_weights(&self) -> u64 {
        self.clipped_advantage_weights
    }

    pub fn artifact_bytes(
        &self,
        feature_schema: Digest,
        action_schema: Digest,
        training_dataset_sha256: Option<Digest>,
        training_corpus_sha256: &[Digest],
        config: &IqlConfig,
    ) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(&IqlArtifact {
            schema: IQL_MODEL_SCHEMA_V1,
            feature_schema,
            action_schema,
            training_dataset_sha256,
            training_corpus_sha256,
            config,
            model: self,
        })
    }

    fn normalized_state(&self, state: &[f32]) -> Result<Vec<f64>, IqlError> {
        if state.len() != self.feature_width {
            return Err(IqlError::FeatureWidth {
                expected: self.feature_width,
                actual: state.len(),
            });
        }
        if state.iter().any(|value| !value.is_finite()) {
            return Err(IqlError::NonFiniteData);
        }
        Ok(normalize(
            state,
            &self.feature_mean,
            &self.feature_inverse_stddev,
        ))
    }
}

#[derive(Serialize)]
struct IqlArtifact<'a> {
    schema: &'static str,
    feature_schema: Digest,
    action_schema: Digest,
    training_dataset_sha256: Option<Digest>,
    training_corpus_sha256: &'a [Digest],
    config: &'a IqlConfig,
    model: &'a ImplicitQ,
}

#[derive(Clone, Debug, Serialize)]
struct Network {
    feature_width: usize,
    hidden_width: usize,
    output_width: usize,
    input_weights: Vec<f64>,
    hidden_bias: Vec<f64>,
    output_weights: Vec<f64>,
    output_bias: Vec<f64>,
}

impl Network {
    fn initialized(
        feature_width: usize,
        hidden_width: usize,
        output_width: usize,
        rng: &mut Rng,
    ) -> Self {
        let input_scale = (6.0 / (feature_width + hidden_width) as f64).sqrt();
        let output_scale = (6.0 / (hidden_width + output_width) as f64).sqrt();
        Self {
            feature_width,
            hidden_width,
            output_width,
            input_weights: (0..feature_width * hidden_width)
                .map(|_| rng.symmetric(input_scale))
                .collect(),
            hidden_bias: vec![0.0; hidden_width],
            output_weights: (0..hidden_width * output_width)
                .map(|_| rng.symmetric(output_scale))
                .collect(),
            output_bias: vec![0.0; output_width],
        }
    }

    fn hidden(&self, state: &[f64]) -> (Vec<f64>, Vec<bool>) {
        let mut hidden = vec![0.0; self.hidden_width];
        let mut active = vec![false; self.hidden_width];
        for hidden_index in 0..self.hidden_width {
            let offset = hidden_index * self.feature_width;
            let preactivation = self.hidden_bias[hidden_index]
                + (0..self.feature_width)
                    .map(|feature| self.input_weights[offset + feature] * state[feature])
                    .sum::<f64>();
            if preactivation > 0.0 {
                hidden[hidden_index] = preactivation;
                active[hidden_index] = true;
            }
        }
        (hidden, active)
    }

    fn values(&self, state: &[f64]) -> Vec<f64> {
        let (hidden, _) = self.hidden(state);
        (0..self.output_width)
            .map(|output| {
                let offset = output * self.hidden_width;
                self.output_bias[output]
                    + (0..self.hidden_width)
                        .map(|index| self.output_weights[offset + index] * hidden[index])
                        .sum::<f64>()
            })
            .collect()
    }

    fn value(&self, state: &[f64], output: usize) -> f64 {
        self.values(state)[output]
    }

    fn update(
        &mut self,
        state: &[f64],
        output_gradients: &[f64],
        learning_rate: f64,
        gradient_clip: f64,
    ) -> Result<(), IqlError> {
        debug_assert_eq!(output_gradients.len(), self.output_width);
        let (hidden, active) = self.hidden(state);
        let gradients = output_gradients
            .iter()
            .map(|gradient| gradient.clamp(-gradient_clip, gradient_clip))
            .collect::<Vec<_>>();
        let prior_output_weights = self.output_weights.clone();
        for output in 0..self.output_width {
            self.output_bias[output] -= learning_rate * gradients[output];
            let offset = output * self.hidden_width;
            for hidden_index in 0..self.hidden_width {
                self.output_weights[offset + hidden_index] -= learning_rate
                    * (gradients[output] * hidden[hidden_index])
                        .clamp(-gradient_clip, gradient_clip);
            }
        }
        for hidden_index in 0..self.hidden_width {
            if !active[hidden_index] {
                continue;
            }
            let hidden_gradient = (0..self.output_width)
                .map(|output| {
                    gradients[output]
                        * prior_output_weights[output * self.hidden_width + hidden_index]
                })
                .sum::<f64>()
                .clamp(-gradient_clip, gradient_clip);
            self.hidden_bias[hidden_index] -= learning_rate * hidden_gradient;
            let offset = hidden_index * self.feature_width;
            for feature in 0..self.feature_width {
                self.input_weights[offset + feature] -= learning_rate
                    * (hidden_gradient * state[feature]).clamp(-gradient_clip, gradient_clip);
            }
        }
        if !self.all_finite() {
            return Err(IqlError::Diverged { epoch: 0 });
        }
        Ok(())
    }

    fn all_finite(&self) -> bool {
        self.input_weights.iter().all(|value| value.is_finite())
            && self.hidden_bias.iter().all(|value| value.is_finite())
            && self.output_weights.iter().all(|value| value.is_finite())
            && self.output_bias.iter().all(|value| value.is_finite())
    }
}

fn softmax(logits: &[f64]) -> Vec<f64> {
    let maximum = logits.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let exponentials = logits
        .iter()
        .map(|value| (value - maximum).exp())
        .collect::<Vec<_>>();
    let denominator = exponentials.iter().sum::<f64>();
    exponentials
        .into_iter()
        .map(|value| value / denominator)
        .collect()
}

fn validate(
    feature_width: usize,
    actions: &[u32],
    transitions: &[Transition],
    config: &IqlConfig,
) -> Result<(), IqlError> {
    if feature_width == 0 || actions.is_empty() || transitions.is_empty() {
        return Err(IqlError::EmptyData);
    }
    if actions.len() > MAX_FQI_ACTIONS || transitions.len() > MAX_FQI_TRANSITIONS {
        return Err(IqlError::InvalidConfig(
            "data exceeds bounded learner limits",
        ));
    }
    let mut sorted = actions.to_vec();
    sorted.sort_unstable();
    if sorted.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(IqlError::InvalidConfig("actions must be unique"));
    }
    if config.epochs == 0 || config.epochs > MAX_IQL_EPOCHS {
        return Err(IqlError::InvalidConfig("epochs are outside bounds"));
    }
    if config.hidden_width == 0 || config.hidden_width > MAX_IQL_HIDDEN_WIDTH {
        return Err(IqlError::InvalidConfig("hidden width is outside bounds"));
    }
    if !config.learning_rate.is_finite()
        || !(0.0..=1.0).contains(&config.learning_rate)
        || config.learning_rate == 0.0
        || !config.discount.is_finite()
        || !(0.0..=1.0).contains(&config.discount)
        || !config.expectile.is_finite()
        || config.expectile <= 0.5
        || config.expectile >= 1.0
        || !config.advantage_inverse_temperature.is_finite()
        || config.advantage_inverse_temperature <= 0.0
        || config.advantage_inverse_temperature > 100.0
        || !config.max_advantage_weight.is_finite()
        || config.max_advantage_weight < 1.0
        || config.max_advantage_weight > 1_000.0
        || config.target_sync_steps == 0
        || config.target_sync_steps > 1_000_000
        || !config.gradient_clip.is_finite()
        || config.gradient_clip <= 0.0
    {
        return Err(IqlError::InvalidConfig(
            "invalid optimizer or IQL parameter",
        ));
    }
    let mut support = vec![0_usize; sorted.len()];
    for transition in transitions {
        if transition.state.len() != feature_width || transition.next_state.len() != feature_width {
            return Err(IqlError::FeatureWidth {
                expected: feature_width,
                actual: transition.state.len(),
            });
        }
        if transition.duration == 0
            || !transition.reward.is_finite()
            || transition.state.iter().any(|value| !value.is_finite())
            || transition.next_state.iter().any(|value| !value.is_finite())
        {
            return Err(IqlError::NonFiniteData);
        }
        let action = sorted
            .binary_search(&transition.action)
            .map_err(|_| IqlError::UnknownAction(transition.action))?;
        support[action] += 1;
    }
    if support.contains(&0) {
        return Err(IqlError::InvalidConfig(
            "each declared action needs samples",
        ));
    }
    Ok(())
}

fn normalization(feature_width: usize, transitions: &[Transition]) -> (Vec<f64>, Vec<f64>) {
    let count = transitions.len() as f64;
    let mut mean = vec![0.0; feature_width];
    for transition in transitions {
        for (index, value) in transition.state.iter().enumerate() {
            mean[index] += f64::from(*value) / count;
        }
    }
    let mut variance = vec![0.0; feature_width];
    for transition in transitions {
        for (index, value) in transition.state.iter().enumerate() {
            let delta = f64::from(*value) - mean[index];
            variance[index] += delta * delta / count;
        }
    }
    let inverse_stddev = variance
        .into_iter()
        .map(|variance| {
            let stddev = variance.sqrt();
            if stddev > 1e-9 { 1.0 / stddev } else { 1.0 }
        })
        .collect();
    (mean, inverse_stddev)
}

fn normalize(state: &[f32], mean: &[f64], inverse_stddev: &[f64]) -> Vec<f64> {
    state
        .iter()
        .zip(mean)
        .zip(inverse_stddev)
        .map(|((value, mean), inverse)| (f64::from(*value) - mean) * inverse)
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IqlError {
    EmptyData,
    FeatureWidth { expected: usize, actual: usize },
    UnknownAction(u32),
    NonFiniteData,
    InvalidConfig(&'static str),
    Diverged { epoch: usize },
}

impl fmt::Display for IqlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyData => {
                formatter.write_str("IQL requires features, actions, and transitions")
            }
            Self::FeatureWidth { expected, actual } => {
                write!(
                    formatter,
                    "feature width {actual} does not match {expected}"
                )
            }
            Self::UnknownAction(action) => write!(formatter, "unknown IQL action {action}"),
            Self::NonFiniteData => formatter.write_str("IQL data and durations must be valid"),
            Self::InvalidConfig(message) => write!(formatter, "invalid IQL config: {message}"),
            Self::Diverged { epoch } => write!(formatter, "IQL diverged at epoch {epoch}"),
        }
    }
}

impl Error for IqlError {}

struct Rng {
    state: u64,
}

impl Rng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    fn symmetric(&mut self, scale: f64) -> f64 {
        let unit = (self.next_u64() >> 11) as f64 / (1_u64 << 53) as f64;
        (unit * 2.0 - 1.0) * scale
    }

    fn shuffle<T>(&mut self, values: &mut [T]) {
        for index in (1..values.len()).rev() {
            let selected = (self.next_u64() % (index as u64 + 1)) as usize;
            values.swap(index, selected);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WAIT: u32 = 0;
    const ADVANCE: u32 = 1;

    fn transition(state: f32, action: u32, reward: f32) -> Transition {
        Transition {
            state: vec![state],
            action,
            duration: 2,
            reward,
            next_state: vec![state],
            terminal: true,
        }
    }

    #[test]
    fn seeded_iql_clones_only_logged_state_local_actions() {
        let mut transitions = Vec::new();
        for _ in 0..16 {
            transitions.push(transition(-1.0, WAIT, 2.0));
            transitions.push(transition(1.0, ADVANCE, 2.0));
        }
        let config = IqlConfig {
            epochs: 128,
            hidden_width: 8,
            learning_rate: 0.01,
            target_sync_steps: 16,
            seed: 9,
            ..IqlConfig::default()
        };
        let first = ImplicitQ::fit(1, &[WAIT, ADVANCE], &transitions, &config).unwrap();
        let second = ImplicitQ::fit(1, &[WAIT, ADVANCE], &transitions, &config).unwrap();
        assert_eq!(
            serde_json::to_vec(&first).unwrap(),
            serde_json::to_vec(&second).unwrap()
        );
        assert_eq!(first.rank_actions(&[-1.0]).unwrap()[0].action, WAIT);
        assert_eq!(first.rank_actions(&[1.0]).unwrap()[0].action, ADVANCE);
        assert_eq!(first.gradient_updates(), 4096);
        assert_eq!(first.target_synchronizations(), 256);
        assert!(first.mean_advantage_weight().is_finite());
    }

    #[test]
    fn expectile_and_advantage_bounds_fail_closed() {
        let transitions = vec![transition(0.0, WAIT, 0.0)];
        for config in [
            IqlConfig {
                expectile: 0.5,
                ..IqlConfig::default()
            },
            IqlConfig {
                max_advantage_weight: f64::INFINITY,
                ..IqlConfig::default()
            },
        ] {
            assert!(matches!(
                ImplicitQ::fit(1, &[WAIT], &transitions, &config),
                Err(IqlError::InvalidConfig(_))
            ));
        }
    }
}
