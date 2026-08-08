//! Whole-state held-out calibration for pre-terminal goal reachability.
//!
//! Achieved-goal relabeling can create many correlated samples from very few
//! native transitions. This gate withholds complete source states, fits only
//! on other native states, and asks whether the predicted best executable
//! action beats the chance rate implied by each held-out action set. Until it
//! does, reachability remains sibling evidence and cannot control behavior.

use crate::generalized_tactic_value::{
    GeneralizedTacticContext, GeneralizedTacticOutcome, GeneralizedTacticValueError,
    GeneralizedTacticValueModel,
};
use crate::option_transition::OptionTransitionSample;
use crate::stable_group_fold::stable_group_fold;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const GOAL_REACHABILITY_CALIBRATION_SCHEMA_V1: &str =
    "dusklight-goal-reachability-calibration/v1";
pub const GOAL_REACHABILITY_CALIBRATION_SCHEMA_V2: &str =
    "dusklight-goal-reachability-calibration/v2";
pub const GOAL_REACHABILITY_CALIBRATION_FOLDS: usize = 4;
pub const MINIMUM_GOAL_REACHABILITY_CALIBRATION_GROUPS: usize = 8;
const COMPARISON_EPSILON: f32 = 1.0e-4;
const WILSON_Z_95: f64 = 1.959_963_984_540_054;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GoalReachabilityCalibration {
    pub schema: String,
    pub source_transitions: usize,
    pub source_state_groups: usize,
    pub folds: usize,
    pub evaluated_action_predictions: usize,
    pub comparable_state_groups: usize,
    pub ranking_wins: usize,
    pub ranking_win_rate: Option<f64>,
    pub chance_win_rate: Option<f64>,
    pub ranking_win_rate_wilson_lower_bound: Option<f64>,
    pub mean_observed_regret: Option<f64>,
    pub mean_absolute_progress_error: Option<f64>,
    pub progress_sign_accuracy: Option<f64>,
    pub deployment_ready: bool,
}

/// One held-out action prediction and its authentic realized outcome.
///
/// This is diagnostic evidence, not model authority. It deliberately retains
/// the typed action descriptor so a ranking failure can be reproduced without
/// reverse-engineering an option ID.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GoalReachabilityActionDiagnosis {
    pub descriptor: crate::option_values::OptionActionDescriptor,
    pub predicted_goal_progress_per_tick: f32,
    pub observed_goal_progress_per_tick: f32,
    pub nearest_distance: f32,
    pub neighbors: usize,
    pub selected: bool,
    pub observed_best: bool,
}

/// Exact same-state sibling ranking used by the deployment calibration.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GoalReachabilityGroupDiagnosis {
    pub source_state_sha256: dusklight_automation_contracts::artifact::Digest,
    pub fold: usize,
    pub ranking_win: bool,
    pub observed_regret: f32,
    pub actions: Vec<GoalReachabilityActionDiagnosis>,
}

/// Aggregate authority decision plus every comparable held-out ranking behind
/// it. Production snapshots retain only `GoalReachabilityCalibration`; this
/// larger form is materialized solely by explicit diagnostics.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GoalReachabilityCalibrationDiagnosis {
    pub calibration: GoalReachabilityCalibration,
    pub groups: Vec<GoalReachabilityGroupDiagnosis>,
}

