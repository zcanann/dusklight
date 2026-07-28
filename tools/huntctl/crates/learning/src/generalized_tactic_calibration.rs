//! Group-isolated calibration for the local continuous tactic-value model.
//!
//! Route rows are strongly correlated. This evaluator never assigns individual
//! rows randomly: one experiment withholds complete spatial regions and a
//! second withholds complete executable action realizations. A validation
//! group calibrates a conformal return radius and a disjoint test group
//! measures value error, interval coverage, and action-ranking regret.

use crate::artifact::Digest;
use crate::generalized_tactic_value::fitted_q::fit_transition_returns;
use crate::generalized_tactic_value::{
    GeneralizedTacticContext, GeneralizedTacticValueError, GeneralizedTacticValueModel,
};
use crate::option_transition::OptionTransitionSample;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const GENERALIZED_TACTIC_CALIBRATION_SCHEMA_V1: &str =
    "dusklight-generalized-tactic-calibration/v1";
const MINIMUM_UNCERTAINTY_SCALE: f64 = 1.0e-6;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeneralizedTacticCalibrationConfig {
    pub state_region_width: f32,
    pub group_folds: u8,
    pub validation_fold: u8,
    pub test_fold: u8,
    pub interval_coverage_millionths: u32,
    pub fitted_q_iterations: u16,
    pub per_tick_discount: f32,
}

impl Default for GeneralizedTacticCalibrationConfig {
    fn default() -> Self {
        Self {
            state_region_width: 256.0,
            group_folds: 5,
            validation_fold: 0,
            test_fold: 1,
            interval_coverage_millionths: 900_000,
            fitted_q_iterations: 64,
            per_tick_discount: 0.999,
        }
    }
}

