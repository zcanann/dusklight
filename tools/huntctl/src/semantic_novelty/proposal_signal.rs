//! Bounded semantic novelty signal for proposal ordering only.

use super::catalog::SemanticNoveltyAssessment;
use serde::Serialize;
use std::error::Error;
use std::fmt;

pub const SEMANTIC_NOVELTY_PROPOSAL_SIGNAL_SCHEMA: &str =
    "dusklight-semantic-novelty-proposal-signal/v1";
pub const MAX_NOVELTY_SIGNAL_WEIGHT: u64 = 10_000;
pub const MAX_NOVELTY_PROPOSAL_SIGNAL: u64 = 1_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SemanticNoveltyProposalSignalConfig {
    pub first_seen_transition_weight: u64,
    pub rare_combination_weight: u64,
    pub maximum_signal: u64,
}

impl Default for SemanticNoveltyProposalSignalConfig {
    fn default() -> Self {
        Self {
            first_seen_transition_weight: 100,
            rare_combination_weight: 10,
            maximum_signal: 10_000,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SemanticNoveltyProposalSignal {
    pub schema: &'static str,
    pub descriptor_identity: String,
    pub first_seen_transition_component: u64,
    pub rare_state_combination_component: u64,
    pub total: u64,
    pub raw_reason: SemanticNoveltyAssessment,
    proposal_ordering_authority: bool,
    native_leaderboard_authority: bool,
    proof_authority: bool,
    promotion_authority: bool,
}

#[derive(Debug)]
pub struct SemanticNoveltyProposalSignalError(String);

impl fmt::Display for SemanticNoveltyProposalSignalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for SemanticNoveltyProposalSignalError {}

impl SemanticNoveltyProposalSignal {
    pub fn from_assessment(
        assessment: SemanticNoveltyAssessment,
        config: SemanticNoveltyProposalSignalConfig,
    ) -> Result<Self, SemanticNoveltyProposalSignalError> {
        validate_config(config)?;
        let first_seen_transition_component = (assessment.first_seen_transitions.len() as u64)
            .saturating_mul(config.first_seen_transition_weight);
        let rare_state_combination_component = assessment
            .rare_state_combinations
            .iter()
            .map(|reason| {
                assessment
                    .rare_support_episode_ceiling
                    .saturating_add(1)
                    .saturating_sub(reason.prior_supporting_episodes)
                    .saturating_mul(config.rare_combination_weight)
            })
            .fold(0_u64, u64::saturating_add);
        let total = first_seen_transition_component
            .saturating_add(rare_state_combination_component)
            .min(config.maximum_signal);
        Ok(Self {
            schema: SEMANTIC_NOVELTY_PROPOSAL_SIGNAL_SCHEMA,
            descriptor_identity: assessment.descriptor_identity.clone(),
            first_seen_transition_component,
            rare_state_combination_component,
            total,
            raw_reason: assessment,
            proposal_ordering_authority: true,
            native_leaderboard_authority: false,
            proof_authority: false,
            promotion_authority: false,
        })
    }

    pub fn proposal_ordering_score(&self) -> u64 {
        self.total
    }

    pub fn may_order_proposals(&self) -> bool {
        self.proposal_ordering_authority
    }

    pub fn has_native_leaderboard_authority(&self) -> bool {
        self.native_leaderboard_authority
    }

    pub fn has_proof_authority(&self) -> bool {
        self.proof_authority
    }

    pub fn has_promotion_authority(&self) -> bool {
        self.promotion_authority
    }
}

fn validate_config(
    config: SemanticNoveltyProposalSignalConfig,
) -> Result<(), SemanticNoveltyProposalSignalError> {
    if config.first_seen_transition_weight > MAX_NOVELTY_SIGNAL_WEIGHT
        || config.rare_combination_weight > MAX_NOVELTY_SIGNAL_WEIGHT
    {
        return Err(SemanticNoveltyProposalSignalError(format!(
            "semantic novelty weight exceeds {MAX_NOVELTY_SIGNAL_WEIGHT}"
        )));
    }
    if config.maximum_signal == 0 || config.maximum_signal > MAX_NOVELTY_PROPOSAL_SIGNAL {
        return Err(SemanticNoveltyProposalSignalError(format!(
            "semantic novelty maximum must be between 1 and {MAX_NOVELTY_PROPOSAL_SIGNAL}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic_novelty::catalog::{
        RareStateCombinationReason, SEMANTIC_NOVELTY_ASSESSMENT_SCHEMA,
    };
    use crate::semantic_novelty::{
        EventFact, FlagState, SemanticState, SemanticStateCombination, StateTransitionFact,
    };

    fn assessment() -> SemanticNoveltyAssessment {
        let state = SemanticState {
            stage: "F_SP104".into(),
            room: 1,
            layer: 0,
            point: 0,
            player_procedure: Some(3),
        };
        SemanticNoveltyAssessment {
            schema: SEMANTIC_NOVELTY_ASSESSMENT_SCHEMA,
            descriptor_identity: "ab".repeat(32),
            catalog_observed_episodes_before: 10,
            rare_support_episode_ceiling: 3,
            first_seen_transitions: vec![StateTransitionFact {
                from: state.clone(),
                to: SemanticState {
                    player_procedure: Some(7),
                    ..state.clone()
                },
            }],
            rare_state_combinations: vec![RareStateCombinationReason {
                combination: SemanticStateCombination {
                    state,
                    event: EventFact {
                        event_id: 4,
                        mode: 0,
                        status: 1,
                        map_tool_id: 0,
                        name_hash: None,
                    },
                    contact: None,
                    actor_relationships: None,
                    flags: FlagState {
                        record_flags: 0,
                        player_mode_flags: None,
                        event_status: 1,
                        event_mode: 0,
                        goal_configured: None,
                        goal_reached: None,
                    },
                },
                prior_supporting_episodes: 1,
            }],
            semantic_novel: true,
            spatial_distance_used: false,
        }
    }

    #[test]
    fn reward_is_bounded_and_retains_the_raw_semantic_reason() {
        let raw = assessment();
        let signal = SemanticNoveltyProposalSignal::from_assessment(
            raw.clone(),
            SemanticNoveltyProposalSignalConfig {
                first_seen_transition_weight: 100,
                rare_combination_weight: 10,
                maximum_signal: 120,
            },
        )
        .unwrap();
        assert_eq!(signal.first_seen_transition_component, 100);
        assert_eq!(signal.rare_state_combination_component, 30);
        assert_eq!(signal.proposal_ordering_score(), 120);
        assert_eq!(signal.raw_reason, raw);
    }

    #[test]
    fn proposal_signal_has_no_evaluation_or_promotion_authority() {
        let signal = SemanticNoveltyProposalSignal::from_assessment(
            assessment(),
            SemanticNoveltyProposalSignalConfig::default(),
        )
        .unwrap();
        assert!(signal.may_order_proposals());
        assert!(!signal.has_native_leaderboard_authority());
        assert!(!signal.has_proof_authority());
        assert!(!signal.has_promotion_authority());
        let json = serde_json::to_value(&signal).unwrap();
        assert_eq!(json["proposal_ordering_authority"], true);
        assert_eq!(json["native_leaderboard_authority"], false);
        assert_eq!(json["promotion_authority"], false);
        assert_eq!(
            json["raw_reason"]["first_seen_transitions"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
}
