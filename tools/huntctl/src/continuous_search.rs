//! Seeded bounded CEM and full-covariance CMA-ES over typed candidate axes.

use crate::motion_path::StickPath;
use crate::search::{Candidate, MacroAction, SearchError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::error::Error;
use std::f64::consts::TAU;
use std::fmt;

pub const CONTINUOUS_AXES_SCHEMA_V1: &str = "dusklight-continuous-axes/v1";
pub const MAX_CONTINUOUS_DIMENSIONS: usize = 16;
pub const MAX_CONTINUOUS_POPULATION: usize = 512;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContinuousMethod {
    CrossEntropy,
    CmaEs,
}

impl std::str::FromStr for ContinuousMethod {
    type Err = ContinuousSearchError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "cem" | "cross-entropy" => Ok(Self::CrossEntropy),
            "cma-es" | "cmaes" => Ok(Self::CmaEs),
            _ => Err(ContinuousSearchError::new(
                "continuous method must be cem or cma-es",
            )),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContinuousAxes {
    pub schema: String,
    pub axes: Vec<ContinuousAxis>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContinuousAxis {
    pub name: String,
    pub action_index: usize,
    pub parameter: ContinuousParameter,
    pub minimum: f64,
    pub maximum: f64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContinuousParameter {
    MoveHeadingDegrees,
    MoveMagnitude,
    MoveDuration,
    RollHeadingDegrees,
    RollMagnitude,
    RollButtonFrame,
    RollRecoveryFrames,
    MotionPathDuration,
    MotionPathSamplePhaseNumerator,
    MotionPathPointX { point_index: usize },
    MotionPathPointY { point_index: usize },
}

#[derive(Clone, Copy, Debug)]
pub struct ContinuousOptimizerConfig {
    pub method: ContinuousMethod,
    pub population_size: usize,
    pub elite_count: usize,
    /// Normalized initial standard deviation relative to each declared bound.
    pub initial_sigma: f64,
    pub seed: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ContinuousSample {
    pub generation: u32,
    pub sample_index: usize,
    pub normalized: Vec<f64>,
    pub values: Vec<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ContinuousOptimizerSnapshot {
    pub method: ContinuousMethod,
    pub generation: u32,
    pub mean: Vec<f64>,
    pub normalized_mean: Vec<f64>,
    pub sigma: f64,
    pub covariance: Vec<Vec<f64>>,
}

#[derive(Clone, Debug)]
pub struct ContinuousTemplate {
    seed: Candidate,
    axes: ContinuousAxes,
    initial: Vec<f64>,
}

impl ContinuousTemplate {
    pub fn new(seed: Candidate, axes: ContinuousAxes) -> Result<Self, ContinuousSearchError> {
        seed.validate()?;
        if axes.schema != CONTINUOUS_AXES_SCHEMA_V1
            || axes.axes.is_empty()
            || axes.axes.len() > MAX_CONTINUOUS_DIMENSIONS
        {
            return Err(ContinuousSearchError::new(
                "continuous axes require schema v1 and 1..=16 dimensions",
            ));
        }
        let mut names = BTreeSet::new();
        let mut keys = BTreeSet::new();
        let mut initial = Vec::with_capacity(axes.axes.len());
        for axis in &axes.axes {
            if axis.name.is_empty()
                || axis.name.len() > 96
                || !names.insert(axis.name.clone())
                || !axis.minimum.is_finite()
                || !axis.maximum.is_finite()
                || axis.minimum >= axis.maximum
                || !keys.insert((axis.action_index, format!("{:?}", axis.parameter)))
            {
                return Err(ContinuousSearchError::new(
                    "continuous axis names, keys, and finite bounds must be unique and valid",
                ));
            }
            let value = read_parameter(&seed, axis)?;
            if value < axis.minimum || value > axis.maximum {
                return Err(ContinuousSearchError::new(format!(
                    "seed value for axis {:?} lies outside its bounds",
                    axis.name
                )));
            }
            initial.push(value);
            let mut minimum = seed.clone();
            write_parameter(&mut minimum, axis, axis.minimum)?;
            minimum.validate()?;
            let mut maximum = seed.clone();
            write_parameter(&mut maximum, axis, axis.maximum)?;
            maximum.validate()?;
        }
        Ok(Self {
            seed,
            axes,
            initial,
        })
    }

    pub fn dimensions(&self) -> usize {
        self.axes.axes.len()
    }

    pub fn axes(&self) -> &ContinuousAxes {
        &self.axes
    }

    pub fn initial_values(&self) -> &[f64] {
        &self.initial
    }

    pub fn candidate(&self, values: &[f64]) -> Result<Candidate, ContinuousSearchError> {
        if values.len() != self.dimensions() {
            return Err(ContinuousSearchError::new(
                "continuous value width does not match the axis declaration",
            ));
        }
        let mut candidate = self.seed.clone();
        for (axis, value) in self.axes.axes.iter().zip(values) {
            if !value.is_finite() || *value < axis.minimum || *value > axis.maximum {
                return Err(ContinuousSearchError::new(
                    "continuous candidate value is outside its declared bound",
                ));
            }
            write_parameter(&mut candidate, axis, *value)?;
        }
        candidate.validate()?;
        Ok(candidate)
    }

    fn normalize(&self, values: &[f64]) -> Vec<f64> {
        self.axes
            .axes
            .iter()
            .zip(values)
            .map(|(axis, value)| (value - axis.minimum) / (axis.maximum - axis.minimum))
            .collect()
    }

    fn denormalize(&self, normalized: &[f64]) -> Vec<f64> {
        self.axes
            .axes
            .iter()
            .zip(normalized)
            .map(|(axis, value)| axis.minimum + value * (axis.maximum - axis.minimum))
            .collect()
    }

    pub fn values_from_normalized(
        &self,
        normalized: &[f64],
    ) -> Result<Vec<f64>, ContinuousSearchError> {
        if normalized.len() != self.dimensions()
            || normalized
                .iter()
                .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            return Err(ContinuousSearchError::new(
                "normalized continuous values are invalid",
            ));
        }
        Ok(self.denormalize(normalized))
    }
}

#[derive(Clone, Debug)]
pub struct ContinuousOptimizer {
    template: ContinuousTemplate,
    config: ContinuousOptimizerConfig,
    generation: u32,
    mean: Vec<f64>,
    covariance: Vec<Vec<f64>>,
    sigma: f64,
    evolution_path_c: Vec<f64>,
    evolution_path_sigma: Vec<f64>,
    rng: DeterministicRng,
}

// Covariance matrices and evolution paths use explicit stable indices; their
// updates intentionally mirror the mathematical row/column notation.
#[allow(clippy::needless_range_loop)]
impl ContinuousOptimizer {
    pub fn new(
        template: ContinuousTemplate,
        config: ContinuousOptimizerConfig,
    ) -> Result<Self, ContinuousSearchError> {
        if config.population_size < 2
            || config.population_size > MAX_CONTINUOUS_POPULATION
            || config.elite_count == 0
            || config.elite_count > config.population_size
            || !config.initial_sigma.is_finite()
            || !(0.000_001..=1.0).contains(&config.initial_sigma)
        {
            return Err(ContinuousSearchError::new(
                "continuous optimizer population, elite count, or sigma is invalid",
            ));
        }
        let dimensions = template.dimensions();
        let mean = template.normalize(template.initial_values());
        let mut covariance = identity(dimensions);
        let sigma = match config.method {
            ContinuousMethod::CrossEntropy => 1.0,
            ContinuousMethod::CmaEs => config.initial_sigma,
        };
        if config.method == ContinuousMethod::CrossEntropy {
            for (index, row) in covariance.iter_mut().enumerate() {
                row[index] = config.initial_sigma * config.initial_sigma;
            }
        }
        Ok(Self {
            template,
            config,
            generation: 0,
            mean,
            covariance,
            sigma,
            evolution_path_c: vec![0.0; dimensions],
            evolution_path_sigma: vec![0.0; dimensions],
            rng: DeterministicRng::new(config.seed),
        })
    }

    pub fn ask(&mut self) -> Result<Vec<ContinuousSample>, ContinuousSearchError> {
        let transform = cholesky_with_jitter(&self.covariance)?;
        let mut samples = Vec::with_capacity(self.config.population_size);
        for sample_index in 0..self.config.population_size {
            let normalized = if sample_index == 0 {
                self.mean.clone()
            } else {
                let z = (0..self.mean.len())
                    .map(|_| self.rng.normal())
                    .collect::<Vec<_>>();
                let displacement = matrix_vector(&transform, &z);
                self.mean
                    .iter()
                    .zip(displacement)
                    .map(|(mean, value)| {
                        let scale = if self.config.method == ContinuousMethod::CmaEs {
                            self.sigma
                        } else {
                            1.0
                        };
                        (mean + scale * value).clamp(0.0, 1.0)
                    })
                    .collect()
            };
            samples.push(ContinuousSample {
                generation: self.generation,
                sample_index,
                values: self.template.denormalize(&normalized),
                normalized,
            });
        }
        Ok(samples)
    }

    /// Updates from samples sorted best-to-worst by external native rollout
    /// evidence. Fitness magnitudes are intentionally absent; both optimizers
    /// consume only deterministic rank.
    pub fn tell(&mut self, ranked: &[ContinuousSample]) -> Result<(), ContinuousSearchError> {
        if ranked.len() < self.config.elite_count
            || ranked.iter().any(|sample| {
                sample.generation != self.generation
                    || sample.normalized.len() != self.mean.len()
                    || sample.normalized.iter().any(|value| !value.is_finite())
            })
        {
            return Err(ContinuousSearchError::new(
                "continuous optimizer received incomplete or stale ranked samples",
            ));
        }
        match self.config.method {
            ContinuousMethod::CrossEntropy => self.tell_cem(ranked),
            ContinuousMethod::CmaEs => self.tell_cma_es(ranked)?,
        }
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| ContinuousSearchError::new("optimizer generation overflowed"))?;
        Ok(())
    }

    pub fn snapshot(&self) -> ContinuousOptimizerSnapshot {
        ContinuousOptimizerSnapshot {
            method: self.config.method,
            generation: self.generation,
            mean: self.template.denormalize(&self.mean),
            normalized_mean: self.mean.clone(),
            sigma: self.sigma,
            covariance: self.covariance.clone(),
        }
    }

    fn tell_cem(&mut self, ranked: &[ContinuousSample]) {
        let elites = &ranked[..self.config.elite_count];
        let dimensions = self.mean.len();
        let mut elite_mean = vec![0.0; dimensions];
        for sample in elites {
            for (output, value) in elite_mean.iter_mut().zip(&sample.normalized) {
                *output += value / elites.len() as f64;
            }
        }
        let mut elite_covariance = vec![vec![0.0; dimensions]; dimensions];
        for sample in elites {
            let delta = subtract(&sample.normalized, &elite_mean);
            add_outer(&mut elite_covariance, &delta, 1.0 / elites.len() as f64);
        }
        const SMOOTHING: f64 = 0.7;
        for index in 0..dimensions {
            self.mean[index] = (1.0 - SMOOTHING) * self.mean[index] + SMOOTHING * elite_mean[index];
            for column in 0..dimensions {
                self.covariance[index][column] = (1.0 - SMOOTHING) * self.covariance[index][column]
                    + SMOOTHING * elite_covariance[index][column];
            }
            self.covariance[index][index] = self.covariance[index][index].max(1.0e-10);
        }
        symmetrize(&mut self.covariance);
    }

    fn tell_cma_es(&mut self, ranked: &[ContinuousSample]) -> Result<(), ContinuousSearchError> {
        let dimensions = self.mean.len();
        let mu = self.config.elite_count;
        let mut weights = (1..=mu)
            .map(|rank| ((mu as f64 + 0.5).ln() - (rank as f64).ln()).max(0.0))
            .collect::<Vec<_>>();
        let weight_sum: f64 = weights.iter().sum();
        for weight in &mut weights {
            *weight /= weight_sum;
        }
        let mu_eff = 1.0 / weights.iter().map(|weight| weight * weight).sum::<f64>();
        let n = dimensions as f64;
        let cc = (4.0 + mu_eff / n) / (n + 4.0 + 2.0 * mu_eff / n);
        let cs = (mu_eff + 2.0) / (n + mu_eff + 5.0);
        let c1 = 2.0 / ((n + 1.3).powi(2) + mu_eff);
        let cmu =
            (1.0 - c1).min(2.0 * (mu_eff - 2.0 + 1.0 / mu_eff) / ((n + 2.0).powi(2) + mu_eff));
        let damping = 1.0 + 2.0 * (((mu_eff - 1.0) / (n + 1.0)).sqrt() - 1.0).max(0.0) + cs;
        let old_mean = self.mean.clone();
        self.mean.fill(0.0);
        for (sample, weight) in ranked.iter().take(mu).zip(&weights) {
            for (output, value) in self.mean.iter_mut().zip(&sample.normalized) {
                *output += weight * value;
            }
        }
        let y_mean = subtract(&self.mean, &old_mean)
            .into_iter()
            .map(|value| value / self.sigma)
            .collect::<Vec<_>>();
        let cholesky = cholesky_with_jitter(&self.covariance)?;
        let whitened = solve_lower(&cholesky, &y_mean)?;
        let path_scale = (cs * (2.0 - cs) * mu_eff).sqrt();
        for index in 0..dimensions {
            self.evolution_path_sigma[index] =
                (1.0 - cs) * self.evolution_path_sigma[index] + path_scale * whitened[index];
        }
        let path_norm = norm(&self.evolution_path_sigma);
        let expected_norm = n.sqrt() * (1.0 - 1.0 / (4.0 * n) + 1.0 / (21.0 * n * n));
        let generation_factor =
            (1.0 - (1.0 - cs).powf(2.0 * (self.generation as f64 + 1.0))).sqrt();
        let hsig = path_norm / generation_factor < (1.4 + 2.0 / (n + 1.0)) * expected_norm;
        let covariance_path_scale = (cc * (2.0 - cc) * mu_eff).sqrt();
        for index in 0..dimensions {
            self.evolution_path_c[index] = (1.0 - cc) * self.evolution_path_c[index]
                + f64::from(hsig) * covariance_path_scale * y_mean[index];
        }
        let old_covariance = self.covariance.clone();
        let rank_one_decay = if hsig { 0.0 } else { c1 * cc * (2.0 - cc) };
        let mut updated = old_covariance.clone();
        for row in &mut updated {
            for value in row {
                *value *= 1.0 - c1 - cmu + rank_one_decay;
            }
        }
        add_outer(&mut updated, &self.evolution_path_c, c1);
        for (sample, weight) in ranked.iter().take(mu).zip(weights) {
            let y = subtract(&sample.normalized, &old_mean)
                .into_iter()
                .map(|value| value / self.sigma)
                .collect::<Vec<_>>();
            add_outer(&mut updated, &y, cmu * weight);
        }
        for (index, row) in updated.iter_mut().enumerate() {
            row[index] = row[index].max(1.0e-12);
        }
        symmetrize(&mut updated);
        self.covariance = updated;
        self.sigma *= ((cs / damping) * (path_norm / expected_norm - 1.0)).exp();
        self.sigma = self.sigma.clamp(1.0e-6, 2.0);
        Ok(())
    }
}

fn read_parameter(
    candidate: &Candidate,
    axis: &ContinuousAxis,
) -> Result<f64, ContinuousSearchError> {
    let action = candidate.actions.get(axis.action_index).ok_or_else(|| {
        ContinuousSearchError::new("continuous axis action index is out of range")
    })?;
    match (&axis.parameter, action) {
        (ContinuousParameter::MoveHeadingDegrees, MacroAction::Move { angle_degrees, .. })
        | (ContinuousParameter::RollHeadingDegrees, MacroAction::Roll { angle_degrees, .. }) => {
            Ok(f64::from(*angle_degrees))
        }
        (ContinuousParameter::MoveMagnitude, MacroAction::Move { magnitude, .. })
        | (ContinuousParameter::RollMagnitude, MacroAction::Roll { magnitude, .. }) => {
            Ok(f64::from(*magnitude))
        }
        (ContinuousParameter::MoveDuration, MacroAction::Move { frames, .. }) => {
            Ok(f64::from(*frames))
        }
        (ContinuousParameter::RollButtonFrame, MacroAction::Roll { button_frame, .. }) => {
            Ok(f64::from(*button_frame))
        }
        (
            ContinuousParameter::RollRecoveryFrames,
            MacroAction::Roll {
                recovery_frames, ..
            },
        ) => Ok(f64::from(*recovery_frames)),
        (ContinuousParameter::MotionPathDuration, MacroAction::MotionPath { plan }) => {
            Ok(f64::from(plan.duration_ticks))
        }
        (ContinuousParameter::MotionPathSamplePhaseNumerator, MacroAction::MotionPath { plan }) => {
            Ok(f64::from(plan.sample_phase.numerator))
        }
        (
            ContinuousParameter::MotionPathPointX { point_index },
            MacroAction::MotionPath { plan },
        ) => Ok(f64::from(path_point(plan, *point_index)?.x)),
        (
            ContinuousParameter::MotionPathPointY { point_index },
            MacroAction::MotionPath { plan },
        ) => Ok(f64::from(path_point(plan, *point_index)?.y)),
        _ => Err(ContinuousSearchError::new(
            "continuous axis parameter does not match its candidate action",
        )),
    }
}

fn write_parameter(
    candidate: &mut Candidate,
    axis: &ContinuousAxis,
    value: f64,
) -> Result<(), ContinuousSearchError> {
    let action = candidate
        .actions
        .get_mut(axis.action_index)
        .ok_or_else(|| {
            ContinuousSearchError::new("continuous axis action index is out of range")
        })?;
    let rounded = value.round();
    match (&axis.parameter, action) {
        (ContinuousParameter::MoveHeadingDegrees, MacroAction::Move { angle_degrees, .. })
        | (ContinuousParameter::RollHeadingDegrees, MacroAction::Roll { angle_degrees, .. }) => {
            *angle_degrees = convert_i16(rounded)?;
        }
        (ContinuousParameter::MoveMagnitude, MacroAction::Move { magnitude, .. })
        | (ContinuousParameter::RollMagnitude, MacroAction::Roll { magnitude, .. }) => {
            *magnitude = convert_u8(rounded)?;
        }
        (ContinuousParameter::MoveDuration, MacroAction::Move { frames, .. }) => {
            *frames = convert_u32(rounded)?;
        }
        (ContinuousParameter::RollButtonFrame, MacroAction::Roll { button_frame, .. }) => {
            *button_frame = convert_u32(rounded)?;
        }
        (
            ContinuousParameter::RollRecoveryFrames,
            MacroAction::Roll {
                recovery_frames, ..
            },
        ) => {
            *recovery_frames = convert_u32(rounded)?;
        }
        (ContinuousParameter::MotionPathDuration, MacroAction::MotionPath { plan }) => {
            plan.duration_ticks = convert_u32(rounded)?;
        }
        (ContinuousParameter::MotionPathSamplePhaseNumerator, MacroAction::MotionPath { plan }) => {
            plan.sample_phase.numerator = convert_u32(rounded)?
        }
        (
            ContinuousParameter::MotionPathPointX { point_index },
            MacroAction::MotionPath { plan },
        ) => {
            path_point_mut(plan, *point_index)?.x = convert_i16(rounded)?;
        }
        (
            ContinuousParameter::MotionPathPointY { point_index },
            MacroAction::MotionPath { plan },
        ) => {
            path_point_mut(plan, *point_index)?.y = convert_i16(rounded)?;
        }
        _ => {
            return Err(ContinuousSearchError::new(
                "continuous axis parameter does not match its candidate action",
            ));
        }
    }
    Ok(())
}

fn path_point(
    plan: &crate::motion_path::MotionPathPlan,
    index: usize,
) -> Result<&crate::motion_path::StickPoint, ContinuousSearchError> {
    let points: &[crate::motion_path::StickPoint] = match &plan.path {
        StickPath::Waypoint { points }
        | StickPath::Rail { points }
        | StickPath::Spline { points } => points,
        StickPath::Bezier { control } => control,
    };
    points
        .get(index)
        .ok_or_else(|| ContinuousSearchError::new("motion-path point axis is out of range"))
}

fn path_point_mut(
    plan: &mut crate::motion_path::MotionPathPlan,
    index: usize,
) -> Result<&mut crate::motion_path::StickPoint, ContinuousSearchError> {
    let points: &mut [crate::motion_path::StickPoint] = match &mut plan.path {
        StickPath::Waypoint { points }
        | StickPath::Rail { points }
        | StickPath::Spline { points } => points,
        StickPath::Bezier { control } => control,
    };
    points
        .get_mut(index)
        .ok_or_else(|| ContinuousSearchError::new("motion-path point axis is out of range"))
}

fn convert_i16(value: f64) -> Result<i16, ContinuousSearchError> {
    if value < f64::from(i16::MIN) || value > f64::from(i16::MAX) {
        Err(ContinuousSearchError::new(
            "continuous value does not fit i16",
        ))
    } else {
        Ok(value as i16)
    }
}

fn convert_u8(value: f64) -> Result<u8, ContinuousSearchError> {
    if value < 0.0 || value > f64::from(u8::MAX) {
        Err(ContinuousSearchError::new(
            "continuous value does not fit u8",
        ))
    } else {
        Ok(value as u8)
    }
}

fn convert_u32(value: f64) -> Result<u32, ContinuousSearchError> {
    if value < 0.0 || value > f64::from(u32::MAX) {
        Err(ContinuousSearchError::new(
            "continuous value does not fit u32",
        ))
    } else {
        Ok(value as u32)
    }
}

fn identity(size: usize) -> Vec<Vec<f64>> {
    (0..size)
        .map(|row| (0..size).map(|column| f64::from(row == column)).collect())
        .collect()
}

fn cholesky_with_jitter(matrix: &[Vec<f64>]) -> Result<Vec<Vec<f64>>, ContinuousSearchError> {
    for attempt in 0..8 {
        let jitter = if attempt == 0 {
            0.0
        } else {
            10_f64.powi(attempt - 14)
        };
        if let Some(result) = cholesky(matrix, jitter) {
            return Ok(result);
        }
    }
    Err(ContinuousSearchError::new(
        "continuous covariance is not positive definite",
    ))
}

#[allow(clippy::needless_range_loop)]
fn cholesky(matrix: &[Vec<f64>], jitter: f64) -> Option<Vec<Vec<f64>>> {
    let size = matrix.len();
    let mut output = vec![vec![0.0; size]; size];
    for row in 0..size {
        for column in 0..=row {
            let mut sum = matrix[row][column];
            if row == column {
                sum += jitter;
            }
            for index in 0..column {
                sum -= output[row][index] * output[column][index];
            }
            if row == column {
                if !sum.is_finite() || sum <= 0.0 {
                    return None;
                }
                output[row][column] = sum.sqrt();
            } else {
                output[row][column] = sum / output[column][column];
            }
        }
    }
    Some(output)
}

fn matrix_vector(matrix: &[Vec<f64>], vector: &[f64]) -> Vec<f64> {
    matrix
        .iter()
        .map(|row| {
            row.iter()
                .zip(vector)
                .map(|(left, right)| left * right)
                .sum()
        })
        .collect()
}

fn solve_lower(matrix: &[Vec<f64>], vector: &[f64]) -> Result<Vec<f64>, ContinuousSearchError> {
    let mut output = vec![0.0; vector.len()];
    for row in 0..vector.len() {
        let mut value = vector[row];
        for column in 0..row {
            value -= matrix[row][column] * output[column];
        }
        if matrix[row][row] == 0.0 {
            return Err(ContinuousSearchError::new("singular covariance transform"));
        }
        output[row] = value / matrix[row][row];
    }
    Ok(output)
}

fn subtract(left: &[f64], right: &[f64]) -> Vec<f64> {
    left.iter()
        .zip(right)
        .map(|(left, right)| left - right)
        .collect()
}

fn add_outer(matrix: &mut [Vec<f64>], vector: &[f64], scale: f64) {
    for row in 0..vector.len() {
        for column in 0..vector.len() {
            matrix[row][column] += scale * vector[row] * vector[column];
        }
    }
}

#[allow(clippy::needless_range_loop)]
fn symmetrize(matrix: &mut [Vec<f64>]) {
    for row in 0..matrix.len() {
        for column in 0..row {
            let value = (matrix[row][column] + matrix[column][row]) * 0.5;
            matrix[row][column] = value;
            matrix[column][row] = value;
        }
    }
}

fn norm(vector: &[f64]) -> f64 {
    vector.iter().map(|value| value * value).sum::<f64>().sqrt()
}

#[derive(Clone, Debug)]
struct DeterministicRng(u64);

impl DeterministicRng {
    fn new(seed: u64) -> Self {
        Self(seed ^ 0x9e37_79b9_7f4a_7c15)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    fn uniform_open(&mut self) -> f64 {
        (((self.next() >> 11) as f64) + 0.5) / ((1_u64 << 53) as f64)
    }

    fn normal(&mut self) -> f64 {
        let radius = (-2.0 * self.uniform_open().ln()).sqrt();
        radius * (TAU * self.uniform_open()).cos()
    }
}

#[derive(Clone, Debug)]
pub struct ContinuousSearchError(String);

impl ContinuousSearchError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ContinuousSearchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ContinuousSearchError {}

impl From<SearchError> for ContinuousSearchError {
    fn from(value: SearchError) -> Self {
        Self(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{Ancestry, CANDIDATE_SCHEMA, SegmentProfile};
    use crate::tape::TapeBoot;

    fn template() -> ContinuousTemplate {
        ContinuousTemplate::new(
            Candidate {
                schema: CANDIDATE_SCHEMA.into(),
                segment: SegmentProfile::BootToFsp103,
                boot: TapeBoot::Process,
                actions: vec![MacroAction::Move {
                    angle_degrees: 0,
                    magnitude: 64,
                    frames: 30,
                }],
                ancestry: Ancestry::default(),
            },
            ContinuousAxes {
                schema: CONTINUOUS_AXES_SCHEMA_V1.into(),
                axes: vec![
                    ContinuousAxis {
                        name: "heading".into(),
                        action_index: 0,
                        parameter: ContinuousParameter::MoveHeadingDegrees,
                        minimum: -90.0,
                        maximum: 90.0,
                    },
                    ContinuousAxis {
                        name: "magnitude".into(),
                        action_index: 0,
                        parameter: ContinuousParameter::MoveMagnitude,
                        minimum: 1.0,
                        maximum: 127.0,
                    },
                ],
            },
        )
        .unwrap()
    }

    #[test]
    fn cem_and_cma_es_are_seeded_bounded_and_move_toward_ranked_optima() {
        for method in [ContinuousMethod::CrossEntropy, ContinuousMethod::CmaEs] {
            let config = ContinuousOptimizerConfig {
                method,
                population_size: 32,
                elite_count: 8,
                initial_sigma: 0.3,
                seed: 7,
            };
            let mut first = ContinuousOptimizer::new(template(), config).unwrap();
            let mut second = ContinuousOptimizer::new(template(), config).unwrap();
            for _ in 0..12 {
                let mut samples = first.ask().unwrap();
                assert_eq!(samples, second.ask().unwrap());
                samples.sort_by(|left, right| {
                    let objective = |sample: &ContinuousSample| {
                        (sample.values[0] - 45.0).powi(2) + (sample.values[1] - 100.0).powi(2)
                    };
                    objective(left).total_cmp(&objective(right))
                });
                first.tell(&samples).unwrap();
                second.tell(&samples).unwrap();
            }
            let snapshot = first.snapshot();
            assert!((snapshot.mean[0] - 45.0).abs() < 8.0, "{method:?}");
            assert!((snapshot.mean[1] - 100.0).abs() < 8.0, "{method:?}");
            assert_eq!(snapshot.mean, second.snapshot().mean);
            assert!(
                snapshot
                    .normalized_mean
                    .iter()
                    .all(|value| (0.0..=1.0).contains(value))
            );
        }
    }

    #[test]
    fn typed_axes_round_and_validate_the_exact_candidate() {
        let template = template();
        let candidate = template.candidate(&[44.6, 99.6]).unwrap();
        assert!(matches!(
            candidate.actions[0],
            MacroAction::Move {
                angle_degrees: 45,
                magnitude: 100,
                frames: 30
            }
        ));
        assert!(template.candidate(&[91.0, 100.0]).is_err());
    }
}
