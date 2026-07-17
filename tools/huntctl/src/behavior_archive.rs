//! Bounded behavior archive for finite-sample route search.
//!
//! Fastest-first selection alone collapses a population onto one local route.
//! This archive keeps one proved episode per coarse path/procedure descriptor
//! and can retain the entries farthest from the current elites.

use crate::q_search::QEpisode;
use crate::search::LexicographicScore;
use crate::transition_corpus::TransitionCorpus;
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fmt;

pub const MAX_BEHAVIOR_ARCHIVE_ENTRIES: usize = 256;
const POSITION_BIN_WORLD_UNITS: f32 = 256.0;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct BehaviorDescriptor {
    pub terminal_stage: [u8; 8],
    pub terminal_room: i16,
    pub terminal_layer: i16,
    pub terminal_player_procedure: i32,
    pub midpoint_position_bin: [i32; 3],
    pub terminal_position_bin: [i32; 3],
    pub closest_exit_distance_bin: i32,
    pub procedure_signature: u64,
    pub terminal: bool,
}

#[derive(Clone, Debug)]
pub struct ArchivedEpisode {
    pub descriptor: BehaviorDescriptor,
    pub episode: QEpisode,
    pub score: LexicographicScore,
    pub first_seen_generation: u32,
}

#[derive(Clone, Debug, Default)]
pub struct BehaviorArchive {
    entries: BTreeMap<BehaviorDescriptor, ArchivedEpisode>,
}

#[derive(Clone, Debug, Serialize)]
pub struct BehaviorArchiveSummary {
    pub schema: &'static str,
    pub entries: usize,
    pub selected: usize,
    pub selected_candidate_ids: Vec<String>,
    pub selected_descriptors: Vec<BehaviorDescriptor>,
}

#[derive(Debug)]
pub struct BehaviorArchiveError(String);

impl BehaviorArchiveError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for BehaviorArchiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for BehaviorArchiveError {}

impl BehaviorArchive {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn consider(
        &mut self,
        episode: QEpisode,
        score: LexicographicScore,
        generation: u32,
    ) -> Result<(), BehaviorArchiveError> {
        let descriptor = describe_behavior(&episode.corpus)?;
        let candidate_id = episode
            .candidate
            .id()
            .map_err(|error| BehaviorArchiveError::new(error.to_string()))?;
        let replace = self.entries.get(&descriptor).is_none_or(|incumbent| {
            let incumbent_id = incumbent
                .episode
                .candidate
                .id()
                .expect("archive candidates were validated before insertion");
            score > incumbent.score
                || (score == incumbent.score
                    && (episode.candidate.frame_count(), candidate_id.as_str())
                        < (
                            incumbent.episode.candidate.frame_count(),
                            incumbent_id.as_str(),
                        ))
        });
        if replace {
            self.entries.insert(
                descriptor.clone(),
                ArchivedEpisode {
                    descriptor,
                    episode,
                    score,
                    first_seen_generation: generation,
                },
            );
        }
        self.prune();
        Ok(())
    }

    pub fn select_diverse(
        &self,
        excluded_candidate_ids: &HashSet<String>,
        reference_descriptors: &[BehaviorDescriptor],
        budget: usize,
    ) -> Result<Vec<ArchivedEpisode>, BehaviorArchiveError> {
        let mut pool = self
            .entries
            .values()
            .filter_map(|entry| {
                let id = entry.episode.candidate.id().ok()?;
                (!excluded_candidate_ids.contains(&id)).then_some(entry.clone())
            })
            .collect::<Vec<_>>();
        let mut selected = Vec::new();
        while selected.len() < budget && !pool.is_empty() {
            let best_index = pool
                .iter()
                .enumerate()
                .max_by(|(_, left), (_, right)| {
                    let left_novelty =
                        novelty(&left.descriptor, reference_descriptors, selected.as_slice());
                    let right_novelty = novelty(
                        &right.descriptor,
                        reference_descriptors,
                        selected.as_slice(),
                    );
                    left_novelty
                        .cmp(&right_novelty)
                        .then_with(|| left.score.cmp(&right.score))
                        .then_with(|| right.descriptor.cmp(&left.descriptor))
                })
                .map(|(index, _)| index)
                .ok_or_else(|| BehaviorArchiveError::new("behavior archive selection failed"))?;
            selected.push(pool.swap_remove(best_index));
        }
        Ok(selected)
    }

