//! Seeded epsilon-greedy choice over the existing live option-Q ranking.

use crate::artifact::Digest;
use crate::live_tactic_catalog::LiveTacticRanking;
use crate::option_values::OptionActionDescriptor;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const TACTIC_EXPLORATION_SCHEMA_V1: &str = "dusklight-tactic-exploration/v1";
pub const EPSILON_SCALE: u32 = 1_000_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticExplorationConfig {
    pub seed: u64,
    pub epsilon_per_million: u32,
}

impl Default for TacticExplorationConfig {
    fn default() -> Self {
        Self {
            seed: 0,
            epsilon_per_million: 100_000,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TacticSelectionReason {
    Greedy,
    Epsilon,
    UnsupportedBootstrap,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SelectedTactic {
    pub schema: String,
    pub learner_snapshot_sha256: Digest,
    pub decision_index: u64,
    pub descriptor: OptionActionDescriptor,
    pub reason: TacticSelectionReason,
    pub exploration_draw: u32,
}

pub fn choose_tactic(
    ranking: &LiveTacticRanking,
    decision_index: u64,
    config: TacticExplorationConfig,
) -> Result<SelectedTactic, TacticExplorationError> {
    if config.epsilon_per_million > EPSILON_SCALE
        || ranking.learner_snapshot_sha256 == Digest::ZERO
        || ranking.choices.is_empty()
    {
        return Err(TacticExplorationError::InvalidInput);
    }
    let available = ranking
        .choices
        .iter()
        .map(|entry| &entry.descriptor)
        .collect::<Vec<_>>();
    let mut reported = ranking
        .values
        .ranked
        .iter()
        .map(|entry| &entry.descriptor)
        .chain(ranking.values.unsupported.iter())
        .collect::<Vec<_>>();
    reported.sort_by(|left, right| left.option_id.cmp(&right.option_id));
    if reported.len() != available.len()
        || available
            .iter()
            .any(|descriptor| reported.iter().filter(|value| *value == descriptor).count() != 1)
    {
        return Err(TacticExplorationError::DetachedRanking);
    }

    let draw = deterministic_draw(
        config.seed,
        decision_index,
        ranking.learner_snapshot_sha256,
        0,
    );
    let exploration_draw = (draw % u64::from(EPSILON_SCALE)) as u32;
    let bootstrap_unsupported = ranking.values.ranked.is_empty()
        || (ranking.values.ranked[0].mean_q <= 0.0
            && !ranking.values.unsupported.is_empty()
            && exploration_draw >= config.epsilon_per_million);
    let (descriptor, reason) = if bootstrap_unsupported {
        let index = deterministic_index(
            config.seed,
            decision_index,
            ranking.learner_snapshot_sha256,
            ranking.values.unsupported.len(),
        );
        (
            ranking.values.unsupported[index].clone(),
            TacticSelectionReason::UnsupportedBootstrap,
        )
    } else if exploration_draw < config.epsilon_per_million {
        // Finite tactic catalogs should spend exploratory decisions on choices
        // for which the learner has no transition evidence before resampling a
        // known action. This is still epsilon-greedy—the greedy branch is
        // unchanged—but makes basic coverage prompt and demonstrable.
        let exploratory = if ranking.values.unsupported.is_empty() {
            available
        } else {
            ranking.values.unsupported.iter().collect::<Vec<_>>()
        };
        let index = deterministic_index(
            config.seed,
            decision_index,
            ranking.learner_snapshot_sha256,
            exploratory.len(),
        );
        (exploratory[index].clone(), TacticSelectionReason::Epsilon)
    } else {
        (
            ranking.values.ranked[0].descriptor.clone(),
            TacticSelectionReason::Greedy,
        )
    };
    Ok(SelectedTactic {
        schema: TACTIC_EXPLORATION_SCHEMA_V1.into(),
        learner_snapshot_sha256: ranking.learner_snapshot_sha256,
        decision_index,
        descriptor,
        reason,
        exploration_draw,
    })
}

fn deterministic_index(seed: u64, decision_index: u64, state: Digest, len: usize) -> usize {
    (deterministic_draw(seed, decision_index, state, 1) % len as u64) as usize
}

fn deterministic_draw(seed: u64, decision_index: u64, state: Digest, lane: u8) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(TACTIC_EXPLORATION_SCHEMA_V1.as_bytes());
    hasher.update(seed.to_le_bytes());
    hasher.update(decision_index.to_le_bytes());
    hasher.update(state.0);
    hasher.update([lane]);
    let digest = hasher.finalize();
    u64::from_le_bytes(digest[..8].try_into().unwrap())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TacticExplorationError {
    InvalidInput,
    DetachedRanking,
}

impl fmt::Display for TacticExplorationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput => formatter.write_str("tactic exploration input is invalid"),
            Self::DetachedRanking => {
                formatter.write_str("tactic ranking is detached from its live catalog")
            }
        }
    }
}

impl Error for TacticExplorationError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learner_state::LearnerActionMaskEntry;
    use crate::option_values::{AvailableOptionRanking, RankedOption};
    use crate::tactic_asset::TacticDurationBounds;
    use crate::tactic_blueprint::ConcreteTacticChoiceKind;
    use dusklight_control::option_execution::OptionType;
    use std::collections::BTreeMap;

