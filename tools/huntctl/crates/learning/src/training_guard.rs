//! Bounded update-to-data accounting and critic health checks.

use serde::Serialize;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

pub const ONLINE_TRAINING_HEALTH_SCHEMA_V1: &str = "dusklight-online-training-health/v1";
pub const ONLINE_COVERAGE_GATE_SCHEMA_V1: &str = "dusklight-online-coverage-gate/v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct CoverageGuardConfig {
    pub minimum_supported_actions: usize,
    pub minimum_decisions_per_supported_action: u64,
    pub minimum_state_bins: usize,
    pub minimum_effective_decisions: usize,
}

impl Default for CoverageGuardConfig {
    fn default() -> Self {
        Self {
            minimum_supported_actions: 2,
            minimum_decisions_per_supported_action: 2,
            minimum_state_bins: 4,
            minimum_effective_decisions: 4,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageDisposition {
    LearningReady,
    FallbackInsufficientActionSupport,
    FallbackInsufficientStateCoverage,
    FallbackInsufficientActionAndStateCoverage,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OnlineCoverageGate {
    pub schema: &'static str,
    pub effective_decisions: usize,
    pub observed_actions: usize,
    pub supported_actions: usize,
    pub state_bins: usize,
    pub limits: CoverageGuardConfig,
    pub learned_policy_enabled: bool,
    pub fallback_policy: Option<&'static str>,
    pub disposition: CoverageDisposition,
}

impl OnlineCoverageGate {
    pub fn evaluate(
        effective_decisions: usize,
        action_support: &BTreeMap<u32, u64>,
        state_bins: usize,
        limits: CoverageGuardConfig,
    ) -> Result<Self, TrainingGuardError> {
        if limits.minimum_supported_actions < 2
            || limits.minimum_decisions_per_supported_action == 0
            || limits.minimum_state_bins == 0
            || limits.minimum_effective_decisions == 0
            || action_support.values().sum::<u64>() != effective_decisions as u64
        {
            return Err(TrainingGuardError::InvalidConfiguration);
        }
        let supported_actions = action_support
            .values()
            .filter(|count| **count >= limits.minimum_decisions_per_supported_action)
            .count();
        let action_ready = supported_actions >= limits.minimum_supported_actions;
        let state_ready = state_bins >= limits.minimum_state_bins
            && effective_decisions >= limits.minimum_effective_decisions;
        let disposition = match (action_ready, state_ready) {
            (true, true) => CoverageDisposition::LearningReady,
            (false, true) => CoverageDisposition::FallbackInsufficientActionSupport,
            (true, false) => CoverageDisposition::FallbackInsufficientStateCoverage,
            (false, false) => CoverageDisposition::FallbackInsufficientActionAndStateCoverage,
        };
        let learned_policy_enabled = disposition == CoverageDisposition::LearningReady;
        Ok(Self {
            schema: ONLINE_COVERAGE_GATE_SCHEMA_V1,
            effective_decisions,
            observed_actions: action_support.len(),
            supported_actions,
            state_bins,
            limits,
            learned_policy_enabled,
            fallback_policy: (!learned_policy_enabled).then_some("structured_archive_blind_only"),
            disposition,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct TrainingGuardConfig {
    pub maximum_update_to_data_ratio: f64,
    pub maximum_absolute_value: f64,
    pub maximum_critic_disagreement: f64,
}

impl Default for TrainingGuardConfig {
    fn default() -> Self {
        Self {
            maximum_update_to_data_ratio: 32.0,
            maximum_absolute_value: 1.0e6,
            maximum_critic_disagreement: 1.0e6,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CriticSnapshot {
    pub primary: f64,
    pub secondary: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainingHealthDisposition {
    Healthy,
    UpdateToDataExceeded,
    NonFiniteCritic,
    ValueExplosion,
    CriticDivergence,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OnlineTrainingHealth {
    pub schema: &'static str,
    pub data_rows: usize,
    pub optimizer_updates: u64,
    pub update_to_data_ratio: f64,
    pub critic_snapshots: usize,
    pub maximum_absolute_value: f64,
    pub maximum_critic_disagreement: f64,
    pub non_finite_values: usize,
    pub limits: TrainingGuardConfig,
    pub disposition: TrainingHealthDisposition,
}

impl OnlineTrainingHealth {
    pub fn evaluate(
        data_rows: usize,
        optimizer_updates: u64,
        snapshots: &[CriticSnapshot],
        limits: TrainingGuardConfig,
    ) -> Result<Self, TrainingGuardError> {
        validate_inputs(data_rows, &limits)?;
        let update_to_data_ratio = optimizer_updates as f64 / data_rows as f64;
        let mut maximum_absolute_value = 0.0_f64;
        let mut maximum_critic_disagreement = 0.0_f64;
        let mut non_finite_values = 0_usize;
        for snapshot in snapshots {
            for value in [snapshot.primary, snapshot.secondary] {
                if value.is_finite() {
                    maximum_absolute_value = maximum_absolute_value.max(value.abs());
                } else {
                    non_finite_values += 1;
                }
            }
            let disagreement = (snapshot.primary - snapshot.secondary).abs();
            if disagreement.is_finite() {
                maximum_critic_disagreement = maximum_critic_disagreement.max(disagreement);
            }
        }
        let disposition = if non_finite_values > 0 {
            TrainingHealthDisposition::NonFiniteCritic
        } else if update_to_data_ratio > limits.maximum_update_to_data_ratio {
            TrainingHealthDisposition::UpdateToDataExceeded
        } else if maximum_absolute_value > limits.maximum_absolute_value {
            TrainingHealthDisposition::ValueExplosion
        } else if maximum_critic_disagreement > limits.maximum_critic_disagreement {
            TrainingHealthDisposition::CriticDivergence
        } else {
            TrainingHealthDisposition::Healthy
        };
        Ok(Self {
            schema: ONLINE_TRAINING_HEALTH_SCHEMA_V1,
            data_rows,
            optimizer_updates,
            update_to_data_ratio,
            critic_snapshots: snapshots.len(),
            maximum_absolute_value,
            maximum_critic_disagreement,
            non_finite_values,
            limits,
            disposition,
        })
    }

    pub fn require_healthy(self) -> Result<Self, TrainingGuardError> {
        if self.disposition == TrainingHealthDisposition::Healthy {
            Ok(self)
        } else {
            Err(TrainingGuardError::Unhealthy(self))
        }
    }
}

fn validate_inputs(
    data_rows: usize,
    limits: &TrainingGuardConfig,
) -> Result<(), TrainingGuardError> {
    if data_rows == 0
        || !limits.maximum_update_to_data_ratio.is_finite()
        || limits.maximum_update_to_data_ratio <= 0.0
        || !limits.maximum_absolute_value.is_finite()
        || limits.maximum_absolute_value <= 0.0
        || !limits.maximum_critic_disagreement.is_finite()
        || limits.maximum_critic_disagreement <= 0.0
    {
        return Err(TrainingGuardError::InvalidConfiguration);
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub enum TrainingGuardError {
    InvalidConfiguration,
    Unhealthy(OnlineTrainingHealth),
}

impl fmt::Display for TrainingGuardError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration => {
                formatter.write_str("online training guard configuration is invalid")
            }
            Self::Unhealthy(health) => write!(
                formatter,
                "online training rejected: {:?}",
                health.disposition
            ),
        }
    }
}

impl Error for TrainingGuardError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accounts_updates_and_accepts_bounded_critics() {
        let health = OnlineTrainingHealth::evaluate(
            10,
            20,
            &[CriticSnapshot {
                primary: 2.0,
                secondary: 2.5,
            }],
            TrainingGuardConfig::default(),
        )
        .unwrap()
        .require_healthy()
        .unwrap();
        assert_eq!(health.update_to_data_ratio, 2.0);
        assert_eq!(health.maximum_absolute_value, 2.5);
        assert_eq!(health.maximum_critic_disagreement, 0.5);
    }

    #[test]
    fn distinguishes_update_value_disagreement_and_nonfinite_failures() {
        let limits = TrainingGuardConfig {
            maximum_update_to_data_ratio: 2.0,
            maximum_absolute_value: 10.0,
            maximum_critic_disagreement: 3.0,
        };
        for (updates, snapshot, disposition) in [
            (
                3,
                CriticSnapshot {
                    primary: 1.0,
                    secondary: 1.0,
                },
                TrainingHealthDisposition::UpdateToDataExceeded,
            ),
            (
                1,
                CriticSnapshot {
                    primary: 11.0,
                    secondary: 9.0,
                },
                TrainingHealthDisposition::ValueExplosion,
            ),
            (
                1,
                CriticSnapshot {
                    primary: 4.0,
                    secondary: 0.0,
                },
                TrainingHealthDisposition::CriticDivergence,
            ),
            (
                1,
                CriticSnapshot {
                    primary: f64::NAN,
                    secondary: 0.0,
                },
                TrainingHealthDisposition::NonFiniteCritic,
            ),
        ] {
            let health = OnlineTrainingHealth::evaluate(1, updates, &[snapshot], limits).unwrap();
            assert_eq!(health.disposition, disposition);
            assert!(health.require_healthy().is_err());
        }
    }

    #[test]
    fn coverage_gate_distinguishes_action_and_state_fallbacks() {
        let limits = CoverageGuardConfig::default();
        let ready =
            OnlineCoverageGate::evaluate(4, &BTreeMap::from([(0, 2), (1, 2)]), 4, limits).unwrap();
        assert!(ready.learned_policy_enabled);
        assert_eq!(ready.disposition, CoverageDisposition::LearningReady);

        let action_fallback =
            OnlineCoverageGate::evaluate(4, &BTreeMap::from([(0, 4)]), 4, limits).unwrap();
        assert_eq!(
            action_fallback.disposition,
            CoverageDisposition::FallbackInsufficientActionSupport
        );
        let state_fallback =
            OnlineCoverageGate::evaluate(4, &BTreeMap::from([(0, 2), (1, 2)]), 1, limits).unwrap();
        assert_eq!(
            state_fallback.disposition,
            CoverageDisposition::FallbackInsufficientStateCoverage
        );
    }
}
