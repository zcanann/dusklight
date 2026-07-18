//! Episode-grouped partial-observability diagnostics for fixed model inputs.

use crate::artifact::Digest;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const HISTORY_CRITIC_COMPARISON_SCHEMA_V1: &str = "dusklight-history-critic-comparison/v1";
const MAX_EPISODES: usize = 4096;
const MAX_SEQUENCE_STEPS: usize = 250_000;
const MAX_FEATURE_WIDTH: usize = 4096;

#[derive(Clone, Debug)]
pub struct SequenceEpisode {
    pub episode_sha256: Digest,
    /// The caller includes the action/objective when the critic conditions on it.
    pub observations: Vec<Vec<f32>>,
    pub targets: Vec<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct HistoryCriticConfig {
    pub stack_depth: usize,
    pub recurrent_width: usize,
    pub ridge_penalty: f64,
    pub alias_target_tolerance: f32,
    pub minimum_relative_improvement: f64,
    pub seed: u64,
}

impl Default for HistoryCriticConfig {
    fn default() -> Self {
        Self {
            stack_depth: 4,
            recurrent_width: 16,
            ridge_penalty: 1.0e-3,
            alias_target_tolerance: 1.0e-3,
            minimum_relative_improvement: 0.1,
            seed: 0x4849_5354_4f52_5901,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryCriticDisposition {
    SingleFrameSufficient,
    ShortStackPreferred,
    RecurrentPreferred,
    HistoryBenefitInconclusive,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StateAliasingAudit {
    pub exact_state_groups: usize,
    pub repeated_state_groups: usize,
    pub target_aliased_state_groups: usize,
    pub aliased_rows: usize,
    pub maximum_target_spread: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CriticEvaluation {
    pub name: &'static str,
    pub feature_width: usize,
    pub training_rows: usize,
    pub held_out_rows: usize,
    pub ridge_penalty: f64,
    pub training_mse: f64,
    pub held_out_mse: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HistoryCriticComparison {
    pub schema: &'static str,
    pub representation_sha256: Digest,
    pub config: HistoryCriticConfig,
    pub observation_width: usize,
    pub training_episodes: usize,
    pub held_out_episodes: usize,
    pub training_aliasing: StateAliasingAudit,
    pub held_out_aliasing: StateAliasingAudit,
    pub single_frame: CriticEvaluation,
    pub short_stack: CriticEvaluation,
    pub recurrent: CriticEvaluation,
    pub equal_training_row_budget: bool,
    pub equal_held_out_row_budget: bool,
    pub disposition: HistoryCriticDisposition,
    pub promotion_authority: bool,
    pub limitation: &'static str,
    pub comparison_sha256: Digest,
}

impl HistoryCriticComparison {
    pub fn compare(
        representation_sha256: Digest,
        training: &[SequenceEpisode],
        held_out: &[SequenceEpisode],
        config: HistoryCriticConfig,
    ) -> Result<Self, HistoryCriticError> {
        let observation_width = validate_inputs(representation_sha256, training, held_out, config)?;
        let training_aliasing = aliasing_audit(training, config.alias_target_tolerance);
        let held_out_aliasing = aliasing_audit(held_out, config.alias_target_tolerance);

        let train_single = feature_rows(training, FeatureMode::Single, config);
        let test_single = feature_rows(held_out, FeatureMode::Single, config);
        let train_stack = feature_rows(training, FeatureMode::Stack, config);
        let test_stack = feature_rows(held_out, FeatureMode::Stack, config);
        let reservoir = Reservoir::new(observation_width, config.recurrent_width, config.seed);
        let train_recurrent = recurrent_rows(training, &reservoir);
        let test_recurrent = recurrent_rows(held_out, &reservoir);

        let single_frame = fit_evaluate(
            "single_frame",
            &train_single,
            &test_single,
            config.ridge_penalty,
        )?;
        let short_stack = fit_evaluate(
            "short_stack",
            &train_stack,
            &test_stack,
            config.ridge_penalty,
        )?;
        let recurrent = fit_evaluate(
            "deterministic_recurrent_reservoir",
            &train_recurrent,
            &test_recurrent,
            config.ridge_penalty,
        )?;
        let disposition = disposition(
            &training_aliasing,
            &single_frame,
            &short_stack,
            &recurrent,
            config.minimum_relative_improvement,
        );
        let equal_training_row_budget = single_frame.training_rows == short_stack.training_rows
            && short_stack.training_rows == recurrent.training_rows;
        let equal_held_out_row_budget = single_frame.held_out_rows == short_stack.held_out_rows
            && short_stack.held_out_rows == recurrent.held_out_rows;
        if !equal_training_row_budget || !equal_held_out_row_budget {
            return Err(HistoryCriticError::new(
                "history critics did not consume equal row budgets",
            ));
        }
        let mut report = Self {
            schema: HISTORY_CRITIC_COMPARISON_SCHEMA_V1,
            representation_sha256,
            config,
            observation_width,
            training_episodes: training.len(),
            held_out_episodes: held_out.len(),
            training_aliasing,
            held_out_aliasing,
            single_frame,
            short_stack,
            recurrent,
            equal_training_row_budget,
            equal_held_out_row_budget,
            disposition,
            promotion_authority: false,
            limitation: "held-out target MSE is not native objective success",
            comparison_sha256: Digest::ZERO,
        };
        report.comparison_sha256 = report.digest()?;
        Ok(report)
    }

    fn digest(&self) -> Result<Digest, HistoryCriticError> {
        let bytes = serde_json::to_vec(&(
            self.schema,
            self.representation_sha256,
            self.config,
            self.observation_width,
            self.training_episodes,
            self.held_out_episodes,
            &self.training_aliasing,
            &self.held_out_aliasing,
            &self.single_frame,
            &self.short_stack,
            &self.recurrent,
            self.equal_training_row_budget,
            self.equal_held_out_row_budget,
            self.disposition,
            self.promotion_authority,
            self.limitation,
        ))
        .map_err(|error| HistoryCriticError::new(error.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.history-critic-comparison/v1\0");
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }
}

fn validate_inputs(
    representation_sha256: Digest,
    training: &[SequenceEpisode],
    held_out: &[SequenceEpisode],
    config: HistoryCriticConfig,
) -> Result<usize, HistoryCriticError> {
    if representation_sha256 == Digest::ZERO
        || training.is_empty()
        || held_out.is_empty()
        || training.len() > MAX_EPISODES
        || held_out.len() > MAX_EPISODES
        || config.stack_depth < 2
        || config.stack_depth > 32
        || config.recurrent_width == 0
        || config.recurrent_width > 256
        || !config.ridge_penalty.is_finite()
        || config.ridge_penalty <= 0.0
        || !config.alias_target_tolerance.is_finite()
        || config.alias_target_tolerance < 0.0
        || !config.minimum_relative_improvement.is_finite()
        || !(0.0..=1.0).contains(&config.minimum_relative_improvement)
    {
        return Err(HistoryCriticError::new(
            "history critic comparison configuration is invalid",
        ));
    }
    let width = training[0]
        .observations
        .first()
        .map(Vec::len)
        .ok_or_else(|| HistoryCriticError::new("history episode is empty"))?;
    if width == 0 || width > MAX_FEATURE_WIDTH {
        return Err(HistoryCriticError::new(
            "history observation width is invalid",
        ));
    }
    let mut ids = BTreeSet::new();
    let mut steps = 0_usize;
    for episode in training.iter().chain(held_out) {
        steps = steps
            .checked_add(episode.observations.len())
            .ok_or_else(|| HistoryCriticError::new("history step count overflowed"))?;
        if episode.episode_sha256 == Digest::ZERO
            || !ids.insert(episode.episode_sha256)
            || episode.observations.is_empty()
            || episode.observations.len() != episode.targets.len()
            || episode.observations.iter().any(|observation| {
                observation.len() != width || observation.iter().any(|value| !value.is_finite())
            })
            || episode.targets.iter().any(|target| !target.is_finite())
        {
            return Err(HistoryCriticError::new(
                "history episodes are invalid or cross-split duplicated",
            ));
        }
    }
    if steps > MAX_SEQUENCE_STEPS {
        return Err(HistoryCriticError::new(
            "history comparison exceeds its step budget",
        ));
    }
    Ok(width)
}

fn aliasing_audit(episodes: &[SequenceEpisode], tolerance: f32) -> StateAliasingAudit {
    let mut groups = BTreeMap::<Vec<u32>, Vec<f32>>::new();
    for episode in episodes {
        for (observation, target) in episode.observations.iter().zip(&episode.targets) {
            groups
                .entry(observation.iter().map(|value| value.to_bits()).collect())
                .or_default()
                .push(*target);
        }
    }
    let mut repeated_state_groups = 0;
    let mut target_aliased_state_groups = 0;
    let mut aliased_rows = 0;
    let mut maximum_target_spread = 0.0_f32;
    for targets in groups.values() {
        if targets.len() < 2 {
            continue;
        }
        repeated_state_groups += 1;
        let minimum = targets.iter().copied().min_by(f32::total_cmp).unwrap();
        let maximum = targets.iter().copied().max_by(f32::total_cmp).unwrap();
        let spread = maximum - minimum;
        maximum_target_spread = maximum_target_spread.max(spread);
        if spread > tolerance {
            target_aliased_state_groups += 1;
            aliased_rows += targets.len();
        }
    }
    StateAliasingAudit {
        exact_state_groups: groups.len(),
        repeated_state_groups,
        target_aliased_state_groups,
        aliased_rows,
        maximum_target_spread,
    }
}

#[derive(Clone, Copy)]
enum FeatureMode {
    Single,
    Stack,
}

#[derive(Clone)]
struct RegressionRows {
    features: Vec<Vec<f64>>,
    targets: Vec<f64>,
}

fn feature_rows(
    episodes: &[SequenceEpisode],
    mode: FeatureMode,
    config: HistoryCriticConfig,
) -> RegressionRows {
    let width = episodes[0].observations[0].len();
    let mut rows = RegressionRows {
        features: Vec::new(),
        targets: Vec::new(),
    };
    for episode in episodes {
        for step in 0..episode.observations.len() {
            let mut features = vec![1.0];
            match mode {
                FeatureMode::Single => features.extend(
                    episode.observations[step]
                        .iter()
                        .map(|value| f64::from(*value)),
                ),
                FeatureMode::Stack => {
                    for lag in (0..config.stack_depth).rev() {
                        if let Some(index) = step.checked_sub(lag) {
                            features.extend(
                                episode.observations[index]
                                    .iter()
                                    .map(|value| f64::from(*value)),
                            );
                            features.push(1.0);
                        } else {
                            features.extend(std::iter::repeat_n(0.0, width));
                            features.push(0.0);
                        }
                    }
                }
            }
            rows.features.push(features);
            rows.targets.push(f64::from(episode.targets[step]));
        }
    }
    rows
}

struct Reservoir {
    input_weights: Vec<Vec<f64>>,
    recurrent_weights: Vec<Vec<f64>>,
}

impl Reservoir {
    fn new(input_width: usize, hidden_width: usize, seed: u64) -> Self {
        let weight = |row: usize, column: usize, domain: u64, scale: f64| {
            let mixed = mix(seed ^ domain, (row as u64) << 32 | column as u64);
            (mixed as f64 / u64::MAX as f64 * 2.0 - 1.0) * scale
        };
        let input_weights = (0..hidden_width)
            .map(|row| {
                (0..input_width)
                    .map(|column| weight(row, column, 0x494e_5055_54, 0.75))
                    .collect()
            })
            .collect();
        let recurrent_weights = (0..hidden_width)
            .map(|row| {
                (0..hidden_width)
                    .map(|column| {
                        if row == column {
                            0.55
                        } else {
                            weight(row, column, 0x5245_4355_52, 0.08 / hidden_width as f64)
                        }
                    })
                    .collect()
            })
            .collect();
        Self {
            input_weights,
            recurrent_weights,
        }
    }

    fn step(&self, observation: &[f32], previous: &[f64]) -> Vec<f64> {
        self.input_weights
            .iter()
            .zip(&self.recurrent_weights)
            .map(|(input, recurrent)| {
                input
                    .iter()
                    .zip(observation)
                    .map(|(weight, value)| weight * f64::from(*value))
                    .chain(
                        recurrent
                            .iter()
                            .zip(previous)
                            .map(|(weight, value)| weight * value),
                    )
                    .sum::<f64>()
                    .tanh()
            })
            .collect()
    }
}

fn recurrent_rows(episodes: &[SequenceEpisode], reservoir: &Reservoir) -> RegressionRows {
    let hidden_width = reservoir.input_weights.len();
    let mut rows = RegressionRows {
        features: Vec::new(),
        targets: Vec::new(),
    };
    for episode in episodes {
        let mut hidden = vec![0.0; hidden_width];
        for (observation, target) in episode.observations.iter().zip(&episode.targets) {
            hidden = reservoir.step(observation, &hidden);
            let mut features = vec![1.0];
            features.extend(observation.iter().map(|value| f64::from(*value)));
            features.extend(&hidden);
            rows.features.push(features);
            rows.targets.push(f64::from(*target));
        }
    }
    rows
}

fn fit_evaluate(
    name: &'static str,
    training: &RegressionRows,
    held_out: &RegressionRows,
    ridge_penalty: f64,
) -> Result<CriticEvaluation, HistoryCriticError> {
    let weights = ridge_fit(training, ridge_penalty)?;
    Ok(CriticEvaluation {
        name,
        feature_width: weights.len(),
        training_rows: training.features.len(),
        held_out_rows: held_out.features.len(),
        ridge_penalty,
        training_mse: mean_squared_error(training, &weights),
        held_out_mse: mean_squared_error(held_out, &weights),
    })
}

fn ridge_fit(rows: &RegressionRows, penalty: f64) -> Result<Vec<f64>, HistoryCriticError> {
    let width = rows.features[0].len();
    let mut matrix = vec![vec![0.0; width + 1]; width];
    for (features, target) in rows.features.iter().zip(&rows.targets) {
        for row in 0..width {
            matrix[row][width] += features[row] * target;
            for column in 0..width {
                matrix[row][column] += features[row] * features[column];
            }
        }
    }
    for (index, row) in matrix.iter_mut().enumerate().skip(1) {
        row[index] += penalty;
    }
    for pivot in 0..width {
        let best = (pivot..width)
            .max_by(|left, right| {
                matrix[*left][pivot]
                    .abs()
                    .total_cmp(&matrix[*right][pivot].abs())
            })
            .unwrap();
        if matrix[best][pivot].abs() <= 1.0e-12 {
            return Err(HistoryCriticError::new("history ridge system is singular"));
        }
        matrix.swap(pivot, best);
        let divisor = matrix[pivot][pivot];
        for column in pivot..=width {
            matrix[pivot][column] /= divisor;
        }
        for row in 0..width {
            if row == pivot {
                continue;
            }
            let factor = matrix[row][pivot];
            for column in pivot..=width {
                matrix[row][column] -= factor * matrix[pivot][column];
            }
        }
    }
    Ok(matrix.into_iter().map(|row| row[width]).collect())
}

fn mean_squared_error(rows: &RegressionRows, weights: &[f64]) -> f64 {
    rows.features
        .iter()
        .zip(&rows.targets)
        .map(|(features, target)| {
            let predicted = features
                .iter()
                .zip(weights)
                .map(|(feature, weight)| feature * weight)
                .sum::<f64>();
            (predicted - target).powi(2)
        })
        .sum::<f64>()
        / rows.features.len() as f64
}

fn disposition(
    aliasing: &StateAliasingAudit,
    single: &CriticEvaluation,
    stack: &CriticEvaluation,
    recurrent: &CriticEvaluation,
    minimum_improvement: f64,
) -> HistoryCriticDisposition {
    if aliasing.target_aliased_state_groups == 0 {
        return HistoryCriticDisposition::SingleFrameSufficient;
    }
    let denominator = single.held_out_mse.max(1.0e-12);
    let stack_improvement = (single.held_out_mse - stack.held_out_mse) / denominator;
    let recurrent_improvement = (single.held_out_mse - recurrent.held_out_mse) / denominator;
    if stack_improvement < minimum_improvement && recurrent_improvement < minimum_improvement {
        HistoryCriticDisposition::HistoryBenefitInconclusive
    } else if recurrent.held_out_mse < stack.held_out_mse {
        HistoryCriticDisposition::RecurrentPreferred
    } else {
        HistoryCriticDisposition::ShortStackPreferred
    }
}

fn mix(seed: u64, value: u64) -> u64 {
    let mut word = seed ^ value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    word = (word ^ (word >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    word = (word ^ (word >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    word ^ (word >> 31)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryCriticError(String);

impl HistoryCriticError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for HistoryCriticError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for HistoryCriticError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn hidden_cue_episodes(start: u8, count: usize) -> Vec<SequenceEpisode> {
        (0..count)
            .map(|index| {
                let cue = if index % 2 == 0 { -1.0 } else { 1.0 };
                SequenceEpisode {
                    episode_sha256: Digest([start + index as u8; 32]),
                    observations: vec![vec![cue], vec![0.0]],
                    targets: vec![cue, cue],
                }
            })
            .collect()
    }

    #[test]
    fn history_models_improve_when_current_state_aliases_hidden_cue() {
        let report = HistoryCriticComparison::compare(
            Digest([90; 32]),
            &hidden_cue_episodes(1, 20),
            &hidden_cue_episodes(101, 10),
            HistoryCriticConfig {
                stack_depth: 2,
                recurrent_width: 8,
                ..HistoryCriticConfig::default()
            },
        )
        .unwrap();
        assert_eq!(report.training_aliasing.target_aliased_state_groups, 1);
        assert!(report.short_stack.held_out_mse < report.single_frame.held_out_mse * 0.1);
        assert!(report.recurrent.held_out_mse < report.single_frame.held_out_mse * 0.5);
        assert!(matches!(
            report.disposition,
            HistoryCriticDisposition::ShortStackPreferred
                | HistoryCriticDisposition::RecurrentPreferred
        ));
        assert_ne!(report.comparison_sha256, Digest::ZERO);
    }

    #[test]
    fn episode_boundaries_reset_history_and_splits_are_content_disjoint() {
        let training = hidden_cue_episodes(1, 4);
        let held_out = hidden_cue_episodes(20, 4);
        let report = HistoryCriticComparison::compare(
            Digest([90; 32]),
            &training,
            &held_out,
            HistoryCriticConfig::default(),
        )
        .unwrap();
        assert_eq!(report.single_frame.training_rows, 8);
        assert_eq!(report.short_stack.training_rows, 8);
        assert_eq!(report.recurrent.training_rows, 8);

        let mut duplicated = held_out;
        duplicated[0].episode_sha256 = training[0].episode_sha256;
        assert!(
            HistoryCriticComparison::compare(
                Digest([90; 32]),
                &training,
                &duplicated,
                HistoryCriticConfig::default()
            )
            .is_err()
        );
    }

    #[test]
    fn no_target_aliasing_keeps_single_frame_baseline() {
        let episodes = |start: u8| {
            (0..4)
                .map(|index| SequenceEpisode {
                    episode_sha256: Digest([start + index; 32]),
                    observations: vec![vec![-1.0], vec![1.0]],
                    targets: vec![-1.0, 1.0],
                })
                .collect::<Vec<_>>()
        };
        let report = HistoryCriticComparison::compare(
            Digest([90; 32]),
            &episodes(1),
            &episodes(20),
            HistoryCriticConfig::default(),
        )
        .unwrap();
        assert_eq!(
            report.disposition,
            HistoryCriticDisposition::SingleFrameSufficient
        );
    }
}
