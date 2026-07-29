//! Matched controls over the same whole-group tactic calibration splits.

use super::*;
use crate::learner_state::LearnerActionMaskEntry;
use crate::live_tactic_catalog::LiveTacticRanking;
use crate::option_values::{AvailableOptionRanking, OptionActionDescriptor};
use crate::tactic_asset::TacticDurationBounds;
use crate::tactic_blueprint::ConcreteTacticChoiceKind;
use crate::tactic_exploration::{
    TacticExplorationConfig, TacticProposalPolicy, choose_tactic_batch_for_policy,
};

pub const GENERALIZED_TACTIC_CONTROL_COMPARISON_SCHEMA_V2: &str =
    "dusklight-generalized-tactic-control-comparison/v2";
const CONTROL_SEED: u64 = 0x4754_434f_4e54_0002;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GeneralizedTacticControlKind {
    CalibratedOrderingScore,
    ValuePrediction,
    OrderingPolicy,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeneralizedTacticControlMetrics {
    pub model: String,
    pub kind: GeneralizedTacticControlKind,
    pub evaluation_samples: usize,
    pub supported_samples: usize,
    pub unsupported_samples: usize,
    pub mean_error: Option<f64>,
    pub mean_absolute_error: Option<f64>,
    pub root_mean_squared_error: Option<f64>,
    pub calibration_intercept: Option<f64>,
    pub calibration_slope: Option<f64>,
    pub mean_epistemic_signal: Option<f64>,
    pub comparable_states: usize,
    pub comparable_action_pairs: usize,
    pub correctly_ordered_action_pairs: usize,
    pub pairwise_ordering_rate: Option<f64>,
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
    pub control_seed: u64,
    pub config: GeneralizedTacticCalibrationConfig,
    pub state_region: GeneralizedTacticAxisControlComparison,
    pub action_realization: GeneralizedTacticAxisControlComparison,
    pub report_sha256: Digest,
}

impl GeneralizedTacticControlComparisonReport {
    pub fn validate(&self) -> Result<(), GeneralizedTacticCalibrationError> {
        self.config.validate()?;
        if self.schema != GENERALIZED_TACTIC_CONTROL_COMPARISON_SCHEMA_V2
            || self.source_transition_sha256 == Digest::ZERO
            || self.source_transitions < 5
            || self.control_seed != CONTROL_SEED
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
            b"dusklight.generalized-tactic-control-comparison/v2\0",
            &(
                &self.schema,
                self.source_transition_sha256,
                self.source_transitions,
                self.goal_distance_feature,
                self.control_seed,
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
    value_prediction: Option<f64>,
    ordering_score: f64,
    epistemic_signal: Option<f64>,
}

struct ControlModels {
    learner: GeneralizedTacticValueModel,
    action_means: BTreeMap<Digest, f64>,
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
        .filter_map(|(transition, target)| {
            target.map(|target| {
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
        })
        .collect::<Result<Vec<_>, GeneralizedTacticCalibrationError>>()?;
    if rows.len() < 5 {
        return Err(GeneralizedTacticCalibrationError::new(
            "generalized tactic control comparison requires at least five terminal-connected transitions",
        ));
    }
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
        schema: GENERALIZED_TACTIC_CONTROL_COMPARISON_SCHEMA_V2.into(),
        source_transition_sha256: transition_corpus_digest(transitions)?,
        source_transitions: transitions.len(),
        goal_distance_feature,
        control_seed: CONTROL_SEED,
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
    if training.len() < 2 || test.is_empty() {
        return Err(GeneralizedTacticCalibrationError::new(format!(
            "{axis:?} control comparison produced an empty or undersized partition"
        )));
    }
    let models = fit_controls(&training, goal_distance_feature)?;
    let model_metrics = predict_controls(&models, &training, &validation, &test)?;
    let overlap_count = group_overlap_count(&training_set, &validation_set, &test_set);
    Ok(GeneralizedTacticAxisControlComparison {
        axis,
        training_groups,
        validation_groups,
        test_groups,
        group_overlap_count: overlap_count,
        training_samples: training.len(),
        target_kind: "training_terminal_paths_evaluated_against_full_terminal_conditional_ticks"
            .into(),
        models: model_metrics,
    })
}

fn fit_controls(
    training: &[CalibrationRow<'_>],
    goal_distance_feature: usize,
) -> Result<ControlModels, GeneralizedTacticCalibrationError> {
    let transitions = training
        .iter()
        .map(|row| row.transition.clone())
        .collect::<Vec<_>>();
    let learner = GeneralizedTacticValueModel::fit_achieved_goal_returns(
        &transitions,
        goal_distance_feature,
    )?;
    let mut action_totals = BTreeMap::<Digest, (f64, usize)>::new();
    for row in training {
        let action = action_digest(&row.transition.value_sample.action)?;
        let entry = action_totals.entry(action).or_default();
        entry.0 += row.target;
        entry.1 += 1;
    }
    if action_totals.is_empty() {
        return Err(GeneralizedTacticCalibrationError::new(
            "control comparison has no supported actions",
        ));
    }
    Ok(ControlModels {
        learner,
        action_means: action_totals
            .into_iter()
            .map(|(action, (total, count))| (action, total / count as f64))
            .collect(),
    })
}

fn predict_controls(
    models: &ControlModels,
    training: &[CalibrationRow<'_>],
    validation: &[CalibrationRow<'_>],
    rows: &[CalibrationRow<'_>],
) -> Result<Vec<GeneralizedTacticControlMetrics>, GeneralizedTacticCalibrationError> {
    let mut learner = Vec::new();
    let mut action_mean = Vec::new();
    let learner_calibration = score_calibration(
        validation
            .iter()
            .map(|row| {
                let context = GeneralizedTacticContext::from_facts(&row.transition.before)?;
                let estimate = models.learner.predict(
                    &row.transition.value_sample.state,
                    &context,
                    &row.transition.value_sample.action,
                )?;
                Ok((
                    f64::from(estimate.outcome.goal_progress_per_tick),
                    row.target,
                ))
            })
            .collect::<Result<Vec<_>, GeneralizedTacticCalibrationError>>()?
            .as_slice(),
    );
    for row in rows {
        let context = GeneralizedTacticContext::from_facts(&row.transition.before)?;
        let estimate = models.learner.predict(
            &row.transition.value_sample.state,
            &context,
            &row.transition.value_sample.action,
        )?;
        learner.push(ControlPrediction {
            transition: row.transition,
            target: row.target,
            value_prediction: learner_calibration.map(|(intercept, slope)| {
                intercept + slope * f64::from(estimate.outcome.goal_progress_per_tick)
            }),
            ordering_score: f64::from(estimate.outcome.goal_progress_per_tick),
            epistemic_signal: Some(f64::from(estimate.nearest_distance)),
        });
        if let Some(predicted) = models
            .action_means
            .get(&action_digest(&row.transition.value_sample.action)?)
        {
            action_mean.push(ControlPrediction {
                transition: row.transition,
                target: row.target,
                value_prediction: Some(*predicted),
                ordering_score: *predicted,
                epistemic_signal: None,
            });
        }
    }
    let scheduler =
        policy_predictions(training, rows, TacticProposalPolicy::StructuredNonLearning)?;
    let random = policy_predictions(training, rows, TacticProposalPolicy::RandomValid)?;
    Ok(vec![
        control_metrics(
            "pre_terminal_goal_relabel_fitted_q_knn",
            GeneralizedTacticControlKind::CalibratedOrderingScore,
            rows.len(),
            &learner,
        ),
        control_metrics(
            "action_mean",
            GeneralizedTacticControlKind::ValuePrediction,
            rows.len(),
            &action_mean,
        ),
        control_metrics(
            "production_scheduler_only",
            GeneralizedTacticControlKind::OrderingPolicy,
            rows.len(),
            &scheduler,
        ),
        control_metrics(
            "production_random_valid",
            GeneralizedTacticControlKind::OrderingPolicy,
            rows.len(),
            &random,
        ),
    ])
}

fn policy_predictions<'a>(
    training: &[CalibrationRow<'_>],
    rows: &[CalibrationRow<'a>],
    policy: TacticProposalPolicy,
) -> Result<Vec<ControlPrediction<'a>>, GeneralizedTacticCalibrationError> {
    let mut trained_actions = BTreeMap::<Digest, BTreeSet<Digest>>::new();
    for row in training {
        trained_actions
            .entry(row.transition.before_state_sha256)
            .or_default()
            .insert(action_digest(&row.transition.value_sample.action)?);
    }
    let mut by_state = BTreeMap::<Digest, Vec<&CalibrationRow<'a>>>::new();
    for row in rows {
        by_state
            .entry(row.transition.before_state_sha256)
            .or_default()
            .push(row);
    }
    let mut predictions = Vec::with_capacity(rows.len());
    for (state, state_rows) in by_state {
        let mut actions = BTreeMap::<Digest, (&OptionActionDescriptor, u32)>::new();
        for row in &state_rows {
            let descriptor = &row.transition.value_sample.action;
            let digest = action_digest(descriptor)?;
            let duration = row.transition.value_sample.duration_ticks;
            actions
                .entry(digest)
                .and_modify(|entry| entry.1 = entry.1.max(duration))
                .or_insert((descriptor, duration));
        }
        let choices = actions
            .values()
            .map(|(descriptor, duration)| LearnerActionMaskEntry {
                choice_id: descriptor.option_id.clone(),
                kind: ConcreteTacticChoiceKind::CatalogEntry,
                descriptor: (*descriptor).clone(),
                duration: TacticDurationBounds {
                    minimum_ticks: 1,
                    maximum_ticks: *duration,
                },
                applicable: true,
            })
            .collect::<Vec<_>>();
        let untried = actions
            .iter()
            .filter(|(digest, _)| {
                !trained_actions
                    .get(&state)
                    .is_some_and(|trained| trained.contains(digest))
            })
            .map(|(_, (descriptor, _))| (*descriptor).clone())
            .collect::<Vec<_>>();
        let ranking = LiveTacticRanking {
            learner_snapshot_sha256: state,
            action_universe_sha256: state,
            choices,
            values: AvailableOptionRanking {
                ranked: Vec::new(),
                unsupported: actions
                    .values()
                    .map(|(descriptor, _)| (*descriptor).clone())
                    .collect(),
            },
        };
        let selected = choose_tactic_batch_for_policy(
            &ranking,
            evaluation_decision_index(state),
            TacticExplorationConfig {
                seed: CONTROL_SEED,
                epsilon_per_million: 0,
            },
            &untried,
            actions.len(),
            policy,
        )
        .map_err(|error| {
            GeneralizedTacticCalibrationError::new(format!(
                "production {policy:?} control could not rank held-out actions: {error}"
            ))
        })?;
        let selected_count = selected.len();
        let scores = selected
            .iter()
            .enumerate()
            .map(|(index, proposal)| {
                Ok((
                    action_digest(&proposal.descriptor)?,
                    (selected_count - index) as f64,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, GeneralizedTacticCalibrationError>>()?;
        for row in state_rows {
            let action = action_digest(&row.transition.value_sample.action)?;
            let ordering_score = scores.get(&action).copied().ok_or_else(|| {
                GeneralizedTacticCalibrationError::new(
                    "production control omitted an independently realized action",
                )
            })?;
            predictions.push(ControlPrediction {
                transition: row.transition,
                target: row.target,
                value_prediction: None,
                ordering_score,
                epistemic_signal: None,
            });
        }
    }
    Ok(predictions)
}

fn action_digest(
    descriptor: &OptionActionDescriptor,
) -> Result<Digest, GeneralizedTacticCalibrationError> {
    descriptor
        .content_sha256()
        .map_err(|error| GeneralizedTacticCalibrationError::new(error.to_string()))
}

fn evaluation_decision_index(state: Digest) -> u64 {
    u64::from_le_bytes(state.0[..8].try_into().expect("digest prefix"))
}

fn control_metrics(
    model: &str,
    kind: GeneralizedTacticControlKind,
    evaluation_samples: usize,
    rows: &[ControlPrediction<'_>],
) -> GeneralizedTacticControlMetrics {
    let errors = rows
        .iter()
        .filter_map(|row| row.value_prediction.map(|predicted| predicted - row.target))
        .collect::<Vec<_>>();
    let count = errors.len() as f64;
    let (
        comparable_states,
        comparable_action_pairs,
        correctly_ordered_action_pairs,
        ranking_wins,
        observed_regret,
    ) = control_ranking_metrics(rows);
    let epistemic = rows
        .iter()
        .filter_map(|row| row.epistemic_signal)
        .collect::<Vec<_>>();
    let calibration = calibration_line(rows);
    GeneralizedTacticControlMetrics {
        model: model.into(),
        kind,
        evaluation_samples,
        supported_samples: rows.len(),
        unsupported_samples: evaluation_samples - rows.len(),
        mean_error: (!errors.is_empty()).then(|| errors.iter().sum::<f64>() / count),
        mean_absolute_error: (!errors.is_empty())
            .then(|| errors.iter().map(|error| error.abs()).sum::<f64>() / count),
        root_mean_squared_error: (!errors.is_empty())
            .then(|| (errors.iter().map(|error| error.powi(2)).sum::<f64>() / count).sqrt()),
        calibration_intercept: calibration.map(|line| line.0),
        calibration_slope: calibration.map(|line| line.1),
        mean_epistemic_signal: (!epistemic.is_empty())
            .then_some(epistemic.iter().sum::<f64>() / epistemic.len() as f64),
        comparable_states,
        comparable_action_pairs,
        correctly_ordered_action_pairs,
        pairwise_ordering_rate: (comparable_action_pairs != 0)
            .then_some(correctly_ordered_action_pairs as f64 / comparable_action_pairs as f64),
        ranking_wins,
        ranking_win_rate: (comparable_states != 0)
            .then_some(ranking_wins as f64 / comparable_states as f64),
        mean_observed_regret: (comparable_states != 0)
            .then_some(observed_regret / comparable_states as f64),
    }
}

fn calibration_line(rows: &[ControlPrediction<'_>]) -> Option<(f64, f64)> {
    let values = rows
        .iter()
        .filter_map(|row| {
            row.value_prediction
                .map(|predicted| (predicted, row.target))
        })
        .collect::<Vec<_>>();
    score_calibration(&values)
}

fn score_calibration(values: &[(f64, f64)]) -> Option<(f64, f64)> {
    if values.len() < 2 {
        return None;
    }
    let predicted_mean = values.iter().map(|value| value.0).sum::<f64>() / values.len() as f64;
    let target_mean = values.iter().map(|value| value.1).sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| (value.0 - predicted_mean).powi(2))
        .sum::<f64>();
    if variance <= f64::EPSILON {
        return None;
    }
    let slope = values
        .iter()
        .map(|value| (value.0 - predicted_mean) * (value.1 - target_mean))
        .sum::<f64>()
        / variance;
    Some((target_mean - slope * predicted_mean, slope))
}

fn control_ranking_metrics(rows: &[ControlPrediction<'_>]) -> (usize, usize, usize, usize, f64) {
    let mut by_state = BTreeMap::<Digest, BTreeMap<Digest, (f64, f64, usize)>>::new();
    for row in rows {
        let action = action_digest(&row.transition.value_sample.action)
            .expect("validated transition has a valid action identity");
        let aggregate = by_state
            .entry(row.transition.before_state_sha256)
            .or_default()
            .entry(action)
            .or_default();
        aggregate.0 += row.target;
        aggregate.1 += row.ordering_score;
        aggregate.2 += 1;
    }
    let mut comparable = 0;
    let mut comparable_pairs = 0;
    let mut correct_pairs = 0;
    let mut wins = 0;
    let mut regret = 0.0;
    for state_rows in by_state.values().filter(|rows| rows.len() >= 2) {
        let actions = state_rows
            .values()
            .map(|(target, score, count)| (target / *count as f64, score / *count as f64))
            .collect::<Vec<_>>();
        comparable += 1;
        let selected = actions
            .iter()
            .max_by(|left, right| left.1.total_cmp(&right.1))
            .expect("nonempty state rows");
        let best = actions
            .iter()
            .map(|row| row.0)
            .max_by(f64::total_cmp)
            .expect("nonempty state rows");
        wins += usize::from(selected.0 == best);
        regret += best - selected.0;
        for left in 0..actions.len() {
            for right in (left + 1)..actions.len() {
                let target_order = actions[left].0.total_cmp(&actions[right].0);
                if target_order.is_eq() {
                    continue;
                }
                comparable_pairs += 1;
                correct_pairs +=
                    usize::from(actions[left].1.total_cmp(&actions[right].1) == target_order);
            }
        }
    }
    (comparable, comparable_pairs, correct_pairs, wins, regret)
}

fn comparison_axis_is_valid(axis: &GeneralizedTacticAxisControlComparison) -> bool {
    let training = axis.training_groups.iter().collect::<BTreeSet<_>>();
    let validation = axis.validation_groups.iter().collect::<BTreeSet<_>>();
    let test = axis.test_groups.iter().collect::<BTreeSet<_>>();
    let expected_models = [
        (
            "pre_terminal_goal_relabel_fitted_q_knn",
            GeneralizedTacticControlKind::CalibratedOrderingScore,
        ),
        ("action_mean", GeneralizedTacticControlKind::ValuePrediction),
        (
            "production_scheduler_only",
            GeneralizedTacticControlKind::OrderingPolicy,
        ),
        (
            "production_random_valid",
            GeneralizedTacticControlKind::OrderingPolicy,
        ),
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
        && axis.target_kind
            == "training_terminal_paths_evaluated_against_full_terminal_conditional_ticks"
        && axis.models.len() == expected_models.len()
        && axis
            .models
            .iter()
            .zip(expected_models)
            .all(|(metrics, (name, kind))| {
                metrics.model == name && metrics.kind == kind && control_metrics_are_valid(metrics)
            })
}

fn control_metrics_are_valid(metrics: &GeneralizedTacticControlMetrics) -> bool {
    metrics.evaluation_samples > 0
        && metrics.supported_samples + metrics.unsupported_samples == metrics.evaluation_samples
        && metrics.comparable_states >= metrics.ranking_wins
        && metrics.comparable_action_pairs >= metrics.correctly_ordered_action_pairs
        && [
            metrics.mean_error,
            metrics.mean_absolute_error,
            metrics.root_mean_squared_error,
            metrics.calibration_intercept,
            metrics.calibration_slope,
        ]
        .iter()
        .all(|value| value.is_none_or(f64::is_finite))
        && metrics.mean_absolute_error.is_none_or(|value| value >= 0.0)
        && metrics
            .root_mean_squared_error
            .is_none_or(|value| value >= 0.0)
        && match metrics.kind {
            GeneralizedTacticControlKind::CalibratedOrderingScore => {
                metrics.supported_samples == metrics.evaluation_samples
                    && (metrics.mean_error.is_some()
                        == (metrics.mean_absolute_error.is_some()
                            && metrics.root_mean_squared_error.is_some()))
            }
            GeneralizedTacticControlKind::ValuePrediction => {
                (metrics.supported_samples == 0)
                    == (metrics.mean_error.is_none()
                        && metrics.mean_absolute_error.is_none()
                        && metrics.root_mean_squared_error.is_none())
            }
            GeneralizedTacticControlKind::OrderingPolicy => {
                metrics.supported_samples == metrics.evaluation_samples
                    && metrics.mean_error.is_none()
                    && metrics.mean_absolute_error.is_none()
                    && metrics.root_mean_squared_error.is_none()
                    && metrics.calibration_intercept.is_none()
                    && metrics.calibration_slope.is_none()
            }
        }
        && metrics
            .mean_epistemic_signal
            .is_none_or(|value| value.is_finite() && value >= 0.0)
        && metrics
            .pairwise_ordering_rate
            .is_none_or(|value| value.is_finite() && (0.0..=1.0).contains(&value))
        && (metrics.comparable_action_pairs != 0) == metrics.pairwise_ordering_rate.is_some()
        && metrics
            .ranking_win_rate
            .is_none_or(|value| value.is_finite() && (0.0..=1.0).contains(&value))
        && metrics
            .mean_observed_regret
            .is_none_or(|value| value.is_finite() && value >= 0.0)
}
