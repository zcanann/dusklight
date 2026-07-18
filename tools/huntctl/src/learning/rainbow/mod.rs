//! Controlled ablations for components commonly bundled as Rainbow DQN.
//!
//! Components live here as experimental evaluators. They are never silently
//! enabled in the production Double-Q learner and cannot be combined by this
//! API: every report compares exactly one component with the same baseline.

mod n_step;

pub use n_step::{NStepError, aggregate_n_step};

use crate::double_q::ablation::{QComponent, QComponentConfig, QComponentModel};
use crate::double_q::{DoubleQ, DoubleQConfig, DoubleQError, DoubleQEstimate};
use crate::fqi::Transition;
use crate::learning::calibration::{
    DiscreteQCalibrationReport, DiscreteQEstimator, calibrate_discrete_q,
};
use crate::low_data_baselines::{ReturnSample, empirical_return_samples};
use serde::Serialize;
use std::error::Error;
use std::fmt;

pub const RAINBOW_ABLATION_SCHEMA_V1: &str = "dusklight-rainbow-ablation/v1";
pub const MAX_RAINBOW_N_STEP: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RainbowComponent {
    DuelingHeads,
    NStepReturns,
    DistributionalValues,
    NoisyExploration,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RainbowAblationConfig {
    pub critic: DoubleQConfig,
    pub n_step: usize,
    pub distribution_atoms: usize,
    pub distribution_value_minimum: f64,
    pub distribution_value_maximum: f64,
    pub noisy_initial_stddev: f64,
}

impl Default for RainbowAblationConfig {
    fn default() -> Self {
        Self {
            critic: DoubleQConfig::default(),
            n_step: 3,
            distribution_atoms: 51,
            distribution_value_minimum: -100.0,
            distribution_value_maximum: 100.0,
            noisy_initial_stddev: 0.5,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HeldOutMetrics {
    pub transitions: usize,
    pub supported_logged_actions: usize,
    pub unsupported_logged_actions: usize,
    pub mean_absolute_td_error: f64,
    pub root_mean_squared_td_error: f64,
    pub logged_action_greedy_rate: f64,
    pub observed_return_calibration: DiscreteQCalibrationReport,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ComponentEvaluation {
    pub component: RainbowComponent,
    pub changed_parameters: Vec<&'static str>,
    pub baseline: HeldOutMetrics,
    pub treatment: HeldOutMetrics,
    pub mean_absolute_td_error_delta: f64,
    pub baseline_gradient_updates: u64,
    pub treatment_gradient_updates: u64,
    pub equal_gradient_update_budget: bool,
    pub treatment_noise_resamples: Option<u64>,
    pub adopted: bool,
    pub decision: &'static str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RainbowAblationReport {
    pub schema: &'static str,
    pub training_transitions: usize,
    pub held_out_transitions: usize,
    pub evaluation: ComponentEvaluation,
    pub combined_rainbow_configuration: bool,
    pub promotion_authority: bool,
    pub limitations: Vec<&'static str>,
}

impl RainbowAblationReport {
    /// Evaluate a dueling value/advantage head as the sole architecture change.
    pub fn evaluate_dueling_heads(
        feature_width: usize,
        actions: &[u32],
        training: &[Transition],
        training_episode_groups: &[u64],
        held_out: &[Transition],
        held_out_episode_groups: &[u64],
        config: &RainbowAblationConfig,
    ) -> Result<Self, RainbowAblationError> {
        validate_common(training, held_out, config)?;
        if training_episode_groups.len() != training.len() {
            return Err(RainbowAblationError::Invalid(
                "training episode groups must match training transitions",
            ));
        }
        let held_out_samples =
            held_out_return_samples(held_out, held_out_episode_groups, config.critic.discount)?;
        let baseline = DoubleQ::fit(feature_width, actions, training, &config.critic)?;
        let treatment = QComponentModel::fit(
            feature_width,
            actions,
            training,
            training_episode_groups,
            &QComponentConfig {
                critic: config.critic.clone(),
                component: QComponent::DuelingHead,
                ..QComponentConfig::default()
            },
        )?;
        let baseline_metrics = evaluate_model(
            &baseline,
            held_out,
            &held_out_samples,
            config.critic.discount,
        )?;
        let treatment_metrics = evaluate_model(
            &treatment,
            held_out,
            &held_out_samples,
            config.critic.discount,
        )?;
        Ok(Self::single_component(
            training.len(),
            held_out.len(),
            RainbowComponent::DuelingHeads,
            vec!["critic_output_head"],
            baseline_metrics,
            treatment_metrics,
            baseline.gradient_updates(),
            treatment.gradient_updates(),
            None,
        ))
    }

    /// Evaluate categorical value distributions as the sole architecture change.
    pub fn evaluate_distributional_values(
        feature_width: usize,
        actions: &[u32],
        training: &[Transition],
        training_episode_groups: &[u64],
        held_out: &[Transition],
        held_out_episode_groups: &[u64],
        config: &RainbowAblationConfig,
    ) -> Result<Self, RainbowAblationError> {
        validate_common(training, held_out, config)?;
        if training_episode_groups.len() != training.len() {
            return Err(RainbowAblationError::Invalid(
                "training episode groups must match training transitions",
            ));
        }
        let held_out_samples =
            held_out_return_samples(held_out, held_out_episode_groups, config.critic.discount)?;
        let baseline = DoubleQ::fit(feature_width, actions, training, &config.critic)?;
        let treatment = QComponentModel::fit(
            feature_width,
            actions,
            training,
            training_episode_groups,
            &QComponentConfig {
                critic: config.critic.clone(),
                component: QComponent::DistributionalValues,
                distribution_atoms: config.distribution_atoms,
                distribution_value_minimum: config.distribution_value_minimum,
                distribution_value_maximum: config.distribution_value_maximum,
                noisy_initial_stddev: QComponentConfig::default().noisy_initial_stddev,
            },
        )?;
        let baseline_metrics = evaluate_model(
            &baseline,
            held_out,
            &held_out_samples,
            config.critic.discount,
        )?;
        let treatment_metrics = evaluate_model(
            &treatment,
            held_out,
            &held_out_samples,
            config.critic.discount,
        )?;
        Ok(Self::single_component(
            training.len(),
            held_out.len(),
            RainbowComponent::DistributionalValues,
            vec!["critic_output_distribution", "categorical_value_support"],
            baseline_metrics,
            treatment_metrics,
            baseline.gradient_updates(),
            treatment.gradient_updates(),
            None,
        ))
    }

    /// Evaluate learned parameter noise as the sole training-time change.
    pub fn evaluate_noisy_exploration(
        feature_width: usize,
        actions: &[u32],
        training: &[Transition],
        training_episode_groups: &[u64],
        held_out: &[Transition],
        held_out_episode_groups: &[u64],
        config: &RainbowAblationConfig,
    ) -> Result<Self, RainbowAblationError> {
        validate_common(training, held_out, config)?;
        if training_episode_groups.len() != training.len() {
            return Err(RainbowAblationError::Invalid(
                "training episode groups must match training transitions",
            ));
        }
        let held_out_samples =
            held_out_return_samples(held_out, held_out_episode_groups, config.critic.discount)?;
        let baseline = DoubleQ::fit(feature_width, actions, training, &config.critic)?;
        let treatment = QComponentModel::fit(
            feature_width,
            actions,
            training,
            training_episode_groups,
            &QComponentConfig {
                critic: config.critic.clone(),
                component: QComponent::NoisyExploration,
                noisy_initial_stddev: config.noisy_initial_stddev,
                ..QComponentConfig::default()
            },
        )?;
        let baseline_metrics = evaluate_model(
            &baseline,
            held_out,
            &held_out_samples,
            config.critic.discount,
        )?;
        let treatment_metrics = evaluate_model(
            &treatment,
            held_out,
            &held_out_samples,
            config.critic.discount,
        )?;
        Ok(Self::single_component(
            training.len(),
            held_out.len(),
            RainbowComponent::NoisyExploration,
            vec!["critic_output_parameter_noise"],
            baseline_metrics,
            treatment_metrics,
            baseline.gradient_updates(),
            treatment.gradient_updates(),
            Some(treatment.noise_resamples()),
        ))
    }

    /// Evaluate n-step returns as the sole change from deterministic Double-Q.
    pub fn evaluate_n_step(
        feature_width: usize,
        actions: &[u32],
        training: &[Transition],
        training_episode_groups: &[u64],
        held_out: &[Transition],
        held_out_episode_groups: &[u64],
        config: &RainbowAblationConfig,
    ) -> Result<Self, RainbowAblationError> {
        validate_common(training, held_out, config)?;
        if training_episode_groups.len() != training.len() {
            return Err(RainbowAblationError::Invalid(
                "training episode groups must match training transitions",
            ));
        }
        if config.n_step < 2 || config.n_step > MAX_RAINBOW_N_STEP {
            return Err(RainbowAblationError::Invalid(
                "n-step ablation horizon must be within 2..=64",
            ));
        }
        let held_out_samples =
            held_out_return_samples(held_out, held_out_episode_groups, config.critic.discount)?;
        let baseline = DoubleQ::fit(feature_width, actions, training, &config.critic)?;
        let aggregated = aggregate_n_step(
            training,
            training_episode_groups,
            config.n_step,
            config.critic.discount,
        )?;
        let treatment = DoubleQ::fit(feature_width, actions, &aggregated, &config.critic)?;
        let baseline_metrics = evaluate_model(
            &baseline,
            held_out,
            &held_out_samples,
            config.critic.discount,
        )?;
        let treatment_metrics = evaluate_model(
            &treatment,
            held_out,
            &held_out_samples,
            config.critic.discount,
        )?;
        Ok(Self::single_component(
            training.len(),
            held_out.len(),
            RainbowComponent::NStepReturns,
            vec!["bellman_backup_horizon"],
            baseline_metrics,
            treatment_metrics,
            baseline.gradient_updates(),
            treatment.gradient_updates(),
            None,
        ))
    }

    fn single_component(
        training_transitions: usize,
        held_out_transitions: usize,
        component: RainbowComponent,
        changed_parameters: Vec<&'static str>,
        baseline: HeldOutMetrics,
        treatment: HeldOutMetrics,
        baseline_gradient_updates: u64,
        treatment_gradient_updates: u64,
        treatment_noise_resamples: Option<u64>,
    ) -> Self {
        let delta = treatment.mean_absolute_td_error - baseline.mean_absolute_td_error;
        Self {
            schema: RAINBOW_ABLATION_SCHEMA_V1,
            training_transitions,
            held_out_transitions,
            evaluation: ComponentEvaluation {
                component,
                changed_parameters,
                baseline,
                treatment,
                mean_absolute_td_error_delta: delta,
                baseline_gradient_updates,
                treatment_gradient_updates,
                equal_gradient_update_budget: baseline_gradient_updates
                    == treatment_gradient_updates,
                treatment_noise_resamples,
                adopted: false,
                decision: "experimental_only_pending_real_corpus_equal_budget_evidence",
            },
            combined_rainbow_configuration: false,
            promotion_authority: false,
            limitations: vec![
                "held-out Bellman error is not native objective success",
                "logged-action agreement is descriptive and not policy proof",
                "each component requires equal-budget native proposal evaluation before adoption",
            ],
        }
    }
}

fn validate_common(
    training: &[Transition],
    held_out: &[Transition],
    config: &RainbowAblationConfig,
) -> Result<(), RainbowAblationError> {
    if training.is_empty() || held_out.is_empty() {
        return Err(RainbowAblationError::Invalid(
            "ablation requires non-empty training and held-out transitions",
        ));
    }
    if !config.critic.discount.is_finite() || !(0.0..=1.0).contains(&config.critic.discount) {
        return Err(RainbowAblationError::Invalid(
            "ablation discount must be finite and within [0, 1]",
        ));
    }
    Ok(())
}

trait HeldOutQModel {
    fn held_out_estimate(
        &self,
        state: &[f32],
        action: u32,
    ) -> Result<DoubleQEstimate, DoubleQError>;
    fn held_out_rank(&self, state: &[f32]) -> Result<Vec<DoubleQEstimate>, DoubleQError>;
}

impl HeldOutQModel for DoubleQ {
    fn held_out_estimate(
        &self,
        state: &[f32],
        action: u32,
    ) -> Result<DoubleQEstimate, DoubleQError> {
        self.estimate(state, action)
    }

    fn held_out_rank(&self, state: &[f32]) -> Result<Vec<DoubleQEstimate>, DoubleQError> {
        self.rank_actions(state)
    }
}

impl HeldOutQModel for QComponentModel {
    fn held_out_estimate(
        &self,
        state: &[f32],
        action: u32,
    ) -> Result<DoubleQEstimate, DoubleQError> {
        self.estimate(state, action)
    }

    fn held_out_rank(&self, state: &[f32]) -> Result<Vec<DoubleQEstimate>, DoubleQError> {
        self.rank_actions(state)
    }
}

fn held_out_return_samples(
    held_out: &[Transition],
    episode_groups: &[u64],
    discount: f64,
) -> Result<Vec<ReturnSample>, RainbowAblationError> {
    empirical_return_samples(held_out, episode_groups, discount as f32)
        .map_err(|error| RainbowAblationError::Returns(error.to_string()))
}

fn evaluate_model<M: HeldOutQModel + DiscreteQEstimator>(
    model: &M,
    held_out: &[Transition],
    held_out_samples: &[ReturnSample],
    discount: f64,
) -> Result<HeldOutMetrics, RainbowAblationError> {
    let mut absolute_error = 0.0;
    let mut squared_error = 0.0;
    let mut greedy_matches = 0_usize;
    let mut supported_logged_actions = 0_usize;
    let mut unsupported_logged_actions = 0_usize;
    for transition in held_out {
        let prediction = match model.held_out_estimate(&transition.state, transition.action) {
            Ok(estimate) => estimate.mean,
            Err(DoubleQError::UnknownAction(_)) => {
                unsupported_logged_actions += 1;
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        supported_logged_actions += 1;
        let target = if transition.terminal {
            f64::from(transition.reward)
        } else {
            f64::from(transition.reward)
                + discount.powf(f64::from(transition.duration))
                    * model.held_out_rank(&transition.next_state)?[0].mean
        };
        let error = prediction - target;
        absolute_error += error.abs();
        squared_error += error * error;
        if model.held_out_rank(&transition.state)?[0].action == transition.action {
            greedy_matches += 1;
        }
    }
    if supported_logged_actions == 0 {
        return Err(RainbowAblationError::Invalid(
            "held-out data has no actions supported by training",
        ));
    }
    let count = supported_logged_actions as f64;
    let observed_return_calibration = calibrate_discrete_q(
        model,
        held_out_samples,
        "dusklight-q-component-held-out-calibration/v1",
    )
    .map_err(|error| RainbowAblationError::Calibration(error.to_string()))?;
    Ok(HeldOutMetrics {
        transitions: held_out.len(),
        supported_logged_actions,
        unsupported_logged_actions,
        mean_absolute_td_error: absolute_error / count,
        root_mean_squared_td_error: (squared_error / count).sqrt(),
        logged_action_greedy_rate: greedy_matches as f64 / count,
        observed_return_calibration,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RainbowAblationError {
    Invalid(&'static str),
    NStep(String),
    Learner(String),
    Returns(String),
    Calibration(String),
}

impl fmt::Display for RainbowAblationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => write!(formatter, "invalid Rainbow ablation: {message}"),
            Self::NStep(message) => write!(formatter, "n-step aggregation failed: {message}"),
            Self::Learner(message) => {
                write!(formatter, "Rainbow ablation learner failed: {message}")
            }
            Self::Returns(message) => {
                write!(
                    formatter,
                    "Rainbow ablation held-out returns failed: {message}"
                )
            }
            Self::Calibration(message) => {
                write!(formatter, "Rainbow ablation calibration failed: {message}")
            }
        }
    }
}

impl Error for RainbowAblationError {}

impl From<NStepError> for RainbowAblationError {
    fn from(error: NStepError) -> Self {
        Self::NStep(error.to_string())
    }
}

impl From<DoubleQError> for RainbowAblationError {
    fn from(error: DoubleQError) -> Self {
        Self::Learner(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WAIT: u32 = 0;
    const ADVANCE: u32 = 1;

    fn transition(state: f32, action: u32, reward: f32, next: f32, terminal: bool) -> Transition {
        Transition {
            state: vec![state],
            action,
            duration: 1,
            reward,
            next_state: vec![next],
            terminal,
        }
    }

    #[test]
    fn n_step_is_reported_as_the_only_changed_component() {
        let training = vec![
            transition(0.0, ADVANCE, 0.0, 1.0, false),
            transition(1.0, ADVANCE, 5.0, 2.0, true),
            transition(0.0, WAIT, -1.0, 0.0, false),
            transition(0.0, ADVANCE, 0.0, 1.0, false),
        ];
        let config = RainbowAblationConfig {
            critic: DoubleQConfig {
                epochs: 12,
                hidden_width: 8,
                learning_rate: 0.01,
                target_sync_steps: 4,
                seed: 7,
                ..DoubleQConfig::default()
            },
            n_step: 2,
            ..RainbowAblationConfig::default()
        };
        let first = RainbowAblationReport::evaluate_n_step(
            1,
            &[WAIT, ADVANCE],
            &training,
            &[10, 10, 20, 20],
            &training,
            &[10, 10, 20, 20],
            &config,
        )
        .unwrap();
        let second = RainbowAblationReport::evaluate_n_step(
            1,
            &[WAIT, ADVANCE],
            &training,
            &[10, 10, 20, 20],
            &training,
            &[10, 10, 20, 20],
            &config,
        )
        .unwrap();
        assert_eq!(first, second);
        assert_eq!(first.evaluation.component, RainbowComponent::NStepReturns);
        assert_eq!(
            first.evaluation.changed_parameters,
            ["bellman_backup_horizon"]
        );
        assert!(!first.evaluation.adopted);
        assert!(!first.combined_rainbow_configuration);
        assert_eq!(first.evaluation.baseline.transitions, training.len());
        assert!(first.evaluation.equal_gradient_update_budget);
    }

    #[test]
    fn dueling_head_is_reported_as_the_only_changed_component() {
        let training = vec![
            transition(0.0, ADVANCE, 0.0, 1.0, false),
            transition(1.0, ADVANCE, 5.0, 2.0, true),
            transition(0.0, WAIT, -1.0, 0.0, false),
            transition(1.0, WAIT, -1.0, 1.0, true),
        ];
        let config = RainbowAblationConfig {
            critic: DoubleQConfig {
                epochs: 12,
                hidden_width: 8,
                learning_rate: 0.01,
                target_sync_steps: 4,
                seed: 7,
                ..DoubleQConfig::default()
            },
            n_step: 3,
            ..RainbowAblationConfig::default()
        };
        let report = RainbowAblationReport::evaluate_dueling_heads(
            1,
            &[WAIT, ADVANCE],
            &training,
            &[10, 10, 20, 20],
            &training,
            &[10, 10, 20, 20],
            &config,
        )
        .unwrap();
        assert_eq!(report.evaluation.component, RainbowComponent::DuelingHeads);
        assert_eq!(report.evaluation.changed_parameters, ["critic_output_head"]);
        assert_eq!(report.evaluation.baseline.transitions, training.len());
        assert_eq!(report.evaluation.treatment.transitions, training.len());
        assert!(!report.evaluation.adopted);
        assert!(!report.combined_rainbow_configuration);
        assert!(report.evaluation.equal_gradient_update_budget);
    }

    #[test]
    fn distributional_values_are_reported_as_the_only_changed_component() {
        let training = vec![
            transition(0.0, ADVANCE, 0.0, 1.0, false),
            transition(1.0, ADVANCE, 5.0, 2.0, true),
            transition(0.0, WAIT, -1.0, 0.0, false),
            transition(1.0, WAIT, -1.0, 1.0, true),
        ];
        let config = RainbowAblationConfig {
            critic: DoubleQConfig {
                epochs: 12,
                hidden_width: 8,
                learning_rate: 0.01,
                target_sync_steps: 4,
                seed: 7,
                ..DoubleQConfig::default()
            },
            distribution_atoms: 21,
            distribution_value_minimum: -10.0,
            distribution_value_maximum: 10.0,
            ..RainbowAblationConfig::default()
        };
        let report = RainbowAblationReport::evaluate_distributional_values(
            1,
            &[WAIT, ADVANCE],
            &training,
            &[10, 10, 20, 20],
            &training,
            &[10, 10, 20, 20],
            &config,
        )
        .unwrap();
        assert_eq!(
            report.evaluation.component,
            RainbowComponent::DistributionalValues
        );
        assert_eq!(
            report.evaluation.changed_parameters,
            ["critic_output_distribution", "categorical_value_support"]
        );
        assert!(!report.evaluation.adopted);
        assert!(!report.combined_rainbow_configuration);
        assert!(report.evaluation.equal_gradient_update_budget);
    }

    #[test]
    fn noisy_exploration_is_reported_as_the_only_changed_component() {
        let training = vec![
            transition(0.0, ADVANCE, 0.0, 1.0, false),
            transition(1.0, ADVANCE, 5.0, 2.0, true),
            transition(0.0, WAIT, -1.0, 0.0, false),
            transition(1.0, WAIT, -1.0, 1.0, true),
        ];
        let config = RainbowAblationConfig {
            critic: DoubleQConfig {
                epochs: 12,
                hidden_width: 8,
                learning_rate: 0.01,
                target_sync_steps: 4,
                seed: 7,
                ..DoubleQConfig::default()
            },
            noisy_initial_stddev: 0.25,
            ..RainbowAblationConfig::default()
        };
        let report = RainbowAblationReport::evaluate_noisy_exploration(
            1,
            &[WAIT, ADVANCE],
            &training,
            &[10, 10, 20, 20],
            &training,
            &[10, 10, 20, 20],
            &config,
        )
        .unwrap();
        assert_eq!(
            report.evaluation.component,
            RainbowComponent::NoisyExploration
        );
        assert_eq!(
            report.evaluation.changed_parameters,
            ["critic_output_parameter_noise"]
        );
        assert!(report.evaluation.equal_gradient_update_budget);
        assert_eq!(
            report.evaluation.treatment_noise_resamples,
            Some(report.evaluation.treatment_gradient_updates * 4)
        );
        assert!(!report.evaluation.adopted);
        assert!(!report.combined_rainbow_configuration);
    }
}
