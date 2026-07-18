//! Deterministic prioritized replay for the twin Double-Q critic.

use super::{
    Critic, DeterministicRng, DoubleQ, DoubleQConfig, DoubleQError, normalization, normalize,
    validate,
};
use crate::artifact::Digest;
use crate::fqi::Transition;
use serde::Serialize;
use std::error::Error;
use std::fmt;

pub const PRIORITIZED_DOUBLE_Q_SCHEMA_V1: &str = "dusklight-prioritized-double-q-model/v1";

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PrioritizedDoubleQConfig {
    pub critic: DoubleQConfig,
    pub priority_exponent: f64,
    pub importance_exponent_start: f64,
    pub importance_exponent_end: f64,
    pub priority_epsilon: f64,
    pub importance_weight_cap: f64,
    pub replay_seed: u64,
}

impl Default for PrioritizedDoubleQConfig {
    fn default() -> Self {
        Self {
            critic: DoubleQConfig::default(),
            priority_exponent: 0.6,
            importance_exponent_start: 0.4,
            importance_exponent_end: 1.0,
            priority_epsilon: 1e-4,
            importance_weight_cap: 1.0,
            replay_seed: 0x9e91_a7ed_5eed_0001,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReplayDiagnostics {
    pub total_samples: u64,
    pub unique_rows_sampled: usize,
    pub effective_sample_size: f64,
    pub final_priority_minimum: f64,
    pub final_priority_mean: f64,
    pub final_priority_maximum: f64,
    pub mean_importance_weight: f64,
    pub maximum_importance_weight: f64,
    pub clipped_importance_weights: u64,
    pub final_importance_exponent: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct PrioritizedDoubleQ {
    model: DoubleQ,
    replay: ReplayDiagnostics,
    final_priorities: Vec<f64>,
    row_sample_counts: Vec<u64>,
}

impl PrioritizedDoubleQ {
    pub fn fit(
        feature_width: usize,
        actions: &[u32],
        transitions: &[Transition],
        config: &PrioritizedDoubleQConfig,
    ) -> Result<Self, PrioritizedReplayError> {
        validate(feature_width, actions, transitions, &config.critic)?;
        validate_replay(config)?;
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
        let mut initialization_rng = DeterministicRng::new(config.critic.seed);
        let mut critic_a = Critic::initialized(
            feature_width,
            config.critic.hidden_width,
            actions.len(),
            &mut initialization_rng,
        );
        let mut critic_b = Critic::initialized(
            feature_width,
            config.critic.hidden_width,
            actions.len(),
            &mut initialization_rng,
        );
        let mut target_a = critic_a.clone();
        let mut target_b = critic_b.clone();
        let mut replay_rng = Rng::new(config.replay_seed);
        let mut priorities = vec![1.0_f64; transitions.len()];
        let mut tree = Fenwick::new(transitions.len());
        for row in 0..transitions.len() {
            tree.set(row, 1.0);
        }
        let total_updates = config
            .critic
            .epochs
            .checked_mul(transitions.len())
            .ok_or(PrioritizedReplayError::WorkOverflow)?;
        let mut row_sample_counts = vec![0_u64; transitions.len()];
        let mut weight_sum = 0.0;
        let mut weight_square_sum = 0.0;
        let mut maximum_importance_weight = 0.0_f64;
        let mut clipped_importance_weights = 0_u64;
        let mut target_synchronizations = 0_u64;

        for update in 0..total_updates {
            let progress = if total_updates <= 1 {
                1.0
            } else {
                update as f64 / (total_updates - 1) as f64
            };
            let beta = config.importance_exponent_start
                + progress * (config.importance_exponent_end - config.importance_exponent_start);
            let total_priority = tree.total();
            let row = tree.sample(replay_rng.unit() * total_priority);
            let probability = tree.value(row) / total_priority;
            let raw_weight = (transitions.len() as f64 * probability).powf(-beta);
            let importance_weight = raw_weight.min(config.importance_weight_cap);
            if raw_weight > config.importance_weight_cap {
                clipped_importance_weights += 1;
            }
            maximum_importance_weight = maximum_importance_weight.max(importance_weight);
            weight_sum += importance_weight;
            weight_square_sum += importance_weight * importance_weight;
            row_sample_counts[row] += 1;

            let transition = &transitions[row];
            let action = actions
                .binary_search(&transition.action)
                .expect("transition actions were validated");
            let update_a = update % 2 == 0;
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
                return Err(PrioritizedReplayError::NonFiniteTarget { update, row });
            }
            let critic = if update_a {
                &mut critic_a
            } else {
                &mut critic_b
            };
            let absolute_td_error = (critic.value(&states[row], action) - target).abs();
            critic.update(
                &states[row],
                action,
                target,
                config.critic.learning_rate * importance_weight,
                config.critic.gradient_clip,
                0.0,
                1.0,
            )?;
            priorities[row] = absolute_td_error + config.priority_epsilon;
            tree.set(row, priorities[row].powf(config.priority_exponent));
            if (update + 1) % config.critic.target_sync_steps == 0 {
                target_a = critic_a.clone();
                target_b = critic_b.clone();
                target_synchronizations += 1;
            }
        }

        let total_samples = total_updates as u64;
        let priority_sum = priorities.iter().sum::<f64>();
        let model = DoubleQ {
            feature_width,
            actions,
            feature_mean,
            feature_inverse_stddev,
            critic_a,
            critic_b,
            gradient_updates: total_samples,
            target_synchronizations,
        };
        Ok(Self {
            model,
            replay: ReplayDiagnostics {
                total_samples,
                unique_rows_sampled: row_sample_counts.iter().filter(|count| **count > 0).count(),
                effective_sample_size: if weight_square_sum > 0.0 {
                    weight_sum * weight_sum / weight_square_sum
                } else {
                    0.0
                },
                final_priority_minimum: priorities.iter().copied().fold(f64::INFINITY, f64::min),
                final_priority_mean: priority_sum / priorities.len() as f64,
                final_priority_maximum: priorities.iter().copied().fold(0.0, f64::max),
                mean_importance_weight: weight_sum / total_samples as f64,
                maximum_importance_weight,
                clipped_importance_weights,
                final_importance_exponent: config.importance_exponent_end,
            },
            final_priorities: priorities,
            row_sample_counts,
        })
    }

    pub fn rank_actions(
        &self,
        state: &[f32],
    ) -> Result<Vec<super::DoubleQEstimate>, PrioritizedReplayError> {
        Ok(self.model.rank_actions(state)?)
    }

    pub fn diagnostics(&self) -> &ReplayDiagnostics {
        &self.replay
    }

    pub fn row_sample_counts(&self) -> &[u64] {
        &self.row_sample_counts
    }

    pub fn artifact_bytes(
        &self,
        feature_schema: Digest,
        action_schema: Digest,
        training_dataset_sha256: Option<Digest>,
        training_corpus_sha256: &[Digest],
        config: &PrioritizedDoubleQConfig,
    ) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(&Artifact {
            schema: PRIORITIZED_DOUBLE_Q_SCHEMA_V1,
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
struct Artifact<'a> {
    schema: &'static str,
    feature_schema: Digest,
    action_schema: Digest,
    training_dataset_sha256: Option<Digest>,
    training_corpus_sha256: &'a [Digest],
    config: &'a PrioritizedDoubleQConfig,
    model: &'a PrioritizedDoubleQ,
}

#[derive(Debug)]
pub enum PrioritizedReplayError {
    DoubleQ(DoubleQError),
    InvalidConfig(&'static str),
    WorkOverflow,
    NonFiniteTarget { update: usize, row: usize },
}

impl fmt::Display for PrioritizedReplayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DoubleQ(error) => write!(formatter, "prioritized critic: {error}"),
            Self::InvalidConfig(message) => {
                write!(formatter, "invalid prioritized replay config: {message}")
            }
            Self::WorkOverflow => formatter.write_str("prioritized replay work overflowed"),
            Self::NonFiniteTarget { update, row } => {
                write!(
                    formatter,
                    "non-finite replay target at update {update}, row {row}"
                )
            }
        }
    }
}

impl Error for PrioritizedReplayError {}

impl From<DoubleQError> for PrioritizedReplayError {
    fn from(error: DoubleQError) -> Self {
        Self::DoubleQ(error)
    }
}

fn validate_replay(config: &PrioritizedDoubleQConfig) -> Result<(), PrioritizedReplayError> {
    if !config.priority_exponent.is_finite()
        || !(0.0..=1.0).contains(&config.priority_exponent)
        || !config.importance_exponent_start.is_finite()
        || !config.importance_exponent_end.is_finite()
        || !(0.0..=1.0).contains(&config.importance_exponent_start)
        || !(config.importance_exponent_start..=1.0).contains(&config.importance_exponent_end)
        || !config.priority_epsilon.is_finite()
        || config.priority_epsilon <= 0.0
        || !config.importance_weight_cap.is_finite()
        || config.importance_weight_cap <= 0.0
        || config.importance_weight_cap > 10.0
    {
        return Err(PrioritizedReplayError::InvalidConfig(
            "alpha/beta, epsilon, or importance cap is outside bounds",
        ));
    }
    Ok(())
}

struct Fenwick {
    tree: Vec<f64>,
    values: Vec<f64>,
}

impl Fenwick {
    fn new(length: usize) -> Self {
        Self {
            tree: vec![0.0; length + 1],
            values: vec![0.0; length],
        }
    }

    fn set(&mut self, index: usize, value: f64) {
        let delta = value - self.values[index];
        self.values[index] = value;
        let mut cursor = index + 1;
        while cursor < self.tree.len() {
            self.tree[cursor] += delta;
            cursor += cursor & cursor.wrapping_neg();
        }
    }

    fn value(&self, index: usize) -> f64 {
        self.values[index]
    }

    fn total(&self) -> f64 {
        let mut total = 0.0;
        let mut cursor = self.values.len();
        while cursor > 0 {
            total += self.tree[cursor];
            cursor &= cursor - 1;
        }
        total
    }

    fn sample(&self, target: f64) -> usize {
        let mut index = 0_usize;
        let mut accumulated = 0.0;
        let mut bit = 1_usize;
        while bit << 1 < self.tree.len() {
            bit <<= 1;
        }
        while bit > 0 {
            let next = index + bit;
            if next < self.tree.len() && accumulated + self.tree[next] <= target {
                index = next;
                accumulated += self.tree[next];
            }
            bit >>= 1;
        }
        index.min(self.values.len() - 1)
    }
}

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

    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1_u64 << 53) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transition(state: f32, action: u32, reward: f32) -> Transition {
        Transition {
            state: vec![state],
            action,
            duration: 1,
            reward,
            next_state: vec![state],
            terminal: true,
        }
    }

    #[test]
    fn replay_is_seeded_prioritized_bounded_and_diagnostic() {
        let transitions = vec![
            transition(0.0, 0, 0.0),
            transition(1.0, 0, 0.0),
            transition(2.0, 1, 0.0),
            transition(3.0, 1, 20.0),
        ];
        let config = PrioritizedDoubleQConfig {
            critic: DoubleQConfig {
                epochs: 64,
                hidden_width: 8,
                learning_rate: 0.01,
                target_sync_steps: 8,
                seed: 3,
                ..DoubleQConfig::default()
            },
            importance_weight_cap: 0.75,
            replay_seed: 5,
            ..PrioritizedDoubleQConfig::default()
        };
        let first = PrioritizedDoubleQ::fit(1, &[0, 1], &transitions, &config).unwrap();
        let second = PrioritizedDoubleQ::fit(1, &[0, 1], &transitions, &config).unwrap();
        assert_eq!(
            serde_json::to_vec(&first).unwrap(),
            serde_json::to_vec(&second).unwrap()
        );
        assert_eq!(first.diagnostics().total_samples, 256);
        assert!(first.diagnostics().maximum_importance_weight <= 0.75);
        assert!(first.diagnostics().clipped_importance_weights > 0);
        assert!(first.diagnostics().effective_sample_size > 0.0);
        assert!(first.row_sample_counts()[3] > first.row_sample_counts()[0]);
        assert_eq!(first.rank_actions(&[3.0]).unwrap().len(), 2);
    }

    #[test]
    fn fenwick_sampling_and_config_bounds_fail_closed() {
        let mut tree = Fenwick::new(3);
        tree.set(0, 1.0);
        tree.set(1, 2.0);
        tree.set(2, 3.0);
        assert_eq!(tree.total(), 6.0);
        assert_eq!(tree.sample(0.0), 0);
        assert_eq!(tree.sample(1.0), 1);
        assert_eq!(tree.sample(5.9), 2);
        let invalid = PrioritizedDoubleQConfig {
            importance_exponent_start: 0.8,
            importance_exponent_end: 0.7,
            ..PrioritizedDoubleQConfig::default()
        };
        assert!(matches!(
            PrioritizedDoubleQ::fit(1, &[0], &[transition(0.0, 0, 0.0)], &invalid),
            Err(PrioritizedReplayError::InvalidConfig(_))
        ));
    }
}
