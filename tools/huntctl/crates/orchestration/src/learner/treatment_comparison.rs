use super::objective_double_q::GraphDoubleQObjectiveModel;
use super::{
    ActionConditionedGraphLearner, ExactGraphTableLearner, GraphLearnerContract, GraphLearnerError,
    GraphLearningBatch,
};
use crate::state_graph::ExactStateId;
use dusklight_automation_contracts::artifact::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

pub const GRAPH_TREATMENT_COMPARISON_SCHEMA_V1: &str = "dusklight-graph-treatment-comparison/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphObjectiveTreatment {
    DiscreteActionMean,
    StateKnn,
    DiscreteDoubleQ,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GraphTreatmentMetrics {
    pub treatment: GraphObjectiveTreatment,
    pub objective_targets: u64,
    pub objective_predictions: u64,
    pub mean_absolute_tick_error: u64,
    pub mean_error_millionths: u64,
    pub ranked_action_pairs: u64,
    pub correctly_ranked_action_pairs: u64,
    pub passed_gate: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GraphTreatmentComparisonReport {
    pub schema: String,
    pub graph_sha256: Digest,
    pub contract_sha256: Digest,
    pub required_objective_predictions: u64,
    pub maximum_tick_error_millionths: u64,
    pub required_ranked_pairs: u64,
    pub training_rows: u64,
    pub held_out_rows: u64,
    pub metrics: Vec<GraphTreatmentMetrics>,
    pub selected_treatment: Option<GraphObjectiveTreatment>,
    pub report_sha256: Digest,
}

#[derive(Default)]
struct MetricAccumulator {
    targets: u64,
    predictions: u64,
    absolute_error: u128,
    relative_error: u128,
    by_state: BTreeMap<ExactStateId, Vec<(u64, u64)>>,
}

impl GraphTreatmentComparisonReport {
    pub fn build(
        contract: &GraphLearnerContract,
        batch: &GraphLearningBatch,
    ) -> Result<Self, GraphLearnerError> {
        contract.validate()?;
        batch.validate()?;
        let sources = batch
            .rows
            .iter()
            .map(|row| row.source)
            .collect::<BTreeSet<_>>();
        if sources.len() < 2 {
            return Err(GraphLearnerError::Invalid(
                "treatment comparison requires at least two exact state groups",
            ));
        }
        let mut held_out_sources = sources
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(index, source)| ((index + 1) % 5 == 0).then_some(source))
            .collect::<BTreeSet<_>>();
        if held_out_sources.is_empty() {
            held_out_sources.insert(*sources.last().expect("validated sources are nonempty"));
        }
        let mut training = batch.clone();
        training
            .rows
            .retain(|row| !held_out_sources.contains(&row.source));
        let held_out = batch
            .rows
            .iter()
            .filter(|row| held_out_sources.contains(&row.source))
            .collect::<Vec<_>>();
        training.validate()?;
        let exact_learner = ExactGraphTableLearner;
        let snapshot = exact_learner.fit(contract, &training)?;
        let double_q = GraphDoubleQObjectiveModel::fit(&training)?;
        let mut accumulators = BTreeMap::<GraphObjectiveTreatment, MetricAccumulator>::from([
            (
                GraphObjectiveTreatment::DiscreteActionMean,
                MetricAccumulator::default(),
            ),
            (
                GraphObjectiveTreatment::StateKnn,
                MetricAccumulator::default(),
            ),
            (
                GraphObjectiveTreatment::DiscreteDoubleQ,
                MetricAccumulator::default(),
            ),
        ]);
        for row in &held_out {
            let Some(actual) = row.exact_conditional_ticks_to_terminal else {
                continue;
            };
            let action = row
                .action
                .content_sha256()
                .map_err(|error| GraphLearnerError::Action(error.to_string()))?;
            let predictions = [
                (
                    GraphObjectiveTreatment::DiscreteActionMean,
                    snapshot
                        .mean_objective_prediction(action)
                        .and_then(|estimate| estimate.conditional_ticks_to_terminal),
                ),
                (
                    GraphObjectiveTreatment::StateKnn,
                    snapshot
                        .generalized_objective_prediction(&row.source_features, action)
                        .and_then(|estimate| estimate.conditional_ticks_to_terminal),
                ),
                (
                    GraphObjectiveTreatment::DiscreteDoubleQ,
                    double_q
                        .as_ref()
                        .and_then(|model| model.predict(&row.source_features, action))
                        .and_then(|estimate| estimate.conditional_ticks_to_terminal),
                ),
            ];
            for (treatment, predicted) in predictions {
                let accumulator = accumulators
                    .get_mut(&treatment)
                    .expect("all graph treatments have accumulators");
                accumulator.targets += 1;
                if let Some(predicted) = predicted {
                    accumulator.predictions += 1;
                    accumulator.absolute_error = accumulator
                        .absolute_error
                        .saturating_add(u128::from(actual.abs_diff(predicted)));
                    accumulator.relative_error = accumulator
                        .relative_error
                        .saturating_add(u128::from(relative_tick_error(actual, predicted)));
                    accumulator
                        .by_state
                        .entry(row.source)
                        .or_default()
                        .push((actual, predicted));
                }
            }
        }
        let metrics = accumulators
            .into_iter()
            .map(|(treatment, accumulator)| metrics(contract, treatment, accumulator))
            .collect::<Vec<_>>();
        let selected_treatment = select_treatment(&metrics);
        let mut report = Self {
            schema: GRAPH_TREATMENT_COMPARISON_SCHEMA_V1.into(),
            graph_sha256: batch.graph_sha256,
            contract_sha256: contract.content_sha256()?,
            required_objective_predictions: contract.minimum_calibration_objective_predictions,
            maximum_tick_error_millionths: contract.maximum_calibration_tick_error_millionths,
            required_ranked_pairs: contract.minimum_calibration_ranked_pairs,
            training_rows: training.rows.len() as u64,
            held_out_rows: held_out.len() as u64,
            metrics,
            selected_treatment,
            report_sha256: Digest::ZERO,
        };
        report.report_sha256 = report.digest()?;
        report.validate()?;
        Ok(report)
    }

    pub fn validate(&self) -> Result<(), GraphLearnerError> {
        let expected_treatments = [
            GraphObjectiveTreatment::DiscreteActionMean,
            GraphObjectiveTreatment::StateKnn,
            GraphObjectiveTreatment::DiscreteDoubleQ,
        ];
        if self.schema != GRAPH_TREATMENT_COMPARISON_SCHEMA_V1
            || self.graph_sha256 == Digest::ZERO
            || self.contract_sha256 == Digest::ZERO
            || self.required_objective_predictions == 0
            || self.maximum_tick_error_millionths > 1_000_000
            || self.required_ranked_pairs == 0
            || self.training_rows == 0
            || self.held_out_rows == 0
            || self.metrics.len() != expected_treatments.len()
            || self
                .metrics
                .iter()
                .zip(expected_treatments)
                .any(|(metrics, treatment)| {
                    metrics.treatment != treatment
                        || metrics.objective_predictions > metrics.objective_targets
                        || metrics.correctly_ranked_action_pairs > metrics.ranked_action_pairs
                        || metrics.passed_gate
                            != (metrics.objective_predictions
                                >= self.required_objective_predictions
                                && metrics.objective_predictions == metrics.objective_targets
                                && metrics.mean_error_millionths
                                    <= self.maximum_tick_error_millionths
                                && metrics.ranked_action_pairs >= self.required_ranked_pairs)
                })
            || self.selected_treatment != select_treatment(&self.metrics)
            || self.report_sha256 == Digest::ZERO
            || self.report_sha256 != self.digest()?
        {
            return Err(GraphLearnerError::Invalid(
                "graph treatment comparison is detached",
            ));
        }
        Ok(())
    }

    fn digest(&self) -> Result<Digest, GraphLearnerError> {
        let mut canonical = self.clone();
        canonical.report_sha256 = Digest::ZERO;
        let raw = serde_cbor::to_vec(&canonical)
            .map_err(|error| GraphLearnerError::Serialization(error.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(GRAPH_TREATMENT_COMPARISON_SCHEMA_V1.as_bytes());
        hasher.update(raw);
        Ok(Digest(hasher.finalize().into()))
    }
}

fn select_treatment(metrics: &[GraphTreatmentMetrics]) -> Option<GraphObjectiveTreatment> {
    metrics
        .iter()
        .filter(|metrics| metrics.passed_gate)
        .min_by(|left, right| {
            left.mean_error_millionths
                .cmp(&right.mean_error_millionths)
                .then_with(|| {
                    right
                        .correctly_ranked_action_pairs
                        .cmp(&left.correctly_ranked_action_pairs)
                })
                .then_with(|| left.treatment.cmp(&right.treatment))
        })
        .map(|metrics| metrics.treatment)
}

fn metrics(
    contract: &GraphLearnerContract,
    treatment: GraphObjectiveTreatment,
    accumulator: MetricAccumulator,
) -> GraphTreatmentMetrics {
    let mut ranked_action_pairs = 0_u64;
    let mut correctly_ranked_action_pairs = 0_u64;
    for predictions in accumulator.by_state.values() {
        for left in 0..predictions.len() {
            for right in (left + 1)..predictions.len() {
                let actual = predictions[left].0.cmp(&predictions[right].0);
                if actual == Ordering::Equal {
                    continue;
                }
                ranked_action_pairs += 1;
                correctly_ranked_action_pairs +=
                    u64::from(predictions[left].1.cmp(&predictions[right].1) == actual);
            }
        }
    }
    let mean_error_millionths =
        u64::try_from(accumulator.relative_error / u128::from(accumulator.predictions.max(1)))
            .unwrap_or(u64::MAX);
    let passed_gate = accumulator.predictions >= contract.minimum_calibration_objective_predictions
        && accumulator.predictions == accumulator.targets
        && mean_error_millionths <= contract.maximum_calibration_tick_error_millionths
        && ranked_action_pairs >= contract.minimum_calibration_ranked_pairs;
    GraphTreatmentMetrics {
        treatment,
        objective_targets: accumulator.targets,
        objective_predictions: accumulator.predictions,
        mean_absolute_tick_error: u64::try_from(
            accumulator.absolute_error / u128::from(accumulator.predictions.max(1)),
        )
        .unwrap_or(u64::MAX),
        mean_error_millionths,
        ranked_action_pairs,
        correctly_ranked_action_pairs,
        passed_gate,
    }
}

fn relative_tick_error(actual: u64, predicted: u64) -> u64 {
    u64::try_from(
        u128::from(actual.abs_diff(predicted)).saturating_mul(1_000_000)
            / u128::from(actual.max(predicted).max(1)),
    )
    .unwrap_or(u64::MAX)
}