impl GeneralizedTacticCalibrationConfig {
    pub fn validate(self) -> Result<(), GeneralizedTacticCalibrationError> {
        if !self.state_region_width.is_finite()
            || self.state_region_width <= 0.0
            || self.group_folds < 3
            || self.validation_fold >= self.group_folds
            || self.test_fold >= self.group_folds
            || self.validation_fold == self.test_fold
            || !(500_000..1_000_000).contains(&self.interval_coverage_millionths)
            || self.fitted_q_iterations == 0
            || !self.per_tick_discount.is_finite()
            || !(0.0..=1.0).contains(&self.per_tick_discount)
        {
            return Err(GeneralizedTacticCalibrationError::new(
                "generalized tactic calibration configuration is invalid",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GeneralizedTacticCalibrationAxis {
    StateRegion,
    ActionRealization,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeneralizedTacticCalibrationMetrics {
    pub samples: usize,
    pub mean_error: f64,
    pub mean_absolute_error: f64,
    pub root_mean_squared_error: f64,
    pub mean_raw_uncertainty: f64,
    pub interval_coverage: f64,
    pub mean_interval_radius: f64,
    pub comparable_states: usize,
    pub ranking_wins: usize,
    pub ranking_win_rate: Option<f64>,
    pub mean_observed_regret: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeneralizedTacticAxisCalibration {
    pub axis: GeneralizedTacticCalibrationAxis,
    pub training_samples: usize,
    pub training_groups: Vec<String>,
    pub validation_groups: Vec<String>,
    pub test_groups: Vec<String>,
    pub group_overlap_count: usize,
    pub validation: GeneralizedTacticCalibrationMetrics,
    pub test: GeneralizedTacticCalibrationMetrics,
    pub conformal_multiplier: f64,
    pub nominal_interval_coverage: f64,
    pub test_coverage_at_least_nominal: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeneralizedTacticCalibrationReport {
    pub schema: String,
    pub source_transition_sha256: Digest,
    pub source_transitions: usize,
    pub goal_distance_feature: usize,
    pub target_kind: String,
    pub config: GeneralizedTacticCalibrationConfig,
    pub state_region: GeneralizedTacticAxisCalibration,
    pub action_realization: GeneralizedTacticAxisCalibration,
    pub report_sha256: Digest,
}

impl GeneralizedTacticCalibrationReport {
    pub fn validate(&self) -> Result<(), GeneralizedTacticCalibrationError> {
        self.config.validate()?;
        if self.schema != GENERALIZED_TACTIC_CALIBRATION_SCHEMA_V1
            || self.source_transition_sha256 == Digest::ZERO
            || self.source_transitions < 5
            || self.target_kind != "full_authenticated_transition_graph_fitted_q"
            || self.state_region.axis != GeneralizedTacticCalibrationAxis::StateRegion
            || self.action_realization.axis != GeneralizedTacticCalibrationAxis::ActionRealization
            || self.state_region.nominal_interval_coverage
                != f64::from(self.config.interval_coverage_millionths) / 1_000_000.0
            || self.action_realization.nominal_interval_coverage
                != f64::from(self.config.interval_coverage_millionths) / 1_000_000.0
            || !axis_is_valid(&self.state_region)
            || !axis_is_valid(&self.action_realization)
            || self.report_sha256 == Digest::ZERO
            || self.report_sha256 != self.digest()?
        {
            return Err(GeneralizedTacticCalibrationError::new(
                "generalized tactic calibration report is invalid or detached",
            ));
        }
        Ok(())
    }

    fn digest(&self) -> Result<Digest, GeneralizedTacticCalibrationError> {
        canonical_digest(
            b"dusklight.generalized-tactic-calibration/v1\0",
            &(
                &self.schema,
                self.source_transition_sha256,
                self.source_transitions,
                self.goal_distance_feature,
                &self.target_kind,
                self.config,
                &self.state_region,
                &self.action_realization,
            ),
        )
    }
}

#[derive(Clone)]
struct CalibrationRow<'a> {
    transition: &'a OptionTransitionSample,
    target: f64,
    state_region: String,
    action_realization: String,
}

#[derive(Clone)]
struct PredictionRow<'a> {
    transition: &'a OptionTransitionSample,
    target: f64,
    predicted: f64,
    raw_uncertainty: f64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneralizedTacticCalibrationError(String);

impl GeneralizedTacticCalibrationError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for GeneralizedTacticCalibrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for GeneralizedTacticCalibrationError {}

impl From<GeneralizedTacticValueError> for GeneralizedTacticCalibrationError {
    fn from(error: GeneralizedTacticValueError) -> Self {
        Self::new(error.to_string())
    }
}

pub fn calibrate_generalized_tactic_value(
    transitions: &[OptionTransitionSample],
    goal_distance_feature: usize,
    config: GeneralizedTacticCalibrationConfig,
) -> Result<GeneralizedTacticCalibrationReport, GeneralizedTacticCalibrationError> {
    config.validate()?;
    if transitions.len() < 5 {
        return Err(GeneralizedTacticCalibrationError::new(
            "generalized tactic calibration requires at least five transitions",
        ));
    }
    for transition in transitions {
        transition
            .validate()
            .map_err(|error| GeneralizedTacticCalibrationError::new(error.to_string()))?;
        if goal_distance_feature >= transition.value_sample.state.len() {
            return Err(GeneralizedTacticCalibrationError::new(
                "goal-distance feature is outside the transition state",
            ));
        }
    }
    let fitted = fit_transition_returns(
        transitions,
        usize::from(config.fitted_q_iterations),
        config.per_tick_discount,
    )?;
    let rows = transitions
        .iter()
        .zip(fitted.values)
        .map(|(transition, target)| {
            Ok(CalibrationRow {
                transition,
                target: f64::from(target),
                state_region: state_region(transition, config.state_region_width)?,
                action_realization: transition
                    .value_sample
                    .action
                    .content_sha256()
                    .map_err(|error| GeneralizedTacticCalibrationError::new(error.to_string()))?
                    .to_string(),
            })
        })
        .collect::<Result<Vec<_>, GeneralizedTacticCalibrationError>>()?;
    let state_region = calibrate_axis(
        &rows,
        goal_distance_feature,
        config,
        GeneralizedTacticCalibrationAxis::StateRegion,
    )?;
    let action_realization = calibrate_axis(
        &rows,
        goal_distance_feature,
        config,
        GeneralizedTacticCalibrationAxis::ActionRealization,
    )?;
    let mut report = GeneralizedTacticCalibrationReport {
        schema: GENERALIZED_TACTIC_CALIBRATION_SCHEMA_V1.into(),
        source_transition_sha256: transition_corpus_digest(transitions)?,
        source_transitions: transitions.len(),
        goal_distance_feature,
        target_kind: "full_authenticated_transition_graph_fitted_q".into(),
        config,
        state_region,
        action_realization,
        report_sha256: Digest::ZERO,
    };
    report.report_sha256 = report.digest()?;
    report.validate()?;
    Ok(report)
}

fn calibrate_axis(
    rows: &[CalibrationRow<'_>],
    goal_distance_feature: usize,
    config: GeneralizedTacticCalibrationConfig,
    axis: GeneralizedTacticCalibrationAxis,
) -> Result<GeneralizedTacticAxisCalibration, GeneralizedTacticCalibrationError> {
    let groups = rows
        .iter()
        .map(|row| row_group(row, axis))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if groups.len() < usize::from(config.group_folds) {
        return Err(GeneralizedTacticCalibrationError::new(format!(
            "{axis:?} calibration has fewer groups than declared folds"
        )));
    }
    let mut training_groups = Vec::new();
    let mut validation_groups = Vec::new();
    let mut test_groups = Vec::new();
    for (index, group) in groups.into_iter().enumerate() {
        match (index % usize::from(config.group_folds)) as u8 {
            fold if fold == config.validation_fold => validation_groups.push(group),
            fold if fold == config.test_fold => test_groups.push(group),
            _ => training_groups.push(group),
        }
    }
    let training_set = training_groups.iter().collect::<BTreeSet<_>>();
    let validation_set = validation_groups.iter().collect::<BTreeSet<_>>();
    let test_set = test_groups.iter().collect::<BTreeSet<_>>();
    let training = rows
        .iter()
        .filter(|row| training_set.contains(row_group(row, axis)))
        .map(|row| row.transition.clone())
        .collect::<Vec<_>>();
    let validation = rows
        .iter()
        .filter(|row| validation_set.contains(row_group(row, axis)))
        .cloned()
        .collect::<Vec<_>>();
    let test = rows
        .iter()
        .filter(|row| test_set.contains(row_group(row, axis)))
        .cloned()
        .collect::<Vec<_>>();
    if training.len() < 2 || validation.is_empty() || test.is_empty() {
        return Err(GeneralizedTacticCalibrationError::new(format!(
            "{axis:?} calibration produced an empty or undersized partition"
        )));
    }
    let model = GeneralizedTacticValueModel::fit_fitted_q_transitions(
        &training,
        goal_distance_feature,
        usize::from(config.fitted_q_iterations),
        config.per_tick_discount,
    )?;
    let training_targets = rows
        .iter()
        .filter(|row| training_set.contains(row_group(row, axis)))
        .map(|row| row.target)
        .collect::<Vec<_>>();
    let return_scale = standard_deviation(&training_targets).max(MINIMUM_UNCERTAINTY_SCALE);
    let validation_predictions = predict_rows(&model, &validation, return_scale)?;
    let nominal = f64::from(config.interval_coverage_millionths) / 1_000_000.0;
    let multiplier = conformal_multiplier(&validation_predictions, nominal);
    let validation_metrics = metrics(&validation_predictions, multiplier);
    let test_predictions = predict_rows(&model, &test, return_scale)?;
    let test_metrics = metrics(&test_predictions, multiplier);
    let overlap_count = group_overlap_count(&training_set, &validation_set, &test_set);
    Ok(GeneralizedTacticAxisCalibration {
        axis,
        training_samples: training.len(),
        training_groups,
        validation_groups,
        test_groups,
        group_overlap_count: overlap_count,
        validation: validation_metrics,
        test_coverage_at_least_nominal: test_metrics.interval_coverage >= nominal,
        test: test_metrics,
        conformal_multiplier: multiplier,
        nominal_interval_coverage: nominal,
    })
}

fn row_group<'a>(
    row: &'a CalibrationRow<'_>,
    axis: GeneralizedTacticCalibrationAxis,
) -> &'a String {
    match axis {
        GeneralizedTacticCalibrationAxis::StateRegion => &row.state_region,
        GeneralizedTacticCalibrationAxis::ActionRealization => &row.action_realization,
    }
}

fn predict_rows<'a>(
    model: &GeneralizedTacticValueModel,
    rows: &[CalibrationRow<'a>],
    return_scale: f64,
) -> Result<Vec<PredictionRow<'a>>, GeneralizedTacticCalibrationError> {
    rows.iter()
        .map(|row| {
            let context = GeneralizedTacticContext::from_facts(&row.transition.before)?;
            let estimate = model.predict(
                &row.transition.value_sample.state,
                &context,
                &row.transition.value_sample.action,
            )?;
            let neighbor_term = 1.0 / (estimate.neighbors.max(1) as f64).sqrt();
            let distance_term = f64::from(estimate.nearest_distance.max(0.0)).sqrt();
            Ok(PredictionRow {
                transition: row.transition,
                target: row.target,
                predicted: f64::from(estimate.outcome.reward),
                raw_uncertainty: (return_scale * (neighbor_term + distance_term))
                    .max(MINIMUM_UNCERTAINTY_SCALE),
            })
        })
        .collect()
}

fn conformal_multiplier(rows: &[PredictionRow<'_>], coverage: f64) -> f64 {
    let mut ratios = rows
        .iter()
        .map(|row| (row.predicted - row.target).abs() / row.raw_uncertainty)
        .collect::<Vec<_>>();
    ratios.sort_by(f64::total_cmp);
    let rank = (((ratios.len() + 1) as f64 * coverage).ceil() as usize)
        .saturating_sub(1)
        .min(ratios.len() - 1);
    ratios[rank].max(1.0)
}

fn metrics(rows: &[PredictionRow<'_>], multiplier: f64) -> GeneralizedTacticCalibrationMetrics {
    let count = rows.len() as f64;
    let errors = rows
        .iter()
        .map(|row| row.predicted - row.target)
        .collect::<Vec<_>>();
    let radii = rows
        .iter()
        .map(|row| row.raw_uncertainty * multiplier)
        .collect::<Vec<_>>();
    let (comparable_states, ranking_wins, observed_regret) = ranking_metrics(rows);
    GeneralizedTacticCalibrationMetrics {
        samples: rows.len(),
        mean_error: errors.iter().sum::<f64>() / count,
        mean_absolute_error: errors.iter().map(|error| error.abs()).sum::<f64>() / count,
        root_mean_squared_error: (errors.iter().map(|error| error.powi(2)).sum::<f64>() / count)
            .sqrt(),
        mean_raw_uncertainty: rows.iter().map(|row| row.raw_uncertainty).sum::<f64>() / count,
        interval_coverage: errors
            .iter()
            .zip(&radii)
            .filter(|(error, radius)| error.abs() <= **radius)
            .count() as f64
            / count,
        mean_interval_radius: radii.iter().sum::<f64>() / count,
        comparable_states,
        ranking_wins,
        ranking_win_rate: (comparable_states != 0)
            .then_some(ranking_wins as f64 / comparable_states as f64),
        mean_observed_regret: (comparable_states != 0)
            .then_some(observed_regret / comparable_states as f64),
    }
}

fn ranking_metrics(rows: &[PredictionRow<'_>]) -> (usize, usize, f64) {
    let mut by_state = BTreeMap::<Digest, Vec<&PredictionRow<'_>>>::new();
    for row in rows {
        by_state
            .entry(row.transition.before_state_sha256)
            .or_default()
            .push(row);
    }
    let mut comparable = 0;
    let mut wins = 0;
    let mut regret = 0.0;
    for state_rows in by_state.values().filter(|rows| rows.len() >= 2) {
        comparable += 1;
        let selected = state_rows
            .iter()
            .max_by(|left, right| left.predicted.total_cmp(&right.predicted))
            .expect("nonempty state rows");
        let best = state_rows
            .iter()
            .map(|row| row.target)
            .max_by(f64::total_cmp)
            .expect("nonempty state rows");
        wins += usize::from(selected.target == best);
        regret += best - selected.target;
    }
    (comparable, wins, regret)
}

fn state_region(
    transition: &OptionTransitionSample,
    width: f32,
) -> Result<String, GeneralizedTacticCalibrationError> {
    let facts = &transition.before;
    let x = f32::from_bits(facts.player.position_f32_bits[0]);
    let z = f32::from_bits(facts.player.position_f32_bits[2]);
    if !x.is_finite() || !z.is_finite() {
        return Err(GeneralizedTacticCalibrationError::new(
            "state-region position is non-finite",
        ));
    }
    Ok(format!(
        "{}:{}:{}:{}:{}:{}",
        facts.world.stage,
        facts.world.room,
        facts.world.layer.unwrap_or(i8::MIN),
        facts.player.procedure.unwrap_or(u16::MAX),
        (x / width).floor() as i64,
        (z / width).floor() as i64,
    ))
}

fn group_overlap_count(
    training: &BTreeSet<&String>,
    validation: &BTreeSet<&String>,
    test: &BTreeSet<&String>,
) -> usize {
    training.intersection(validation).count()
        + training.intersection(test).count()
        + validation.intersection(test).count()
}

fn standard_deviation(values: &[f64]) -> f64 {
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    (values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / values.len() as f64)
        .sqrt()
}

fn transition_corpus_digest(
    transitions: &[OptionTransitionSample],
) -> Result<Digest, GeneralizedTacticCalibrationError> {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.generalized-tactic-calibration-source/v1\0");
    hasher.update((transitions.len() as u64).to_le_bytes());
    for transition in transitions {
        hasher.update(
            transition
                .replay_identity_sha256()
                .map_err(|error| GeneralizedTacticCalibrationError::new(error.to_string()))?
                .as_bytes(),
        );
    }
    Ok(Digest(hasher.finalize().into()))
}

fn axis_is_valid(axis: &GeneralizedTacticAxisCalibration) -> bool {
    let training = axis.training_groups.iter().collect::<BTreeSet<_>>();
    let validation = axis.validation_groups.iter().collect::<BTreeSet<_>>();
    let test = axis.test_groups.iter().collect::<BTreeSet<_>>();
    axis.training_samples >= 2
        && !axis.training_groups.is_empty()
        && !axis.validation_groups.is_empty()
        && !axis.test_groups.is_empty()
        && training.len() == axis.training_groups.len()
        && validation.len() == axis.validation_groups.len()
        && test.len() == axis.test_groups.len()
        && axis.group_overlap_count == group_overlap_count(&training, &validation, &test)
        && axis.group_overlap_count == 0
        && axis.conformal_multiplier.is_finite()
        && axis.conformal_multiplier >= 1.0
        && axis.nominal_interval_coverage > 0.0
        && axis.nominal_interval_coverage < 1.0
        && metrics_are_valid(&axis.validation)
        && metrics_are_valid(&axis.test)
        && axis.test_coverage_at_least_nominal
            == (axis.test.interval_coverage >= axis.nominal_interval_coverage)
}

fn metrics_are_valid(metrics: &GeneralizedTacticCalibrationMetrics) -> bool {
    metrics.samples > 0
        && metrics.comparable_states >= metrics.ranking_wins
        && [
            metrics.mean_error,
            metrics.mean_absolute_error,
            metrics.root_mean_squared_error,
            metrics.mean_raw_uncertainty,
            metrics.interval_coverage,
            metrics.mean_interval_radius,
        ]
        .iter()
        .all(|value| value.is_finite())
        && metrics.mean_absolute_error >= 0.0
        && metrics.root_mean_squared_error >= 0.0
        && metrics.mean_raw_uncertainty > 0.0
        && (0.0..=1.0).contains(&metrics.interval_coverage)
        && metrics.mean_interval_radius > 0.0
        && metrics
            .ranking_win_rate
            .is_none_or(|value| value.is_finite() && (0.0..=1.0).contains(&value))
        && metrics
            .mean_observed_regret
            .is_none_or(|value| value.is_finite() && value >= 0.0)
}

fn canonical_digest(
    domain: &[u8],
    value: &impl Serialize,
) -> Result<Digest, GeneralizedTacticCalibrationError> {
    let encoded = serde_json::to_vec(value)
        .map_err(|error| GeneralizedTacticCalibrationError::new(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((encoded.len() as u64).to_le_bytes());
    hasher.update(encoded);
    Ok(Digest(hasher.finalize().into()))
}

mod comparison;
pub use comparison::{
    GENERALIZED_TACTIC_CONTROL_COMPARISON_SCHEMA_V1, GeneralizedTacticControlComparisonReport,
    compare_generalized_tactic_controls,
};

#[cfg(test)]
mod tests;
