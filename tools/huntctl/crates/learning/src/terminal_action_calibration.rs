//! Held-out deployment gate for native-terminal action ranking.
//!
//! A first successful route supplies terminal targets, but it does not prove
//! that a state-action regressor can choose between alternative controllers.
//! This evaluator withholds complete native source states, refits on the
//! remaining graph, and compares the predicted best executable action with
//! exact authenticated ticks-to-terminal for same-state siblings. Until the
//! ranking repeatedly beats the chance rate of those sibling sets, the model
//! may request an evaluated proposal but cannot control the retained action.

use crate::generalized_tactic_value::fitted_q::fit_transition_returns;
use crate::generalized_tactic_value::{GeneralizedTacticContext, GeneralizedTacticValueError};
use crate::option_transition::OptionTransitionSample;
use crate::stable_group_fold::stable_group_fold;
use crate::tactic_value_treatment::ContinuousTacticDoubleQModel;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const TERMINAL_ACTION_CALIBRATION_SCHEMA_V1: &str = "dusklight-terminal-action-calibration/v1";
pub const TERMINAL_ACTION_CALIBRATION_SCHEMA_V2: &str = "dusklight-terminal-action-calibration/v2";
pub const TERMINAL_ACTION_CALIBRATION_FOLDS: usize = 4;
pub const MINIMUM_TERMINAL_ACTION_CALIBRATION_GROUPS: usize = 8;
const COMPARISON_EPSILON_TICKS: f32 = 1.0e-4;
const WILSON_Z_95: f64 = 1.959_963_984_540_054;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalActionCalibration {
    pub schema: String,
    pub source_transitions: usize,
    pub source_state_groups: usize,
    pub folds: usize,
    pub fitted_folds: usize,
    pub terminal_supported_transitions: usize,
    pub evaluated_action_predictions: usize,
    pub comparable_state_groups: usize,
    pub ranking_wins: usize,
    pub ranking_win_rate: Option<f64>,
    pub chance_win_rate: Option<f64>,
    pub ranking_win_rate_wilson_lower_bound: Option<f64>,
    pub mean_observed_regret_ticks: Option<f64>,
    pub deployment_ready: bool,
}

impl TerminalActionCalibration {
    pub fn validate(&self) -> Result<(), GeneralizedTacticValueError> {
        let expected_ready = readiness(
            self.fitted_folds,
            self.comparable_state_groups,
            self.ranking_win_rate_wilson_lower_bound,
            self.chance_win_rate,
        );
        if !matches!(
            self.schema.as_str(),
            TERMINAL_ACTION_CALIBRATION_SCHEMA_V1 | TERMINAL_ACTION_CALIBRATION_SCHEMA_V2
        ) || self.folds != TERMINAL_ACTION_CALIBRATION_FOLDS
            || self.fitted_folds > self.folds
            || self.source_state_groups > self.source_transitions
            || self.terminal_supported_transitions > self.source_transitions
            || self.comparable_state_groups > self.source_state_groups
            || self.ranking_wins > self.comparable_state_groups
            || self.evaluated_action_predictions < self.comparable_state_groups.saturating_mul(2)
            || [
                self.ranking_win_rate,
                self.chance_win_rate,
                self.ranking_win_rate_wilson_lower_bound,
            ]
            .into_iter()
            .flatten()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
            || self
                .mean_observed_regret_ticks
                .is_some_and(|value| !value.is_finite() || value < 0.0)
            || (self.comparable_state_groups == 0)
                != (self.ranking_win_rate.is_none()
                    && self.chance_win_rate.is_none()
                    && self.ranking_win_rate_wilson_lower_bound.is_none()
                    && self.mean_observed_regret_ticks.is_none())
            || self.deployment_ready != expected_ready
        {
            return Err(GeneralizedTacticValueError::InvalidConfig);
        }
        Ok(())
    }
}

