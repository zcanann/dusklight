//! Corpus-scale readiness gates for neural offline-RL comparisons.

use crate::artifact::Digest;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const RL_SCALE_READINESS_SCHEMA_V1: &str = "dusklight-rl-scale-readiness/v1";

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
}
