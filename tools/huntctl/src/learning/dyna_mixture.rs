//! Real-rooted Dyna rollout selection with strict uncertainty cutoffs.

use super::local_dynamics::LocalDynamicsErrorReport;
use crate::artifact::Digest;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const DYNA_MIXTURE_REPORT_SCHEMA_V1: &str = "dusklight-dyna-mixture-report/v1";
const MAX_REAL_TRANSITIONS: usize = 1_000_000;
const MAX_ROLLOUT_CANDIDATES: usize = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct DynaMixtureConfig {
    pub maximum_horizon_ticks: u16,
    pub maximum_epistemic_standard_deviation: f64,
    pub maximum_event_mismatch_probability: f64,
    pub maximum_ood_distance: f64,
    pub maximum_model_to_real_ratio: f64,
    pub model_sample_weight: f64,
}

impl Default for DynaMixtureConfig {
    fn default() -> Self {
        Self {
            maximum_horizon_ticks: 4,
            maximum_epistemic_standard_deviation: 0.05,
            maximum_event_mismatch_probability: 0.01,
            maximum_ood_distance: 0.1,
            maximum_model_to_real_ratio: 0.25,
            model_sample_weight: 0.1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct DynaRolloutCandidate {
    pub rollout_sha256: Digest,
    pub root_real_transition_sha256: Digest,
    pub model_sha256: Digest,
    pub horizon_ticks: u16,
    pub epistemic_standard_deviation: f64,
    pub event_mismatch_probability: f64,
    pub ood_distance: f64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct DynaRejectionCounts {
    pub horizon: usize,
    pub epistemic_uncertainty: usize,
    pub event_risk: usize,
    pub out_of_distribution: usize,
    pub mixture_budget: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DynaMixtureReport {
    pub schema: &'static str,
    pub local_dynamics_report_sha256: Digest,
    pub model_sha256: Digest,
    pub real_corpus_sha256: Digest,
    pub config: DynaMixtureConfig,
    pub real_transitions: usize,
    pub rollout_candidates: usize,
    pub selected_rollout_sha256: Vec<Digest>,
    pub selected_model_to_real_ratio: f64,
    pub rejection_counts: DynaRejectionCounts,
    pub synthetic_rollouts_are_real_rooted: bool,
    pub recursive_model_rollouts_allowed: bool,
    pub real_only_held_out_evaluation_required: bool,
    pub mixture_is_research_only: bool,
    pub promotion_authority: bool,
    pub report_sha256: Digest,
}

impl DynaMixtureReport {
    pub fn select(
        error_report: &LocalDynamicsErrorReport,
        real_corpus_sha256: Digest,
        real_transition_sha256: &[Digest],
        candidates: &[DynaRolloutCandidate],
        config: DynaMixtureConfig,
    ) -> Result<Self, DynaMixtureError> {
        error_report
            .validate()
            .map_err(|error| DynaMixtureError::new(error.to_string()))?;
        validate_inputs(
            error_report,
            real_corpus_sha256,
            real_transition_sha256,
            candidates,
            config,
        )?;
        let mut rejection_counts = DynaRejectionCounts::default();
        let mut eligible = Vec::new();
        for candidate in candidates {
            let mut rejected = false;
            if candidate.horizon_ticks > config.maximum_horizon_ticks {
                rejection_counts.horizon += 1;
                rejected = true;
            }
            if candidate.epistemic_standard_deviation > config.maximum_epistemic_standard_deviation
            {
                rejection_counts.epistemic_uncertainty += 1;
                rejected = true;
            }
            if candidate.event_mismatch_probability > config.maximum_event_mismatch_probability {
                rejection_counts.event_risk += 1;
                rejected = true;
            }
            if candidate.ood_distance > config.maximum_ood_distance {
                rejection_counts.out_of_distribution += 1;
                rejected = true;
            }
            if !rejected {
                eligible.push(*candidate);
            }
        }
        eligible.sort_by(|left, right| {
            left.epistemic_standard_deviation
                .total_cmp(&right.epistemic_standard_deviation)
                .then_with(|| {
                    left.event_mismatch_probability
                        .total_cmp(&right.event_mismatch_probability)
                })
                .then_with(|| left.ood_distance.total_cmp(&right.ood_distance))
                .then_with(|| left.horizon_ticks.cmp(&right.horizon_ticks))
                .then_with(|| left.rollout_sha256.cmp(&right.rollout_sha256))
        });
        let maximum_model_rows = (real_transition_sha256.len() as f64
            * config.maximum_model_to_real_ratio)
            .floor() as usize;
        if eligible.len() > maximum_model_rows {
            rejection_counts.mixture_budget = eligible.len() - maximum_model_rows;
            eligible.truncate(maximum_model_rows);
        }
        let selected_rollout_sha256 = eligible
            .iter()
            .map(|candidate| candidate.rollout_sha256)
            .collect::<Vec<_>>();
        let selected_model_to_real_ratio =
            selected_rollout_sha256.len() as f64 / real_transition_sha256.len() as f64;
        let mut report = Self {
            schema: DYNA_MIXTURE_REPORT_SCHEMA_V1,
            local_dynamics_report_sha256: error_report.report_sha256,
            model_sha256: error_report.model_sha256,
            real_corpus_sha256,
            config,
            real_transitions: real_transition_sha256.len(),
            rollout_candidates: candidates.len(),
            selected_rollout_sha256,
            selected_model_to_real_ratio,
            rejection_counts,
            synthetic_rollouts_are_real_rooted: true,
            recursive_model_rollouts_allowed: false,
            real_only_held_out_evaluation_required: true,
            mixture_is_research_only: true,
            promotion_authority: false,
            report_sha256: Digest::ZERO,
        };
        report.report_sha256 = report.digest()?;
        Ok(report)
    }

    fn digest(&self) -> Result<Digest, DynaMixtureError> {
        canonical_digest(
            b"dusklight.dyna-mixture-report/v1\0",
            &(
                self.schema,
                self.local_dynamics_report_sha256,
                self.model_sha256,
                self.real_corpus_sha256,
                self.config,
                self.real_transitions,
                self.rollout_candidates,
                &self.selected_rollout_sha256,
                self.selected_model_to_real_ratio,
                &self.rejection_counts,
                self.synthetic_rollouts_are_real_rooted,
                self.recursive_model_rollouts_allowed,
                self.real_only_held_out_evaluation_required,
                self.mixture_is_research_only,
                self.promotion_authority,
            ),
        )
    }
}

fn validate_inputs(
    error_report: &LocalDynamicsErrorReport,
    real_corpus_sha256: Digest,
    real_transition_sha256: &[Digest],
    candidates: &[DynaRolloutCandidate],
    config: DynaMixtureConfig,
) -> Result<(), DynaMixtureError> {
    if !error_report.training_authorized
        || real_corpus_sha256 == Digest::ZERO
        || real_transition_sha256.is_empty()
        || real_transition_sha256.len() > MAX_REAL_TRANSITIONS
        || candidates.len() > MAX_ROLLOUT_CANDIDATES
        || config.maximum_horizon_ticks == 0
        || config.maximum_horizon_ticks > error_report.config.maximum_horizon_ticks
        || !config.maximum_epistemic_standard_deviation.is_finite()
        || config.maximum_epistemic_standard_deviation < 0.0
        || !config.maximum_event_mismatch_probability.is_finite()
        || !(0.0..=1.0).contains(&config.maximum_event_mismatch_probability)
        || !config.maximum_ood_distance.is_finite()
        || config.maximum_ood_distance < 0.0
        || !config.maximum_model_to_real_ratio.is_finite()
        || !(0.0..=1.0).contains(&config.maximum_model_to_real_ratio)
        || !config.model_sample_weight.is_finite()
        || !(0.0..=0.25).contains(&config.model_sample_weight)
    {
        return Err(DynaMixtureError::new(
            "Dyna mixture readiness or configuration is invalid",
        ));
    }
    let real = real_transition_sha256
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let rollouts = candidates
        .iter()
        .map(|candidate| candidate.rollout_sha256)
        .collect::<BTreeSet<_>>();
    if real.len() != real_transition_sha256.len()
        || real.contains(&Digest::ZERO)
        || rollouts.len() != candidates.len()
        || rollouts.contains(&Digest::ZERO)
        || candidates.iter().any(|candidate| {
            candidate.model_sha256 != error_report.model_sha256
                || !real.contains(&candidate.root_real_transition_sha256)
                || candidate.horizon_ticks == 0
                || !candidate.epistemic_standard_deviation.is_finite()
                || candidate.epistemic_standard_deviation < 0.0
                || !candidate.event_mismatch_probability.is_finite()
                || !(0.0..=1.0).contains(&candidate.event_mismatch_probability)
                || !candidate.ood_distance.is_finite()
                || candidate.ood_distance < 0.0
        })
    {
        return Err(DynaMixtureError::new(
            "Dyna rollout identities, ancestry, or uncertainty are invalid",
        ));
    }
    Ok(())
}

fn canonical_digest<T: Serialize>(domain: &[u8], value: &T) -> Result<Digest, DynaMixtureError> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| DynaMixtureError::new(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DynaMixtureError(String);

impl DynaMixtureError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for DynaMixtureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for DynaMixtureError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning::local_dynamics::{
        DynamicsProbeDomain, LocalDynamicsGateConfig, LocalDynamicsProbe,
    };

    fn error_report() -> LocalDynamicsErrorReport {
        let probes = [
            DynamicsProbeDomain::Contact,
            DynamicsProbeDomain::ProcedureTransition,
            DynamicsProbeDomain::RngSensitiveBranch,
            DynamicsProbeDomain::ActorInteraction,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, domain)| {
            LocalDynamicsProbe::seal(
                domain,
                Digest([10 + index as u8; 32]),
                1,
                vec![0.0],
                vec![0.0],
                1,
                1,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
        LocalDynamicsErrorReport::evaluate(
            Digest([1; 32]),
            Digest([2; 32]),
            Digest([3; 32]),
            Digest([4; 32]),
            &probes,
            LocalDynamicsGateConfig {
                maximum_horizon_ticks: 4,
                minimum_probes_per_domain: 1,
                minimum_episodes_per_domain: 1,
                ..LocalDynamicsGateConfig::default()
            },
        )
        .unwrap()
    }

    #[test]
    fn strict_cutoffs_and_ratio_cap_bound_real_rooted_rollouts() {
        let readiness = error_report();
        let real = (20..28).map(|id| Digest([id; 32])).collect::<Vec<_>>();
        let candidates = (0..5)
            .map(|index| DynaRolloutCandidate {
                rollout_sha256: Digest([50 + index; 32]),
                root_real_transition_sha256: real[index as usize],
                model_sha256: readiness.model_sha256,
                horizon_ticks: 2,
                epistemic_standard_deviation: if index == 4 { 0.5 } else { 0.01 },
                event_mismatch_probability: 0.0,
                ood_distance: 0.01,
            })
            .collect::<Vec<_>>();
        let report = DynaMixtureReport::select(
            &readiness,
            Digest([9; 32]),
            &real,
            &candidates,
            DynaMixtureConfig::default(),
        )
        .unwrap();
        assert_eq!(report.selected_rollout_sha256.len(), 2);
        assert_eq!(report.rejection_counts.epistemic_uncertainty, 1);
        assert_eq!(report.rejection_counts.mixture_budget, 2);
        assert!(!report.recursive_model_rollouts_allowed);
        assert!(report.real_only_held_out_evaluation_required);
        assert!(!report.promotion_authority);
    }

    #[test]
    fn detached_model_or_nonreal_root_fails_closed() {
        let readiness = error_report();
        let real = vec![Digest([20; 32])];
        let mut candidate = DynaRolloutCandidate {
            rollout_sha256: Digest([30; 32]),
            root_real_transition_sha256: real[0],
            model_sha256: Digest([99; 32]),
            horizon_ticks: 1,
            epistemic_standard_deviation: 0.0,
            event_mismatch_probability: 0.0,
            ood_distance: 0.0,
        };
        assert!(
            DynaMixtureReport::select(
                &readiness,
                Digest([9; 32]),
                &real,
                &[candidate],
                DynaMixtureConfig::default()
            )
            .is_err()
        );
        candidate.model_sha256 = readiness.model_sha256;
        candidate.root_real_transition_sha256 = Digest([21; 32]);
        assert!(
            DynaMixtureReport::select(
                &readiness,
                Digest([9; 32]),
                &real,
                &[candidate],
                DynaMixtureConfig::default()
            )
            .is_err()
        );
    }
}
