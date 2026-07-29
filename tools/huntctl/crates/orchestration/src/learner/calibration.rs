use super::{
    ActionConditionedGraphLearner, ExactGraphTableLearner, GraphAuxiliaryPrediction,
    GraphLearnerContract, GraphLearnerError, GraphLearningBatch,
};
use crate::state_graph::ExactStateId;
use dusklight_automation_contracts::artifact::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

pub const HELD_OUT_GRAPH_CALIBRATION_SCHEMA_V2: &str = "dusklight-held-out-graph-calibration/v2";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HeldOutGraphCalibrationReport {
    pub schema: String,
    pub graph_sha256: Digest,
    pub contract_sha256: Digest,
    pub required_objective_predictions: u64,
    pub maximum_tick_error_millionths: u64,
    pub required_ranked_pairs: u64,
    pub required_error_improvement_millionths: u64,
    pub required_ranking_improvement_millionths: u64,
    pub training_rows: u64,
    pub held_out_rows: u64,
    pub held_out_state_groups: u64,
    pub independently_realized_action_rows: u64,
    pub unseen_action_rows: u64,
    pub auxiliary_predictions: u64,
    pub auxiliary_mean_error_millionths: u64,
    pub objective_targets: u64,
    pub independently_realized_objective_targets: u64,
    pub objective_predictions: u64,
    pub objective_mean_absolute_tick_error: u64,
    pub objective_mean_error_millionths: u64,
    pub mean_baseline_error_millionths: u64,
    pub objective_error_improvement_millionths: i64,
    pub ranked_state_action_pairs: u64,
    pub correctly_ranked_state_action_pairs: u64,
    pub mean_baseline_correctly_ranked_state_action_pairs: u64,
    pub ranking_accuracy_improvement_millionths: i64,
    pub objective_calibration_gate_passed: bool,
    pub report_sha256: Digest,
}

