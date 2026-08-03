//! Deterministic deletion-only refinement of an authenticated scratch route.

use dusklight_automation_contracts::artifact::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;

const MAX_INCUMBENT_ACTIONS: usize = 100_000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScratchDeletionSearch {
    incumbent_action_sequence: Vec<usize>,
    incumbent_sha256: Digest,
    attempted_candidate_sha256s: BTreeSet<Digest>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ScratchDeletionCandidate {
    pub removed_action_index: usize,
    pub action_sequence: Vec<usize>,
    pub action_sequence_sha256: Digest,
}

impl ScratchDeletionSearch {
    pub fn new(incumbent_action_sequence: Vec<usize>) -> Result<Self, String> {
        if incumbent_action_sequence.is_empty()
            || incumbent_action_sequence.len() > MAX_INCUMBENT_ACTIONS
        {
            return Err("scratch deletion incumbent length is invalid".into());
        }
        let incumbent_sha256 = action_sequence_sha256(&incumbent_action_sequence)?;
        Ok(Self {
            incumbent_action_sequence,
            incumbent_sha256,
            attempted_candidate_sha256s: BTreeSet::new(),
        })
    }

    pub fn validate(&self, seed: u64, action_count: usize) -> Result<(), String> {
        if self.incumbent_action_sequence.is_empty()
            || self.incumbent_action_sequence.len() > MAX_INCUMBENT_ACTIONS
            || self
                .incumbent_action_sequence
                .iter()
                .any(|action| *action >= action_count)
            || self.incumbent_sha256 != action_sequence_sha256(&self.incumbent_action_sequence)?
        {
            return Err("scratch deletion search is invalid".into());
        }
        let candidate_ids = self
            .candidates(seed)?
            .into_iter()
            .map(|candidate| candidate.action_sequence_sha256)
            .collect::<BTreeSet<_>>();
        if !self.attempted_candidate_sha256s.is_subset(&candidate_ids) {
            return Err("scratch deletion attempts are detached from the incumbent".into());
        }
        Ok(())
    }

    pub fn next_candidate(&self, seed: u64) -> Result<Option<ScratchDeletionCandidate>, String> {
        Ok(self.candidates(seed)?.into_iter().find(|candidate| {
            !self
                .attempted_candidate_sha256s
                .contains(&candidate.action_sequence_sha256)
        }))
    }

    pub fn finish_attempt(
        &mut self,
        seed: u64,
        candidate: &ScratchDeletionCandidate,
        accepted_incumbent: Option<Vec<usize>>,
    ) -> Result<(), String> {
        let expected = self
            .candidates(seed)?
            .into_iter()
            .find(|expected| expected.action_sequence_sha256 == candidate.action_sequence_sha256)
            .ok_or_else(|| {
                "scratch deletion candidate is detached from the incumbent".to_owned()
            })?;
        if expected != *candidate {
            return Err("scratch deletion candidate identity is inconsistent".into());
        }
        if let Some(incumbent) = accepted_incumbent {
            *self = Self::new(incumbent)?;
        } else if !self
            .attempted_candidate_sha256s
            .insert(candidate.action_sequence_sha256)
        {
            return Err("scratch deletion candidate was attempted twice".into());
        }
        Ok(())
    }

    pub fn remaining_candidates(&self, seed: u64) -> Result<usize, String> {
        Ok(self
            .candidates(seed)?
            .into_iter()
            .filter(|candidate| {
                !self
                    .attempted_candidate_sha256s
                    .contains(&candidate.action_sequence_sha256)
            })
            .count())
    }

    fn candidates(&self, seed: u64) -> Result<Vec<ScratchDeletionCandidate>, String> {
        let mut unique_sequences = BTreeSet::new();
        let mut ranked = Vec::new();
        for removed_action_index in 0..self.incumbent_action_sequence.len() {
            let mut action_sequence = self.incumbent_action_sequence.clone();
            action_sequence.remove(removed_action_index);
            if action_sequence.is_empty() || !unique_sequences.insert(action_sequence.clone()) {
                continue;
            }
            let action_sequence_sha256 = action_sequence_sha256(&action_sequence)?;
            let rank = candidate_rank(
                seed,
                self.incumbent_sha256,
                action_sequence_sha256,
                removed_action_index,
            );
            ranked.push((
                rank,
                ScratchDeletionCandidate {
                    removed_action_index,
                    action_sequence,
                    action_sequence_sha256,
                },
            ));
        }
        ranked.sort_by_key(|(rank, candidate)| {
            (
                *rank,
                candidate.action_sequence_sha256,
                candidate.removed_action_index,
            )
        });
        Ok(ranked.into_iter().map(|(_, candidate)| candidate).collect())
    }
}

pub(crate) fn action_sequence_sha256(sequence: &[usize]) -> Result<Digest, String> {
    let bytes = serde_cbor::to_vec(&sequence).map_err(|error| error.to_string())?;
    Ok(Digest(Sha256::digest(bytes).into()))
}

fn candidate_rank(
    seed: u64,
    incumbent_sha256: Digest,
    candidate_sha256: Digest,
    removed_action_index: usize,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight-scratch-deletion/v1");
    hasher.update(seed.to_le_bytes());
    hasher.update(incumbent_sha256.0);
    hasher.update(candidate_sha256.0);
    hasher.update((removed_action_index as u64).to_le_bytes());
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_are_complete_unique_and_deterministic() {
        let search = ScratchDeletionSearch::new(vec![1, 1, 2]).unwrap();
        let first = search.candidates(17).unwrap();
        let second = search.candidates(17).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.len(), 2);
        assert_eq!(
            first
                .iter()
                .map(|candidate| candidate.action_sequence.clone())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([vec![1, 1], vec![1, 2]])
        );
    }

    #[test]
    fn attempts_resume_without_duplicates_and_failures_preserve_the_incumbent() {
        let mut search = ScratchDeletionSearch::new(vec![1, 2, 3]).unwrap();
        let incumbent = search.clone();
        let first = search.next_candidate(23).unwrap().unwrap();
        search.finish_attempt(23, &first, None).unwrap();
        assert_eq!(
            search.incumbent_action_sequence,
            incumbent.incumbent_action_sequence
        );

        let encoded = serde_cbor::to_vec(&search).unwrap();
        let mut resumed: ScratchDeletionSearch = serde_cbor::from_slice(&encoded).unwrap();
        let second = resumed.next_candidate(23).unwrap().unwrap();
        assert_ne!(first.action_sequence_sha256, second.action_sequence_sha256);
        resumed.finish_attempt(23, &second, None).unwrap();
        let third = resumed.next_candidate(23).unwrap().unwrap();
        resumed.finish_attempt(23, &third, None).unwrap();
        assert!(resumed.next_candidate(23).unwrap().is_none());
        assert!(resumed.finish_attempt(23, &first, None).is_err());
    }

    #[test]
    fn accepted_candidate_resets_search_to_the_new_incumbent() {
        let mut search = ScratchDeletionSearch::new(vec![1, 2, 3]).unwrap();
        let candidate = search.next_candidate(29).unwrap().unwrap();
        search
            .finish_attempt(29, &candidate, Some(candidate.action_sequence.clone()))
            .unwrap();
        assert_eq!(search.incumbent_action_sequence, candidate.action_sequence);
        assert!(search.attempted_candidate_sha256s.is_empty());
        assert_eq!(search.remaining_candidates(29).unwrap(), 2);
    }
}
