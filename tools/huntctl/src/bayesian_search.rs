//! Bounded deterministic Gaussian-process acquisition for expensive tactics.

use serde::Serialize;
use std::collections::BTreeSet;
use std::error::Error;
use std::f64::consts::{PI, SQRT_2};
use std::fmt;

// Exact GP fitting is cubic in the observation count. Keep this low enough that a
// malformed or over-ambitious run cannot turn optimizer bookkeeping into the
// dominant workload (these searches are intended for expensive native rollouts).
pub const MAX_BAYESIAN_OBSERVATIONS: usize = 512;
pub const MAX_BAYESIAN_POOL: usize = 65_536;
const PRIMES: [u32; 16] = [2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37, 41, 43, 47, 53];

#[derive(Clone, Copy, Debug)]
pub struct BayesianConfig {
    pub dimensions: usize,
    pub initial_samples: usize,
    pub acquisition_pool: usize,
    pub length_scale: f64,
    pub observation_noise: f64,
    pub exploration: f64,
    pub seed: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BayesianProposal {
    pub generation: u32,
    pub proposal_index: usize,
    pub normalized: Vec<f64>,
    pub acquisition: f64,
    pub predicted_mean: f64,
    pub predicted_standard_deviation: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BayesianObservation {
    pub normalized: Vec<f64>,
    /// Empirical native rank utility in [0, 1], with larger being better.
    pub rank_utility: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct BayesianSnapshot {
    pub schema: &'static str,
    pub generation: u32,
    pub observations: usize,
    pub best_rank_utility: Option<f64>,
    pub length_scale: f64,
    pub observation_noise: f64,
    pub exploration: f64,
    pub next_sequence_index: u64,
}

#[derive(Clone, Debug)]
pub struct BayesianOptimizer {
    config: BayesianConfig,
    generation: u32,
    sequence_index: u64,
    observations: Vec<BayesianObservation>,
    seen: BTreeSet<Vec<u64>>,
}

impl BayesianOptimizer {
    pub fn new(config: BayesianConfig) -> Result<Self, BayesianError> {
        if config.dimensions == 0
            || config.dimensions > PRIMES.len()
            || config.initial_samples == 0
            || config.initial_samples > MAX_BAYESIAN_OBSERVATIONS
            || config.acquisition_pool < 16
            || config.acquisition_pool > MAX_BAYESIAN_POOL
            || !config.length_scale.is_finite()
            || !(0.001..=10.0).contains(&config.length_scale)
            || !config.observation_noise.is_finite()
            || !(1.0e-12..=1.0).contains(&config.observation_noise)
            || !config.exploration.is_finite()
            || !(0.0..=1.0).contains(&config.exploration)
        {
            return Err(BayesianError::new("invalid bounded Bayesian configuration"));
        }
        Ok(Self {
            config,
            generation: 0,
            sequence_index: config.seed % 1_000_003 + 1,
            observations: Vec::new(),
            seen: BTreeSet::new(),
        })
    }

    pub fn ask(&mut self, count: usize) -> Result<Vec<BayesianProposal>, BayesianError> {
        if count == 0 || count > 512 {
            return Err(BayesianError::new("Bayesian batch size is outside 1..=512"));
        }
        let model = if self.observations.len() >= self.config.initial_samples {
            Some(GaussianProcess::fit(&self.observations, &self.config)?)
        } else {
            None
        };
        let pool_size = if model.is_some() {
            self.config.acquisition_pool
        } else {
            count.saturating_mul(8).max(count)
        };
        let best = self
            .observations
            .iter()
            .map(|observation| observation.rank_utility)
            .reduce(f64::max)
            .unwrap_or(0.0);
        let mut pool = Vec::with_capacity(pool_size);
        while pool.len() < pool_size {
            let point = self.next_point();
            let key = point
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>();
            if self.seen.contains(&key) {
                continue;
            }
            let (mean, variance) = model
                .as_ref()
                .map(|model| model.predict(&point))
                .transpose()?
                .unwrap_or((0.0, 1.0));
            let standard_deviation = variance.max(0.0).sqrt();
            let acquisition = if model.is_some() {
                expected_improvement(mean, standard_deviation, best, self.config.exploration)
            } else {
                // Initial design order remains the deterministic sequence order.
                -(pool.len() as f64)
            };
            pool.push(BayesianProposal {
                generation: self.generation,
                proposal_index: 0,
                normalized: point,
                acquisition,
                predicted_mean: mean,
                predicted_standard_deviation: standard_deviation,
            });
        }
        pool.sort_by(|left, right| {
            right
                .acquisition
                .total_cmp(&left.acquisition)
                .then_with(|| lexicographic_f64(&left.normalized, &right.normalized))
        });
        pool.truncate(count);
        for (index, proposal) in pool.iter_mut().enumerate() {
            proposal.proposal_index = index;
        }
        Ok(pool)
    }

    pub fn tell(&mut self, observations: Vec<BayesianObservation>) -> Result<(), BayesianError> {
        if observations.is_empty()
            || self.observations.len() + observations.len() > MAX_BAYESIAN_OBSERVATIONS
        {
            return Err(BayesianError::new("invalid Bayesian observation batch"));
        }
        for observation in observations {
            if observation.normalized.len() != self.config.dimensions
                || observation
                    .normalized
                    .iter()
                    .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
                || !observation.rank_utility.is_finite()
                || !(0.0..=1.0).contains(&observation.rank_utility)
            {
                return Err(BayesianError::new("invalid Bayesian observation"));
            }
            let key = observation
                .normalized
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>();
            if !self.seen.insert(key) {
                return Err(BayesianError::new("duplicate Bayesian observation"));
            }
            self.observations.push(observation);
        }
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| BayesianError::new("Bayesian generation overflowed"))?;
        Ok(())
    }

    pub fn snapshot(&self) -> BayesianSnapshot {
        BayesianSnapshot {
            schema: "dusklight-bayesian-optimizer/v1",
            generation: self.generation,
            observations: self.observations.len(),
            best_rank_utility: self
                .observations
                .iter()
                .map(|observation| observation.rank_utility)
                .reduce(f64::max),
            length_scale: self.config.length_scale,
            observation_noise: self.config.observation_noise,
            exploration: self.config.exploration,
            next_sequence_index: self.sequence_index,
        }
    }

    fn next_point(&mut self) -> Vec<f64> {
        let index = self.sequence_index;
        self.sequence_index += 1;
        PRIMES[..self.config.dimensions]
            .iter()
            .enumerate()
            .map(|(dimension, base)| {
                let shift = fractional_hash(self.config.seed, dimension as u64);
                (radical_inverse(index, *base) + shift).fract()
            })
            .collect()
    }
}

struct GaussianProcess {
    points: Vec<Vec<f64>>,
    alpha: Vec<f64>,
    cholesky: Vec<Vec<f64>>,
    length_scale: f64,
}

impl GaussianProcess {
    fn fit(
        observations: &[BayesianObservation],
        config: &BayesianConfig,
    ) -> Result<Self, BayesianError> {
        let count = observations.len();
        let mut covariance = vec![vec![0.0; count]; count];
        for row in 0..count {
            for column in 0..=row {
                let value = rbf(
                    &observations[row].normalized,
                    &observations[column].normalized,
                    config.length_scale,
                );
                covariance[row][column] = value;
                covariance[column][row] = value;
            }
            covariance[row][row] += config.observation_noise;
        }
        let cholesky = cholesky(&covariance)?;
        let targets = observations
            .iter()
            .map(|observation| observation.rank_utility)
            .collect::<Vec<_>>();
        let intermediate = solve_lower(&cholesky, &targets)?;
        let alpha = solve_upper_transpose(&cholesky, &intermediate)?;
        Ok(Self {
            points: observations
                .iter()
                .map(|observation| observation.normalized.clone())
                .collect(),
            alpha,
            cholesky,
            length_scale: config.length_scale,
        })
    }

    fn predict(&self, point: &[f64]) -> Result<(f64, f64), BayesianError> {
        let kernel = self
            .points
            .iter()
            .map(|observed| rbf(observed, point, self.length_scale))
            .collect::<Vec<_>>();
        let mean = dot(&kernel, &self.alpha);
        let solved = solve_lower(&self.cholesky, &kernel)?;
        let variance = (1.0 - dot(&solved, &solved)).max(1.0e-12);
        Ok((mean, variance))
    }
}

fn expected_improvement(mean: f64, sigma: f64, best: f64, exploration: f64) -> f64 {
    if sigma <= 1.0e-12 {
        return 0.0;
    }
    let improvement = mean - best - exploration;
    let z = improvement / sigma;
    improvement * normal_cdf(z) + sigma * normal_pdf(z)
}

fn normal_pdf(value: f64) -> f64 {
    (-0.5 * value * value).exp() / (2.0 * PI).sqrt()
}

fn normal_cdf(value: f64) -> f64 {
    0.5 * (1.0 + erf_approx(value / SQRT_2))
}

fn erf_approx(value: f64) -> f64 {
    // Abramowitz-Stegun 7.1.26, deterministic and amply accurate for
    // acquisition ranking (maximum absolute error about 1.5e-7).
    let sign = if value < 0.0 { -1.0 } else { 1.0 };
    let x = value.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let polynomial =
        (((((1.061_405_429 * t - 1.453_152_027) * t) + 1.421_413_741) * t - 0.284_496_736) * t
            + 0.254_829_592)
            * t;
    sign * (1.0 - polynomial * (-x * x).exp())
}

fn rbf(left: &[f64], right: &[f64], length_scale: f64) -> f64 {
    let squared_distance = left
        .iter()
        .zip(right)
        .map(|(left, right)| (left - right).powi(2))
        .sum::<f64>();
    (-0.5 * squared_distance / (length_scale * length_scale)).exp()
}

fn radical_inverse(mut index: u64, base: u32) -> f64 {
    let inverse = 1.0 / f64::from(base);
    let mut factor = inverse;
    let mut output = 0.0;
    while index != 0 {
        output += (index % u64::from(base)) as f64 * factor;
        index /= u64::from(base);
        factor *= inverse;
    }
    output
}

fn fractional_hash(seed: u64, dimension: u64) -> f64 {
    let mut value = seed ^ dimension.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    ((value >> 11) as f64) / ((1_u64 << 53) as f64)
}

fn lexicographic_f64(left: &[f64], right: &[f64]) -> std::cmp::Ordering {
    left.iter()
        .zip(right)
        .find_map(|(left, right)| {
            let ordering = left.total_cmp(right);
            (ordering != std::cmp::Ordering::Equal).then_some(ordering)
        })
        .unwrap_or(std::cmp::Ordering::Equal)
}

fn cholesky(matrix: &[Vec<f64>]) -> Result<Vec<Vec<f64>>, BayesianError> {
    let count = matrix.len();
    let mut output = vec![vec![0.0; count]; count];
    for row in 0..count {
        for column in 0..=row {
            let mut value = matrix[row][column];
            for index in 0..column {
                value -= output[row][index] * output[column][index];
            }
            if row == column {
                if value <= 0.0 || !value.is_finite() {
                    return Err(BayesianError::new(
                        "Gaussian-process covariance is singular",
                    ));
                }
                output[row][column] = value.sqrt();
            } else {
                output[row][column] = value / output[column][column];
            }
        }
    }
    Ok(output)
}

fn solve_lower(matrix: &[Vec<f64>], values: &[f64]) -> Result<Vec<f64>, BayesianError> {
    let mut output = vec![0.0; values.len()];
    for row in 0..values.len() {
        let mut value = values[row];
        for column in 0..row {
            value -= matrix[row][column] * output[column];
        }
        if matrix[row][row] == 0.0 {
            return Err(BayesianError::new("singular Gaussian-process factor"));
        }
        output[row] = value / matrix[row][row];
    }
    Ok(output)
}

fn solve_upper_transpose(matrix: &[Vec<f64>], values: &[f64]) -> Result<Vec<f64>, BayesianError> {
    let mut output = vec![0.0; values.len()];
    for row in (0..values.len()).rev() {
        let mut value = values[row];
        for column in row + 1..values.len() {
            value -= matrix[column][row] * output[column];
        }
        if matrix[row][row] == 0.0 {
            return Err(BayesianError::new("singular Gaussian-process factor"));
        }
        output[row] = value / matrix[row][row];
    }
    Ok(output)
}

fn dot(left: &[f64], right: &[f64]) -> f64 {
    left.iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum()
}

#[derive(Clone, Debug)]
pub struct BayesianError(String);

impl BayesianError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for BayesianError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for BayesianError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_expected_improvement_concentrates_on_a_smooth_bounded_peak() {
        let config = BayesianConfig {
            dimensions: 2,
            initial_samples: 8,
            acquisition_pool: 512,
            length_scale: 0.2,
            observation_noise: 1.0e-6,
            exploration: 0.01,
            seed: 17,
        };
        let mut first = BayesianOptimizer::new(config).unwrap();
        let mut second = BayesianOptimizer::new(config).unwrap();
        let objective = |point: &[f64]| {
            (1.0 - ((point[0] - 0.73).powi(2) + (point[1] - 0.21).powi(2)) * 4.0).clamp(0.0, 1.0)
        };
        let mut best: f64 = 0.0;
        for _ in 0..8 {
            let proposals = first.ask(4).unwrap();
            assert_eq!(proposals, second.ask(4).unwrap());
            let observations = proposals
                .into_iter()
                .map(|proposal| {
                    let utility = objective(&proposal.normalized);
                    best = best.max(utility);
                    BayesianObservation {
                        normalized: proposal.normalized,
                        rank_utility: utility,
                    }
                })
                .collect::<Vec<_>>();
            first.tell(observations.clone()).unwrap();
            second.tell(observations).unwrap();
        }
        assert!(best > 0.98);
        assert_eq!(first.snapshot().observations, 32);
        assert_eq!(
            first.snapshot().next_sequence_index,
            second.snapshot().next_sequence_index
        );
    }

    #[test]
    fn duplicate_or_out_of_bounds_observations_are_rejected() {
        let mut optimizer = BayesianOptimizer::new(BayesianConfig {
            dimensions: 1,
            initial_samples: 1,
            acquisition_pool: 16,
            length_scale: 0.2,
            observation_noise: 1.0e-6,
            exploration: 0.0,
            seed: 1,
        })
        .unwrap();
        let observation = BayesianObservation {
            normalized: vec![0.5],
            rank_utility: 1.0,
        };
        optimizer.tell(vec![observation.clone()]).unwrap();
        assert!(optimizer.tell(vec![observation]).is_err());
    }
}
