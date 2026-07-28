//! Matched controls over the same whole-group tactic calibration splits.

use super::*;
use crate::double_q::{ConservativeQ, ConservativeQConfig, DoubleQ, DoubleQConfig};
use crate::fqi::Transition as FqiTransition;
use crate::generalized_tactic_value::prediction::{action_class, regression_features};
use crate::tactic_value_treatment::ContinuousTacticValueModel;

pub const GENERALIZED_TACTIC_CONTROL_COMPARISON_SCHEMA_V1: &str =
    "dusklight-generalized-tactic-control-comparison/v1";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeneralizedTacticControlMetrics {
    pub model: String,
    pub evaluation_samples: usize,
    pub supported_samples: usize,
    pub unsupported_samples: usize,
    pub mean_error: f64,
    pub mean_absolute_error: f64,
    pub root_mean_squared_error: f64,
    pub mean_epistemic_signal: Option<f64>,
    pub comparable_states: usize,
    pub ranking_wins: usize,
    pub ranking_win_rate: Option<f64>,
    pub mean_observed_regret: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeneralizedTacticAxisControlComparison {
    pub axis: GeneralizedTacticCalibrationAxis,
    pub training_groups: Vec<String>,
    pub validation_groups: Vec<String>,
    pub test_groups: Vec<String>,
    pub group_overlap_count: usize,
    pub training_samples: usize,
    pub target_kind: String,
    pub models: Vec<GeneralizedTacticControlMetrics>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeneralizedTacticControlComparisonReport {
    pub schema: String,
    pub source_transition_sha256: Digest,
    pub source_transitions: usize,
    pub goal_distance_feature: usize,
    pub config: GeneralizedTacticCalibrationConfig,
    pub state_region: GeneralizedTacticAxisControlComparison,
    pub action_realization: GeneralizedTacticAxisControlComparison,
    pub report_sha256: Digest,
}

impl GeneralizedTacticControlComparisonReport {
    pub fn validate(&self) -> Result<(), GeneralizedTacticCalibrationError> {
        self.config.validate()?;
        if self.schema != GENERALIZED_TACTIC_CONTROL_COMPARISON_SCHEMA_V1
            || self.source_transition_sha256 == Digest::ZERO
            || self.source_transitions < 5
            || self.state_region.axis != GeneralizedTacticCalibrationAxis::StateRegion
            || self.action_realization.axis != GeneralizedTacticCalibrationAxis::ActionRealization
            || !comparison_axis_is_valid(&self.state_region)
            || !comparison_axis_is_valid(&self.action_realization)
            || self.report_sha256 == Digest::ZERO
            || self.report_sha256 != self.digest()?
        {
            return Err(GeneralizedTacticCalibrationError::new(
                "generalized tactic control comparison is invalid or detached",
            ));
        }
        Ok(())
    }

    pub(super) fn digest(&self) -> Result<Digest, GeneralizedTacticCalibrationError> {
        canonical_digest(
            b"dusklight.generalized-tactic-control-comparison/v1\0",
            &(
                &self.schema,
                self.source_transition_sha256,
                self.source_transitions,
                self.goal_distance_feature,
                self.config,
                &self.state_region,
                &self.action_realization,
            ),
        )
    }
}

#[derive(Clone)]
struct ControlPrediction<'a> {
    transition: &'a OptionTransitionSample,
    target: f64,
    predicted: f64,
    epistemic_signal: Option<f64>,
}

struct ControlModels {
    local: GeneralizedTacticValueModel,
    fitted_q: ContinuousTacticValueModel,
    double_q: DoubleQ,
    conservative_q: ConservativeQ,
    action_classes: BTreeSet<u32>,
}

pub fn compare_generalized_tactic_controls(
    transitions: &[OptionTransitionSample],
    goal_distance_feature: usize,
    config: GeneralizedTacticCalibrationConfig,
) -> Result<GeneralizedTacticControlComparisonReport, GeneralizedTacticCalibrationError> {
    config.validate()?;
    if transitions.len() < 5 {
        return Err(GeneralizedTacticCalibrationError::new(
            "generalized tactic control comparison requires at least five transitions",
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
    let state_region = compare_axis(
        &rows,
        goal_distance_feature,
        config,
        GeneralizedTacticCalibrationAxis::StateRegion,
    )?;
    let action_realization = compare_axis(
        &rows,
        goal_distance_feature,
        config,
        GeneralizedTacticCalibrationAxis::ActionRealization,
    )?;
    let mut report = GeneralizedTacticControlComparisonReport {
        schema: GENERALIZED_TACTIC_CONTROL_COMPARISON_SCHEMA_V1.into(),
        source_transition_sha256: transition_corpus_digest(transitions)?,
        source_transitions: transitions.len(),
        goal_distance_feature,
        config,
        state_region,
        action_realization,
        report_sha256: Digest::ZERO,
    };
    report.report_sha256 = report.digest()?;
    report.validate()?;
    Ok(report)
}

fn compare_axis(
    rows: &[CalibrationRow<'_>],
    goal_distance_feature: usize,
    config: GeneralizedTacticCalibrationConfig,
    axis: GeneralizedTacticCalibrationAxis,
) -> Result<GeneralizedTacticAxisControlComparison, GeneralizedTacticCalibrationError> {
    let groups = rows
        .iter()
        .map(|row| row_group(row, axis))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if groups.len() < usize::from(config.group_folds) {
        return Err(GeneralizedTacticCalibrationError::new(format!(
            "{axis:?} control comparison has fewer groups than declared folds"
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
        .cloned()
        .collect::<Vec<_>>();
    let test = rows
        .iter()
        .filter(|row| test_set.contains(row_group(row, axis)))
        .cloned()
        .collect::<Vec<_>>();
    if training.len() < 2 || test.is_empty() {
        return Err(GeneralizedTacticCalibrationError::new(format!(
            "{axis:?} control comparison produced an empty or undersized partition"
        )));
    }
    let models = fit_controls(&training, goal_distance_feature, config)?;
    let model_metrics = predict_controls(&models, &test)?;
    let overlap_count = group_overlap_count(&training_set, &validation_set, &test_set);
    Ok(GeneralizedTacticAxisControlComparison {
        axis,
        training_groups,
        validation_groups,
        test_groups,
        group_overlap_count: overlap_count,
        training_samples: training.len(),
        target_kind: "training_subgraph_fitted_q_evaluated_against_full_graph_fitted_q".into(),
        models: model_metrics,
    })
}

fn fit_controls(
    training: &[CalibrationRow<'_>],
    goal_distance_feature: usize,
    config: GeneralizedTacticCalibrationConfig,
) -> Result<ControlModels, GeneralizedTacticCalibrationError> {
    let transitions = training
        .iter()
        .map(|row| row.transition.clone())
        .collect::<Vec<_>>();
    let local = GeneralizedTacticValueModel::fit_fitted_q_transitions(
        &transitions,
        goal_distance_feature,
        usize::from(config.fitted_q_iterations),
        config.per_tick_discount,
    )?;
    let targets = fit_transition_returns(
        &transitions,
        usize::from(config.fitted_q_iterations),
        config.per_tick_discount,
    )?
    .values;
    let regression = training
        .iter()
        .zip(targets)
        .map(|(row, target)| regression_transition(row.transition, target))
        .collect::<Result<Vec<_>, _>>()?;
    let width = regression[0].state.len();
    let fitted_q = ContinuousTacticValueModel::fit(
        &transitions,
        goal_distance_feature,
        usize::from(config.fitted_q_iterations),
        config.per_tick_discount,
    )?;
    let action_classes = regression
        .iter()
        .map(|row| row.action)
        .collect::<BTreeSet<_>>();
    if action_classes.is_empty() {
        return Err(GeneralizedTacticCalibrationError::new(
            "control comparison has no supported action classes",
        ));
    }
    let actions = action_classes.iter().copied().collect::<Vec<_>>();
    let double_config = DoubleQConfig {
        epochs: 128,
        hidden_width: 32,
        learning_rate: 0.003,
        discount: f64::from(config.per_tick_discount),
        target_sync_steps: 64,
        gradient_clip: 10.0,
        seed: 0x4754_4344_5141_0001,
    };
    let double_q = DoubleQ::fit(width, &actions, &regression, &double_config)
        .map_err(|error| GeneralizedTacticCalibrationError::new(error.to_string()))?;
    let conservative_q = ConservativeQ::fit(
        width,
        &actions,
        &regression,
        &ConservativeQConfig {
            double_q: DoubleQConfig {
                seed: 0x4754_4343_514c_0001,
                ..double_config
            },
            conservative_weight: 0.1,
            temperature: 1.0,
        },
    )
    .map_err(|error| GeneralizedTacticCalibrationError::new(error.to_string()))?;
    Ok(ControlModels {
        local,
        fitted_q,
        double_q,
        conservative_q,
        action_classes,
    })
}

fn regression_transition(
    transition: &OptionTransitionSample,
    target: f32,
) -> Result<FqiTransition, GeneralizedTacticCalibrationError> {
    let context = GeneralizedTacticContext::from_facts(&transition.before)?;
    let state = regression_features(
        &transition.value_sample.state,
        &context,
        &transition.value_sample.action,
    )?;
    Ok(FqiTransition {
        state: state.clone(),
        action: action_class(&transition.value_sample.action.option_type),
        duration: transition.value_sample.duration_ticks,
        reward: target,
        next_state: state,
        terminal: true,
    })
}

fn predict_controls(
    models: &ControlModels,
    rows: &[CalibrationRow<'_>],
) -> Result<Vec<GeneralizedTacticControlMetrics>, GeneralizedTacticCalibrationError> {
    let mut local = Vec::new();
    let mut fitted_q = Vec::new();
    let mut double_q = Vec::new();
    let mut conservative_q = Vec::new();
    let mut structured = Vec::new();
    for row in rows {
        let context = GeneralizedTacticContext::from_facts(&row.transition.before)?;
        let local_estimate = models.local.predict(
            &row.transition.value_sample.state,
            &context,
            &row.transition.value_sample.action,
        )?;
        local.push(ControlPrediction {
            transition: row.transition,
            target: row.target,
            predicted: f64::from(local_estimate.outcome.reward),
            epistemic_signal: Some(f64::from(local_estimate.nearest_distance)),
        });
        let fitted_estimate = models.fitted_q.predict(
            &row.transition.value_sample.state,
            &context,
            &row.transition.value_sample.action,
        )?;
        fitted_q.push(ControlPrediction {
            transition: row.transition,
            target: row.target,
            predicted: fitted_estimate.mean_q,
            epistemic_signal: Some(fitted_estimate.ensemble_variance.sqrt()),
        });
        let regression = regression_transition(row.transition, row.target as f32)?;
        if models.action_classes.contains(&regression.action) {
            let estimate = models
                .double_q
                .estimate(&regression.state, regression.action)
                .map_err(|error| GeneralizedTacticCalibrationError::new(error.to_string()))?;
            double_q.push(ControlPrediction {
                transition: row.transition,
                target: row.target,
                predicted: estimate.mean,
                epistemic_signal: Some(estimate.critic_disagreement),
            });
            let estimate = models
                .conservative_q
                .estimate(&regression.state, regression.action)
                .map_err(|error| GeneralizedTacticCalibrationError::new(error.to_string()))?;
            conservative_q.push(ControlPrediction {
                transition: row.transition,
                target: row.target,
                predicted: estimate.mean,
                epistemic_signal: Some(estimate.critic_disagreement),
            });
        }
        structured.push(ControlPrediction {
            transition: row.transition,
            target: row.target,
            predicted: -0.01 * f64::from(row.transition.value_sample.duration_ticks),
            epistemic_signal: None,
        });
    }
    Ok(vec![
        control_metrics("local_generalized_fitted_q_knn", rows.len(), &local),
        control_metrics("continuous_fitted_q_forest", rows.len(), &fitted_q),
        control_metrics("continuous_double_q", rows.len(), &double_q),
        control_metrics(
            "continuous_conservative_offline_q",
            rows.len(),
            &conservative_q,
        ),
        control_metrics("structured_shortest_valid_action", rows.len(), &structured),
    ])
}

fn control_metrics(
    model: &str,
    evaluation_samples: usize,
    rows: &[ControlPrediction<'_>],
) -> GeneralizedTacticControlMetrics {
    let count = rows.len() as f64;
    let errors = rows
        .iter()
        .map(|row| row.predicted - row.target)
        .collect::<Vec<_>>();
    let (comparable_states, ranking_wins, observed_regret) = control_ranking_metrics(rows);
    let epistemic = rows
        .iter()
        .filter_map(|row| row.epistemic_signal)
        .collect::<Vec<_>>();
    GeneralizedTacticControlMetrics {
        model: model.into(),
        evaluation_samples,
        supported_samples: rows.len(),
        unsupported_samples: evaluation_samples - rows.len(),
        mean_error: errors.iter().sum::<f64>() / count,
        mean_absolute_error: errors.iter().map(|error| error.abs()).sum::<f64>() / count,
        root_mean_squared_error: (errors.iter().map(|error| error.powi(2)).sum::<f64>() / count)
            .sqrt(),
        mean_epistemic_signal: (!epistemic.is_empty())
            .then_some(epistemic.iter().sum::<f64>() / epistemic.len() as f64),
        comparable_states,
        ranking_wins,
        ranking_win_rate: (comparable_states != 0)
            .then_some(ranking_wins as f64 / comparable_states as f64),
        mean_observed_regret: (comparable_states != 0)
            .then_some(observed_regret / comparable_states as f64),
    }
}

fn control_ranking_metrics(rows: &[ControlPrediction<'_>]) -> (usize, usize, f64) {
    let mut by_state = BTreeMap::<Digest, Vec<&ControlPrediction<'_>>>::new();
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

fn comparison_axis_is_valid(axis: &GeneralizedTacticAxisControlComparison) -> bool {
    let training = axis.training_groups.iter().collect::<BTreeSet<_>>();
    let validation = axis.validation_groups.iter().collect::<BTreeSet<_>>();
    let test = axis.test_groups.iter().collect::<BTreeSet<_>>();
    let expected_models = [
        "local_generalized_fitted_q_knn",
        "continuous_fitted_q_forest",
        "continuous_double_q",
        "continuous_conservative_offline_q",
        "structured_shortest_valid_action",
    ];
    axis.training_samples >= 2
        && !training.is_empty()
        && !validation.is_empty()
        && !test.is_empty()
        && training.len() == axis.training_groups.len()
        && validation.len() == axis.validation_groups.len()
        && test.len() == axis.test_groups.len()
        && axis.group_overlap_count == group_overlap_count(&training, &validation, &test)
        && axis.group_overlap_count == 0
        && axis.target_kind == "training_subgraph_fitted_q_evaluated_against_full_graph_fitted_q"
        && axis.models.len() == expected_models.len()
        && axis
            .models
            .iter()
            .zip(expected_models)
            .all(|(metrics, name)| metrics.model == name && control_metrics_are_valid(metrics))
}

fn control_metrics_are_valid(metrics: &GeneralizedTacticControlMetrics) -> bool {
    metrics.evaluation_samples > 0
        && metrics.supported_samples > 0
        && metrics.supported_samples + metrics.unsupported_samples == metrics.evaluation_samples
        && metrics.comparable_states >= metrics.ranking_wins
        && [
            metrics.mean_error,
            metrics.mean_absolute_error,
            metrics.root_mean_squared_error,
        ]
        .iter()
        .all(|value| value.is_finite())
        && metrics.mean_absolute_error >= 0.0
        && metrics.root_mean_squared_error >= 0.0
        && metrics
            .mean_epistemic_signal
            .is_none_or(|value| value.is_finite() && value >= 0.0)
        && metrics
            .ranking_win_rate
            .is_none_or(|value| value.is_finite() && (0.0..=1.0).contains(&value))
        && metrics
            .mean_observed_regret
            .is_none_or(|value| value.is_finite() && value >= 0.0)
}