    fn descriptor(id: &str, option_type: OptionType) -> OptionActionDescriptor {
        OptionActionDescriptor {
            option_id: id.into(),
            option_type,
            parameters: BTreeMap::new(),
        }
    }

    fn choice(descriptor: OptionActionDescriptor) -> LearnerActionMaskEntry {
        LearnerActionMaskEntry {
            choice_id: descriptor.option_id.clone(),
            kind: ConcreteTacticChoiceKind::CatalogEntry,
            descriptor,
            duration: TacticDurationBounds {
                minimum_ticks: 1,
                maximum_ticks: 1,
            },
            applicable: true,
        }
    }

    #[test]
    fn zero_epsilon_is_greedy_and_seeded_exploration_is_reproducible() {
        let wait = descriptor("wait", OptionType::Neutral);
        let roll = descriptor("roll", OptionType::Roll);
        let ranking = LiveTacticRanking {
            learner_snapshot_sha256: Digest([1; 32]),
            action_universe_sha256: Digest([2; 32]),
            choices: vec![choice(wait.clone()), choice(roll.clone())],
            values: AvailableOptionRanking {
                ranked: vec![
                    RankedOption {
                        action_id: 1,
                        descriptor: roll.clone(),
                        mean_q: 5.0,
                        ensemble_variance: 0.0,
                    },
                    RankedOption {
                        action_id: 0,
                        descriptor: wait,
                        mean_q: 1.0,
                        ensemble_variance: 0.0,
                    },
                ],
                unsupported: Vec::new(),
            },
        };
        let greedy = choose_tactic(
            &ranking,
            7,
            TacticExplorationConfig {
                seed: 99,
                epsilon_per_million: 0,
            },
        )
        .unwrap();
        assert_eq!(greedy.descriptor, roll);
        assert_eq!(greedy.reason, TacticSelectionReason::Greedy);

        let config = TacticExplorationConfig {
            seed: 99,
            epsilon_per_million: EPSILON_SCALE,
        };
        assert_eq!(
            choose_tactic(&ranking, 7, config).unwrap(),
            choose_tactic(&ranking, 7, config).unwrap()
        );
        assert_eq!(
            choose_tactic(&ranking, 7, config).unwrap().reason,
            TacticSelectionReason::Epsilon
        );
    }

    #[test]
    fn an_untrained_catalog_bootstraps_without_fabricating_q() {
        let wait = descriptor("wait", OptionType::Neutral);
        let ranking = LiveTacticRanking {
            learner_snapshot_sha256: Digest([1; 32]),
            action_universe_sha256: Digest([2; 32]),
            choices: vec![choice(wait.clone())],
            values: AvailableOptionRanking {
                ranked: Vec::new(),
                unsupported: vec![wait.clone()],
            },
        };
        let selected = choose_tactic(&ranking, 0, TacticExplorationConfig::default()).unwrap();
        assert_eq!(selected.descriptor, wait);
        assert_eq!(selected.reason, TacticSelectionReason::UnsupportedBootstrap);
    }

    #[test]
    fn nonpositive_known_values_bootstrap_an_unsupported_tactic() {
        let wait = descriptor("wait", OptionType::Neutral);
        let move_forward = descriptor("move", OptionType::Move);
        let ranking = LiveTacticRanking {
            learner_snapshot_sha256: Digest([1; 32]),
            action_universe_sha256: Digest([2; 32]),
            choices: vec![choice(move_forward.clone()), choice(wait.clone())],
            values: AvailableOptionRanking {
                ranked: vec![RankedOption {
                    action_id: 0,
                    descriptor: wait,
                    mean_q: -0.01,
                    ensemble_variance: 0.0,
                }],
                unsupported: vec![move_forward.clone()],
            },
        };
        let selected = choose_tactic(
            &ranking,
            0,
            TacticExplorationConfig {
                seed: 7,
                epsilon_per_million: 0,
            },
        )
        .unwrap();
        assert_eq!(selected.descriptor, move_forward);
        assert_eq!(selected.reason, TacticSelectionReason::UnsupportedBootstrap);
    }

    #[test]
    fn epsilon_exploration_prioritizes_untried_tactics() {
        let known = descriptor("known", OptionType::Neutral);
        let fresh = descriptor("fresh", OptionType::Move);
        let ranking = LiveTacticRanking {
            learner_snapshot_sha256: Digest([3; 32]),
            action_universe_sha256: Digest([4; 32]),
            choices: vec![choice(fresh.clone()), choice(known.clone())],
            values: AvailableOptionRanking {
                ranked: vec![RankedOption {
                    action_id: 1,
                    descriptor: known,
                    mean_q: 5.0,
                    ensemble_variance: 0.0,
                }],
                unsupported: vec![fresh.clone()],
            },
        };
        for seed in 0..16 {
            let selected = choose_tactic(
                &ranking,
                0,
                TacticExplorationConfig {
                    seed,
                    epsilon_per_million: EPSILON_SCALE,
                },
            )
            .unwrap();
            assert_eq!(selected.descriptor, fresh);
            assert_eq!(selected.reason, TacticSelectionReason::Epsilon);
        }
    }
}
