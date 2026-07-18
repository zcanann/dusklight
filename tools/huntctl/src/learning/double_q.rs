//! Small deterministic offline Double-Q learner for discrete action schemas.
//!
//! This is deliberately a bounded baseline rather than a general neural
//! training framework. Two one-hidden-layer critics are updated alternately.
//! The updating critic selects the next action while the other critic's frozen
//! target copy evaluates it. Target copies are synchronized after a declared
//! number of gradient updates.

pub mod prioritized;

use crate::artifact::Digest;
use crate::fqi::Transition;
use serde::Serialize;
use std::error::Error;
use std::fmt;

pub const DOUBLE_Q_MODEL_SCHEMA_V1: &str = "dusklight-double-q-model/v1";
pub const CONSERVATIVE_Q_MODEL_SCHEMA_V1: &str = "dusklight-conservative-q-model/v1";
pub const MAX_DOUBLE_Q_EPOCHS: usize = 256;
pub const MAX_DOUBLE_Q_HIDDEN_WIDTH: usize = 128;
pub const MAX_DOUBLE_Q_TARGET_SYNC_STEPS: usize = 1_000_000;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DoubleQConfig {
    pub epochs: usize,
    pub hidden_width: usize,
    pub learning_rate: f64,
    pub discount: f64,
    pub target_sync_steps: usize,
    pub gradient_clip: f64,
    pub seed: u64,
}

