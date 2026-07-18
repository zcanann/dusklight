//! Episode-level first-seen and low-support semantic novelty accounting.

use super::{SemanticNoveltyDescriptor, SemanticStateCombination, StateTransitionFact};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const SEMANTIC_NOVELTY_CATALOG_SCHEMA: &str = "dusklight-semantic-novelty-catalog/v1";
pub const SEMANTIC_NOVELTY_ASSESSMENT_SCHEMA: &str = "dusklight-semantic-novelty-assessment/v1";
pub const MAX_RARE_SUPPORT_EPISODES: u64 = 1_000;
pub const MAX_TRACKED_SEMANTIC_FACTS: usize = 65_536;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SemanticNoveltyCatalogConfig {
    /// A combination with at most this many prior supporting episodes is rare.
    pub rare_support_episode_ceiling: u64,
}

impl Default for SemanticNoveltyCatalogConfig {
    fn default() -> Self {
        Self {
            rare_support_episode_ceiling: 3,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RareStateCombinationReason {
    pub combination: SemanticStateCombination,
    pub prior_supporting_episodes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SemanticNoveltyAssessment {
    pub schema: &'static str,
    pub descriptor_identity: String,
    pub catalog_observed_episodes_before: u64,
    pub rare_support_episode_ceiling: u64,
    pub first_seen_transitions: Vec<StateTransitionFact>,
    pub rare_state_combinations: Vec<RareStateCombinationReason>,
    pub semantic_novel: bool,
    pub spatial_distance_used: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CountedTransition {
    pub transition: StateTransitionFact,
    pub supporting_episodes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CountedStateCombination {
    pub combination: SemanticStateCombination,
    pub supporting_episodes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SemanticNoveltyCatalogSnapshot {
    pub schema: &'static str,
    pub observed_episodes: u64,
    pub transition_support: Vec<CountedTransition>,
    pub state_combination_support: Vec<CountedStateCombination>,
}

#[derive(Clone, Debug, Default)]
pub struct SemanticNoveltyCatalog {
    observed_episodes: u64,
    transition_support: BTreeMap<StateTransitionFact, u64>,
    state_combination_support: BTreeMap<SemanticStateCombination, u64>,
}

#[derive(Debug)]
pub struct SemanticNoveltyCatalogError(String);

impl fmt::Display for SemanticNoveltyCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for SemanticNoveltyCatalogError {}

impl SemanticNoveltyCatalog {
    pub fn observed_episodes(&self) -> u64 {
        self.observed_episodes
    }

    pub fn assess(
        &self,
        descriptor: &SemanticNoveltyDescriptor,
        config: SemanticNoveltyCatalogConfig,
    ) -> Result<SemanticNoveltyAssessment, SemanticNoveltyCatalogError> {
        validate_config(config)?;
        let first_seen_transitions = descriptor
            .state_transitions
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter(|transition| !self.transition_support.contains_key(transition))
            .collect::<Vec<_>>();
        let rare_state_combinations = descriptor
            .state_combinations
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter_map(|combination| {
                let prior_supporting_episodes = self
                    .state_combination_support
                    .get(&combination)
                    .copied()
                    .unwrap_or(0);
                (prior_supporting_episodes <= config.rare_support_episode_ceiling).then_some(
                    RareStateCombinationReason {
                        combination,
                        prior_supporting_episodes,
                    },
                )
            })
            .collect::<Vec<_>>();
        Ok(SemanticNoveltyAssessment {
            schema: SEMANTIC_NOVELTY_ASSESSMENT_SCHEMA,
            descriptor_identity: descriptor.identity(),
            catalog_observed_episodes_before: self.observed_episodes,
            rare_support_episode_ceiling: config.rare_support_episode_ceiling,
            semantic_novel: !first_seen_transitions.is_empty()
                || !rare_state_combinations.is_empty(),
            first_seen_transitions,
            rare_state_combinations,
            spatial_distance_used: false,
        })
    }

    pub fn record(
        &mut self,
        descriptor: &SemanticNoveltyDescriptor,
    ) -> Result<(), SemanticNoveltyCatalogError> {
        let transitions = descriptor
            .state_transitions
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let combinations = descriptor
            .state_combinations
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let new_transition_count = transitions
            .iter()
            .filter(|fact| !self.transition_support.contains_key(*fact))
            .count();
        let new_combination_count = combinations
            .iter()
            .filter(|fact| !self.state_combination_support.contains_key(*fact))
            .count();
        if self.transition_support.len() + new_transition_count > MAX_TRACKED_SEMANTIC_FACTS {
            return Err(SemanticNoveltyCatalogError(format!(
                "semantic transition catalog exceeds {MAX_TRACKED_SEMANTIC_FACTS} facts"
            )));
        }
        if self.state_combination_support.len() + new_combination_count > MAX_TRACKED_SEMANTIC_FACTS
        {
            return Err(SemanticNoveltyCatalogError(format!(
                "semantic state-combination catalog exceeds {MAX_TRACKED_SEMANTIC_FACTS} facts"
            )));
        }
        for transition in transitions {
            let support = self.transition_support.entry(transition).or_default();
            *support = support.saturating_add(1);
        }
        for combination in combinations {
            let support = self
                .state_combination_support
                .entry(combination)
                .or_default();
            *support = support.saturating_add(1);
        }
        self.observed_episodes = self.observed_episodes.saturating_add(1);
        Ok(())
    }

    pub fn assess_and_record(
        &mut self,
        descriptor: &SemanticNoveltyDescriptor,
        config: SemanticNoveltyCatalogConfig,
    ) -> Result<SemanticNoveltyAssessment, SemanticNoveltyCatalogError> {
        let assessment = self.assess(descriptor, config)?;
        self.record(descriptor)?;
        Ok(assessment)
    }

    pub fn snapshot(&self) -> SemanticNoveltyCatalogSnapshot {
        SemanticNoveltyCatalogSnapshot {
            schema: SEMANTIC_NOVELTY_CATALOG_SCHEMA,
            observed_episodes: self.observed_episodes,
            transition_support: self
                .transition_support
                .iter()
                .map(|(transition, supporting_episodes)| CountedTransition {
                    transition: transition.clone(),
                    supporting_episodes: *supporting_episodes,
                })
                .collect(),
            state_combination_support: self
                .state_combination_support
                .iter()
                .map(
                    |(combination, supporting_episodes)| CountedStateCombination {
                        combination: combination.clone(),
                        supporting_episodes: *supporting_episodes,
                    },
                )
                .collect(),
        }
    }
}

fn validate_config(
    config: SemanticNoveltyCatalogConfig,
) -> Result<(), SemanticNoveltyCatalogError> {
    if config.rare_support_episode_ceiling > MAX_RARE_SUPPORT_EPISODES {
        return Err(SemanticNoveltyCatalogError(format!(
            "rare support ceiling exceeds {MAX_RARE_SUPPORT_EPISODES} episodes"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tape::TapeBoot;
    use crate::trace::{DecodedTrace, TraceRecord};

    fn descriptor(procedures: &[u16], position_x: f32) -> SemanticNoveltyDescriptor {
        let records = procedures
            .iter()
            .map(|procedure| TraceRecord {
                stage_name: "F_SP104".into(),
                room: 1,
                player_session_process_id: Some(1),
                player_proc_id: Some(*procedure),
                position: [position_x, 0.0, 0.0],
                ..TraceRecord::default()
            })
            .collect();
        SemanticNoveltyDescriptor::from_trace(
            &DecodedTrace {
                version: 5,
                boot: TapeBoot::Process,
                tick_rate_numerator: 30,
                tick_rate_denominator: 1,
                requested_channels: 0,
                capacity_exhausted: false,
                retention: None,
                channel_formats: BTreeMap::new(),
                records,
            },
            Vec::new(),
        )
        .unwrap()
    }

    #[test]
    fn first_seen_transition_is_detected_without_spatial_distance() {
        let mut catalog = SemanticNoveltyCatalog::default();
        catalog.record(&descriptor(&[3, 4], 100.0)).unwrap();
        let assessment = catalog
            .assess(
                &descriptor(&[3, 7], 100.0),
                SemanticNoveltyCatalogConfig::default(),
            )
            .unwrap();
        assert_eq!(assessment.first_seen_transitions.len(), 1);
        assert_eq!(
            assessment.first_seen_transitions[0].to.player_procedure,
            Some(7)
        );
        assert!(assessment.semantic_novel);
        assert!(!assessment.spatial_distance_used);
    }

    #[test]
    fn state_combinations_count_support_once_per_episode() {
        let repeated = descriptor(&[3, 3, 3], 100.0);
        let mut catalog = SemanticNoveltyCatalog::default();
        catalog.record(&repeated).unwrap();
        assert_eq!(catalog.observed_episodes(), 1);
        assert_eq!(catalog.snapshot().state_combination_support.len(), 1);
        assert_eq!(
            catalog.snapshot().state_combination_support[0].supporting_episodes,
            1
        );
    }

    #[test]
    fn common_combinations_age_out_of_the_rare_set() {
        let descriptor = descriptor(&[3], 100.0);
        let mut catalog = SemanticNoveltyCatalog::default();
        let config = SemanticNoveltyCatalogConfig {
            rare_support_episode_ceiling: 1,
        };
        assert_eq!(
            catalog
                .assess_and_record(&descriptor, config)
                .unwrap()
                .rare_state_combinations[0]
                .prior_supporting_episodes,
            0
        );
        assert_eq!(
            catalog
                .assess_and_record(&descriptor, config)
                .unwrap()
                .rare_state_combinations[0]
                .prior_supporting_episodes,
            1
        );
        let assessment = catalog.assess(&descriptor, config).unwrap();
        assert!(assessment.rare_state_combinations.is_empty());
        assert!(!assessment.semantic_novel);
    }
}
