//! Deterministic bounded prioritized replay with importance diagnostics.

use serde::Serialize;
use std::error::Error;
use std::fmt;

pub const MAX_REPLAY_ENTRIES: usize = 250_000;
pub const MAX_REPLAY_PRIORITY: f64 = 1_000_000.0;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PrioritizedReplayConfig {
    pub alpha: f64,
    pub beta: f64,
    pub priority_epsilon: f64,
    pub maximum_importance_weight: f64,
}

impl Default for PrioritizedReplayConfig {
    fn default() -> Self {
        Self {
            alpha: 0.6,
            beta: 0.4,
            priority_epsilon: 1e-3,
            maximum_importance_weight: 10.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ReplaySample {
    pub index: usize,
    pub probability: f64,
    pub importance_weight: f64,
    pub importance_weight_clipped: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PrioritizedReplayDiagnostics {
    pub entries: usize,
    pub samples: u64,
    pub priority_updates: u64,
    pub clipped_importance_weights: u64,
    pub minimum_priority: f64,
    pub maximum_priority: f64,
    pub maximum_observed_importance_weight: f64,
    pub mean_importance_weight: Option<f64>,
    pub effective_sample_size: Option<f64>,
    pub priority_entropy: f64,
}

pub struct PrioritizedReplay {
    config: PrioritizedReplayConfig,
    priorities: Vec<f64>,
    scaled: Vec<f64>,
    tree: FenwickTree,
    rng: Rng,
    samples: u64,
    priority_updates: u64,
    clipped_importance_weights: u64,
    maximum_observed_importance_weight: f64,
    importance_weight_sum: f64,
    importance_weight_square_sum: f64,
}

impl PrioritizedReplay {
    pub fn new(
        initial_priorities: &[f64],
        config: PrioritizedReplayConfig,
        seed: u64,
    ) -> Result<Self, PrioritizedReplayError> {
        validate_config(&config)?;
        if initial_priorities.is_empty() || initial_priorities.len() > MAX_REPLAY_ENTRIES {
            return Err(PrioritizedReplayError::new("invalid replay entry count"));
        }
        let priorities = initial_priorities
            .iter()
            .map(|priority| bounded_priority(*priority, &config))
            .collect::<Result<Vec<_>, _>>()?;
        let scaled = priorities
            .iter()
            .map(|priority| priority.powf(config.alpha))
            .collect::<Vec<_>>();
        let tree = FenwickTree::new(&scaled);
        if !tree.total().is_finite() || tree.total() <= 0.0 {
            return Err(PrioritizedReplayError::new("invalid replay priority mass"));
        }
        Ok(Self {
            config,
            priorities,
            scaled,
            tree,
            rng: Rng::new(seed),
            samples: 0,
            priority_updates: 0,
            clipped_importance_weights: 0,
            maximum_observed_importance_weight: 0.0,
            importance_weight_sum: 0.0,
            importance_weight_square_sum: 0.0,
        })
    }

    pub fn sample(&mut self) -> Result<ReplaySample, PrioritizedReplayError> {
        let total = self.tree.total();
        if !total.is_finite() || total <= 0.0 {
            return Err(PrioritizedReplayError::new(
                "replay priority mass is invalid",
            ));
        }
        let target = self.rng.unit() * total;
        let index = self.tree.find_prefix(target);
        let probability = self.scaled[index] / total;
        let raw_importance = (self.priorities.len() as f64 * probability).powf(-self.config.beta);
        let importance_weight = raw_importance.min(self.config.maximum_importance_weight);
        let clipped = raw_importance > self.config.maximum_importance_weight;
        if !probability.is_finite() || probability <= 0.0 || !importance_weight.is_finite() {
            return Err(PrioritizedReplayError::new("replay sample is non-finite"));
        }
        self.samples += 1;
        self.clipped_importance_weights += u64::from(clipped);
        self.maximum_observed_importance_weight = self
            .maximum_observed_importance_weight
            .max(importance_weight);
        self.importance_weight_sum += importance_weight;
        self.importance_weight_square_sum += importance_weight * importance_weight;
        Ok(ReplaySample {
            index,
            probability,
            importance_weight,
            importance_weight_clipped: clipped,
        })
    }

    pub fn update_td_error(
        &mut self,
        index: usize,
        absolute_td_error: f64,
    ) -> Result<(), PrioritizedReplayError> {
        if index >= self.priorities.len()
            || !absolute_td_error.is_finite()
            || absolute_td_error < 0.0
        {
            return Err(PrioritizedReplayError::new(
                "invalid replay priority update",
            ));
        }
        let priority = bounded_priority(absolute_td_error, &self.config)?;
        let scaled = priority.powf(self.config.alpha);
        self.tree.add(index, scaled - self.scaled[index]);
        self.priorities[index] = priority;
        self.scaled[index] = scaled;
        self.priority_updates += 1;
        Ok(())
    }

    pub fn diagnostics(&self) -> PrioritizedReplayDiagnostics {
        let total = self.tree.total();
        let priority_entropy = self
            .scaled
            .iter()
            .map(|weight| weight / total)
            .filter(|probability| *probability > 0.0)
            .map(|probability| -probability * probability.ln())
            .sum();
        PrioritizedReplayDiagnostics {
            entries: self.priorities.len(),
            samples: self.samples,
            priority_updates: self.priority_updates,
            clipped_importance_weights: self.clipped_importance_weights,
            minimum_priority: self.priorities.iter().copied().reduce(f64::min).unwrap(),
            maximum_priority: self.priorities.iter().copied().reduce(f64::max).unwrap(),
            maximum_observed_importance_weight: self.maximum_observed_importance_weight,
            mean_importance_weight: (self.samples != 0)
                .then_some(self.importance_weight_sum / self.samples as f64),
            effective_sample_size: (self.importance_weight_square_sum > 0.0)
                .then_some(self.importance_weight_sum.powi(2) / self.importance_weight_square_sum),
            priority_entropy,
        }
    }
}

fn validate_config(config: &PrioritizedReplayConfig) -> Result<(), PrioritizedReplayError> {
    if !config.alpha.is_finite()
        || !(0.0..=1.0).contains(&config.alpha)
        || !config.beta.is_finite()
        || !(0.0..=1.0).contains(&config.beta)
        || !config.priority_epsilon.is_finite()
        || config.priority_epsilon <= 0.0
        || config.priority_epsilon > 1.0
        || !config.maximum_importance_weight.is_finite()
        || !(1.0..=100.0).contains(&config.maximum_importance_weight)
    {
        return Err(PrioritizedReplayError::new(
            "invalid prioritized replay config",
        ));
    }
    Ok(())
}

fn bounded_priority(
    value: f64,
    config: &PrioritizedReplayConfig,
) -> Result<f64, PrioritizedReplayError> {
    if !value.is_finite() || value < 0.0 {
        return Err(PrioritizedReplayError::new(
            "priority must be finite and nonnegative",
        ));
    }
    Ok((value + config.priority_epsilon).min(MAX_REPLAY_PRIORITY))
}

struct FenwickTree {
    values: Vec<f64>,
}

impl FenwickTree {
    fn new(weights: &[f64]) -> Self {
        let mut tree = Self {
            values: vec![0.0; weights.len() + 1],
        };
        for (index, weight) in weights.iter().copied().enumerate() {
            tree.add(index, weight);
        }
        tree
    }

    fn add(&mut self, index: usize, delta: f64) {
        let mut position = index + 1;
        while position < self.values.len() {
            self.values[position] += delta;
            position += position & position.wrapping_neg();
        }
    }

    fn total(&self) -> f64 {
        let mut sum = 0.0;
        let mut position = self.values.len() - 1;
        while position != 0 {
            sum += self.values[position];
            position &= position - 1;
        }
        sum
    }

    fn find_prefix(&self, target: f64) -> usize {
        let mut index = 0_usize;
        let mut accumulated = 0.0;
        let mut bit = 1_usize << ((self.values.len() - 1).ilog2());
        while bit != 0 {
            let next = index + bit;
            if next < self.values.len() && accumulated + self.values[next] <= target {
                index = next;
                accumulated += self.values[next];
            }
            bit >>= 1;
        }
        index.min(self.values.len() - 2)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrioritizedReplayError(String);

impl PrioritizedReplayError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for PrioritizedReplayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for PrioritizedReplayError {}

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

    #[test]
    fn seeded_sampling_prioritizes_updates_and_caps_importance_correction() {
        let config = PrioritizedReplayConfig {
            alpha: 1.0,
            beta: 1.0,
            priority_epsilon: 0.001,
            maximum_importance_weight: 2.0,
        };
        let mut left = PrioritizedReplay::new(&[1.0, 1.0, 20.0], config.clone(), 7).unwrap();
        let mut right = PrioritizedReplay::new(&[1.0, 1.0, 20.0], config, 7).unwrap();
        let left_samples = (0..1_000)
            .map(|_| left.sample().unwrap())
            .collect::<Vec<_>>();
        let right_samples = (0..1_000)
            .map(|_| right.sample().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(left_samples, right_samples);
        assert!(
            left_samples
                .iter()
                .filter(|sample| sample.index == 2)
                .count()
                > 800
        );
        assert!(
            left_samples
                .iter()
                .all(|sample| sample.importance_weight <= 2.0)
        );
        left.update_td_error(0, 100.0).unwrap();
        let diagnostics = left.diagnostics();
        assert_eq!(diagnostics.samples, 1_000);
        assert_eq!(diagnostics.priority_updates, 1);
        assert!(diagnostics.clipped_importance_weights > 0);
        assert!(diagnostics.effective_sample_size.unwrap() <= 1_000.0);
        assert!(diagnostics.priority_entropy.is_finite());
    }
}