impl Default for DoubleQConfig {
    fn default() -> Self {
        Self {
            epochs: 64,
            hidden_width: 32,
            learning_rate: 0.001,
            discount: 0.995,
            target_sync_steps: 256,
            gradient_clip: 10.0,
            seed: 0xd0ab_1e01_5eed_0001,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ConservativeQConfig {
    pub double_q: DoubleQConfig,
    /// Weight of `T * logsumexp(Q(s, .) / T) - Q(s, observed_action)`.
    pub conservative_weight: f64,
    /// Temperature used by the discrete log-sum-exp penalty.
    pub temperature: f64,
}

impl Default for ConservativeQConfig {
    fn default() -> Self {
        Self {
            double_q: DoubleQConfig::default(),
            conservative_weight: 1.0,
            temperature: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct DoubleQEstimate {
    pub action: u32,
    pub mean: f64,
    pub critic_a: f64,
    pub critic_b: f64,
    pub critic_disagreement: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct DoubleQ {
    feature_width: usize,
    actions: Vec<u32>,
    feature_mean: Vec<f64>,
    feature_inverse_stddev: Vec<f64>,
    critic_a: Critic,
    critic_b: Critic,
    gradient_updates: u64,
    target_synchronizations: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ConservativeQ {
    model: DoubleQ,
    conservative_updates: u64,
    mean_conservative_gap: f64,
}

impl DoubleQ {
    pub fn fit(
        feature_width: usize,
        actions: &[u32],
        transitions: &[Transition],
        config: &DoubleQConfig,
    ) -> Result<Self, DoubleQError> {
        Self::fit_internal(feature_width, actions, transitions, config, 0.0, 1.0)
    }

    fn fit_internal(
        feature_width: usize,
        actions: &[u32],
        transitions: &[Transition],
        config: &DoubleQConfig,
        conservative_weight: f64,
        conservative_temperature: f64,
    ) -> Result<Self, DoubleQError> {
        validate(feature_width, actions, transitions, config)?;
        let mut actions = actions.to_vec();
        actions.sort_unstable();
        let (feature_mean, feature_inverse_stddev) = normalization(feature_width, transitions);
        let normalized_states = transitions
            .iter()
            .map(|transition| normalize(&transition.state, &feature_mean, &feature_inverse_stddev))
            .collect::<Vec<_>>();
        let normalized_next_states = transitions
            .iter()
            .map(|transition| {
                normalize(
                    &transition.next_state,
                    &feature_mean,
                    &feature_inverse_stddev,
                )
            })
            .collect::<Vec<_>>();

        let mut rng = DeterministicRng::new(config.seed);
        let mut critic_a =
            Critic::initialized(feature_width, config.hidden_width, actions.len(), &mut rng);
        let mut critic_b =
            Critic::initialized(feature_width, config.hidden_width, actions.len(), &mut rng);
        let mut target_a = critic_a.clone();
        let mut target_b = critic_b.clone();
        let mut order = (0..transitions.len()).collect::<Vec<_>>();
        let mut gradient_updates = 0_u64;
        let mut target_synchronizations = 0_u64;

        for epoch in 0..config.epochs {
            rng.shuffle(&mut order);
            for (position, row) in order.iter().copied().enumerate() {
                let transition = &transitions[row];
                let action_index = actions
                    .binary_search(&transition.action)
                    .expect("transition actions were validated");
                let update_a = (epoch + position) % 2 == 0;
                let target = if transition.terminal {
                    f64::from(transition.reward)
                } else {
                    let selector = if update_a { &critic_a } else { &critic_b };
                    let evaluator = if update_a { &target_b } else { &target_a };
                    let next_action = selector.best_action(&normalized_next_states[row]);
                    f64::from(transition.reward)
                        + config.discount.powf(f64::from(transition.duration))
                            * evaluator.value(&normalized_next_states[row], next_action)
                };
                if !target.is_finite() {
                    return Err(DoubleQError::NonFiniteTarget { epoch, row });
                }
                let critic = if update_a {
                    &mut critic_a
                } else {
                    &mut critic_b
                };
                critic.update(
                    &normalized_states[row],
                    action_index,
                    target,
                    config.learning_rate,
                    config.gradient_clip,
                    conservative_weight,
                    conservative_temperature,
                )?;
                gradient_updates += 1;
                if gradient_updates % config.target_sync_steps as u64 == 0 {
                    target_a = critic_a.clone();
                    target_b = critic_b.clone();
                    target_synchronizations += 1;
                }
            }
        }

        Ok(Self {
            feature_width,
            actions,
            feature_mean,
            feature_inverse_stddev,
            critic_a,
            critic_b,
            gradient_updates,
            target_synchronizations,
        })
    }

    pub fn feature_width(&self) -> usize {
        self.feature_width
    }

    pub fn actions(&self) -> &[u32] {
        &self.actions
    }

    pub fn gradient_updates(&self) -> u64 {
        self.gradient_updates
    }

    pub fn target_synchronizations(&self) -> u64 {
        self.target_synchronizations
    }

    pub fn estimate(&self, state: &[f32], action: u32) -> Result<DoubleQEstimate, DoubleQError> {
        let state = self.normalized_state(state)?;
        let action_index = self
            .actions
            .binary_search(&action)
            .map_err(|_| DoubleQError::UnknownAction(action))?;
        Ok(self.estimate_normalized(&state, action_index))
    }

    pub fn rank_actions(&self, state: &[f32]) -> Result<Vec<DoubleQEstimate>, DoubleQError> {
        let state = self.normalized_state(state)?;
        let mut ranking = self
            .actions
            .iter()
            .enumerate()
            .map(|(action_index, _)| self.estimate_normalized(&state, action_index))
            .collect::<Vec<_>>();
        ranking.sort_by(|left, right| {
            right
                .mean
                .total_cmp(&left.mean)
                .then_with(|| {
                    left.critic_disagreement
                        .total_cmp(&right.critic_disagreement)
                })
                .then_with(|| left.action.cmp(&right.action))
        });
        Ok(ranking)
    }

    pub fn artifact_bytes(
        &self,
        feature_schema: Digest,
        action_schema: Digest,
        training_dataset_sha256: Option<Digest>,
        training_corpus_sha256: &[Digest],
        config: &DoubleQConfig,
    ) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(&DoubleQArtifact {
            schema: DOUBLE_Q_MODEL_SCHEMA_V1,
            feature_schema,
            action_schema,
            training_dataset_sha256,
            training_corpus_sha256,
            config,
            model: self,
        })
    }

    fn normalized_state(&self, state: &[f32]) -> Result<Vec<f64>, DoubleQError> {
        if state.len() != self.feature_width {
            return Err(DoubleQError::FeatureWidth {
                expected: self.feature_width,
                actual: state.len(),
            });
        }
        if state.iter().any(|value| !value.is_finite()) {
            return Err(DoubleQError::NonFiniteFeature);
        }
        Ok(normalize(
            state,
            &self.feature_mean,
            &self.feature_inverse_stddev,
        ))
    }

    fn estimate_normalized(&self, state: &[f64], action_index: usize) -> DoubleQEstimate {
        let critic_a = self.critic_a.value(state, action_index);
        let critic_b = self.critic_b.value(state, action_index);
        DoubleQEstimate {
            action: self.actions[action_index],
            mean: (critic_a + critic_b) * 0.5,
            critic_a,
            critic_b,
            critic_disagreement: (critic_a - critic_b).abs(),
        }
    }
}

impl ConservativeQ {
    pub fn fit(
        feature_width: usize,
        actions: &[u32],
        transitions: &[Transition],
        config: &ConservativeQConfig,
    ) -> Result<Self, DoubleQError> {
        if !config.conservative_weight.is_finite()
            || config.conservative_weight <= 0.0
            || config.conservative_weight > 100.0
        {
            return Err(DoubleQError::InvalidConfig(
                "conservative weight must be finite and within (0, 100]",
            ));
        }
        if !config.temperature.is_finite()
            || config.temperature <= 0.0
            || config.temperature > 100.0
        {
            return Err(DoubleQError::InvalidConfig(
                "CQL temperature must be finite and within (0, 100]",
            ));
        }
        let model = DoubleQ::fit_internal(
            feature_width,
            actions,
            transitions,
            &config.double_q,
            config.conservative_weight,
            config.temperature,
        )?;
        let mut conservative_gap_sum = 0.0;
        for transition in transitions {
            let state = model.normalized_state(&transition.state)?;
            let observed_action = model
                .actions
                .binary_search(&transition.action)
                .expect("transition actions were validated");
            conservative_gap_sum += conservative_gap(
                &model.critic_a.values(&state),
                observed_action,
                config.temperature,
            );
            conservative_gap_sum += conservative_gap(
                &model.critic_b.values(&state),
                observed_action,
                config.temperature,
            );
        }
        let conservative_updates = model.gradient_updates();
        let mean_conservative_gap = conservative_gap_sum / (transitions.len() as f64 * 2.0);
        Ok(Self {
            model,
            conservative_updates,
            mean_conservative_gap,
        })
    }

    pub fn feature_width(&self) -> usize {
        self.model.feature_width()
    }

    pub fn actions(&self) -> &[u32] {
        self.model.actions()
    }

    pub fn gradient_updates(&self) -> u64 {
        self.model.gradient_updates()
    }

    pub fn target_synchronizations(&self) -> u64 {
        self.model.target_synchronizations()
    }

    pub fn conservative_updates(&self) -> u64 {
        self.conservative_updates
    }

    pub fn mean_conservative_gap(&self) -> f64 {
        self.mean_conservative_gap
    }

    pub fn estimate(&self, state: &[f32], action: u32) -> Result<DoubleQEstimate, DoubleQError> {
        self.model.estimate(state, action)
    }

    pub fn rank_actions(&self, state: &[f32]) -> Result<Vec<DoubleQEstimate>, DoubleQError> {
        self.model.rank_actions(state)
    }

    pub fn artifact_bytes(
        &self,
        feature_schema: Digest,
        action_schema: Digest,
        training_dataset_sha256: Option<Digest>,
        training_corpus_sha256: &[Digest],
        config: &ConservativeQConfig,
    ) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(&ConservativeQArtifact {
            schema: CONSERVATIVE_Q_MODEL_SCHEMA_V1,
            feature_schema,
            action_schema,
            training_dataset_sha256,
            training_corpus_sha256,
            config,
            model: self,
        })
    }
}

#[derive(Serialize)]
struct DoubleQArtifact<'a> {
    schema: &'static str,
    feature_schema: Digest,
    action_schema: Digest,
    training_dataset_sha256: Option<Digest>,
    training_corpus_sha256: &'a [Digest],
    config: &'a DoubleQConfig,
    model: &'a DoubleQ,
}

#[derive(Serialize)]
struct ConservativeQArtifact<'a> {
    schema: &'static str,
    feature_schema: Digest,
    action_schema: Digest,
    training_dataset_sha256: Option<Digest>,
    training_corpus_sha256: &'a [Digest],
    config: &'a ConservativeQConfig,
    model: &'a ConservativeQ,
}

#[derive(Clone, Debug, Serialize)]
struct Critic {
    feature_width: usize,
    hidden_width: usize,
    action_count: usize,
    input_weights: Vec<f64>,
    hidden_bias: Vec<f64>,
    output_weights: Vec<f64>,
    output_bias: Vec<f64>,
}

impl Critic {
    fn initialized(
        feature_width: usize,
        hidden_width: usize,
        action_count: usize,
        rng: &mut DeterministicRng,
    ) -> Self {
        let input_scale = (6.0 / (feature_width + hidden_width) as f64).sqrt();
        let output_scale = (6.0 / (hidden_width + action_count) as f64).sqrt();
        Self {
            feature_width,
            hidden_width,
            action_count,
            input_weights: (0..feature_width * hidden_width)
                .map(|_| rng.symmetric(input_scale))
                .collect(),
            hidden_bias: vec![0.0; hidden_width],
            output_weights: (0..hidden_width * action_count)
                .map(|_| rng.symmetric(output_scale))
                .collect(),
            output_bias: vec![0.0; action_count],
        }
    }

    fn hidden(&self, state: &[f64]) -> (Vec<f64>, Vec<bool>) {
        let mut hidden = vec![0.0; self.hidden_width];
        let mut active = vec![false; self.hidden_width];
        for hidden_index in 0..self.hidden_width {
            let mut value = self.hidden_bias[hidden_index];
            let offset = hidden_index * self.feature_width;
            for feature in 0..self.feature_width {
                value += self.input_weights[offset + feature] * state[feature];
            }
            if value > 0.0 {
                hidden[hidden_index] = value;
                active[hidden_index] = true;
            }
        }
        (hidden, active)
    }

    fn value(&self, state: &[f64], action: usize) -> f64 {
        let (hidden, _) = self.hidden(state);
        let offset = action * self.hidden_width;
        self.output_bias[action]
            + hidden
                .iter()
                .enumerate()
                .map(|(index, value)| self.output_weights[offset + index] * value)
                .sum::<f64>()
    }

    fn values(&self, state: &[f64]) -> Vec<f64> {
        let (hidden, _) = self.hidden(state);
        (0..self.action_count)
            .map(|action| {
                let offset = action * self.hidden_width;
                self.output_bias[action]
                    + hidden
                        .iter()
                        .enumerate()
                        .map(|(index, value)| self.output_weights[offset + index] * value)
                        .sum::<f64>()
            })
            .collect()
    }

    fn best_action(&self, state: &[f64]) -> usize {
        self.values(state)
            .into_iter()
            .enumerate()
            .max_by(|(left_index, left), (right_index, right)| {
                left.total_cmp(right)
                    .then_with(|| right_index.cmp(left_index))
            })
            .map(|(index, _)| index)
            .expect("action count is validated as nonzero")
    }

    fn update(
        &mut self,
        state: &[f64],
        action: usize,
        target: f64,
        learning_rate: f64,
        gradient_clip: f64,
        conservative_weight: f64,
        conservative_temperature: f64,
    ) -> Result<(), DoubleQError> {
        let (hidden, active) = self.hidden(state);
        let values = (0..self.action_count)
            .map(|output_action| {
                let offset = output_action * self.hidden_width;
                self.output_bias[output_action]
                    + hidden
                        .iter()
                        .enumerate()
                        .map(|(index, value)| self.output_weights[offset + index] * value)
                        .sum::<f64>()
            })
            .collect::<Vec<_>>();
        let td_error = (values[action] - target).clamp(-gradient_clip, gradient_clip);
        let mut output_gradients = vec![0.0; self.action_count];
        output_gradients[action] = td_error;
        if conservative_weight > 0.0 {
            let maximum = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let exponentials = values
                .iter()
                .map(|value| ((value - maximum) / conservative_temperature).exp())
                .collect::<Vec<_>>();
            let denominator = exponentials.iter().sum::<f64>();
            for output_action in 0..self.action_count {
                let observed = if output_action == action { 1.0 } else { 0.0 };
                output_gradients[output_action] +=
                    conservative_weight * (exponentials[output_action] / denominator - observed);
            }
        }
        let prior_output_weights = self.output_weights.clone();
        for output_action in 0..self.action_count {
            let gradient = output_gradients[output_action].clamp(-gradient_clip, gradient_clip);
            self.output_bias[output_action] -= learning_rate * gradient;
            let output_offset = output_action * self.hidden_width;
            for hidden_index in 0..self.hidden_width {
                self.output_weights[output_offset + hidden_index] -= learning_rate
                    * (gradient * hidden[hidden_index]).clamp(-gradient_clip, gradient_clip);
            }
        }
        for hidden_index in 0..self.hidden_width {
            if active[hidden_index] {
                let hidden_gradient = (0..self.action_count)
                    .map(|output_action| {
                        output_gradients[output_action]
                            * prior_output_weights[output_action * self.hidden_width + hidden_index]
                    })
                    .sum::<f64>()
                    .clamp(-gradient_clip, gradient_clip);
                self.hidden_bias[hidden_index] -= learning_rate * hidden_gradient;
                let input_offset = hidden_index * self.feature_width;
                for feature in 0..self.feature_width {
                    self.input_weights[input_offset + feature] -= learning_rate
                        * (hidden_gradient * state[feature]).clamp(-gradient_clip, gradient_clip);
                }
            }
        }
        if self.output_bias.iter().any(|value| !value.is_finite())
            || self.input_weights.iter().any(|value| !value.is_finite())
            || self.hidden_bias.iter().any(|value| !value.is_finite())
            || self.output_weights.iter().any(|value| !value.is_finite())
        {
            return Err(DoubleQError::Diverged);
        }
        Ok(())
    }
}

fn validate(
    feature_width: usize,
    actions: &[u32],
    transitions: &[Transition],
    config: &DoubleQConfig,
) -> Result<(), DoubleQError> {
    if feature_width == 0 {
        return Err(DoubleQError::EmptyFeatures);
    }
    if actions.is_empty() {
        return Err(DoubleQError::EmptyActions);
    }
    if actions.len() > crate::fqi::MAX_FQI_ACTIONS {
        return Err(DoubleQError::InvalidConfig("action count exceeds 128"));
    }
    let mut sorted_actions = actions.to_vec();
    sorted_actions.sort_unstable();
    if let Some(action) = sorted_actions
        .windows(2)
        .find(|pair| pair[0] == pair[1])
        .map(|pair| pair[0])
    {
        return Err(DoubleQError::DuplicateAction(action));
    }
    if transitions.is_empty() {
        return Err(DoubleQError::EmptyTransitions);
    }
    if transitions.len() > crate::fqi::MAX_FQI_TRANSITIONS {
        return Err(DoubleQError::InvalidConfig(
            "transition count exceeds 250000",
        ));
    }
    if config.epochs == 0 || config.epochs > MAX_DOUBLE_Q_EPOCHS {
        return Err(DoubleQError::InvalidConfig("epochs are outside bounds"));
    }
    if config.hidden_width == 0 || config.hidden_width > MAX_DOUBLE_Q_HIDDEN_WIDTH {
        return Err(DoubleQError::InvalidConfig(
            "hidden width is outside bounds",
        ));
    }
    if config.target_sync_steps == 0 || config.target_sync_steps > MAX_DOUBLE_Q_TARGET_SYNC_STEPS {
        return Err(DoubleQError::InvalidConfig(
            "target synchronization interval is outside bounds",
        ));
    }
    if !config.learning_rate.is_finite() || !(0.0..=1.0).contains(&config.learning_rate) {
        return Err(DoubleQError::InvalidConfig("invalid learning rate"));
    }
    if config.learning_rate == 0.0 {
        return Err(DoubleQError::InvalidConfig("invalid learning rate"));
    }
    if !config.discount.is_finite() || !(0.0..=1.0).contains(&config.discount) {
        return Err(DoubleQError::InvalidConfig("invalid discount"));
    }
    if !config.gradient_clip.is_finite() || config.gradient_clip <= 0.0 {
        return Err(DoubleQError::InvalidConfig("invalid gradient clip"));
    }
    let mut support = vec![0_usize; sorted_actions.len()];
    for transition in transitions {
        if transition.state.len() != feature_width {
            return Err(DoubleQError::FeatureWidth {
                expected: feature_width,
                actual: transition.state.len(),
            });
        }
        if transition.next_state.len() != feature_width {
            return Err(DoubleQError::FeatureWidth {
                expected: feature_width,
                actual: transition.next_state.len(),
            });
        }
        if transition.duration == 0 {
            return Err(DoubleQError::ZeroDuration);
        }
        if !transition.reward.is_finite()
            || transition.state.iter().any(|value| !value.is_finite())
            || transition.next_state.iter().any(|value| !value.is_finite())
        {
            return Err(DoubleQError::NonFiniteFeature);
        }
        let action_index = sorted_actions
            .binary_search(&transition.action)
            .map_err(|_| DoubleQError::UnknownAction(transition.action))?;
        support[action_index] += 1;
    }
    if let Some((index, _)) = support.iter().enumerate().find(|(_, count)| **count == 0) {
        return Err(DoubleQError::MissingActionSamples(sorted_actions[index]));
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
        .map(|value| {
            let stddev = value.sqrt();
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
        .map(|((value, mean), inverse_stddev)| (f64::from(*value) - mean) * inverse_stddev)
        .collect()
}

fn conservative_gap(values: &[f64], observed_action: usize, temperature: f64) -> f64 {
    let maximum = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let denominator = values
        .iter()
        .map(|value| ((value - maximum) / temperature).exp())
        .sum::<f64>();
    maximum + temperature * denominator.ln() - values[observed_action]
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DoubleQError {
    EmptyFeatures,
    EmptyActions,
    DuplicateAction(u32),
    MissingActionSamples(u32),
    UnknownAction(u32),
    EmptyTransitions,
    FeatureWidth { expected: usize, actual: usize },
    ZeroDuration,
    NonFiniteFeature,
    InvalidConfig(&'static str),
    NonFiniteTarget { epoch: usize, row: usize },
    Diverged,
}

impl fmt::Display for DoubleQError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyFeatures => formatter.write_str("Double-Q requires at least one feature"),
            Self::EmptyActions => formatter.write_str("Double-Q requires at least one action"),
            Self::DuplicateAction(action) => write!(formatter, "duplicate action {action}"),
            Self::MissingActionSamples(action) => {
                write!(formatter, "action {action} has no training samples")
            }
            Self::UnknownAction(action) => write!(formatter, "unknown action {action}"),
            Self::EmptyTransitions => formatter.write_str("Double-Q requires transitions"),
            Self::FeatureWidth { expected, actual } => {
                write!(
                    formatter,
                    "feature width {actual} does not match {expected}"
                )
            }
            Self::ZeroDuration => formatter.write_str("transition duration must be nonzero"),
            Self::NonFiniteFeature => formatter.write_str("features and rewards must be finite"),
            Self::InvalidConfig(message) => write!(formatter, "invalid Double-Q config: {message}"),
            Self::NonFiniteTarget { epoch, row } => {
                write!(
                    formatter,
                    "non-finite Double-Q target at epoch {epoch}, row {row}"
                )
            }
            Self::Diverged => formatter.write_str("Double-Q critic diverged to non-finite weights"),
        }
    }
}

impl Error for DoubleQError {}

struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
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

    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / ((1_u64 << 53) as f64))
    }

    fn symmetric(&mut self, scale: f64) -> f64 {
        (self.unit() * 2.0 - 1.0) * scale
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

    fn transition(state: f32, action: u32, reward: f32, terminal: bool) -> Transition {
        Transition {
            state: vec![state],
            action,
            duration: 1,
            reward,
            next_state: vec![state + 1.0],
            terminal,
        }
    }

    #[test]
    fn seeded_training_is_byte_deterministic_and_learns_terminal_preference() {
        let transitions = vec![
            transition(0.0, WAIT, -1.0, true),
            transition(0.0, ADVANCE, 3.0, true),
            transition(1.0, WAIT, -1.0, true),
            transition(1.0, ADVANCE, 3.0, true),
        ];
        let config = DoubleQConfig {
            epochs: 256,
            hidden_width: 8,
            learning_rate: 0.01,
            target_sync_steps: 3,
            ..DoubleQConfig::default()
        };
        let first = DoubleQ::fit(1, &[WAIT, ADVANCE], &transitions, &config).unwrap();
        let second = DoubleQ::fit(1, &[WAIT, ADVANCE], &transitions, &config).unwrap();
        assert_eq!(
            serde_json::to_vec(&first).unwrap(),
            serde_json::to_vec(&second).unwrap()
        );
        assert_eq!(first.rank_actions(&[0.5]).unwrap()[0].action, ADVANCE);
        assert_eq!(first.gradient_updates(), 1024);
        assert_eq!(first.target_synchronizations(), 341);
    }

    #[test]
    fn target_sync_and_duration_configuration_are_bounded() {
        let transitions = vec![transition(0.0, WAIT, 0.0, false)];
        let invalid = DoubleQConfig {
            target_sync_steps: 0,
            ..DoubleQConfig::default()
        };
        assert!(matches!(
            DoubleQ::fit(1, &[WAIT], &transitions, &invalid),
            Err(DoubleQError::InvalidConfig(_))
        ));
        let zero_duration = vec![Transition {
            duration: 0,
            ..transitions[0].clone()
        }];
        assert_eq!(
            DoubleQ::fit(1, &[WAIT], &zero_duration, &DoubleQConfig::default()).unwrap_err(),
            DoubleQError::ZeroDuration
        );
    }

    #[test]
    fn conservative_penalty_separates_state_local_unsupported_actions() {
        let mut transitions = Vec::new();
        for _ in 0..16 {
            transitions.push(transition(-1.0, WAIT, 0.0, true));
            transitions.push(transition(1.0, ADVANCE, 0.0, true));
        }
        let base = DoubleQConfig {
            epochs: 128,
            hidden_width: 8,
            learning_rate: 0.01,
            target_sync_steps: 32,
            seed: 11,
            ..DoubleQConfig::default()
        };
        let ordinary = DoubleQ::fit(1, &[WAIT, ADVANCE], &transitions, &base).unwrap();
        let conservative = ConservativeQ::fit(
            1,
            &[WAIT, ADVANCE],
            &transitions,
            &ConservativeQConfig {
                double_q: base,
                conservative_weight: 1.0,
                temperature: 1.0,
            },
        )
        .unwrap();
        let ordinary_gap = ordinary.estimate(&[-1.0], WAIT).unwrap().mean
            - ordinary.estimate(&[-1.0], ADVANCE).unwrap().mean;
        let conservative_gap = conservative.estimate(&[-1.0], WAIT).unwrap().mean
            - conservative.estimate(&[-1.0], ADVANCE).unwrap().mean;
        assert!(conservative_gap > ordinary_gap + 0.5);
        assert_eq!(conservative.rank_actions(&[-1.0]).unwrap()[0].action, WAIT);
    }
}
