//! Explicit state-action value treatments for live tactic acquisition.
//!
//! A treatment is part of execution identity. The continuous forest embeds an
//! executable action descriptor beside the typed state, then regresses the
//! same delayed fitted-Q return used by the local generalized control. It does
//! not consume trajectory outcomes as utility.

use crate::double_q::{DoubleQ, DoubleQConfig};
use crate::fqi::{FittedQ, FqiConfig, Transition as FqiTransition};
use crate::generalized_tactic_value::fitted_q::fit_transition_returns;
use crate::generalized_tactic_value::prediction::{action_class, regression_features};
use crate::generalized_tactic_value::{
    GeneralizedTacticContext, GeneralizedTacticValueError, MAX_FITTED_Q_BACKUP_ITERATIONS,
};
use crate::option_transition::OptionTransitionSample;
use crate::option_values::OptionActionDescriptor;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const CONTINUOUS_FOREST_ACTION: u32 = 0;
const CONTINUOUS_FOREST_SEED: u64 = 0x4754_4351_4649_0001;
const CONTINUOUS_DOUBLE_Q_SEED: u64 = 0x4754_4344_5141_0001;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TacticValueTreatment {
    #[default]
    LocalGeneralizedFittedQKnnV1,
    GoalRelabeledFittedQKnnV2,
    ContinuousFittedQForestV1,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContinuousTacticValueEstimate {
    pub descriptor: OptionActionDescriptor,
    pub mean_q: f64,
    pub ensemble_variance: f64,
}

/// Immutable randomized regression forest over continuous state-action
/// features and delayed fitted-Q targets.
#[derive(Clone, Debug, Serialize)]
pub struct ContinuousTacticValueModel {
    forest: FittedQ,
}

/// Continuous state/action regressor used after authenticated terminal support
/// exists.
///
/// The fitted-Q graph supplies delayed native-terminal targets. Descriptor
/// factors remain in the state vector while the broad option type is the
/// discrete Double-Q action head. This is the same held-out control used by
/// generalized tactic calibration, promoted here without changing reward or
/// terminal authority.
#[derive(Clone, Debug, Serialize)]
pub struct ContinuousTacticDoubleQModel {
    model: DoubleQ,
    supported_action_classes: BTreeSet<u32>,
}

impl ContinuousTacticDoubleQModel {
    pub fn fit(
        transitions: &[OptionTransitionSample],
        goal_distance_feature: usize,
        fitted_q_iterations: usize,
        per_tick_discount: f32,
    ) -> Result<Self, GeneralizedTacticValueError> {
        if transitions.len() < 2 {
            return Err(GeneralizedTacticValueError::SampleCount);
        }
        if transitions
            .iter()
            .any(|transition| goal_distance_feature >= transition.value_sample.state.len())
        {
            return Err(GeneralizedTacticValueError::FeatureWidth);
        }
        let targets =
            fit_transition_returns(transitions, fitted_q_iterations, per_tick_discount)?.values;
        let rows = transitions
            .iter()
            .zip(targets)
            .filter_map(|(transition, target)| {
                target.map(|target| {
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
                })
            })
            .collect::<Result<Vec<_>, GeneralizedTacticValueError>>()?;
        if rows.len() < 2 {
            return Err(GeneralizedTacticValueError::SampleCount);
        }
        let supported_action_classes = rows.iter().map(|row| row.action).collect::<BTreeSet<_>>();
        let actions = supported_action_classes.iter().copied().collect::<Vec<_>>();
        let model = DoubleQ::fit(
            rows[0].state.len(),
            &actions,
            &rows,
            &ContinuousTacticDoubleQModel::config(per_tick_discount),
        )
        .map_err(|error| GeneralizedTacticValueError::InvalidTransition(error.to_string()))?;
        Ok(Self {
            model,
            supported_action_classes,
        })
    }

    pub fn rank(
        &self,
        state_features: &[f32],
        context: &GeneralizedTacticContext,
        descriptors: &[OptionActionDescriptor],
    ) -> Result<Vec<ContinuousTacticValueEstimate>, GeneralizedTacticValueError> {
        let mut estimates = descriptors
            .iter()
            .filter(|descriptor| {
                self.supported_action_classes
                    .contains(&action_class(&descriptor.option_type))
            })
            .map(|descriptor| {
                let features = regression_features(state_features, context, descriptor)?;
                let estimate = self
                    .model
                    .estimate(&features, action_class(&descriptor.option_type))
                    .map_err(|error| {
                        GeneralizedTacticValueError::InvalidTransition(error.to_string())
                    })?;
                Ok(ContinuousTacticValueEstimate {
                    descriptor: descriptor.clone(),
                    mean_q: estimate.mean,
                    ensemble_variance: estimate.critic_disagreement.powi(2),
                })
            })
            .collect::<Result<Vec<_>, GeneralizedTacticValueError>>()?;
        estimates.sort_by(|left, right| {
            right
                .mean_q
                .total_cmp(&left.mean_q)
                .then_with(|| left.ensemble_variance.total_cmp(&right.ensemble_variance))
                .then_with(|| left.descriptor.option_id.cmp(&right.descriptor.option_id))
        });
        Ok(estimates)
    }

    fn config(per_tick_discount: f32) -> DoubleQConfig {
        DoubleQConfig {
            epochs: 128,
            hidden_width: 32,
            learning_rate: 0.003,
            discount: f64::from(per_tick_discount),
            target_sync_steps: 64,
            gradient_clip: 10.0,
            seed: CONTINUOUS_DOUBLE_Q_SEED,
        }
    }
}

impl ContinuousTacticValueModel {
    pub fn fit(
        transitions: &[OptionTransitionSample],
        goal_distance_feature: usize,
        fitted_q_iterations: usize,
        per_tick_discount: f32,
    ) -> Result<Self, GeneralizedTacticValueError> {
        if transitions.len() < 2 {
            return Err(GeneralizedTacticValueError::SampleCount);
        }
        if fitted_q_iterations == 0
            || fitted_q_iterations > MAX_FITTED_Q_BACKUP_ITERATIONS
            || !per_tick_discount.is_finite()
            || !(0.0..=1.0).contains(&per_tick_discount)
            || per_tick_discount == 0.0
        {
            return Err(GeneralizedTacticValueError::InvalidConfig);
        }
        if transitions
            .iter()
            .any(|transition| goal_distance_feature >= transition.value_sample.state.len())
        {
            return Err(GeneralizedTacticValueError::FeatureWidth);
        }
        let targets =
            fit_transition_returns(transitions, fitted_q_iterations, per_tick_discount)?.values;
        let rows = transitions
            .iter()
            .zip(targets)
            .filter_map(|(transition, target)| {
                target.map(|target| {
                    let context = GeneralizedTacticContext::from_facts(&transition.before)?;
                    let state = regression_features(
                        &transition.value_sample.state,
                        &context,
                        &transition.value_sample.action,
                    )?;
                    Ok(FqiTransition {
                        state: state.clone(),
                        action: CONTINUOUS_FOREST_ACTION,
                        duration: transition.value_sample.duration_ticks,
                        reward: target,
                        next_state: state,
                        // Delayed credit is already present in `target`. Marking
                        // these regression rows terminal prevents a second Bellman
                        // backup inside the generic forest implementation.
                        terminal: true,
                    })
                })
            })
            .collect::<Result<Vec<_>, GeneralizedTacticValueError>>()?;
        if rows.len() < 2 {
            return Err(GeneralizedTacticValueError::SampleCount);
        }
        let feature_width = rows[0].state.len();
        let forest = FittedQ::fit(
            feature_width,
            &[CONTINUOUS_FOREST_ACTION],
            &rows,
            &continuous_forest_config(),
        )
        .map_err(|error| GeneralizedTacticValueError::InvalidTransition(error.to_string()))?;
        Ok(Self { forest })
    }

    pub fn predict(
        &self,
        state_features: &[f32],
        context: &GeneralizedTacticContext,
        descriptor: &OptionActionDescriptor,
    ) -> Result<ContinuousTacticValueEstimate, GeneralizedTacticValueError> {
        let features = regression_features(state_features, context, descriptor)?;
        let estimate = self
            .forest
            .estimate(&features, CONTINUOUS_FOREST_ACTION)
            .map_err(|error| GeneralizedTacticValueError::InvalidTransition(error.to_string()))?;
        Ok(ContinuousTacticValueEstimate {
            descriptor: descriptor.clone(),
            mean_q: estimate.mean,
            ensemble_variance: estimate.variance,
        })
    }

    pub fn rank(
        &self,
        state_features: &[f32],
        context: &GeneralizedTacticContext,
        descriptors: &[OptionActionDescriptor],
    ) -> Result<Vec<ContinuousTacticValueEstimate>, GeneralizedTacticValueError> {
        let mut estimates = descriptors
            .iter()
            .map(|descriptor| self.predict(state_features, context, descriptor))
            .collect::<Result<Vec<_>, _>>()?;
        estimates.sort_by(|left, right| {
            right
                .mean_q
                .total_cmp(&left.mean_q)
                .then_with(|| left.ensemble_variance.total_cmp(&right.ensemble_variance))
                .then_with(|| left.descriptor.option_id.cmp(&right.descriptor.option_id))
        });
        Ok(estimates)
    }
}

fn continuous_forest_config() -> FqiConfig {
    FqiConfig {
        iterations: 1,
        trees_per_action: 15,
        max_tree_depth: 6,
        min_samples_leaf: 1,
        bootstrap: true,
        seed: CONTINUOUS_FOREST_SEED,
        ..FqiConfig::default()
    }
}