pub fn calibrate_terminal_action_ranking(
    transitions: &[OptionTransitionSample],
    goal_distance_feature: usize,
    fitted_q_iterations: usize,
    per_tick_discount: f32,
    universal_action_head: bool,
) -> Result<TerminalActionCalibration, GeneralizedTacticValueError> {
    for transition in transitions {
        transition
            .validate()
            .map_err(|error| GeneralizedTacticValueError::InvalidTransition(error.to_string()))?;
        if goal_distance_feature >= transition.value_sample.state.len() {
            return Err(GeneralizedTacticValueError::FeatureWidth);
        }
    }
    let targets =
        fit_transition_returns(transitions, fitted_q_iterations, per_tick_discount)?.values;
    let groups = transitions.iter().enumerate().fold(
        BTreeMap::<_, Vec<usize>>::new(),
        |mut groups, (index, transition)| {
            groups
                .entry(transition.before_state_sha256)
                .or_default()
                .push(index);
            groups
        },
    );
    let indexed_groups = groups.into_iter().collect::<Vec<_>>();
    let terminal_supported_transitions = targets.iter().flatten().count();
    let mut fitted_folds = 0_usize;
    let mut evaluated_action_predictions = 0_usize;
    let mut comparable_state_groups = 0_usize;
    let mut ranking_wins = 0_usize;
    let mut chance_sum = 0.0_f64;
    let mut regret_sum = 0.0_f64;

    for fold in 0..TERMINAL_ACTION_CALIBRATION_FOLDS {
        let training = indexed_groups
            .iter()
            .filter(|(state, _)| {
                stable_group_fold(*state, TERMINAL_ACTION_CALIBRATION_FOLDS) != fold
            })
            .flat_map(|(_, rows)| rows.iter().map(|index| transitions[*index].clone()))
            .collect::<Vec<_>>();
        let fitted = if universal_action_head {
            ContinuousTacticDoubleQModel::fit_universal_action_head(
                &training,
                goal_distance_feature,
                fitted_q_iterations,
                per_tick_discount,
            )
        } else {
            ContinuousTacticDoubleQModel::fit(
                &training,
                goal_distance_feature,
                fitted_q_iterations,
                per_tick_discount,
            )
        };
        let model = match fitted {
            Ok(model) => model,
            Err(GeneralizedTacticValueError::SampleCount) => continue,
            Err(error) => return Err(error),
        };
        fitted_folds = fitted_folds.saturating_add(1);
        for (_, rows) in indexed_groups.iter().filter(|(state, _)| {
            stable_group_fold(*state, TERMINAL_ACTION_CALIBRATION_FOLDS) == fold
        }) {
            evaluate_group(
                &model,
                transitions,
                &targets,
                rows,
                &mut evaluated_action_predictions,
                &mut comparable_state_groups,
                &mut ranking_wins,
                &mut chance_sum,
                &mut regret_sum,
            )?;
        }
    }

    let ranking_win_rate = ratio(ranking_wins, comparable_state_groups);
    let chance_win_rate =
        (comparable_state_groups != 0).then_some(chance_sum / comparable_state_groups as f64);
    let ranking_win_rate_wilson_lower_bound = (comparable_state_groups != 0)
        .then_some(wilson_lower_bound(ranking_wins, comparable_state_groups));
    let mut report = TerminalActionCalibration {
        schema: TERMINAL_ACTION_CALIBRATION_SCHEMA_V2.into(),
        source_transitions: transitions.len(),
        source_state_groups: indexed_groups.len(),
        folds: TERMINAL_ACTION_CALIBRATION_FOLDS,
        fitted_folds,
        terminal_supported_transitions,
        evaluated_action_predictions,
        comparable_state_groups,
        ranking_wins,
        ranking_win_rate,
        chance_win_rate,
        ranking_win_rate_wilson_lower_bound,
        mean_observed_regret_ticks: (comparable_state_groups != 0)
            .then_some(regret_sum / comparable_state_groups as f64),
        deployment_ready: false,
    };
    report.deployment_ready = readiness(
        report.fitted_folds,
        report.comparable_state_groups,
        report.ranking_win_rate_wilson_lower_bound,
        report.chance_win_rate,
    );
    report.validate()?;
    Ok(report)
}