impl GoalReachabilityCalibration {
    pub fn validate(&self) -> Result<(), GeneralizedTacticValueError> {
        let rates = [
            self.ranking_win_rate,
            self.chance_win_rate,
            self.ranking_win_rate_wilson_lower_bound,
            self.progress_sign_accuracy,
        ];
        let expected_ready = readiness(
            self.comparable_state_groups,
            self.ranking_win_rate_wilson_lower_bound,
            self.chance_win_rate,
        );
        if !matches!(
            self.schema.as_str(),
            GOAL_REACHABILITY_CALIBRATION_SCHEMA_V1 | GOAL_REACHABILITY_CALIBRATION_SCHEMA_V2
        ) || self.folds != GOAL_REACHABILITY_CALIBRATION_FOLDS
            || self.source_state_groups > self.source_transitions
            || self.comparable_state_groups > self.source_state_groups
            || self.ranking_wins > self.comparable_state_groups
            || rates
                .into_iter()
                .flatten()
                .any(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
            || self
                .mean_observed_regret
                .is_some_and(|value| !value.is_finite() || value < 0.0)
            || self
                .mean_absolute_progress_error
                .is_some_and(|value| !value.is_finite() || value < 0.0)
            || (self.comparable_state_groups == 0)
                != (self.ranking_win_rate.is_none()
                    && self.chance_win_rate.is_none()
                    && self.ranking_win_rate_wilson_lower_bound.is_none()
                    && self.mean_observed_regret.is_none())
            || (self.evaluated_action_predictions == 0)
                != (self.mean_absolute_progress_error.is_none()
                    && self.progress_sign_accuracy.is_none())
            || self.deployment_ready != expected_ready
        {
            return Err(GeneralizedTacticValueError::InvalidConfig);
        }
        Ok(())
    }
}

pub fn calibrate_goal_reachability(
    transitions: &[OptionTransitionSample],
    goal_distance_feature: usize,
) -> Result<GoalReachabilityCalibration, GeneralizedTacticValueError> {
    calibrate_goal_reachability_with_diagnosis(transitions, goal_distance_feature)
        .map(|diagnosis| diagnosis.calibration)
}

pub fn calibrate_goal_reachability_with_diagnosis(
    transitions: &[OptionTransitionSample],
    goal_distance_feature: usize,
) -> Result<GoalReachabilityCalibrationDiagnosis, GeneralizedTacticValueError> {
    for transition in transitions {
        transition
            .validate()
            .map_err(|error| GeneralizedTacticValueError::InvalidTransition(error.to_string()))?;
        if goal_distance_feature >= transition.value_sample.state.len() {
            return Err(GeneralizedTacticValueError::FeatureWidth);
        }
    }
    let groups = transitions.iter().fold(
        BTreeMap::<_, Vec<&OptionTransitionSample>>::new(),
        |mut groups, transition| {
            groups
                .entry(transition.before_state_sha256)
                .or_default()
                .push(transition);
            groups
        },
    );
    let indexed_groups = groups.into_iter().collect::<Vec<_>>();
    let mut evaluated_action_predictions = 0_usize;
    let mut comparable_state_groups = 0_usize;
    let mut ranking_wins = 0_usize;
    let mut chance_sum = 0.0_f64;
    let mut regret_sum = 0.0_f64;
    let mut absolute_error_sum = 0.0_f64;
    let mut sign_correct = 0_usize;
    let mut group_diagnoses = Vec::new();

    for fold in 0..GOAL_REACHABILITY_CALIBRATION_FOLDS {
        let training = indexed_groups
            .iter()
            .filter(|(state, _)| {
                stable_group_fold(*state, GOAL_REACHABILITY_CALIBRATION_FOLDS) != fold
            })
            .flat_map(|(_, rows)| rows.iter().copied().cloned())
            .collect::<Vec<_>>();
        if training.len() < 2 {
            continue;
        }
        let model = GeneralizedTacticValueModel::fit_achieved_goal_returns(
            &training,
            goal_distance_feature,
        )?;
        for (_, rows) in indexed_groups.iter().filter(|(state, _)| {
            stable_group_fold(*state, GOAL_REACHABILITY_CALIBRATION_FOLDS) == fold
        }) {
            if let Some(diagnosis) = evaluate_group(
                &model,
                rows,
                fold,
                goal_distance_feature,
                &mut evaluated_action_predictions,
                &mut comparable_state_groups,
                &mut ranking_wins,
                &mut chance_sum,
                &mut regret_sum,
                &mut absolute_error_sum,
                &mut sign_correct,
            )? {
                group_diagnoses.push(diagnosis);
            }
        }
    }

    let ranking_win_rate = ratio(ranking_wins, comparable_state_groups);
    let chance_win_rate =
        (comparable_state_groups != 0).then_some(chance_sum / comparable_state_groups as f64);
    let ranking_win_rate_wilson_lower_bound = (comparable_state_groups != 0)
        .then_some(wilson_lower_bound(ranking_wins, comparable_state_groups));
    let mut report = GoalReachabilityCalibration {
        schema: GOAL_REACHABILITY_CALIBRATION_SCHEMA_V2.into(),
        source_transitions: transitions.len(),
        source_state_groups: indexed_groups.len(),
        folds: GOAL_REACHABILITY_CALIBRATION_FOLDS,
        evaluated_action_predictions,
        comparable_state_groups,
        ranking_wins,
        ranking_win_rate,
        chance_win_rate,
        ranking_win_rate_wilson_lower_bound,
        mean_observed_regret: (comparable_state_groups != 0)
            .then_some(regret_sum / comparable_state_groups as f64),
        mean_absolute_progress_error: (evaluated_action_predictions != 0)
            .then_some(absolute_error_sum / evaluated_action_predictions as f64),
        progress_sign_accuracy: ratio(sign_correct, evaluated_action_predictions),
        deployment_ready: false,
    };
    report.deployment_ready = readiness(
        report.comparable_state_groups,
        report.ranking_win_rate_wilson_lower_bound,
        report.chance_win_rate,
    );
    report.validate()?;
    Ok(GoalReachabilityCalibrationDiagnosis {
        calibration: report,
        groups: group_diagnoses,
    })
}

#[allow(clippy::too_many_arguments)]
fn evaluate_group(
    model: &GeneralizedTacticValueModel,
    rows: &[&OptionTransitionSample],
    fold: usize,
    goal_distance_feature: usize,
    evaluated_action_predictions: &mut usize,
    comparable_state_groups: &mut usize,
    ranking_wins: &mut usize,
    chance_sum: &mut f64,
    regret_sum: &mut f64,
    absolute_error_sum: &mut f64,
    sign_correct: &mut usize,
) -> Result<Option<GoalReachabilityGroupDiagnosis>, GeneralizedTacticValueError> {
    let Some(first) = rows.first() else {
        return Ok(None);
    };
    let descriptors = rows
        .iter()
        .map(|transition| transition.value_sample.action.clone())
        .collect::<Vec<_>>();
    if descriptors
        .iter()
        .enumerate()
        .any(|(index, descriptor)| descriptors[..index].contains(descriptor))
        || rows.iter().any(|transition| {
            transition.before_state_sha256 != first.before_state_sha256
                || transition.value_sample.state != first.value_sample.state
        })
    {
        return Err(GeneralizedTacticValueError::InvalidTransition(
            "goal reachability calibration source-state group is inconsistent".into(),
        ));
    }
    let context = GeneralizedTacticContext::from_facts(&first.before)?;
    let estimates =
        model.rank_goal_reachability(&first.value_sample.state, &context, &descriptors)?;
    let actual = rows
        .iter()
        .map(|transition| {
            Ok((
                transition.value_sample.action.clone(),
                GeneralizedTacticOutcome::from_transition(transition, goal_distance_feature)?
                    .goal_progress_per_tick,
            ))
        })
        .collect::<Result<Vec<_>, GeneralizedTacticValueError>>()?;
    for estimate in &estimates {
        let observed = actual
            .iter()
            .find_map(|(descriptor, observed)| {
                (descriptor == &estimate.descriptor).then_some(*observed)
            })
            .ok_or_else(|| {
                GeneralizedTacticValueError::InvalidTransition(
                    "goal reachability calibration prediction is detached".into(),
                )
            })?;
        let predicted = estimate.outcome.goal_progress_per_tick;
        *evaluated_action_predictions = evaluated_action_predictions.saturating_add(1);
        *absolute_error_sum += f64::from((predicted - observed).abs());
        if (predicted > 0.0) == (observed > 0.0) {
            *sign_correct = sign_correct.saturating_add(1);
        }
    }
    if actual.len() < 2 {
        return Ok(None);
    }
    let best = actual
        .iter()
        .map(|(_, progress)| *progress)
        .max_by(f32::total_cmp)
        .expect("nonempty held-out action group");
    let selected = estimates
        .first()
        .and_then(|estimate| {
            actual.iter().find_map(|(descriptor, progress)| {
                (descriptor == &estimate.descriptor).then_some(*progress)
            })
        })
        .ok_or_else(|| {
            GeneralizedTacticValueError::InvalidTransition(
                "goal reachability calibration prediction is detached".into(),
            )
        })?;
    *comparable_state_groups = comparable_state_groups.saturating_add(1);
    *ranking_wins = ranking_wins.saturating_add(usize::from(selected >= best - COMPARISON_EPSILON));
    *chance_sum += 1.0 / actual.len() as f64;
    let observed_regret = (best - selected).max(0.0);
    *regret_sum += f64::from(observed_regret);
    let actions = estimates
        .iter()
        .enumerate()
        .map(|(index, estimate)| {
            let observed_goal_progress_per_tick = actual
                .iter()
                .find_map(|(descriptor, observed)| {
                    (descriptor == &estimate.descriptor).then_some(*observed)
                })
                .expect("validated prediction remains attached to held-out outcome");
            GoalReachabilityActionDiagnosis {
                descriptor: estimate.descriptor.clone(),
                predicted_goal_progress_per_tick: estimate.outcome.goal_progress_per_tick,
                observed_goal_progress_per_tick,
                nearest_distance: estimate.nearest_distance,
                neighbors: estimate.neighbors,
                selected: index == 0,
                observed_best: observed_goal_progress_per_tick >= best - COMPARISON_EPSILON,
            }
        })
        .collect();
    Ok(Some(GoalReachabilityGroupDiagnosis {
        source_state_sha256: first.before_state_sha256,
        fold,
        ranking_win: selected >= best - COMPARISON_EPSILON,
        observed_regret,
        actions,
    }))
}

fn ratio(numerator: usize, denominator: usize) -> Option<f64> {
    (denominator != 0).then_some(numerator as f64 / denominator as f64)
}

fn wilson_lower_bound(wins: usize, total: usize) -> f64 {
    if total == 0 {
        return 0.0;
    }
    let n = total as f64;
    let proportion = wins as f64 / n;
    let z_squared = WILSON_Z_95 * WILSON_Z_95;
    let denominator = 1.0 + z_squared / n;
    let center = proportion + z_squared / (2.0 * n);
    let radius =
        WILSON_Z_95 * ((proportion * (1.0 - proportion) + z_squared / (4.0 * n)) / n).sqrt();
    ((center - radius) / denominator).clamp(0.0, 1.0)
}

fn readiness(
    comparable_state_groups: usize,
    lower_bound: Option<f64>,
    chance_rate: Option<f64>,
) -> bool {
    comparable_state_groups >= MINIMUM_GOAL_REACHABILITY_CALIBRATION_GROUPS
        && lower_bound
            .zip(chance_rate)
            .is_some_and(|(lower, chance)| lower > chance)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wilson_readiness_requires_repeated_better_than_chance_ranking() {
        assert!(!readiness(7, Some(1.0), Some(0.5)));
        assert!(!readiness(16, Some(wilson_lower_bound(11, 16)), Some(0.5)));
        assert!(readiness(16, Some(wilson_lower_bound(12, 16)), Some(0.5)));
        assert!(wilson_lower_bound(12, 16) > 0.5);
    }

    #[test]
    fn empty_calibration_is_valid_and_cannot_deploy() {
        let report = calibrate_goal_reachability(&[], 0).unwrap();
        assert_eq!(report.schema, GOAL_REACHABILITY_CALIBRATION_SCHEMA_V2);
        assert_eq!(report.source_transitions, 0);
        assert!(!report.deployment_ready);
        report.validate().unwrap();
    }
}
