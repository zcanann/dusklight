//! Isolated Double-Q component variants for controlled ablation studies.
//!
//! A model selects exactly one component. Combined Rainbow-style configurations
//! are intentionally not representable here.

use super::{
    Critic, DeterministicRng, DoubleQConfig, DoubleQError, DoubleQEstimate, normalization,
    normalize, validate,
};
use crate::fqi::Transition;
use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QComponent {
    Baseline,
    DuelingHead,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct QComponentConfig {
    pub critic: DoubleQConfig,
    pub component: QComponent,
}

impl Default for QComponentConfig {
    fn default() -> Self {
        Self {
            critic: DoubleQConfig::default(),
            component: QComponent::Baseline,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct QComponentModel {
    feature_width: usize,
    actions: Vec<u32>,
    feature_mean: Vec<f64>,
    feature_inverse_stddev: Vec<f64>,
    component: QComponent,
    critic_a: ComponentCritic,
    critic_b: ComponentCritic,
    gradient_updates: u64,
    target_synchronizations: u64,
    parameters_per_critic: usize,
}

impl QComponentModel {
    pub fn fit(
        feature_width: usize,
        actions: &[u32],
        transitions: &[Transition],
        episode_groups: &[u64],
        config: &QComponentConfig,
    ) -> Result<Self, DoubleQError> {
        validate(feature_width, actions, transitions, &config.critic)?;
        if episode_groups.len() != transitions.len() {
            return Err(DoubleQError::InvalidConfig(
                "episode-group count must match transition count",
            ));
        }
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
        let mut rng = DeterministicRng::new(config.critic.seed);
        let mut critic_a = ComponentCritic::initialized(
            config.component,
            feature_width,
            config.critic.hidden_width,
            actions.len(),
            &mut rng,
        );
        let mut critic_b = ComponentCritic::initialized(
            config.component,
            feature_width,
            config.critic.hidden_width,
            actions.len(),
            &mut rng,
        );
        let parameters_per_critic = critic_a.parameter_count();
        let mut target_a = critic_a.clone();
        let mut target_b = critic_b.clone();
        let mut order = (0..transitions.len()).collect::<Vec<_>>();
        let mut gradient_updates = 0_u64;
        let mut target_synchronizations = 0_u64;

        for epoch in 0..config.critic.epochs {
            rng.shuffle(&mut order);
            for (position, row) in order.iter().copied().enumerate() {
                let transition = &transitions[row];
                let action = actions
                    .binary_search(&transition.action)
                    .expect("transition actions were validated");
                let update_a = (epoch + position) % 2 == 0;
                let target = if transition.terminal {
                    f64::from(transition.reward)
                } else {
                    let selector = if update_a { &critic_a } else { &critic_b };
                    let evaluator = if update_a { &target_b } else { &target_a };
                    let next_action = selector.best_action(&next_states[row]);
                    f64::from(transition.reward)
                        + config.critic.discount.powf(f64::from(transition.duration))
                            * evaluator.value(&next_states[row], next_action)
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
                    &states[row],
                    action,
                    target,
                    config.critic.learning_rate,
                    config.critic.gradient_clip,
                )?;
                gradient_updates += 1;
                if gradient_updates % config.critic.target_sync_steps as u64 == 0 {
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
            component: config.component,
            critic_a,
            critic_b,
            gradient_updates,
            target_synchronizations,
            parameters_per_critic,
        })
    }

    pub fn component(&self) -> QComponent {
        self.component
    }

    pub fn gradient_updates(&self) -> u64 {
        self.gradient_updates
    }

    pub fn target_synchronizations(&self) -> u64 {
        self.target_synchronizations
    }

    pub fn parameters_per_critic(&self) -> usize {
        self.parameters_per_critic
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
            .map(|(action, _)| self.estimate_normalized(&state, action))
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

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "architecture", rename_all = "snake_case")]
enum ComponentCritic {
    Standard(Critic),
    Dueling(DuelingCritic),
}

impl ComponentCritic {
    fn initialized(
        component: QComponent,
        feature_width: usize,
        hidden_width: usize,
        action_count: usize,
        rng: &mut DeterministicRng,
    ) -> Self {
        match component {
            QComponent::Baseline => Self::Standard(Critic::initialized(
                feature_width,
                hidden_width,
                action_count,
                rng,
            )),
            QComponent::DuelingHead => Self::Dueling(DuelingCritic::initialized(
                feature_width,
                hidden_width,
                action_count,
                rng,
            )),
        }
    }

    fn value(&self, state: &[f64], action: usize) -> f64 {
        match self {
            Self::Standard(critic) => critic.value(state, action),
            Self::Dueling(critic) => critic.value(state, action),
        }
    }

    fn best_action(&self, state: &[f64]) -> usize {
        match self {
            Self::Standard(critic) => critic.best_action(state),
            Self::Dueling(critic) => critic.best_action(state),
        }
    }

    fn update(
        &mut self,
        state: &[f64],
        action: usize,
        target: f64,
        learning_rate: f64,
        gradient_clip: f64,
    ) -> Result<(), DoubleQError> {
        match self {
            Self::Standard(critic) => critic.update(
                state,
                action,
                target,
                learning_rate,
                gradient_clip,
                0.0,
                1.0,
            ),
            Self::Dueling(critic) => {
                critic.update(state, action, target, learning_rate, gradient_clip)
            }
        }
    }

    fn parameter_count(&self) -> usize {
        match self {
            Self::Standard(critic) => {
                critic.input_weights.len()
                    + critic.hidden_bias.len()
                    + critic.output_weights.len()
                    + critic.output_bias.len()
            }
            Self::Dueling(critic) => critic.parameter_count(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct DuelingCritic {
    feature_width: usize,
    hidden_width: usize,
    action_count: usize,
    input_weights: Vec<f64>,
    hidden_bias: Vec<f64>,
    value_weights: Vec<f64>,
    value_bias: f64,
    advantage_weights: Vec<f64>,
    advantage_bias: Vec<f64>,
}

impl DuelingCritic {
    fn initialized(
        feature_width: usize,
        hidden_width: usize,
        action_count: usize,
        rng: &mut DeterministicRng,
    ) -> Self {
        let input_scale = (6.0 / (feature_width + hidden_width) as f64).sqrt();
        let output_scale = (6.0 / (hidden_width + action_count + 1) as f64).sqrt();
        Self {
            feature_width,
            hidden_width,
            action_count,
            input_weights: (0..feature_width * hidden_width)
                .map(|_| rng.symmetric(input_scale))
                .collect(),
            hidden_bias: vec![0.0; hidden_width],
            value_weights: (0..hidden_width)
                .map(|_| rng.symmetric(output_scale))
                .collect(),
            value_bias: 0.0,
            advantage_weights: (0..hidden_width * action_count)
                .map(|_| rng.symmetric(output_scale))
                .collect(),
            advantage_bias: vec![0.0; action_count],
        }
    }

    fn hidden(&self, state: &[f64]) -> (Vec<f64>, Vec<bool>) {
        let mut hidden = vec![0.0; self.hidden_width];
        let mut active = vec![false; self.hidden_width];
        for hidden_index in 0..self.hidden_width {
            let offset = hidden_index * self.feature_width;
            let mut value = self.hidden_bias[hidden_index];
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

    fn values_from_hidden(&self, hidden: &[f64]) -> Vec<f64> {
        let value = self.value_bias
            + self
                .value_weights
                .iter()
                .zip(hidden)
                .map(|(weight, hidden)| weight * hidden)
                .sum::<f64>();
        let advantages = (0..self.action_count)
            .map(|action| {
                let offset = action * self.hidden_width;
                self.advantage_bias[action]
                    + hidden
                        .iter()
                        .enumerate()
                        .map(|(index, hidden)| self.advantage_weights[offset + index] * hidden)
                        .sum::<f64>()
            })
            .collect::<Vec<_>>();
        let mean_advantage = advantages.iter().sum::<f64>() / self.action_count as f64;
        advantages
            .into_iter()
            .map(|advantage| value + advantage - mean_advantage)
            .collect()
    }

    fn value(&self, state: &[f64], action: usize) -> f64 {
        let (hidden, _) = self.hidden(state);
        self.values_from_hidden(&hidden)[action]
    }

    fn best_action(&self, state: &[f64]) -> usize {
        let (hidden, _) = self.hidden(state);
        self.values_from_hidden(&hidden)
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
    ) -> Result<(), DoubleQError> {
        let (hidden, active) = self.hidden(state);
        let values = self.values_from_hidden(&hidden);
        let error = (values[action] - target).clamp(-gradient_clip, gradient_clip);
        let prior_value_weights = self.value_weights.clone();
        let prior_advantage_weights = self.advantage_weights.clone();
        let inverse_actions = 1.0 / self.action_count as f64;
        let advantage_gradients = (0..self.action_count)
            .map(|output_action| {
                error
                    * if output_action == action {
                        1.0 - inverse_actions
                    } else {
                        -inverse_actions
                    }
            })
            .collect::<Vec<_>>();

        self.value_bias -= learning_rate * error;
        for hidden_index in 0..self.hidden_width {
            self.value_weights[hidden_index] -=
                learning_rate * (error * hidden[hidden_index]).clamp(-gradient_clip, gradient_clip);
        }
        for (output_action, gradient) in advantage_gradients.iter().copied().enumerate() {
            let gradient = gradient.clamp(-gradient_clip, gradient_clip);
            self.advantage_bias[output_action] -= learning_rate * gradient;
            let offset = output_action * self.hidden_width;
            for hidden_index in 0..self.hidden_width {
                self.advantage_weights[offset + hidden_index] -= learning_rate
                    * (gradient * hidden[hidden_index]).clamp(-gradient_clip, gradient_clip);
            }
        }
        for hidden_index in 0..self.hidden_width {
            if active[hidden_index] {
                let advantage_gradient = (0..self.action_count)
                    .map(|output_action| {
                        advantage_gradients[output_action]
                            * prior_advantage_weights
                                [output_action * self.hidden_width + hidden_index]
                    })
                    .sum::<f64>();
                let hidden_gradient = (error * prior_value_weights[hidden_index]
                    + advantage_gradient)
                    .clamp(-gradient_clip, gradient_clip);
                self.hidden_bias[hidden_index] -= learning_rate * hidden_gradient;
                let offset = hidden_index * self.feature_width;
                for feature in 0..self.feature_width {
                    self.input_weights[offset + feature] -= learning_rate
                        * (hidden_gradient * state[feature]).clamp(-gradient_clip, gradient_clip);
                }
            }
        }
        if !self.value_bias.is_finite()
            || self.input_weights.iter().any(|value| !value.is_finite())
            || self.hidden_bias.iter().any(|value| !value.is_finite())
            || self.value_weights.iter().any(|value| !value.is_finite())
            || self
                .advantage_weights
                .iter()
                .any(|value| !value.is_finite())
            || self.advantage_bias.iter().any(|value| !value.is_finite())
        {
            return Err(DoubleQError::Diverged);
        }
        Ok(())
    }

    fn parameter_count(&self) -> usize {
        self.input_weights.len()
            + self.hidden_bias.len()
            + self.value_weights.len()
            + 1
            + self.advantage_weights.len()
            + self.advantage_bias.len()
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
            duration: 1,
            reward,
            next_state: vec![state + 1.0],
            terminal: true,
        }
    }

    fn fixture() -> Vec<Transition> {
        vec![
            transition(0.0, WAIT, -1.0),
            transition(0.0, ADVANCE, 3.0),
            transition(1.0, WAIT, -1.0),
            transition(1.0, ADVANCE, 3.0),
        ]
    }

    fn config(component: QComponent) -> QComponentConfig {
        QComponentConfig {
            critic: DoubleQConfig {
                epochs: 256,
                hidden_width: 8,
                learning_rate: 0.01,
                target_sync_steps: 3,
                seed: 7,
                ..DoubleQConfig::default()
            },
            component,
        }
    }

    #[test]
    fn baseline_is_exactly_the_existing_double_q_training_path() {
        let transitions = fixture();
        let existing = super::super::DoubleQ::fit(
            1,
            &[WAIT, ADVANCE],
            &transitions,
            &config(QComponent::Baseline).critic,
        )
        .unwrap();
        let ablation = QComponentModel::fit(
            1,
            &[WAIT, ADVANCE],
            &transitions,
            &[0, 0, 1, 1],
            &config(QComponent::Baseline),
        )
        .unwrap();
        assert_eq!(
            existing.rank_actions(&[0.5]).unwrap(),
            ablation.rank_actions(&[0.5]).unwrap()
        );
        assert_eq!(existing.gradient_updates(), ablation.gradient_updates());
        assert_eq!(
            existing.target_synchronizations(),
            ablation.target_synchronizations()
        );
    }

    #[test]
    fn dueling_head_is_seeded_and_learns_the_terminal_preference() {
        let transitions = fixture();
        let config = config(QComponent::DuelingHead);
        let first = QComponentModel::fit(1, &[WAIT, ADVANCE], &transitions, &[0, 0, 1, 1], &config)
            .unwrap();
        let second =
            QComponentModel::fit(1, &[WAIT, ADVANCE], &transitions, &[0, 0, 1, 1], &config)
                .unwrap();
        assert_eq!(
            serde_json::to_vec(&first).unwrap(),
            serde_json::to_vec(&second).unwrap()
        );
        assert_eq!(first.rank_actions(&[0.5]).unwrap()[0].action, ADVANCE);
        assert_eq!(first.gradient_updates(), 1024);
        assert!(first.parameters_per_critic() > 0);
    }
}
