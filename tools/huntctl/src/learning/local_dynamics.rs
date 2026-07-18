//! Held-out prediction-error gate for short-horizon local dynamics research.

use crate::artifact::Digest;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const LOCAL_DYNAMICS_REPORT_SCHEMA_V1: &str = "dusklight-local-dynamics-error-report/v1";
pub const LOCAL_DYNAMICS_PERMIT_SCHEMA_V1: &str = "dusklight-local-dynamics-training-permit/v1";
const MAX_PROBES: usize = 1_000_000;
const MAX_TARGET_WIDTH: usize = 256;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DynamicsProbeDomain {
    Contact,
    ProcedureTransition,
    RngSensitiveBranch,
    ActorInteraction,
}

impl DynamicsProbeDomain {
    const ALL: [Self; 4] = [
        Self::Contact,
        Self::ProcedureTransition,
        Self::RngSensitiveBranch,
        Self::ActorInteraction,
    ];
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LocalDynamicsProbe {
    pub domain: DynamicsProbeDomain,
    pub episode_sha256: Digest,
    pub horizon_ticks: u16,
    pub predicted_normalized_delta: Vec<f32>,
    pub observed_normalized_delta: Vec<f32>,
    pub predicted_event: u32,
    pub observed_event: u32,
    pub probe_sha256: Digest,
}

impl LocalDynamicsProbe {
    pub fn seal(
        domain: DynamicsProbeDomain,
        episode_sha256: Digest,
        horizon_ticks: u16,
        predicted_normalized_delta: Vec<f32>,
        observed_normalized_delta: Vec<f32>,
        predicted_event: u32,
        observed_event: u32,
    ) -> Result<Self, LocalDynamicsError> {
        let mut probe = Self {
            domain,
            episode_sha256,
            horizon_ticks,
            predicted_normalized_delta,
            observed_normalized_delta,
            predicted_event,
            observed_event,
            probe_sha256: Digest::ZERO,
        };
        probe.probe_sha256 = probe.digest()?;
        probe.validate(u16::MAX)?;
        Ok(probe)
    }

    fn validate(&self, maximum_horizon_ticks: u16) -> Result<(), LocalDynamicsError> {
        if self.episode_sha256 == Digest::ZERO
            || self.horizon_ticks == 0
            || self.horizon_ticks > maximum_horizon_ticks
            || self.predicted_normalized_delta.is_empty()
            || self.predicted_normalized_delta.len() > MAX_TARGET_WIDTH
            || self.predicted_normalized_delta.len() != self.observed_normalized_delta.len()
            || self
                .predicted_normalized_delta
                .iter()
                .chain(&self.observed_normalized_delta)
                .any(|value| !value.is_finite())
            || self.probe_sha256 != self.digest()?
        {
            return Err(LocalDynamicsError::new(
                "local-dynamics probe is invalid or detached",
            ));
        }
        Ok(())
    }