impl HeldOutGraphCalibrationReport {
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
                "held-out calibration requires at least two exact state groups",
            ));
        }
        let mut held_out_sources = sources
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(index, source)| ((index + 1) % 5 == 0).then_some(source))
            .collect::<BTreeSet<_>>();
        if held_out_sources.is_empty() {
            held_out_sources.insert(*sources.last().expect("two sources are nonempty"));
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
        if training.rows.is_empty() || held_out.is_empty() {
            return Err(GraphLearnerError::Invalid(
                "held-out calibration split is empty",
            ));
        }
        training.validate()?;
        let learner = ExactGraphTableLearner;
        let snapshot = learner.fit(contract, &training)?;
        let trained_actions = training
            .rows
            .iter()
            .map(|row| {
                row.action
                    .content_sha256()
                    .map_err(|error| GraphLearnerError::Action(error.to_string()))
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        let mut auxiliary_predictions = 0_u64;
        let mut auxiliary_error = 0_u128;
        let mut independently_realized_action_rows = 0_u64;
        let mut unseen_action_rows = 0_u64;
        let mut objective_targets = 0_u64;
        let mut independently_realized_objective_targets = 0_u64;
        let mut objective_predictions = 0_u64;
        let mut absolute_tick_error = 0_u128;
        let mut objective_error = 0_u128;
        let mut mean_baseline_error = 0_u128;
        let mut ranked_predictions = BTreeMap::<ExactStateId, Vec<(u64, u64, u64)>>::new();
        for row in &held_out {
            let action_sha256 = row
                .action
                .content_sha256()
                .map_err(|error| GraphLearnerError::Action(error.to_string()))?;
            if trained_actions.contains(&action_sha256) {
                independently_realized_action_rows += 1;
            } else {
                unseen_action_rows += 1;
            }
            if let Some(prediction) =
                snapshot.generalized_auxiliary_prediction(&row.source_features, action_sha256)
            {
                auxiliary_predictions += 1;
                auxiliary_error = auxiliary_error
                    .saturating_add(u128::from(auxiliary_error_for_row(row, &prediction)));
            }
            if row.exact_conditional_ticks_to_terminal.is_some() {
                objective_targets += 1;
            }
            let Some(actual_ticks) = row.exact_conditional_ticks_to_terminal else {
                continue;
            };
            if !trained_actions.contains(&action_sha256) {
                continue;
            }
            independently_realized_objective_targets += 1;
            let estimate =
                snapshot.generalized_objective_prediction(&row.source_features, action_sha256);
            let baseline = snapshot.mean_objective_prediction(action_sha256);
            if let (Some(predicted_ticks), Some(baseline_ticks)) = (
                estimate.and_then(|value| value.conditional_ticks_to_terminal),
                baseline.and_then(|value| value.conditional_ticks_to_terminal),
            ) {
                objective_predictions += 1;
                absolute_tick_error = absolute_tick_error
                    .saturating_add(u128::from(actual_ticks.abs_diff(predicted_ticks)));
                objective_error = objective_error.saturating_add(u128::from(relative_tick_error(
                    actual_ticks,
                    predicted_ticks,
                )));
                mean_baseline_error = mean_baseline_error.saturating_add(u128::from(
                    relative_tick_error(actual_ticks, baseline_ticks),
                ));
                ranked_predictions.entry(row.source).or_default().push((
                    actual_ticks,
                    predicted_ticks,
                    baseline_ticks,
                ));
            }
        }
        let mut ranked_state_action_pairs = 0_u64;
        let mut correctly_ranked_state_action_pairs = 0_u64;
        let mut mean_baseline_correctly_ranked_state_action_pairs = 0_u64;
        for predictions in ranked_predictions.values() {
            for left in 0..predictions.len() {
                for right in (left + 1)..predictions.len() {
                    let actual = predictions[left].0.cmp(&predictions[right].0);
                    if actual == Ordering::Equal {
                        continue;
                    }
                    ranked_state_action_pairs += 1;
                    correctly_ranked_state_action_pairs +=
                        u64::from(predictions[left].1.cmp(&predictions[right].1) == actual);
                    mean_baseline_correctly_ranked_state_action_pairs +=
                        u64::from(predictions[left].2.cmp(&predictions[right].2) == actual);
                }
            }
        }
        let objective_mean_error_millionths = mean_u128(objective_error, objective_predictions);
        let mean_baseline_error_millionths = mean_u128(mean_baseline_error, objective_predictions);
        let ranking_accuracy_millionths = ratio_per_million(
            correctly_ranked_state_action_pairs,
            ranked_state_action_pairs,
        );
        let mean_baseline_ranking_accuracy_millionths = ratio_per_million(
            mean_baseline_correctly_ranked_state_action_pairs,
            ranked_state_action_pairs,
        );
        let objective_error_improvement_millionths = signed_difference(
            mean_baseline_error_millionths,
            objective_mean_error_millionths,
        );
        let ranking_accuracy_improvement_millionths = signed_difference(
            ranking_accuracy_millionths,
            mean_baseline_ranking_accuracy_millionths,
        );
        let objective_calibration_gate_passed = objective_predictions
            >= contract.minimum_calibration_objective_predictions
            && objective_predictions == independently_realized_objective_targets
            && objective_mean_error_millionths
                <= contract.maximum_calibration_tick_error_millionths
            && objective_error_improvement_millionths
                >= contract.minimum_calibration_error_improvement_millionths as i64
            && ranked_state_action_pairs >= contract.minimum_calibration_ranked_pairs
            && ranking_accuracy_improvement_millionths
                >= contract.minimum_calibration_ranking_improvement_millionths as i64;
        let mut report = Self {
            schema: HELD_OUT_GRAPH_CALIBRATION_SCHEMA_V2.into(),
            graph_sha256: batch.graph_sha256,
            contract_sha256: contract.content_sha256()?,
            required_objective_predictions: contract.minimum_calibration_objective_predictions,
            maximum_tick_error_millionths: contract.maximum_calibration_tick_error_millionths,
            required_ranked_pairs: contract.minimum_calibration_ranked_pairs,
            required_error_improvement_millionths: contract
                .minimum_calibration_error_improvement_millionths,
            required_ranking_improvement_millionths: contract
                .minimum_calibration_ranking_improvement_millionths,
            training_rows: training.rows.len() as u64,
            held_out_rows: held_out.len() as u64,
            held_out_state_groups: held_out_sources.len() as u64,
            independently_realized_action_rows,
            unseen_action_rows,
            auxiliary_predictions,
            auxiliary_mean_error_millionths: u64::try_from(
                auxiliary_error / u128::from(auxiliary_predictions.max(1)),
            )
            .unwrap_or(u64::MAX),
            objective_targets,
            independently_realized_objective_targets,
            objective_predictions,
            objective_mean_absolute_tick_error: mean_u128(
                absolute_tick_error,
                objective_predictions,
            ),
            objective_mean_error_millionths,
            mean_baseline_error_millionths,
            objective_error_improvement_millionths,
            ranked_state_action_pairs,
            correctly_ranked_state_action_pairs,
            mean_baseline_correctly_ranked_state_action_pairs,
            ranking_accuracy_improvement_millionths,
            objective_calibration_gate_passed,
            report_sha256: Digest::ZERO,
        };
        report.report_sha256 = report.digest()?;
        report.validate()?;
        Ok(report)
    }

    pub fn validate(&self) -> Result<(), GraphLearnerError> {
        let expected_gate = self.objective_predictions >= self.required_objective_predictions
            && self.objective_predictions == self.independently_realized_objective_targets
            && self.objective_mean_error_millionths <= self.maximum_tick_error_millionths
            && self.objective_error_improvement_millionths
                >= self.required_error_improvement_millionths as i64
            && self.ranked_state_action_pairs >= self.required_ranked_pairs
            && self.ranking_accuracy_improvement_millionths
                >= self.required_ranking_improvement_millionths as i64;
        if self.schema != HELD_OUT_GRAPH_CALIBRATION_SCHEMA_V2
            || self.graph_sha256 == Digest::ZERO
            || self.contract_sha256 == Digest::ZERO
            || self.required_objective_predictions == 0
            || self.maximum_tick_error_millionths > 1_000_000
            || self.required_ranked_pairs == 0
            || self.required_error_improvement_millionths == 0
            || self.required_error_improvement_millionths > 1_000_000
            || self.required_ranking_improvement_millionths == 0
            || self.required_ranking_improvement_millionths > 1_000_000
            || self.training_rows == 0
            || self.held_out_rows == 0
            || self.held_out_state_groups == 0
            || self.independently_realized_action_rows + self.unseen_action_rows
                != self.held_out_rows
            || self.auxiliary_predictions > self.independently_realized_action_rows
            || self.independently_realized_objective_targets > self.objective_targets
            || self.independently_realized_objective_targets
                > self.independently_realized_action_rows
            || self.objective_predictions > self.objective_targets
            || self.objective_predictions > self.independently_realized_objective_targets
            || self.correctly_ranked_state_action_pairs > self.ranked_state_action_pairs
            || self.mean_baseline_correctly_ranked_state_action_pairs
                > self.ranked_state_action_pairs
            || self.objective_calibration_gate_passed != expected_gate
            || self.report_sha256 == Digest::ZERO
            || self.report_sha256 != self.digest()?
        {
            return Err(GraphLearnerError::Invalid(
                "held-out graph calibration report is detached",
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
        hasher.update(HELD_OUT_GRAPH_CALIBRATION_SCHEMA_V2.as_bytes());
        hasher.update(raw);
        Ok(Digest(hasher.finalize().into()))
    }
}

fn mean_u128(total: u128, count: u64) -> u64 {
    u64::try_from(total / u128::from(count.max(1))).unwrap_or(u64::MAX)
}

fn ratio_per_million(numerator: u64, denominator: u64) -> u64 {
    u64::try_from(u128::from(numerator).saturating_mul(1_000_000) / u128::from(denominator.max(1)))
        .unwrap_or(u64::MAX)
}

fn signed_difference(left: u64, right: u64) -> i64 {
    let difference = i128::from(left) - i128::from(right);
    difference.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

fn relative_tick_error(actual: u64, predicted: u64) -> u64 {
    u64::try_from(
        u128::from(actual.abs_diff(predicted)).saturating_mul(1_000_000)
            / u128::from(actual.max(predicted).max(1)),
    )
    .unwrap_or(u64::MAX)
}

fn auxiliary_error_for_row(
    row: &super::GraphExpansionLearningTarget,
    prediction: &GraphAuxiliaryPrediction,
) -> u64 {
    let duration_scale = u64::from(
        row.realized_duration_ticks
            .max(prediction.realized_duration_ticks)
            .max(1),
    );
    let duration = u64::from(
        row.realized_duration_ticks
            .abs_diff(prediction.realized_duration_ticks),
    )
    .saturating_mul(1_000_000)
        / duration_scale;
    let acceptance = u64::from(
        prediction
            .action_acceptance_per_million
            .abs_diff(u32::from(row.action_accepted) * 1_000_000),
    );
    let terminal = u64::from(
        prediction
            .immediate_terminal_per_million
            .abs_diff(u32::from(row.immediate_terminal) * 1_000_000),
    );
    let prompt =
        u64::from(prediction.prompted_action_status != row.prompted_action_status) * 1_000_000;
    let features = mean_feature_error(row, &prediction.next_state_feature_f32_bits);
    duration
        .saturating_add(acceptance)
        .saturating_add(terminal)
        .saturating_add(prompt)
        .saturating_add(features)
        / 5
}

fn mean_feature_error(row: &super::GraphExpansionLearningTarget, predicted_bits: &[u32]) -> u64 {
    if predicted_bits.len() != row.target_features.len() {
        return 1_000_000;
    }
    let error = row
        .target_features
        .iter()
        .zip(predicted_bits)
        .map(|(actual, predicted)| {
            let predicted = f32::from_bits(*predicted);
            ((*actual - predicted).abs() / (actual.abs() + 1.0)).min(1.0)
        })
        .sum::<f32>()
        / row.target_features.len().max(1) as f32;
    (error * 1_000_000.0).round() as u64
}
