//! Finite-batch Q-guided proposals for deterministic movement tapes.
//!
//! This module never scores or promotes gameplay. It consumes already proved
//! episode corpora, proposes small state-aware tape interventions, and hands
//! ordinary candidates back to the native evaluator.

use crate::fqi::{FittedQ, FqiConfig, Transition as FqiTransition};
use crate::offline_rl::{
    MOVEMENT_CATEGORICAL_FEATURES_V1, canonical_movement_pad_v2, movement_action_id_v2,
    movement_action_schema_digest_v2, movement_feature_schema_digest_v1,
};
use crate::search::{Ancestry, Candidate};
use crate::transition_corpus::TransitionCorpus;
use serde::Serialize;
use std::collections::{BTreeSet, HashSet};
use std::error::Error;
use std::fmt;

const MAX_PROPOSAL_STATES: usize = 4096;
const EXPLORATION_WEIGHT: f64 = 1.5;

#[derive(Clone, Debug)]
pub struct QEpisode {
    pub candidate: Candidate,
    pub corpus: TransitionCorpus,
}

#[derive(Clone, Copy, Debug)]
pub struct QProposalConfig {
    pub generation: u32,
    pub max_proposals: usize,
    pub iterations: usize,
    pub trees_per_action: usize,
    pub seed: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct QProposalSummary {
    pub schema: &'static str,
    pub training_transitions: usize,
    pub training_actions: usize,
    pub proposal_states: usize,
    pub exploit_interventions: usize,
    pub exploratory_interventions: usize,
    pub proposals: usize,
}

#[derive(Clone, Debug)]
pub struct QProposalBatch {
    pub candidates: Vec<Candidate>,
    pub summary: QProposalSummary,
}

#[derive(Debug)]
pub struct QSearchError(String);

impl QSearchError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for QSearchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for QSearchError {}

#[derive(Clone, Copy, Debug)]
enum ProposalKind {
    Exploit,
    Explore,
}

#[derive(Clone, Copy, Debug)]
struct Intervention {
    episode: usize,
    frame: usize,
    action: u32,
    score: f64,
    kind: ProposalKind,
}

pub fn propose_q_candidates(
    training_corpora: &[TransitionCorpus],
    episodes: &[QEpisode],
    config: QProposalConfig,
) -> Result<QProposalBatch, QSearchError> {
    if training_corpora.is_empty() || episodes.is_empty() || config.max_proposals == 0 {
        return Err(QSearchError::new(
            "Q proposals require training corpora, aligned parent episodes, and a nonzero budget",
        ));
    }
    let feature_schema = movement_feature_schema_digest_v1();
    let action_schema = movement_action_schema_digest_v2();
    let mut transitions = Vec::new();
    let mut actions = BTreeSet::new();
    for corpus in training_corpora {
        corpus
            .validate()
            .map_err(|error| QSearchError::new(error.to_string()))?;
        if corpus.feature_schema != feature_schema
            || corpus.action_schema != action_schema
            || corpus.feature_count == 0
        {
            return Err(QSearchError::new(
                "Q proposal corpus does not use the authenticated movement schemas",
            ));
        }
        for transition in &corpus.transitions {
            actions.insert(transition.action.action_id);
            transitions.push(FqiTransition {
                state: transition.state.clone(),
                action: transition.action.action_id,
                duration: transition.duration_ticks,
                reward: transition.reward,
                next_state: transition.next_state.clone(),
                terminal: transition.terminal,
            });
        }
    }
    if actions.len() < 2 {
        return Err(QSearchError::new(
            "Q proposals require observed support for at least two action classes",
        ));
    }
    let actions: Vec<_> = actions.into_iter().collect();
    let feature_width = training_corpora[0].feature_count as usize;
    let model = FittedQ::fit(
        feature_width,
        &actions,
        &transitions,
        &FqiConfig {
            iterations: config.iterations,
            trees_per_action: config.trees_per_action,
            max_tree_depth: 8,
            seed: config.seed,
            categorical_features: MOVEMENT_CATEGORICAL_FEATURES_V1.to_vec(),
            ..FqiConfig::default()
        },
    )
    .map_err(|error| QSearchError::new(error.to_string()))?;

    let aligned = episodes
        .iter()
        .map(validate_episode_alignment)
        .collect::<Result<Vec<_>, _>>()?;
    let total_states: usize = aligned
        .iter()
        .map(|episode| episode.corpus.transitions.len())
        .sum();
    let stride = total_states.div_ceil(MAX_PROPOSAL_STATES).max(1);
    let mut exploit = Vec::new();
    let mut explore = Vec::new();
    let mut considered = 0;
    let mut ordinal = 0;
    for (episode_index, episode) in aligned.iter().enumerate() {
        for (frame, transition) in episode.corpus.transitions.iter().enumerate() {
            let take = ordinal % stride == 0;
            ordinal += 1;
            if !take {
                continue;
            }
            considered += 1;
            let current = model
                .estimate(&transition.state, transition.action.action_id)
                .map_err(|error| QSearchError::new(error.to_string()))?;
            let alternatives: Vec<_> = model
                .rank_actions(&transition.state)
                .map_err(|error| QSearchError::new(error.to_string()))?
                .into_iter()
                .filter(|estimate| estimate.action != transition.action.action_id)
                .collect();
            if let Some(best) = alternatives.first() {
                exploit.push(Intervention {
                    episode: episode_index,
                    frame,
                    action: best.action,
                    score: best.mean - current.mean,
                    kind: ProposalKind::Exploit,
                });
            }
            if let Some(best) = alternatives.into_iter().max_by(|left, right| {
                exploration_score(*left, current.mean)
                    .total_cmp(&exploration_score(*right, current.mean))
            }) {
                explore.push(Intervention {
                    episode: episode_index,
                    frame,
                    action: best.action,
                    score: exploration_score(best, current.mean),
                    kind: ProposalKind::Explore,
                });
            }
        }
    }
    sort_interventions(&mut exploit);
    sort_interventions(&mut explore);

    let mut candidates = Vec::new();
    let mut ids = episodes
        .iter()
        .map(|episode| {
            episode
                .candidate
                .id()
                .map_err(|error| QSearchError::new(error.to_string()))
        })
        .collect::<Result<HashSet<_>, _>>()?;
    let mut exploit_index = 0;
    let mut explore_index = 0;
    let mut choose_exploit = true;
    let maximum_attempts = exploit.len().saturating_add(explore.len());
    for attempt in 0..maximum_attempts {
        if candidates.len() >= config.max_proposals {
            break;
        }
        let selected = if choose_exploit {
            let value = exploit.get(exploit_index).copied();
            exploit_index += usize::from(value.is_some());
            value.or_else(|| {
                let value = explore.get(explore_index).copied();
                explore_index += usize::from(value.is_some());
                value
            })
        } else {
            let value = explore.get(explore_index).copied();
            explore_index += usize::from(value.is_some());
            value.or_else(|| {
                let value = exploit.get(exploit_index).copied();
                exploit_index += usize::from(value.is_some());
                value
            })
        };
        choose_exploit = !choose_exploit;
        let Some(intervention) = selected else {
            break;
        };
        let width = [1_usize, 2, 4][attempt % 3];
        let mut tape = aligned[intervention.episode].tape.clone();
        let pad = canonical_movement_pad_v2(intervention.action)
            .ok_or_else(|| QSearchError::new("fitted Q returned an unknown movement action"))?;
        let end = intervention
            .frame
            .saturating_add(width)
            .min(tape.frames.len());
        for frame in &mut tape.frames[intervention.frame..end] {
            frame.pads[0] = pad;
        }
        let parent = &episodes[intervention.episode].candidate;
        let mut candidate = Candidate::from_absolute_tape(parent.segment, &tape)
            .map_err(|error| QSearchError::new(error.to_string()))?;
        candidate.ancestry = Ancestry {
            generation: config.generation,
            parent_id: Some(
                parent
                    .id()
                    .map_err(|error| QSearchError::new(error.to_string()))?,
            ),
            mutation: Some(format!(
                "q_{:?}[{}..{}]=action{} score={:.6}",
                intervention.kind, intervention.frame, end, intervention.action, intervention.score
            )),
        };
        let id = candidate
            .id()
            .map_err(|error| QSearchError::new(error.to_string()))?;
        if ids.insert(id) {
            candidates.push(candidate);
        }
    }

    Ok(QProposalBatch {
        summary: QProposalSummary {
            schema: "dusklight-q-proposals/v1",
            training_transitions: transitions.len(),
            training_actions: actions.len(),
            proposal_states: considered,
            exploit_interventions: exploit.len(),
            exploratory_interventions: explore.len(),
            proposals: candidates.len(),
        },
        candidates,
    })
}

struct AlignedEpisode<'a> {
    corpus: &'a TransitionCorpus,
    tape: crate::tape::InputTape,
}

