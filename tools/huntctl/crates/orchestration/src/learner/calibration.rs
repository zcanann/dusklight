use super::{
    ActionConditionedGraphLearner, ExactGraphTableLearner, GraphActionInput,
    GraphAuxiliaryPrediction, GraphLearnerContract, GraphLearnerError, GraphLearningBatch,
    GraphNodeInput,
};
use dusklight_automation_contracts::artifact::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;

pub const HELD_OUT_GRAPH_CALIBRATION_SCHEMA_V1: &str = "dusklight-held-out-graph-calibration/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HeldOutGraphCalibrationReport {
    pub schema: String,
    pub graph_sha256: Digest,
    pub contract_sha256: Digest,
    pub training_rows: u64,
    pub held_out_rows: u64,
    pub held_out_state_groups: u64,
    pub independently_realized_action_rows: u64,
    pub unseen_action_rows: u64,
    pub auxiliary_predictions: u64,
    pub auxiliary_mean_error_millionths: u64,
    pub objective_targets: u64,
    pub objective_predictions: u64,
    pub ranked_state_action_pairs: u64,
    pub correctly_ranked_state_action_pairs: u64,
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
        let mut objective_predictions = 0_u64;
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
            let estimate = learner
                .rank(
                    &snapshot,
                    &GraphNodeInput {
                        id: row.source,
                        state: row.source_state.clone(),
                        graph_visits: row.graph_visits,
                    },
                    &[GraphActionInput {
                        expansion_sha256: row.expansion_sha256,
                        action: row.action.clone(),
                        graph_visits: row.graph_visits,
                    }],
                )?
                .pop()
                .expect("one held-out action produces one estimate");
            if estimate.conditional_ticks_to_terminal.is_some() {
                objective_predictions += 1;
            }
        }
        let mut report = Self {
            schema: HELD_OUT_GRAPH_CALIBRATION_SCHEMA_V1.into(),
            graph_sha256: batch.graph_sha256,
            contract_sha256: contract.content_sha256()?,
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
            objective_predictions,
            // Pairwise ranking remains explicitly unsupported until at least
            // two independently realized actions share a held-out source.
            ranked_state_action_pairs: 0,
            correctly_ranked_state_action_pairs: 0,
            report_sha256: Digest::ZERO,
        };
        report.report_sha256 = report.digest()?;
        report.validate()?;
        Ok(report)
    }

    pub fn validate(&self) -> Result<(), GraphLearnerError> {
        if self.schema != HELD_OUT_GRAPH_CALIBRATION_SCHEMA_V1
            || self.graph_sha256 == Digest::ZERO
            || self.contract_sha256 == Digest::ZERO
            || self.training_rows == 0
            || self.held_out_rows == 0
            || self.held_out_state_groups == 0
            || self.independently_realized_action_rows + self.unseen_action_rows
                != self.held_out_rows
            || self.auxiliary_predictions > self.independently_realized_action_rows
            || self.objective_predictions > self.objective_targets
            || self.correctly_ranked_state_action_pairs > self.ranked_state_action_pairs
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
        hasher.update(HELD_OUT_GRAPH_CALIBRATION_SCHEMA_V1.as_bytes());
        hasher.update(raw);
        Ok(Digest(hasher.finalize().into()))
    }
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