    pub fn summary(
        &self,
        selected: &[ArchivedEpisode],
    ) -> Result<BehaviorArchiveSummary, BehaviorArchiveError> {
        Ok(BehaviorArchiveSummary {
            schema: "dusklight-behavior-archive/v1",
            entries: self.entries.len(),
            selected: selected.len(),
            selected_candidate_ids: selected
                .iter()
                .map(|entry| {
                    entry
                        .episode
                        .candidate
                        .id()
                        .map_err(|error| BehaviorArchiveError::new(error.to_string()))
                })
                .collect::<Result<_, _>>()?,
            selected_descriptors: selected
                .iter()
                .map(|entry| entry.descriptor.clone())
                .collect(),
        })
    }

    fn prune(&mut self) {
        while self.entries.len() > MAX_BEHAVIOR_ARCHIVE_ENTRIES {
            let remove = self
                .entries
                .iter()
                .min_by(|(_, left), (_, right)| {
                    left.score
                        .cmp(&right.score)
                        .then_with(|| right.first_seen_generation.cmp(&left.first_seen_generation))
                })
                .map(|(descriptor, _)| descriptor.clone());
            if let Some(descriptor) = remove {
                self.entries.remove(&descriptor);
            }
        }
    }
}

pub fn describe_behavior(
    corpus: &TransitionCorpus,
) -> Result<BehaviorDescriptor, BehaviorArchiveError> {
    corpus
        .validate()
        .map_err(|error| BehaviorArchiveError::new(error.to_string()))?;
    if corpus.feature_count < 49 || corpus.transitions.is_empty() {
        return Err(BehaviorArchiveError::new(
            "behavior descriptors require a nonempty movement-state corpus",
        ));
    }
    let midpoint = &corpus.transitions[corpus.transitions.len() / 2].next_state;
    let terminal_transition = corpus.transitions.last().expect("nonempty was checked");
    let terminal = &terminal_transition.next_state;
    let mut stage = [0_u8; 8];
    for (index, byte) in stage.iter_mut().enumerate() {
        *byte = (terminal[index] * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    let mut closest_exit = None::<f32>;
    let mut signature = 0xcbf29ce484222325_u64;
    let mut previous_procedure = None;
    for transition in &corpus.transitions {
        for state in [&transition.state, &transition.next_state] {
            if state[41] == 1.0 {
                closest_exit =
                    Some(closest_exit.map_or(state[46], |distance| distance.min(state[46])));
            }
            let procedure = state[16].round() as i32;
            if previous_procedure != Some(procedure) {
                signature ^= procedure as u32 as u64;
                signature = signature.wrapping_mul(0x100000001b3);
                previous_procedure = Some(procedure);
            }
        }
    }
    Ok(BehaviorDescriptor {
        terminal_stage: stage,
        terminal_room: terminal[8].round() as i16,
        terminal_layer: terminal[9].round() as i16,
        terminal_player_procedure: terminal[16].round() as i32,
        midpoint_position_bin: position_bin(midpoint),
        terminal_position_bin: position_bin(terminal),
        closest_exit_distance_bin: closest_exit
            .map_or(-1, |distance| (distance * 8192.0 / 128.0).round() as i32),
        procedure_signature: signature,
        terminal: terminal_transition.terminal,
    })
}

fn position_bin(state: &[f32]) -> [i32; 3] {
    [
        (state[17] * 8192.0 / POSITION_BIN_WORLD_UNITS).round() as i32,
        (state[18] * 8192.0 / POSITION_BIN_WORLD_UNITS).round() as i32,
        (state[19] * 8192.0 / POSITION_BIN_WORLD_UNITS).round() as i32,
    ]
}

fn novelty(
    descriptor: &BehaviorDescriptor,
    references: &[BehaviorDescriptor],
    selected: &[ArchivedEpisode],
) -> u128 {
    references
        .iter()
        .chain(selected.iter().map(|entry| &entry.descriptor))
        .map(|other| descriptor_distance(descriptor, other))
        .min()
        .unwrap_or(u128::MAX)
}

fn descriptor_distance(left: &BehaviorDescriptor, right: &BehaviorDescriptor) -> u128 {
    let mut distance = 0_u128;
    if left.terminal_stage != right.terminal_stage {
        distance += 1_u128 << 96;
    }
    if left.terminal_room != right.terminal_room || left.terminal_layer != right.terminal_layer {
        distance += 1_u128 << 80;
    }
    if left.terminal_player_procedure != right.terminal_player_procedure {
        distance += 1_u128 << 64;
    }
    if left.procedure_signature != right.procedure_signature {
        distance += 1_u128 << 48;
    }
    for (left, right) in left
        .midpoint_position_bin
        .iter()
        .chain(&left.terminal_position_bin)
        .zip(
            right
                .midpoint_position_bin
                .iter()
                .chain(&right.terminal_position_bin),
        )
    {
        distance += u128::from(left.abs_diff(*right)).pow(2);
    }
    distance
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::Digest;
    use crate::offline_rl::{
        canonical_movement_pad_v2, movement_action_schema_digest_v2,
        movement_feature_schema_digest_v1,
    };
    use crate::search::{Candidate, SegmentProfile};
    use crate::tape::{InputFrame, InputTape, RawPadState};
    use crate::transition_corpus::{MacroAction, StateReference, StateReferenceKind, Transition};

    fn episode(x: f32, procedure: f32, frames: usize) -> QEpisode {
        let disconnected = RawPadState {
            connected: false,
            error: -1,
            ..RawPadState::default()
        };
        let tape = InputTape {
            frames: (0..frames)
                .map(|_| InputFrame {
                    owned_ports: 1,
                    pads: [
                        canonical_movement_pad_v2(1).unwrap(),
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
        let transitions = (0..frames)
            .map(|index| {
                let mut state = vec![0.0; 49];
                state[0] = f32::from(b'F') / 255.0;
                state[16] = procedure;
                state[17] = (x * index as f32 / frames as f32) / 8192.0;
                let mut next_state = state.clone();
                next_state[17] = (x * (index + 1) as f32 / frames as f32) / 8192.0;
                Transition {
                    source: StateReference {
                        kind: StateReferenceKind::Boundary,
                        digest: Digest([index as u8 + 1; 32]),
                    },
                    state,
                    action: MacroAction {
                        action_id: 1,
                        macro_kind: 2,
                        parameters: vec![0, 127, 0],
                    },
                    duration_ticks: 1,
                    reward: -1.0,
                    next: StateReference {
                        kind: StateReferenceKind::Boundary,
                        digest: Digest([index as u8 + 2; 32]),
                    },
                    next_state,
                    terminal: index + 1 == frames,
                }
            })
            .collect();
        QEpisode {
            candidate,
            corpus: TransitionCorpus::new(
                movement_feature_schema_digest_v1(),
                movement_action_schema_digest_v2(),
                49,
                transitions,
            )
            .unwrap(),
        }
    }

    fn score(tick: u64) -> LexicographicScore {
        LexicographicScore {
            milestone_depth: 2,
            successes: 1,
            attempts: 1,
            median_first_hit_tick: tick,
            best_first_hit_tick: tick,
        }
    }

    #[test]
    fn archive_keeps_distinct_paths_and_replaces_same_descriptor_with_faster_episode() {
        let mut archive = BehaviorArchive::default();
        archive
            .consider(episode(100.0, 4.0, 8), score(10), 0)
            .unwrap();
        archive
            .consider(episode(110.0, 4.0, 7), score(9), 1)
            .unwrap();
        archive
            .consider(episode(900.0, 7.0, 12), score(20), 1)
            .unwrap();
        assert_eq!(archive.len(), 2);

        let reference = describe_behavior(&episode(100.0, 4.0, 8).corpus).unwrap();
        let selected = archive
            .select_diverse(&HashSet::new(), &[reference], 1)
            .unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].descriptor.terminal_player_procedure, 7);
    }
}