fn validate_episode_alignment(episode: &QEpisode) -> Result<AlignedEpisode<'_>, QSearchError> {
    episode
        .corpus
        .validate()
        .map_err(|error| QSearchError::new(error.to_string()))?;
    let tape = episode
        .candidate
        .compile()
        .map_err(|error| QSearchError::new(error.to_string()))?;
    if episode.corpus.transitions.is_empty() || episode.corpus.transitions.len() > tape.frames.len()
    {
        return Err(QSearchError::new(
            "Q parent episode is empty or longer than its candidate suffix",
        ));
    }
    for (index, transition) in episode.corpus.transitions.iter().enumerate() {
        let pad = tape.frames[index].pads[0];
        let expected = [
            i16::from(pad.stick_x),
            i16::from(pad.stick_y),
            pad.buttons as i16,
        ];
        if transition.duration_ticks != 1
            || transition.action.parameters != expected
            || movement_action_id_v2(pad) != Some(transition.action.action_id)
        {
            return Err(QSearchError::new(format!(
                "Q parent episode action {index} is not aligned with its candidate tape"
            )));
        }
    }
    Ok(AlignedEpisode {
        corpus: &episode.corpus,
        tape,
    })
}

fn exploration_score(estimate: crate::fqi::QEstimate, current_mean: f64) -> f64 {
    estimate.mean - current_mean + EXPLORATION_WEIGHT * estimate.variance.max(0.0).sqrt()
}

