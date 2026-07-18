//! Held-out calibration for discrete fitted-Q proposal models.

use crate::double_q::{DoubleQ, DoubleQError};
use crate::fqi::{FittedQ, FqiError};
use crate::low_data_baselines::ReturnSample;
use serde::Serialize;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

pub const MAX_CALIBRATION_SAMPLES: usize = 250_000;
const CALIBRATION_BINS: usize = 10;

#[derive(Clone, Debug, Serialize)]
pub struct CalibrationBin {
    pub samples: usize,
    pub minimum_prediction: f64,
    pub maximum_prediction: f64,
    pub mean_prediction: f64,
    pub mean_observed_return: f64,
    pub mean_absolute_error: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct DiscreteQCalibrationReport {
    pub schema: &'static str,
    pub held_out_samples: usize,
    pub supported_observed_action_samples: usize,
    pub unsupported_observed_action_samples: usize,
    pub mean_error: f64,
    pub mean_absolute_error: f64,
    pub root_mean_squared_error: f64,
    pub bins: Vec<CalibrationBin>,
    pub exact_state_groups: usize,
    pub proposal_comparable_states: usize,
    pub proposal_wins: usize,
    pub proposal_win_rate: Option<f64>,
    pub unsupported_proposals: usize,
    pub mean_observed_regret: Option<f64>,
}

pub type FqiCalibrationReport = DiscreteQCalibrationReport;

pub trait DiscreteQEstimator {
    fn calibration_estimate(&self, state: &[f32], action: u32) -> Result<Option<f64>, String>;
    fn calibration_best_action(&self, state: &[f32]) -> Result<u32, String>;
}

impl DiscreteQEstimator for FittedQ {
    fn calibration_estimate(&self, state: &[f32], action: u32) -> Result<Option<f64>, String> {
        match self.estimate(state, action) {
            Ok(estimate) => Ok(Some(estimate.mean)),
            Err(FqiError::UnknownAction(_)) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn calibration_best_action(&self, state: &[f32]) -> Result<u32, String> {
        self.best_action(state)
            .map(|estimate| estimate.action)
            .map_err(|error| error.to_string())
    }
}

impl DiscreteQEstimator for DoubleQ {
    fn calibration_estimate(&self, state: &[f32], action: u32) -> Result<Option<f64>, String> {
        match self.estimate(state, action) {
            Ok(estimate) => Ok(Some(estimate.mean)),
            Err(DoubleQError::UnknownAction(_)) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn calibration_best_action(&self, state: &[f32]) -> Result<u32, String> {
        self.rank_actions(state)
            .map_err(|error| error.to_string())?
            .first()
            .map(|estimate| estimate.action)
            .ok_or_else(|| "discrete Q model returned an empty ranking".into())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CalibrationError(String);

impl CalibrationError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for CalibrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for CalibrationError {}

pub fn calibrate_fitted_q(
    model: &FittedQ,
    samples: &[ReturnSample],
) -> Result<FqiCalibrationReport, CalibrationError> {
    calibrate_discrete_q(model, samples, "dusklight-fitted-q-calibration/v1")
}

pub fn calibrate_discrete_q<M: DiscreteQEstimator>(
    model: &M,
    samples: &[ReturnSample],
    schema: &'static str,
) -> Result<DiscreteQCalibrationReport, CalibrationError> {
    if samples.is_empty() || samples.len() > MAX_CALIBRATION_SAMPLES {
        return Err(CalibrationError::new(
            "invalid held-out calibration sample count",
        ));
    }
    let mut predictions = Vec::with_capacity(samples.len());
    let mut unsupported_observed_action_samples = 0_usize;
    let mut state_actions = BTreeMap::<Vec<u32>, BTreeMap<u32, (f64, usize)>>::new();
    for sample in samples {
        let estimate = match model
            .calibration_estimate(&sample.state, sample.action)
            .map_err(CalibrationError::new)?
        {
            Some(estimate) => Some(estimate),
            None => {
                unsupported_observed_action_samples += 1;
                None
            }
        };
        if let Some(estimate) = estimate {
            if !sample.return_to_go.is_finite() || !estimate.is_finite() {
                return Err(CalibrationError::new("non-finite calibration value"));
            }
            predictions.push((estimate, sample.return_to_go));
        }
        let key = sample
            .state
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>();
        let entry = state_actions
            .entry(key)
            .or_default()
            .entry(sample.action)
            .or_default();
        entry.0 += sample.return_to_go;
        entry.1 += 1;
    }
    if predictions.is_empty() {
        return Err(CalibrationError::new(
            "held-out data has no actions supported by the fitted model",
        ));
    }
    let count = predictions.len() as f64;
    let mean_error = predictions
        .iter()
        .map(|(predicted, observed)| predicted - observed)
        .sum::<f64>()
        / count;
    let mean_absolute_error = predictions
        .iter()
        .map(|(predicted, observed)| (predicted - observed).abs())
        .sum::<f64>()
        / count;
    let root_mean_squared_error = (predictions
        .iter()
        .map(|(predicted, observed)| (predicted - observed).powi(2))
        .sum::<f64>()
        / count)
        .sqrt();

    predictions.sort_by(|left, right| {
        left.0
            .total_cmp(&right.0)
            .then_with(|| left.1.total_cmp(&right.1))
    });
    let bin_width = predictions.len().div_ceil(CALIBRATION_BINS).max(1);
    let bins = predictions
        .chunks(bin_width)
        .map(|chunk| CalibrationBin {
            samples: chunk.len(),
            minimum_prediction: chunk.first().unwrap().0,
            maximum_prediction: chunk.last().unwrap().0,
            mean_prediction: chunk.iter().map(|value| value.0).sum::<f64>() / chunk.len() as f64,
            mean_observed_return: chunk.iter().map(|value| value.1).sum::<f64>()
                / chunk.len() as f64,
            mean_absolute_error: chunk
                .iter()
                .map(|value| (value.0 - value.1).abs())
                .sum::<f64>()
                / chunk.len() as f64,
        })
        .collect();

    let mut proposal_comparable_states = 0_usize;
    let mut proposal_wins = 0_usize;
    let mut unsupported_proposals = 0_usize;
    let mut observed_regret = 0.0_f64;
    for (state_bits, action_returns) in &state_actions {
        let state = state_bits
            .iter()
            .map(|bits| f32::from_bits(*bits))
            .collect::<Vec<_>>();
        let proposed = model
            .calibration_best_action(&state)
            .map_err(CalibrationError::new)?;
        let means = action_returns
            .iter()
            .map(|(action, (sum, support))| (*action, *sum / *support as f64))
            .collect::<BTreeMap<_, _>>();
        let best = means.values().copied().reduce(f64::max).unwrap();
        if let Some(proposed_return) = means.get(&proposed) {
            proposal_comparable_states += 1;
            proposal_wins += usize::from(*proposed_return == best);
            observed_regret += best - proposed_return;
        } else {
            unsupported_proposals += 1;
        }
    }
    Ok(DiscreteQCalibrationReport {
        schema,
        held_out_samples: samples.len(),
        supported_observed_action_samples: predictions.len(),
        unsupported_observed_action_samples,
        mean_error,
        mean_absolute_error,
        root_mean_squared_error,
        bins,
        exact_state_groups: state_actions.len(),
        proposal_comparable_states,
        proposal_wins,
        proposal_win_rate: (proposal_comparable_states != 0)
            .then_some(proposal_wins as f64 / proposal_comparable_states as f64),
        unsupported_proposals,
        mean_observed_regret: (proposal_comparable_states != 0)
            .then_some(observed_regret / proposal_comparable_states as f64),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::double_q::DoubleQConfig;
    use crate::fqi::{FqiConfig, Transition};

    #[test]
    fn reports_held_out_error_bins_and_exact_state_proposal_wins() {
        let training = vec![
            Transition {
                state: vec![0.0],
                action: 1,
                duration: 1,
                reward: 5.0,
                next_state: vec![1.0],
                terminal: true,
            },
            Transition {
                state: vec![0.0],
                action: 2,
                duration: 1,
                reward: 1.0,
                next_state: vec![1.0],
                terminal: true,
            },
        ];
        let model = FittedQ::fit(
            1,
            &[1, 2],
            &training,
            &FqiConfig {
                iterations: 1,
                trees_per_action: 1,
                bootstrap: false,
                ..FqiConfig::default()
            },
        )
        .unwrap();
        let samples = vec![
            ReturnSample {
                state: vec![0.0],
                action: 1,
                return_to_go: 4.0,
                episode_group: 10,
            },
            ReturnSample {
                state: vec![0.0],
                action: 2,
                return_to_go: 0.0,
                episode_group: 11,
            },
        ];
        let report = calibrate_fitted_q(&model, &samples).unwrap();
        assert_eq!(report.held_out_samples, 2);
        assert_eq!(report.proposal_comparable_states, 1);
        assert_eq!(report.proposal_wins, 1);
        assert_eq!(report.proposal_win_rate, Some(1.0));
        assert_eq!(report.mean_absolute_error, 1.0);
        assert_eq!(report.bins.iter().map(|bin| bin.samples).sum::<usize>(), 2);

        let double_q = DoubleQ::fit(
            1,
            &[1, 2],
            &training,
            &DoubleQConfig {
                epochs: 64,
                hidden_width: 4,
                learning_rate: 0.01,
                target_sync_steps: 8,
                seed: 7,
                ..DoubleQConfig::default()
            },
        )
        .unwrap();
        let neural =
            calibrate_discrete_q(&double_q, &samples, "dusklight-double-q-calibration/v1").unwrap();
        assert_eq!(neural.schema, "dusklight-double-q-calibration/v1");
        assert_eq!(neural.held_out_samples, report.held_out_samples);
        assert_eq!(
            neural.supported_observed_action_samples,
            report.supported_observed_action_samples
        );
        assert_eq!(neural.proposal_comparable_states, 1);
        assert_eq!(neural.proposal_wins, 1);
    }
}