#[allow(clippy::too_many_arguments)]
fn evaluate_group(
    model: &ContinuousTacticDoubleQModel,
    transitions: &[OptionTransitionSample],
    targets: &[Option<f32>],
    rows: &[usize],
    evaluated_action_predictions: &mut usize,
    comparable_state_groups: &mut usize,
    ranking_wins: &mut usize,
    chance_sum: &mut f64,
    regret_sum: &mut f64,
) -> Result<(), GeneralizedTacticValueError> {
    let Some(first_index) = rows.first() else {
        return Ok(());
    };
    let first = &transitions[*first_index];
    let mut supported = Vec::<(&OptionTransitionSample, f32)>::new();
    for index in rows {
        let transition = &transitions[*index];
        if transition.before_state_sha256 != first.before_state_sha256
            || transition.value_sample.state != first.value_sample.state
        {
            return Err(GeneralizedTacticValueError::InvalidTransition(
                "terminal action calibration source-state group is inconsistent".into(),
            ));
        }
        if let Some(target) = targets[*index] {
            if supported.iter().any(|(existing, existing_target)| {
                existing.value_sample.action == transition.value_sample.action
                    && existing_target.to_bits() != target.to_bits()
            }) {
                return Err(GeneralizedTacticValueError::InvalidTransition(
                    "terminal action calibration has conflicting duplicate actions".into(),
                ));
            }
            if !supported
                .iter()
                .any(|(existing, _)| existing.value_sample.action == transition.value_sample.action)
            {
                supported.push((transition, target));
            }
        }
    }
    if supported.len() < 2 {
        return Ok(());
    }
    let descriptors = supported
        .iter()
        .map(|(transition, _)| transition.value_sample.action.clone())
        .collect::<Vec<_>>();
    let context = GeneralizedTacticContext::from_facts(&first.before)?;
    let estimates = model.rank(&first.value_sample.state, &context, &descriptors)?;
    let ranked = estimates
        .iter()
        .filter_map(|estimate| {
            supported.iter().find_map(|(transition, target)| {
                (transition.value_sample.action == estimate.descriptor).then_some(*target)
            })
        })
        .collect::<Vec<_>>();
    *evaluated_action_predictions = evaluated_action_predictions.saturating_add(ranked.len());
    if ranked.len() < 2 {
        return Ok(());
    }
    let selected = ranked[0];
    let best = ranked
        .iter()
        .copied()
        .max_by(f32::total_cmp)
        .expect("comparable terminal action group");
    *comparable_state_groups = comparable_state_groups.saturating_add(1);
    *ranking_wins =
        ranking_wins.saturating_add(usize::from(selected >= best - COMPARISON_EPSILON_TICKS));
    *chance_sum += 1.0 / ranked.len() as f64;
    *regret_sum += f64::from((best - selected).max(0.0));
    Ok(())
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
    fitted_folds: usize,
    comparable_state_groups: usize,
    lower_bound: Option<f64>,
    chance_rate: Option<f64>,
) -> bool {
    fitted_folds == TERMINAL_ACTION_CALIBRATION_FOLDS
        && comparable_state_groups >= MINIMUM_TERMINAL_ACTION_CALIBRATION_GROUPS
        && lower_bound
            .zip(chance_rate)
            .is_some_and(|(lower, chance)| lower > chance)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::Digest;
    use crate::fact_snapshot::{FactSnapshot, FactTerminalReason};
    use crate::tactic_features::GoalConditionedTacticFeatureEncoder;
    use crate::tape::{InputFrame, InputTape};
    use dusklight_control::option_execution::{
        OptionCondition, OptionEndReason, OptionExecution, OptionParameter, OptionType, TapeRange,
    };
    use dusklight_evidence::native_episode_shard::NativeEpisodeShard;
    use std::collections::BTreeMap;

    fn terminal_transition(state_group: usize, duration: u32) -> OptionTransitionSample {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        let step = &shard.episodes[0].steps[0];
        let mut before =
            FactSnapshot::from_native_learning(&step.pre_input, &[], None, Vec::new()).unwrap();
        let mut after = FactSnapshot::from_native_learning(
            &step.post_simulation,
            &[step.pre_input.clone()],
            None,
            Vec::new(),
        )
        .unwrap();
        before.terminal.configured = Some(true);
        before.terminal.reached = Some(false);
        before.terminal.reason = FactTerminalReason::None;
        let x = state_group as f32 * 32.0;
        before.player.position_f32_bits[0] = x.to_bits();
        before.player.position_f32_bits[2] = 0.0_f32.to_bits();
        after.world = before.world.clone();
        after.player = before.player.clone();
        after.event = before.event.clone();
        after.channels = before.channels.clone();
        after.boundary_index = before.boundary_index.saturating_add(1);
        after.tape_frame = before.tape_frame + u64::from(duration) - 1;
        after.simulation_tick = before.simulation_tick + u64::from(duration) - 1;
        after.player.position_f32_bits[0] = (x + 1.0).to_bits();
        after.terminal.configured = Some(true);
        after.terminal.reached = Some(true);
        after.terminal.reason = FactTerminalReason::GoalReached;
        after.terminal.first_hit_tick = Some(after.simulation_tick);
        let mut tape = InputTape {
            frames: vec![InputFrame::default(); after.tape_frame as usize + 1],
            ..InputTape::default()
        };
        tape.frames[before.tape_frame as usize].pads[0].stick_y = 100;
        let action = format!("duration-{duration}");
        let execution = OptionExecution::capture(
            action,
            OptionType::Move,
            BTreeMap::from([(
                "duration_ticks".into(),
                OptionParameter::Unsigned(u64::from(duration)),
            )]),
            duration,
            duration,
            OptionCondition::DurationElapsed,
            Vec::new(),
            OptionEndReason::Completed,
            &tape,
            TapeRange {
                start_frame: before.tape_frame,
                end_frame_exclusive: after.tape_frame + 1,
            },
        )
        .unwrap();
        let encoder = GoalConditionedTacticFeatureEncoder::new([10_000.0, 0.0, 0.0]).unwrap();
        OptionTransitionSample::capture(
            encoder.schema_sha256,
            Digest([2; 32]),
            Digest([3; 32]),
            before,
            after,
            execution,
            &tape,
            -(duration as f32),
            true,
            |facts| encoder.encode(facts),
        )
        .unwrap()
    }

    #[test]
    fn empty_calibration_is_valid_and_cannot_deploy() {
        let report = calibrate_terminal_action_ranking(&[], 0, 8, 0.999, true).unwrap();
        assert_eq!(report.schema, TERMINAL_ACTION_CALIBRATION_SCHEMA_V2);
        assert_eq!(report.source_transitions, 0);
        assert_eq!(report.fitted_folds, 0);
        assert!(!report.deployment_ready);
        report.validate().unwrap();
    }

    #[test]
    fn readiness_requires_all_folds_and_repeated_better_than_chance_ranking() {
        assert!(!readiness(3, 32, Some(1.0), Some(0.5)));
        assert!(!readiness(4, 7, Some(1.0), Some(0.5)));
        assert!(!readiness(
            4,
            16,
            Some(wilson_lower_bound(11, 16)),
            Some(0.5)
        ));
        assert!(readiness(
            4,
            16,
            Some(wilson_lower_bound(12, 16)),
            Some(0.5)
        ));
    }

    #[test]
    fn held_out_same_state_siblings_can_earn_deployment_authority() {
        let corpus = (0..16)
            .flat_map(|state| [1, 4].map(move |duration| terminal_transition(state, duration)))
            .collect::<Vec<_>>();
        let report = calibrate_terminal_action_ranking(&corpus, 0, 16, 1.0, true).unwrap();

        report.validate().unwrap();
        assert_eq!(report.fitted_folds, TERMINAL_ACTION_CALIBRATION_FOLDS);
        assert_eq!(report.comparable_state_groups, 16);
        assert_eq!(report.ranking_wins, 16);
        assert!(report.deployment_ready);
    }
}