fn sort_interventions(interventions: &mut [Intervention]) {
    interventions.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.episode.cmp(&right.episode))
            .then_with(|| left.frame.cmp(&right.frame))
            .then_with(|| left.action.cmp(&right.action))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::Digest;
    use crate::search::SegmentProfile;
    use crate::tape::{InputFrame, InputTape, RawPadState};
    use crate::transition_corpus::{MacroAction, StateReference, StateReferenceKind, Transition};

    fn corpus_for(candidate: &Candidate) -> TransitionCorpus {
        let tape = candidate.compile().unwrap();
        let transitions = tape
            .frames
            .iter()
            .enumerate()
            .map(|(index, frame)| {
                let action_id = movement_action_id_v2(frame.pads[0]).unwrap();
                let mut state = vec![0.0; 49];
                state[17] = index as f32;
                let mut next_state = state.clone();
                next_state[17] += 1.0;
                Transition {
                    source: StateReference {
                        kind: StateReferenceKind::Boundary,
                        digest: Digest([index as u8 + 1; 32]),
                    },
                    state,
                    action: MacroAction {
                        action_id,
                        macro_kind: 2,
                        parameters: vec![
                            i16::from(frame.pads[0].stick_x),
                            i16::from(frame.pads[0].stick_y),
                            frame.pads[0].buttons as i16,
                        ],
                    },
                    duration_ticks: 1,
                    reward: -1.0,
                    next: StateReference {
                        kind: StateReferenceKind::Boundary,
                        digest: Digest([index as u8 + 2; 32]),
                    },
                    next_state,
                    terminal: index + 1 == tape.frames.len(),
                }
            })
            .collect();
        TransitionCorpus::new(
            movement_feature_schema_digest_v1(),
            movement_action_schema_digest_v2(),
            49,
            transitions,
        )
        .unwrap()
    }

    #[test]
    fn fitted_q_proposals_are_deterministic_aligned_ordinary_candidates() {
        let disconnected = RawPadState {
            connected: false,
            error: -1,
            ..RawPadState::default()
        };
        let tape = InputTape {
            frames: (0..8)
                .map(|index| InputFrame {
                    owned_ports: 1,
                    pads: [
                        canonical_movement_pad_v2(if index % 2 == 0 { 1 } else { 18 }).unwrap(),
                        disconnected,
                        disconnected,
                        disconnected,
                    ],
                    ..InputFrame::default()
                })
                .collect(),
            ..InputTape::default()
        };
        let candidate =
            Candidate::from_absolute_tape(SegmentProfile::Fsp103ToFsp104, &tape).unwrap();
        let corpus = corpus_for(&candidate);
        let episodes = [QEpisode {
            candidate: candidate.clone(),
            corpus: corpus.clone(),
        }];
        let config = QProposalConfig {
            generation: 1,
            max_proposals: 4,
            iterations: 4,
            trees_per_action: 3,
            seed: 7,
        };
        let first = propose_q_candidates(std::slice::from_ref(&corpus), &episodes, config).unwrap();
        let second = propose_q_candidates(&[corpus], &episodes, config).unwrap();
        assert!(!first.candidates.is_empty());
        assert_eq!(first.summary.proposals, first.candidates.len());
        assert_eq!(
            first
                .candidates
                .iter()
                .map(Candidate::id)
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
            second
                .candidates
                .iter()
                .map(Candidate::id)
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        );
        assert!(first.candidates.iter().all(|proposal| {
            proposal.validate().is_ok()
                && proposal.ancestry.parent_id.as_deref() == Some(candidate.id().unwrap().as_str())
                && proposal
                    .ancestry
                    .mutation
                    .as_deref()
                    .is_some_and(|mutation| mutation.starts_with("q_"))
        }));
    }
}
