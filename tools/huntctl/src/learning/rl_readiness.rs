//! Corpus-scale readiness gates for neural offline-RL comparisons.

use crate::artifact::Digest;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const RL_SCALE_READINESS_SCHEMA_V1: &str = "dusklight-rl-scale-readiness/v1";
pub const RL_COVERAGE_READINESS_SCHEMA_V1: &str = "dusklight-rl-coverage-readiness/v1";
pub const RL_COMPARISON_READINESS_SCHEMA_V1: &str = "dusklight-rl-comparison-readiness/v1";
pub const RL_PROPOSAL_READINESS_SCHEMA_V1: &str = "dusklight-rl-proposal-readiness/v1";
const MAX_COVERAGE_REGIONS: usize = 100_000;
const MAX_REQUIRED_ACTIONS: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct RlScaleReadinessConfig {
    pub minimum_diverse_episodes: usize,
    pub minimum_option_decisions: u64,
}

impl Default for RlScaleReadinessConfig {
    fn default() -> Self {
        Self {
            minimum_diverse_episodes: 500,
            minimum_option_decisions: 50_000,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RlScaleReadinessDisposition {
    ReadyForNeuralComparison,
    InsufficientEpisodes,
    InsufficientDecisions,
    InsufficientEpisodesAndDecisions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RlScaleReadinessReport {
    pub schema: &'static str,
    pub objective_sha256: Digest,
    pub corpus_manifest_sha256: Digest,
    pub episode_manifest_sha256: Digest,
    pub diverse_episodes: usize,
    pub option_decisions: u64,
    pub config: RlScaleReadinessConfig,
    pub disposition: RlScaleReadinessDisposition,
    pub neural_comparison_meaningful: bool,
    pub promotion_authority: bool,
    pub report_sha256: Digest,
}

impl RlScaleReadinessReport {
    pub fn assess(
        objective_sha256: Digest,
        corpus_manifest_sha256: Digest,
        episode_sha256: &[Digest],
        option_decisions: u64,
        config: RlScaleReadinessConfig,
    ) -> Result<Self, RlReadinessError> {
        if objective_sha256 == Digest::ZERO
            || corpus_manifest_sha256 == Digest::ZERO
            || episode_sha256.is_empty()
            || episode_sha256.contains(&Digest::ZERO)
            || config.minimum_diverse_episodes == 0
            || config.minimum_option_decisions == 0
        {
            return Err(RlReadinessError::new("RL scale readiness input is invalid"));
        }
        let unique = episode_sha256.iter().copied().collect::<BTreeSet<_>>();
        let episode_manifest_sha256 = digest_episodes(&unique);
        let episode_ready = unique.len() >= config.minimum_diverse_episodes;
        let decision_ready = option_decisions >= config.minimum_option_decisions;
        let disposition = match (episode_ready, decision_ready) {
            (true, true) => RlScaleReadinessDisposition::ReadyForNeuralComparison,
            (false, true) => RlScaleReadinessDisposition::InsufficientEpisodes,
            (true, false) => RlScaleReadinessDisposition::InsufficientDecisions,
            (false, false) => RlScaleReadinessDisposition::InsufficientEpisodesAndDecisions,
        };
        let mut report = Self {
            schema: RL_SCALE_READINESS_SCHEMA_V1,
            objective_sha256,
            corpus_manifest_sha256,
            episode_manifest_sha256,
            diverse_episodes: unique.len(),
            option_decisions,
            config,
            disposition,
            neural_comparison_meaningful: disposition
                == RlScaleReadinessDisposition::ReadyForNeuralComparison,
            promotion_authority: false,
            report_sha256: Digest::ZERO,
        };
        let bytes = serde_json::to_vec(&(
            report.schema,
            report.objective_sha256,
            report.corpus_manifest_sha256,
            report.episode_manifest_sha256,
            report.diverse_episodes,
            report.option_decisions,
            report.config,
            report.disposition,
            report.neural_comparison_meaningful,
            report.promotion_authority,
        ))
        .map_err(|error| RlReadinessError::new(error.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.rl-scale-readiness/v1\0");
        hasher.update(bytes);
        report.report_sha256 = Digest(hasher.finalize().into());
        Ok(report)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RegionActionCoverage {
    pub player_procedure: String,
    pub spatial_phase: String,
    pub required_action_ids: Vec<u32>,
    pub observed_action_decisions: BTreeMap<u32, u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct RlCoverageReadinessConfig {
    pub minimum_decisions_per_required_action: u64,
    pub minimum_region_support_ratio: f64,
}

impl Default for RlCoverageReadinessConfig {
    fn default() -> Self {
        Self {
            minimum_decisions_per_required_action: 16,
            minimum_region_support_ratio: 0.9,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct UnsupportedCoverageRegion {
    pub player_procedure: String,
    pub spatial_phase: String,
    pub required_actions: usize,
    pub supported_actions: usize,
    pub support_ratio_millionths: u32,
    pub insufficient_action_ids: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RlCoverageReadinessReport {
    pub schema: &'static str,
    pub objective_sha256: Digest,
    pub corpus_manifest_sha256: Digest,
    pub region_manifest_sha256: Digest,
    pub config: RlCoverageReadinessConfig,
    pub regions: usize,
    pub unsupported_regions: Vec<UnsupportedCoverageRegion>,
    pub broad_support_in_every_region: bool,
    pub promotion_authority: bool,
    pub report_sha256: Digest,
}

impl RlCoverageReadinessReport {
    pub fn assess(
        objective_sha256: Digest,
        corpus_manifest_sha256: Digest,
        regions: &[RegionActionCoverage],
        config: RlCoverageReadinessConfig,
    ) -> Result<Self, RlReadinessError> {
        validate_coverage_inputs(objective_sha256, corpus_manifest_sha256, regions, config)?;
        let region_manifest_sha256 =
            canonical_digest(b"dusklight.rl-coverage-regions/v1\0", &regions)?;
        let unsupported_regions = regions
            .iter()
            .filter_map(|region| unsupported_region(region, config))
            .collect::<Vec<_>>();
        let broad_support_in_every_region = unsupported_regions.iter().all(|region| {
            f64::from(region.support_ratio_millionths) / 1_000_000.0
                >= config.minimum_region_support_ratio
        });
        let mut report = Self {
            schema: RL_COVERAGE_READINESS_SCHEMA_V1,
            objective_sha256,
            corpus_manifest_sha256,
            region_manifest_sha256,
            config,
            regions: regions.len(),
            unsupported_regions,
            broad_support_in_every_region,
            promotion_authority: false,
            report_sha256: Digest::ZERO,
        };
        report.report_sha256 = canonical_digest(
            b"dusklight.rl-coverage-readiness/v1\0",
            &(
                report.schema,
                report.objective_sha256,
                report.corpus_manifest_sha256,
                report.region_manifest_sha256,
                report.config,
                report.regions,
                &report.unsupported_regions,
                report.broad_support_in_every_region,
                report.promotion_authority,
            ),
        )?;
        Ok(report)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct RlComparisonReadinessConfig {
    pub minimum_held_out_episodes: usize,
    pub minimum_boundary_families: usize,
    pub minimum_cold_replay_repetitions: usize,
    pub minimum_native_success_improvement: f64,
}

impl Default for RlComparisonReadinessConfig {
    fn default() -> Self {
        Self {
            minimum_held_out_episodes: 100,
            minimum_boundary_families: 5,
            minimum_cold_replay_repetitions: 3,
            minimum_native_success_improvement: 0.02,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct EqualBudgetNativeComparison {
    pub comparison_report_sha256: Digest,
    pub cold_replay_report_sha256: Digest,
    pub cold_replay_repetitions: usize,
    pub cold_replay_stable: bool,
    pub candidate_sample_budget: usize,
    pub tree_fqi_sample_budget: usize,
    pub structured_specialist_sample_budget: usize,
    pub candidate_native_success_rate: f64,
    pub tree_fqi_native_success_rate: f64,
    pub structured_specialist_native_success_rate: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RlComparisonReadinessReport {
    pub schema: &'static str,
    pub objective_sha256: Digest,
    pub model_sha256: Digest,
    pub held_out_episode_manifest_sha256: Digest,
    pub boundary_family_manifest_sha256: Digest,
    pub held_out_episodes: usize,
    pub held_out_boundary_families: usize,
    pub config: RlComparisonReadinessConfig,
    pub comparison: EqualBudgetNativeComparison,
    pub equal_sample_budget: bool,
    pub stable_cold_replay_ready: bool,
    pub stronger_than_tree_and_structured: bool,
    pub held_out_comparison_ready: bool,
    pub promotion_authority: bool,
    pub report_sha256: Digest,
}

impl RlComparisonReadinessReport {
    pub fn assess(
        objective_sha256: Digest,
        model_sha256: Digest,
        held_out_episode_sha256: &[Digest],
        boundary_family_sha256: &[Digest],
        comparison: EqualBudgetNativeComparison,
        config: RlComparisonReadinessConfig,
    ) -> Result<Self, RlReadinessError> {
        validate_comparison_inputs(
            objective_sha256,
            model_sha256,
            held_out_episode_sha256,
            boundary_family_sha256,
            comparison,
            config,
        )?;
        let held_out_episodes = held_out_episode_sha256
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let boundary_families = boundary_family_sha256
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let held_out_episode_manifest_sha256 =
            digest_set(b"dusklight.rl-held-out-episodes/v1\0", &held_out_episodes);
        let boundary_family_manifest_sha256 = digest_set(
            b"dusklight.rl-held-out-boundary-families/v1\0",
            &boundary_families,
        );
        let equal_sample_budget = comparison.candidate_sample_budget
            == comparison.tree_fqi_sample_budget
            && comparison.candidate_sample_budget == comparison.structured_specialist_sample_budget;
        let stable_cold_replay_ready = comparison.cold_replay_stable
            && comparison.cold_replay_repetitions >= config.minimum_cold_replay_repetitions;
        let strongest_baseline = comparison
            .tree_fqi_native_success_rate
            .max(comparison.structured_specialist_native_success_rate);
        let stronger_than_tree_and_structured = comparison.candidate_native_success_rate
            >= strongest_baseline + config.minimum_native_success_improvement;
        let held_out_comparison_ready = held_out_episodes.len() >= config.minimum_held_out_episodes
            && boundary_families.len() >= config.minimum_boundary_families
            && equal_sample_budget
            && stable_cold_replay_ready
            && stronger_than_tree_and_structured;
        let mut report = Self {
            schema: RL_COMPARISON_READINESS_SCHEMA_V1,
            objective_sha256,
            model_sha256,
            held_out_episode_manifest_sha256,
            boundary_family_manifest_sha256,
            held_out_episodes: held_out_episodes.len(),
            held_out_boundary_families: boundary_families.len(),
            config,
            comparison,
            equal_sample_budget,
            stable_cold_replay_ready,
            stronger_than_tree_and_structured,
            held_out_comparison_ready,
            promotion_authority: false,
            report_sha256: Digest::ZERO,
        };
        report.report_sha256 = canonical_digest(
            b"dusklight.rl-comparison-readiness/v1\0",
            &(
                report.schema,
                report.objective_sha256,
                report.model_sha256,
                report.held_out_episode_manifest_sha256,
                report.boundary_family_manifest_sha256,
                report.held_out_episodes,
                report.held_out_boundary_families,
                report.config,
                report.comparison,
                report.equal_sample_budget,
                report.stable_cold_replay_ready,
                report.stronger_than_tree_and_structured,
                report.held_out_comparison_ready,
                report.promotion_authority,
            ),
        )?;
        Ok(report)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct RlProposalReadinessConfig {
    pub maximum_expected_calibration_error: f64,
    pub maximum_unsupported_simulator_fraction: f64,
    pub minimum_known_ood_rejection_recall: f64,
}

impl Default for RlProposalReadinessConfig {
    fn default() -> Self {
        Self {
            maximum_expected_calibration_error: 0.1,
            maximum_unsupported_simulator_fraction: 0.2,
            minimum_known_ood_rejection_recall: 0.8,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ProposalQualityEvidence {
    pub calibration_report_sha256: Digest,
    pub ood_report_sha256: Digest,
    pub expected_calibration_error: f64,
    pub simulator_proposals: usize,
    pub unsupported_simulator_proposals: usize,
    pub known_ood_candidates: usize,
    pub known_ood_rejected_before_rollout: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RlProposalReadinessReport {
    pub schema: &'static str,
    pub objective_sha256: Digest,
    pub model_sha256: Digest,
    pub config: RlProposalReadinessConfig,
    pub evidence: ProposalQualityEvidence,
    pub unsupported_simulator_fraction: f64,
    pub known_ood_rejection_recall: f64,
    pub calibration_ready: bool,
    pub ood_diagnostics_ready: bool,
    pub simulator_budget_ready: bool,
    pub proposal_quality_ready: bool,
    pub promotion_authority: bool,
    pub report_sha256: Digest,
}

impl RlProposalReadinessReport {
    pub fn assess(
        objective_sha256: Digest,
        model_sha256: Digest,
        evidence: ProposalQualityEvidence,
        config: RlProposalReadinessConfig,
    ) -> Result<Self, RlReadinessError> {
        validate_proposal_inputs(objective_sha256, model_sha256, evidence, config)?;
        let unsupported_simulator_fraction =
            evidence.unsupported_simulator_proposals as f64 / evidence.simulator_proposals as f64;
        let known_ood_rejection_recall = evidence.known_ood_rejected_before_rollout as f64
            / evidence.known_ood_candidates as f64;
        let calibration_ready =
            evidence.expected_calibration_error <= config.maximum_expected_calibration_error;
        let ood_diagnostics_ready =
            known_ood_rejection_recall >= config.minimum_known_ood_rejection_recall;
        let simulator_budget_ready =
            unsupported_simulator_fraction <= config.maximum_unsupported_simulator_fraction;
        let proposal_quality_ready =
            calibration_ready && ood_diagnostics_ready && simulator_budget_ready;
        let mut report = Self {
            schema: RL_PROPOSAL_READINESS_SCHEMA_V1,
            objective_sha256,
            model_sha256,
            config,
            evidence,
            unsupported_simulator_fraction,
            known_ood_rejection_recall,
            calibration_ready,
            ood_diagnostics_ready,
            simulator_budget_ready,
            proposal_quality_ready,
            promotion_authority: false,
            report_sha256: Digest::ZERO,
        };
        report.report_sha256 = canonical_digest(
            b"dusklight.rl-proposal-readiness/v1\0",
            &(
                report.schema,
                report.objective_sha256,
                report.model_sha256,
                report.config,
                report.evidence,
                report.unsupported_simulator_fraction,
                report.known_ood_rejection_recall,
                report.calibration_ready,
                report.ood_diagnostics_ready,
                report.simulator_budget_ready,
                report.proposal_quality_ready,
                report.promotion_authority,
            ),
        )?;
        Ok(report)
    }
}

fn validate_proposal_inputs(
    objective_sha256: Digest,
    model_sha256: Digest,
    evidence: ProposalQualityEvidence,
    config: RlProposalReadinessConfig,
) -> Result<(), RlReadinessError> {
    if objective_sha256 == Digest::ZERO
        || model_sha256 == Digest::ZERO
        || evidence.calibration_report_sha256 == Digest::ZERO
        || evidence.ood_report_sha256 == Digest::ZERO
        || !valid_unit(evidence.expected_calibration_error)
        || evidence.simulator_proposals == 0
        || evidence.unsupported_simulator_proposals > evidence.simulator_proposals
        || evidence.known_ood_candidates == 0
        || evidence.known_ood_rejected_before_rollout > evidence.known_ood_candidates
        || !valid_unit(config.maximum_expected_calibration_error)
        || !valid_unit(config.maximum_unsupported_simulator_fraction)
        || !valid_unit(config.minimum_known_ood_rejection_recall)
    {
        return Err(RlReadinessError::new(
            "RL proposal readiness input is invalid",
        ));
    }
    Ok(())
}

fn validate_comparison_inputs(
    objective_sha256: Digest,
    model_sha256: Digest,
    held_out_episode_sha256: &[Digest],
    boundary_family_sha256: &[Digest],
    comparison: EqualBudgetNativeComparison,
    config: RlComparisonReadinessConfig,
) -> Result<(), RlReadinessError> {
    if objective_sha256 == Digest::ZERO
        || model_sha256 == Digest::ZERO
        || held_out_episode_sha256.is_empty()
        || held_out_episode_sha256.contains(&Digest::ZERO)
        || boundary_family_sha256.is_empty()
        || boundary_family_sha256.contains(&Digest::ZERO)
        || comparison.comparison_report_sha256 == Digest::ZERO
        || comparison.cold_replay_report_sha256 == Digest::ZERO
        || comparison.cold_replay_repetitions < 2
        || comparison.candidate_sample_budget == 0
        || comparison.tree_fqi_sample_budget == 0
        || comparison.structured_specialist_sample_budget == 0
        || !valid_unit(comparison.candidate_native_success_rate)
        || !valid_unit(comparison.tree_fqi_native_success_rate)
        || !valid_unit(comparison.structured_specialist_native_success_rate)
        || config.minimum_held_out_episodes == 0
        || config.minimum_boundary_families == 0
        || config.minimum_cold_replay_repetitions < 2
        || !valid_unit(config.minimum_native_success_improvement)
    {
        return Err(RlReadinessError::new(
            "RL held-out comparison readiness input is invalid",
        ));
    }
    Ok(())
}

fn valid_unit(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn digest_set(domain: &[u8], values: &BTreeSet<Digest>) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((values.len() as u64).to_le_bytes());
    for value in values {
        hasher.update(value.0);
    }
    Digest(hasher.finalize().into())
}

fn unsupported_region(
    region: &RegionActionCoverage,
    config: RlCoverageReadinessConfig,
) -> Option<UnsupportedCoverageRegion> {
    let insufficient_action_ids = region
        .required_action_ids
        .iter()
        .copied()
        .filter(|action| {
            region
                .observed_action_decisions
                .get(action)
                .copied()
                .unwrap_or(0)
                < config.minimum_decisions_per_required_action
        })
        .collect::<Vec<_>>();
    if insufficient_action_ids.is_empty() {
        return None;
    }
    let supported_actions = region.required_action_ids.len() - insufficient_action_ids.len();
    Some(UnsupportedCoverageRegion {
        player_procedure: region.player_procedure.clone(),
        spatial_phase: region.spatial_phase.clone(),
        required_actions: region.required_action_ids.len(),
        supported_actions,
        support_ratio_millionths: ((supported_actions as u64 * 1_000_000)
            / region.required_action_ids.len() as u64) as u32,
        insufficient_action_ids,
    })
}

fn validate_coverage_inputs(
    objective_sha256: Digest,
    corpus_manifest_sha256: Digest,
    regions: &[RegionActionCoverage],
    config: RlCoverageReadinessConfig,
) -> Result<(), RlReadinessError> {
    if objective_sha256 == Digest::ZERO
        || corpus_manifest_sha256 == Digest::ZERO
        || regions.is_empty()
        || regions.len() > MAX_COVERAGE_REGIONS
        || config.minimum_decisions_per_required_action == 0
        || !config.minimum_region_support_ratio.is_finite()
        || !(0.0..=1.0).contains(&config.minimum_region_support_ratio)
    {
        return Err(RlReadinessError::new(
            "RL coverage readiness input is invalid",
        ));
    }
    let mut identities = BTreeSet::new();
    if regions.iter().any(|region| {
        !valid_label(&region.player_procedure)
            || !valid_label(&region.spatial_phase)
            || !identities.insert((
                region.player_procedure.as_str(),
                region.spatial_phase.as_str(),
            ))
            || region.required_action_ids.is_empty()
            || region.required_action_ids.len() > MAX_REQUIRED_ACTIONS
            || !region
                .required_action_ids
                .windows(2)
                .all(|pair| pair[0] < pair[1])
    }) {
        return Err(RlReadinessError::new(
            "RL coverage region is invalid or duplicated",
        ));
    }
    Ok(())
}

fn valid_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-/".contains(&byte))
}

fn canonical_digest<T: Serialize>(domain: &[u8], value: &T) -> Result<Digest, RlReadinessError> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| RlReadinessError::new(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

fn digest_episodes(episodes: &BTreeSet<Digest>) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.rl-diverse-episodes/v1\0");
    hasher.update((episodes.len() as u64).to_le_bytes());
    for episode in episodes {
        hasher.update(episode.0);
    }
    Digest(hasher.finalize().into())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RlReadinessError(String);
impl RlReadinessError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}
impl fmt::Display for RlReadinessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl Error for RlReadinessError {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn five_hundred_unique_episodes_and_fifty_thousand_decisions_are_both_required() {
        let episodes = (0..500_u16)
            .map(|value| {
                let mut bytes = [0_u8; 32];
                bytes[..2].copy_from_slice(&(value + 1).to_le_bytes());
                Digest(bytes)
            })
            .collect::<Vec<_>>();
        let ready = RlScaleReadinessReport::assess(
            Digest([1; 32]),
            Digest([2; 32]),
            &episodes,
            50_000,
            RlScaleReadinessConfig::default(),
        )
        .unwrap();
        assert!(ready.neural_comparison_meaningful);
        let small = RlScaleReadinessReport::assess(
            Digest([1; 32]),
            Digest([2; 32]),
            &episodes[..499],
            49_999,
            RlScaleReadinessConfig::default(),
        )
        .unwrap();
        assert_eq!(
            small.disposition,
            RlScaleReadinessDisposition::InsufficientEpisodesAndDecisions
        );
        assert!(!small.neural_comparison_meaningful);
        assert!(!small.promotion_authority);
    }

    #[test]
    fn low_support_actions_remain_explicit_per_procedure_and_spatial_phase() {
        let regions = vec![RegionActionCoverage {
            player_procedure: "roll".into(),
            spatial_phase: "approach".into(),
            required_action_ids: vec![0, 1, 2, 3],
            observed_action_decisions: BTreeMap::from([(0, 20), (1, 20), (2, 20), (3, 0)]),
        }];
        let report = RlCoverageReadinessReport::assess(
            Digest([1; 32]),
            Digest([2; 32]),
            &regions,
            RlCoverageReadinessConfig {
                minimum_region_support_ratio: 0.8,
                ..RlCoverageReadinessConfig::default()
            },
        )
        .unwrap();
        assert!(!report.broad_support_in_every_region);
        assert_eq!(report.unsupported_regions.len(), 1);
        assert_eq!(
            report.unsupported_regions[0].insufficient_action_ids,
            vec![3]
        );

        let mut broad = regions;
        broad[0].observed_action_decisions.insert(3, 20);
        let report = RlCoverageReadinessReport::assess(
            Digest([1; 32]),
            Digest([2; 32]),
            &broad,
            RlCoverageReadinessConfig::default(),
        )
        .unwrap();
        assert!(report.broad_support_in_every_region);
        assert!(report.unsupported_regions.is_empty());
        assert!(!report.promotion_authority);
        assert_ne!(report.report_sha256, Digest::ZERO);
    }

    #[test]
    fn held_out_boundary_and_cold_replay_comparison_requires_equal_budget_and_win() {
        let episodes = (1..=3).map(|id| Digest([id; 32])).collect::<Vec<_>>();
        let boundaries = (10..=11).map(|id| Digest([id; 32])).collect::<Vec<_>>();
        let comparison = EqualBudgetNativeComparison {
            comparison_report_sha256: Digest([20; 32]),
            cold_replay_report_sha256: Digest([21; 32]),
            cold_replay_repetitions: 3,
            cold_replay_stable: true,
            candidate_sample_budget: 100,
            tree_fqi_sample_budget: 100,
            structured_specialist_sample_budget: 100,
            candidate_native_success_rate: 0.8,
            tree_fqi_native_success_rate: 0.6,
            structured_specialist_native_success_rate: 0.7,
        };
        let config = RlComparisonReadinessConfig {
            minimum_held_out_episodes: 3,
            minimum_boundary_families: 2,
            ..RlComparisonReadinessConfig::default()
        };
        let ready = RlComparisonReadinessReport::assess(
            Digest([1; 32]),
            Digest([2; 32]),
            &episodes,
            &boundaries,
            comparison,
            config,
        )
        .unwrap();
        assert!(ready.equal_sample_budget);
        assert!(ready.stable_cold_replay_ready);
        assert!(ready.stronger_than_tree_and_structured);
        assert!(ready.held_out_comparison_ready);

        let mut unequal = comparison;
        unequal.structured_specialist_sample_budget = 90;
        unequal.cold_replay_stable = false;
        let blocked = RlComparisonReadinessReport::assess(
            Digest([1; 32]),
            Digest([2; 32]),
            &episodes,
            &boundaries,
            unequal,
            config,
        )
        .unwrap();
        assert!(!blocked.equal_sample_budget);
        assert!(!blocked.stable_cold_replay_ready);
        assert!(!blocked.held_out_comparison_ready);
        assert!(!blocked.promotion_authority);
    }

    #[test]
    fn calibrated_ood_filter_must_avoid_unsupported_simulator_spend() {
        let evidence = ProposalQualityEvidence {
            calibration_report_sha256: Digest([3; 32]),
            ood_report_sha256: Digest([4; 32]),
            expected_calibration_error: 0.05,
            simulator_proposals: 100,
            unsupported_simulator_proposals: 10,
            known_ood_candidates: 100,
            known_ood_rejected_before_rollout: 90,
        };
        let ready = RlProposalReadinessReport::assess(
            Digest([1; 32]),
            Digest([2; 32]),
            evidence,
            RlProposalReadinessConfig::default(),
        )
        .unwrap();
        assert!(ready.calibration_ready);
        assert!(ready.ood_diagnostics_ready);
        assert!(ready.simulator_budget_ready);
        assert!(ready.proposal_quality_ready);
        assert!(!ready.promotion_authority);

        let weak = RlProposalReadinessReport::assess(
            Digest([1; 32]),
            Digest([2; 32]),
            ProposalQualityEvidence {
                expected_calibration_error: 0.2,
                unsupported_simulator_proposals: 60,
                known_ood_rejected_before_rollout: 20,
                ..evidence
            },
            RlProposalReadinessConfig::default(),
        )
        .unwrap();
        assert_eq!(weak.unsupported_simulator_fraction, 0.6);
        assert!(!weak.calibration_ready);
        assert!(!weak.ood_diagnostics_ready);
        assert!(!weak.simulator_budget_ready);
        assert!(!weak.proposal_quality_ready);
    }
}