    fn digest(&self) -> Result<Digest, LocalDynamicsError> {
        canonical_digest(
            b"dusklight.local-dynamics-probe/v1\0",
            &(
                self.domain,
                self.episode_sha256,
                self.horizon_ticks,
                &self.predicted_normalized_delta,
                &self.observed_normalized_delta,
                self.predicted_event,
                self.observed_event,
            ),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct LocalDynamicsGateConfig {
    pub maximum_horizon_ticks: u16,
    pub minimum_probes_per_domain: usize,
    pub minimum_episodes_per_domain: usize,
    pub maximum_normalized_rmse: f64,
    pub maximum_event_error_rate: f64,
}

impl Default for LocalDynamicsGateConfig {
    fn default() -> Self {
        Self {
            maximum_horizon_ticks: 8,
            minimum_probes_per_domain: 128,
            minimum_episodes_per_domain: 32,
            maximum_normalized_rmse: 0.25,
            maximum_event_error_rate: 0.1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LocalDynamicsDomainSummary {
    pub domain: DynamicsProbeDomain,
    pub probes: usize,
    pub episodes: usize,
    pub normalized_rmse: f64,
    pub event_error_rate: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalDynamicsDisposition {
    ReadyForBoundedTraining,
    InsufficientHeldOutCoverage,
    PredictionErrorTooHigh,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LocalDynamicsErrorReport {
    pub schema: &'static str,
    pub model_sha256: Digest,
    pub held_out_corpus_sha256: Digest,
    pub normalization_sha256: Digest,
    pub config: LocalDynamicsGateConfig,
    pub domains: Vec<LocalDynamicsDomainSummary>,
    pub disposition: LocalDynamicsDisposition,
    pub training_authorized: bool,
    pub promotion_authority: bool,
    pub report_sha256: Digest,
}

impl LocalDynamicsErrorReport {
    pub fn evaluate(
        model_sha256: Digest,
        held_out_corpus_sha256: Digest,
        normalization_sha256: Digest,
        probes: &[LocalDynamicsProbe],
        config: LocalDynamicsGateConfig,
    ) -> Result<Self, LocalDynamicsError> {
        validate_config(
            model_sha256,
            held_out_corpus_sha256,
            normalization_sha256,
            probes,
            config,
        )?;
        let mut domains = Vec::with_capacity(DynamicsProbeDomain::ALL.len());
        let mut coverage_ready = true;
        let mut error_ready = true;
        for domain in DynamicsProbeDomain::ALL {
            let selected = probes
                .iter()
                .filter(|probe| probe.domain == domain)
                .collect::<Vec<_>>();
            let episodes = selected
                .iter()
                .map(|probe| probe.episode_sha256)
                .collect::<BTreeSet<_>>()
                .len();
            let values = selected
                .iter()
                .map(|probe| probe.predicted_normalized_delta.len())
                .sum::<usize>();
            let squared_error = selected
                .iter()
                .flat_map(|probe| {
                    probe
                        .predicted_normalized_delta
                        .iter()
                        .zip(&probe.observed_normalized_delta)
                })
                .map(|(predicted, observed)| {
                    let error = f64::from(*predicted) - f64::from(*observed);
                    error * error
                })
                .sum::<f64>();
            let normalized_rmse = if values == 0 {
                f64::INFINITY
            } else {
                (squared_error / values as f64).sqrt()
            };
            let event_errors = selected
                .iter()
                .filter(|probe| probe.predicted_event != probe.observed_event)
                .count();
            let event_error_rate = if selected.is_empty() {
                1.0
            } else {
                event_errors as f64 / selected.len() as f64
            };
            coverage_ready &= selected.len() >= config.minimum_probes_per_domain
                && episodes >= config.minimum_episodes_per_domain;
            error_ready &= normalized_rmse <= config.maximum_normalized_rmse
                && event_error_rate <= config.maximum_event_error_rate;
            domains.push(LocalDynamicsDomainSummary {
                domain,
                probes: selected.len(),
                episodes,
                normalized_rmse,
                event_error_rate,
            });
        }
        let disposition = if !coverage_ready {
            LocalDynamicsDisposition::InsufficientHeldOutCoverage
        } else if !error_ready {
            LocalDynamicsDisposition::PredictionErrorTooHigh
        } else {
            LocalDynamicsDisposition::ReadyForBoundedTraining
        };
        let mut report = Self {
            schema: LOCAL_DYNAMICS_REPORT_SCHEMA_V1,
            model_sha256,
            held_out_corpus_sha256,
            normalization_sha256,
            config,
            domains,
            disposition,
            training_authorized: disposition == LocalDynamicsDisposition::ReadyForBoundedTraining,
            promotion_authority: false,
            report_sha256: Digest::ZERO,
        };
        report.report_sha256 = report.digest()?;
        Ok(report)
    }

    fn digest(&self) -> Result<Digest, LocalDynamicsError> {
        canonical_digest(
            b"dusklight.local-dynamics-error-report/v1\0",
            &(
                self.schema,
                self.model_sha256,
                self.held_out_corpus_sha256,
                self.normalization_sha256,
                self.config,
                &self.domains,
                self.disposition,
                self.training_authorized,
                self.promotion_authority,
            ),
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LocalDynamicsTrainingPermit {
    pub schema: &'static str,
    pub report_sha256: Digest,
    pub maximum_horizon_ticks: u16,
    pub held_out_measurement_required: bool,
    pub promotion_authority: bool,
}

impl LocalDynamicsTrainingPermit {
    pub fn issue(report: &LocalDynamicsErrorReport) -> Result<Self, LocalDynamicsError> {
        if report.report_sha256 != report.digest()?
            || report.disposition != LocalDynamicsDisposition::ReadyForBoundedTraining
            || !report.training_authorized
            || report.promotion_authority
        {
            return Err(LocalDynamicsError::new(
                "local dynamics training requires a passing held-out error report",
            ));
        }
        Ok(Self {
            schema: LOCAL_DYNAMICS_PERMIT_SCHEMA_V1,
            report_sha256: report.report_sha256,
            maximum_horizon_ticks: report.config.maximum_horizon_ticks,
            held_out_measurement_required: true,
            promotion_authority: false,
        })
    }
}

fn validate_config(
    model_sha256: Digest,
    held_out_corpus_sha256: Digest,
    normalization_sha256: Digest,
    probes: &[LocalDynamicsProbe],
    config: LocalDynamicsGateConfig,
) -> Result<(), LocalDynamicsError> {
    if [model_sha256, held_out_corpus_sha256, normalization_sha256].contains(&Digest::ZERO)
        || probes.is_empty()
        || probes.len() > MAX_PROBES
        || config.maximum_horizon_ticks == 0
        || config.minimum_probes_per_domain == 0
        || config.minimum_episodes_per_domain == 0
        || !config.maximum_normalized_rmse.is_finite()
        || config.maximum_normalized_rmse < 0.0
        || !config.maximum_event_error_rate.is_finite()
        || !(0.0..=1.0).contains(&config.maximum_event_error_rate)
    {
        return Err(LocalDynamicsError::new(
            "local dynamics gate configuration is invalid",
        ));
    }
    let mut identities = BTreeSet::new();
    for probe in probes {
        probe.validate(config.maximum_horizon_ticks)?;
        if !identities.insert(probe.probe_sha256) {
            return Err(LocalDynamicsError::new(
                "local dynamics probe is duplicated",
            ));
        }
    }
    Ok(())
}

fn canonical_digest<T: Serialize>(domain: &[u8], value: &T) -> Result<Digest, LocalDynamicsError> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| LocalDynamicsError::new(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalDynamicsError(String);

impl LocalDynamicsError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}
impl fmt::Display for LocalDynamicsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}
impl Error for LocalDynamicsError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn probes(error: f32, domains: &[DynamicsProbeDomain]) -> Vec<LocalDynamicsProbe> {
        let mut probes = Vec::new();
        let mut identity = 1_u8;
        for &domain in domains {
            for episode in 0..2_u8 {
                for _ in 0..2 {
                    let mut probe = LocalDynamicsProbe::seal(
                        domain,
                        Digest([episode + 10; 32]),
                        4,
                        vec![error, 0.0],
                        vec![0.0, 0.0],
                        1,
                        1,
                    )
                    .unwrap();
                    // Give repeated numeric fixtures distinct authenticated identities.
                    probe.observed_event = u32::from(identity);
                    probe.predicted_event = u32::from(identity);
                    probe.probe_sha256 = probe.digest().unwrap();
                    probes.push(probe);
                    identity += 1;
                }
            }
        }
        probes
    }

    fn config() -> LocalDynamicsGateConfig {
        LocalDynamicsGateConfig {
            maximum_horizon_ticks: 4,
            minimum_probes_per_domain: 4,
            minimum_episodes_per_domain: 2,
            maximum_normalized_rmse: 0.1,
            maximum_event_error_rate: 0.0,
        }
    }

    #[test]
    fn all_four_held_out_domains_are_required_before_training() {
        let all = probes(0.01, &DynamicsProbeDomain::ALL);
        let report = LocalDynamicsErrorReport::evaluate(
            Digest([1; 32]),
            Digest([2; 32]),
            Digest([3; 32]),
            &all,
            config(),
        )
        .unwrap();
        assert_eq!(
            report.disposition,
            LocalDynamicsDisposition::ReadyForBoundedTraining
        );
        assert_eq!(report.domains.len(), 4);
        let permit = LocalDynamicsTrainingPermit::issue(&report).unwrap();
        assert_eq!(permit.maximum_horizon_ticks, 4);
        assert!(!permit.promotion_authority);

        let incomplete = probes(0.01, &DynamicsProbeDomain::ALL[..3]);
        let report = LocalDynamicsErrorReport::evaluate(
            Digest([1; 32]),
            Digest([2; 32]),
            Digest([3; 32]),
            &incomplete,
            config(),
        )
        .unwrap();
        assert_eq!(
            report.disposition,
            LocalDynamicsDisposition::InsufficientHeldOutCoverage
        );
        assert!(LocalDynamicsTrainingPermit::issue(&report).is_err());
    }

    #[test]
    fn excessive_prediction_error_denies_the_training_permit() {
        let report = LocalDynamicsErrorReport::evaluate(
            Digest([1; 32]),
            Digest([2; 32]),
            Digest([3; 32]),
            &probes(1.0, &DynamicsProbeDomain::ALL),
            config(),
        )
        .unwrap();
        assert_eq!(
            report.disposition,
            LocalDynamicsDisposition::PredictionErrorTooHigh
        );
        assert!(LocalDynamicsTrainingPermit::issue(&report).is_err());
    }
}
