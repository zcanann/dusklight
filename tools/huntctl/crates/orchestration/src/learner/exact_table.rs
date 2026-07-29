use super::objective_knn::{ActionObjectiveModel, ActionObjectiveSample};
use super::{
    ActionConditionedGraphLearner, GraphActionInput, GraphLearnerContract, GraphLearnerError,
    GraphLearningBatch, GraphNodeInput, GraphTargetSupport, LearnedGraphActionEstimate,
};
use crate::state_graph::ExactStateId;
use dusklight_automation_contracts::artifact::Digest;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub struct ExactGraphTableLearner;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactGraphTableSnapshot {
    pub contract_sha256: Digest,
    pub graph_sha256: Digest,
    estimates: BTreeMap<(ExactStateId, Digest), LearnedGraphActionEstimate>,
    exact_auxiliary: BTreeMap<(ExactStateId, Digest), GraphAuxiliaryPrediction>,
    action_auxiliary: BTreeMap<Digest, ActionAuxiliaryModel>,
    action_objectives: BTreeMap<Digest, ActionObjectiveModel>,
    node_features: BTreeMap<ExactStateId, Vec<u32>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphAuxiliaryPrediction {
    pub next_state_feature_f32_bits: Vec<u32>,
    pub realized_duration_ticks: u32,
    pub action_acceptance_per_million: u32,
    pub prompted_action_status: Option<u8>,
    pub immediate_terminal_per_million: u32,
    pub support_rows: u64,
    pub prediction_error_millionths: u64,
    pub generalized: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct ActionAuxiliaryAccumulator {
    rows: u64,
    duration_sum: u64,
    accepted: u64,
    terminal: u64,
    prompt_counts: BTreeMap<Option<u8>, u64>,
    delta_sums: Vec<f64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ActionAuxiliaryModel {
    mean_delta_f32_bits: Vec<u32>,
    duration_ticks: u32,
    acceptance_per_million: u32,
    prompted_action_status: Option<u8>,
    immediate_terminal_per_million: u32,
    support_rows: u64,
    prediction_error_millionths: u64,
}

impl ExactGraphTableSnapshot {
    pub fn estimate(
        &self,
        state: ExactStateId,
        action_sha256: Digest,
    ) -> Option<LearnedGraphActionEstimate> {
        self.estimates.get(&(state, action_sha256)).copied()
    }

    pub fn len(&self) -> usize {
        self.estimates.len()
    }

    pub fn is_empty(&self) -> bool {
        self.estimates.is_empty()
    }

    pub fn auxiliary_prediction(
        &self,
        state: ExactStateId,
        action_sha256: Digest,
    ) -> Option<GraphAuxiliaryPrediction> {
        if let Some(prediction) = self.exact_auxiliary.get(&(state, action_sha256)) {
            return Some(prediction.clone());
        }
        let model = self.action_auxiliary.get(&action_sha256)?;
        let source = self.node_features.get(&state)?;
        if source.len() != model.mean_delta_f32_bits.len() {
            return None;
        }
        Some(GraphAuxiliaryPrediction {
            next_state_feature_f32_bits: predict_next_features(source, model),
            realized_duration_ticks: model.duration_ticks,
            action_acceptance_per_million: model.acceptance_per_million,
            prompted_action_status: model.prompted_action_status,
            immediate_terminal_per_million: model.immediate_terminal_per_million,
            support_rows: model.support_rows,
            prediction_error_millionths: model.prediction_error_millionths,
            generalized: true,
        })
    }

    pub fn generalized_auxiliary_prediction(
        &self,
        source_features: &[f32],
        action_sha256: Digest,
    ) -> Option<GraphAuxiliaryPrediction> {
        let model = self.action_auxiliary.get(&action_sha256)?;
        if source_features.len() != model.mean_delta_f32_bits.len()
            || source_features.iter().any(|value| !value.is_finite())
        {
            return None;
        }
        let source = source_features
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>();
        Some(GraphAuxiliaryPrediction {
            next_state_feature_f32_bits: predict_next_features(&source, model),
            realized_duration_ticks: model.duration_ticks,
            action_acceptance_per_million: model.acceptance_per_million,
            prompted_action_status: model.prompted_action_status,
            immediate_terminal_per_million: model.immediate_terminal_per_million,
            support_rows: model.support_rows,
            prediction_error_millionths: model.prediction_error_millionths,
            generalized: true,
        })
    }

    /// Predicts conditional ticks from independently realized terminal paths
    /// for the same action. Open continuations are right-censored and never
    /// become negative terminal labels. Consequently this baseline deliberately
    /// leaves terminal-support probability unsupported.
    pub fn generalized_objective_prediction(
        &self,
        source_features: &[f32],
        action_sha256: Digest,
    ) -> Option<LearnedGraphActionEstimate> {
        let model = self.action_objectives.get(&action_sha256)?;
        model.predict(source_features)
    }

    pub fn mean_objective_prediction(
        &self,
        action_sha256: Digest,
    ) -> Option<LearnedGraphActionEstimate> {
        let model = self.action_objectives.get(&action_sha256)?;
        Some(model.mean_prediction())
    }
}

impl ActionConditionedGraphLearner for ExactGraphTableLearner {
    type Snapshot = ExactGraphTableSnapshot;

    fn fit(
        &self,
        contract: &GraphLearnerContract,
        batch: &GraphLearningBatch,
    ) -> Result<Self::Snapshot, GraphLearnerError> {
        contract.validate()?;
        batch.validate()?;
        let mut estimates = BTreeMap::new();
        let mut exact_auxiliary = BTreeMap::new();
        let mut accumulators = BTreeMap::<Digest, ActionAuxiliaryAccumulator>::new();
        let mut objective_samples = BTreeMap::<Digest, Vec<ActionObjectiveSample>>::new();
        let mut node_features = BTreeMap::new();
        for row in &batch.rows {
            let action_sha256 = row
                .action
                .content_sha256()
                .map_err(|error| GraphLearnerError::Action(error.to_string()))?;
            let estimate = match row.support {
                GraphTargetSupport::ExactTerminalPath => LearnedGraphActionEstimate {
                    terminal_support_per_million: Some(1_000_000),
                    conditional_ticks_to_terminal: row.exact_conditional_ticks_to_terminal,
                    uncertainty_millionths: inverse_visit_uncertainty(row.graph_visits),
                    prediction_error_millionths: 0,
                },
                GraphTargetSupport::OpenContinuationCensored => LearnedGraphActionEstimate {
                    terminal_support_per_million: None,
                    conditional_ticks_to_terminal: None,
                    uncertainty_millionths: inverse_visit_uncertainty(row.graph_visits),
                    prediction_error_millionths: 0,
                },
            };
            if let Some(ticks) = row.exact_conditional_ticks_to_terminal {
                objective_samples
                    .entry(action_sha256)
                    .or_default()
                    .push(ActionObjectiveSample::new(&row.source_features, ticks));
            }
            if estimates
                .insert((row.source, action_sha256), estimate)
                .is_some()
            {
                return Err(GraphLearnerError::Invalid(
                    "exact learner received duplicate state/action targets",
                ));
            }
            insert_node_features(&mut node_features, row.source, &row.source_features)?;
            insert_node_features(&mut node_features, row.target, &row.target_features)?;
            let exact_prediction = GraphAuxiliaryPrediction {
                next_state_feature_f32_bits: row
                    .target_features
                    .iter()
                    .map(|value| value.to_bits())
                    .collect(),
                realized_duration_ticks: row.realized_duration_ticks,
                action_acceptance_per_million: u32::from(row.action_accepted) * 1_000_000,
                prompted_action_status: row.prompted_action_status,
                immediate_terminal_per_million: u32::from(row.immediate_terminal) * 1_000_000,
                support_rows: row.graph_visits,
                prediction_error_millionths: 0,
                generalized: false,
            };
            if exact_auxiliary
                .insert((row.source, action_sha256), exact_prediction)
                .is_some()
            {
                return Err(GraphLearnerError::Invalid(
                    "exact learner received duplicate auxiliary targets",
                ));
            }
            let accumulator =
                accumulators
                    .entry(action_sha256)
                    .or_insert_with(|| ActionAuxiliaryAccumulator {
                        rows: 0,
                        duration_sum: 0,
                        accepted: 0,
                        terminal: 0,
                        prompt_counts: BTreeMap::new(),
                        delta_sums: vec![0.0; row.source_features.len()],
                    });
            if accumulator.delta_sums.len() != row.source_features.len() {
                return Err(GraphLearnerError::Invalid(
                    "one action has incompatible auxiliary feature widths",
                ));
            }
            accumulator.rows = accumulator.rows.saturating_add(1);
            accumulator.duration_sum = accumulator
                .duration_sum
                .saturating_add(u64::from(row.realized_duration_ticks));
            accumulator.accepted = accumulator
                .accepted
                .saturating_add(u64::from(row.action_accepted));
            accumulator.terminal = accumulator
                .terminal
                .saturating_add(u64::from(row.immediate_terminal));
            *accumulator
                .prompt_counts
                .entry(row.prompted_action_status)
                .or_default() += 1;
            for ((sum, source), target) in accumulator
                .delta_sums
                .iter_mut()
                .zip(&row.source_features)
                .zip(&row.target_features)
            {
                *sum += f64::from(*target - *source);
            }
        }
        let mut action_auxiliary = accumulators
            .into_iter()
            .map(|(action, aggregate)| (action, finalize_auxiliary(aggregate)))
            .collect::<BTreeMap<_, _>>();
        let action_identities = action_auxiliary.keys().copied().collect::<Vec<_>>();
        for action_sha256 in action_identities {
            let error = auxiliary_prediction_error(
                batch,
                action_sha256,
                &action_auxiliary[&action_sha256],
            )?;
            action_auxiliary
                .get_mut(&action_sha256)
                .expect("collected auxiliary action identity remains present")
                .prediction_error_millionths = error;
        }
        let action_objectives = objective_samples
            .into_iter()
            .map(|(action, samples)| {
                let model = ActionObjectiveModel::fit(samples)?;
                Ok((action, model))
            })
            .collect::<Result<BTreeMap<_, _>, GraphLearnerError>>()?;
        Ok(ExactGraphTableSnapshot {
            contract_sha256: contract.content_sha256()?,
            graph_sha256: batch.graph_sha256,
            estimates,
            exact_auxiliary,
            action_auxiliary,
            action_objectives,
            node_features,
        })
    }

    fn rank(
        &self,
        snapshot: &Self::Snapshot,
        node: &GraphNodeInput,
        actions: &[GraphActionInput],
    ) -> Result<Vec<LearnedGraphActionEstimate>, GraphLearnerError> {
        node.state
            .validate()
            .map_err(|error| GraphLearnerError::Facts(error.to_string()))?;
        if node
            .state
            .content_sha256()
            .map_err(|error| GraphLearnerError::Facts(error.to_string()))?
            != node.id.state_sha256
            || node.graph_visits == 0
        {
            return Err(GraphLearnerError::Invalid(
                "exact learner node input is detached",
            ));
        }
        actions
            .iter()
            .map(|action| {
                action
                    .action
                    .validate()
                    .map_err(|error| GraphLearnerError::Action(error.to_string()))?;
                if action.expansion_sha256 == Digest::ZERO {
                    return Err(GraphLearnerError::Invalid(
                        "exact learner action identity is missing",
                    ));
                }
                let action_sha256 = action
                    .action
                    .content_sha256()
                    .map_err(|error| GraphLearnerError::Action(error.to_string()))?;
                Ok(snapshot
                    .estimate(node.id, action_sha256)
                    .unwrap_or_else(|| {
                        let generalized =
                            snapshot.node_features.get(&node.id).and_then(|features| {
                                let features = features
                                    .iter()
                                    .map(|value| f32::from_bits(*value))
                                    .collect::<Vec<_>>();
                                snapshot.generalized_objective_prediction(&features, action_sha256)
                            });
                        let auxiliary = snapshot.action_auxiliary.get(&action_sha256);
                        generalized.unwrap_or_else(|| LearnedGraphActionEstimate {
                            terminal_support_per_million: None,
                            conditional_ticks_to_terminal: None,
                            uncertainty_millionths: auxiliary.map_or(u64::MAX, |model| {
                                inverse_visit_uncertainty(model.support_rows)
                            }),
                            prediction_error_millionths: auxiliary
                                .map_or(0, |model| model.prediction_error_millionths),
                        })
                    }))
            })
            .collect()
    }
}

fn insert_node_features(
    features: &mut BTreeMap<ExactStateId, Vec<u32>>,
    node: ExactStateId,
    values: &[f32],
) -> Result<(), GraphLearnerError> {
    let bits = values
        .iter()
        .map(|value| value.to_bits())
        .collect::<Vec<_>>();
    if features
        .insert(node, bits.clone())
        .is_some_and(|existing| existing != bits)
    {
        return Err(GraphLearnerError::Invalid(
            "exact node has conflicting learner features",
        ));
    }
    Ok(())
}

fn predict_next_features(source: &[u32], model: &ActionAuxiliaryModel) -> Vec<u32> {
    source
        .iter()
        .zip(&model.mean_delta_f32_bits)
        .map(|(source, delta)| (f32::from_bits(*source) + f32::from_bits(*delta)).to_bits())
        .collect()
}

fn finalize_auxiliary(aggregate: ActionAuxiliaryAccumulator) -> ActionAuxiliaryModel {
    let rows = aggregate.rows.max(1);
    let prompted_action_status = aggregate
        .prompt_counts
        .into_iter()
        .max_by(|left, right| left.1.cmp(&right.1).then_with(|| right.0.cmp(&left.0)))
        .map(|(status, _)| status)
        .unwrap_or(None);
    ActionAuxiliaryModel {
        mean_delta_f32_bits: aggregate
            .delta_sums
            .into_iter()
            .map(|sum| (sum / rows as f64) as f32)
            .map(f32::to_bits)
            .collect(),
        duration_ticks: u32::try_from(aggregate.duration_sum / rows).unwrap_or(u32::MAX),
        acceptance_per_million: ratio_per_million(aggregate.accepted, rows),
        prompted_action_status,
        immediate_terminal_per_million: ratio_per_million(aggregate.terminal, rows),
        support_rows: rows,
        prediction_error_millionths: 0,
    }
}

fn auxiliary_prediction_error(
    batch: &GraphLearningBatch,
    action_sha256: Digest,
    model: &ActionAuxiliaryModel,
) -> Result<u64, GraphLearnerError> {
    let mut total = 0_u128;
    let mut rows = 0_u128;
    for row in &batch.rows {
        if row
            .action
            .content_sha256()
            .map_err(|error| GraphLearnerError::Action(error.to_string()))?
            != action_sha256
        {
            continue;
        }
        let duration_scale =
            u64::from(row.realized_duration_ticks.max(model.duration_ticks).max(1));
        let duration_error = u64::from(row.realized_duration_ticks.abs_diff(model.duration_ticks))
            .saturating_mul(1_000_000)
            / duration_scale;
        let acceptance_error = u64::from(
            model
                .acceptance_per_million
                .abs_diff(u32::from(row.action_accepted) * 1_000_000),
        );
        let terminal_error = u64::from(
            model
                .immediate_terminal_per_million
                .abs_diff(u32::from(row.immediate_terminal) * 1_000_000),
        );
        let prompt_error =
            u64::from(model.prompted_action_status != row.prompted_action_status) * 1_000_000;
        let feature_error = mean_delta_error(row, &model.mean_delta_f32_bits);
        let row_error = duration_error
            .saturating_add(acceptance_error)
            .saturating_add(terminal_error)
            .saturating_add(prompt_error)
            .saturating_add(feature_error)
            / 5;
        total = total.saturating_add(u128::from(row_error));
        rows += 1;
    }
    Ok(u64::try_from(total / rows.max(1)).unwrap_or(u64::MAX))
}

fn mean_delta_error(row: &super::GraphExpansionLearningTarget, predicted: &[u32]) -> u64 {
    if predicted.len() != row.source_features.len() {
        return 1_000_000;
    }
    let error = row
        .source_features
        .iter()
        .zip(&row.target_features)
        .zip(predicted)
        .map(|((source, target), predicted)| {
            let actual = *target - *source;
            ((actual - f32::from_bits(*predicted)).abs() / (actual.abs() + 1.0)).min(1.0)
        })
        .sum::<f32>()
        / row.source_features.len().max(1) as f32;
    (error * 1_000_000.0).round() as u64
}

fn ratio_per_million(numerator: u64, denominator: u64) -> u32 {
    u32::try_from(u128::from(numerator).saturating_mul(1_000_000) / u128::from(denominator.max(1)))
        .unwrap_or(1_000_000)
}

fn inverse_visit_uncertainty(visits: u64) -> u64 {
    1_000_000_u64 / visits.max(1)
}
