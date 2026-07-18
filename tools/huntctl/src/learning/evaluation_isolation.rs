//! Generation seals that keep evaluation repetitions out of online training.

use crate::artifact::Digest;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const EVALUATION_GENERATION_SEAL_SCHEMA_V1: &str = "dusklight-evaluation-generation-seal/v1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationAttemptInput {
    pub candidate_id: String,
    pub attempt: u32,
    pub worker_id: String,
    /// Present only for the one collection episode retained from evaluation.
    pub transition_corpus_sha256: Option<Digest>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SealedEvaluationCorpus {
    pub candidate_id: String,
    pub source_attempt: u32,
    pub transition_corpus_sha256: Digest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EvaluationGenerationSeal {
    pub schema: &'static str,
    pub evaluation_generation: u32,
    pub minimum_training_generation: u32,
    pub repetitions_per_candidate: u32,
    pub planned_attempts: usize,
    pub completed_attempts: usize,
    pub evaluation_worker_ids: Vec<String>,
    pub evaluation_worker_role: &'static str,
    pub training_consumer_role: &'static str,
    pub proof_repetitions_training_eligible: bool,
    pub proof_repetitions: usize,
    pub admitted_corpora: Vec<SealedEvaluationCorpus>,
    pub seal_sha256: Digest,
}

impl EvaluationGenerationSeal {
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        evaluation_generation: u32,
        repetitions_per_candidate: u32,
        planned_attempts: usize,
        completed_attempts: usize,
        infrastructure_faults: usize,
        attempts: &[EvaluationAttemptInput],
    ) -> Result<Self, EvaluationIsolationError> {
        if repetitions_per_candidate < 2
            || planned_attempts == 0
            || completed_attempts != planned_attempts
            || attempts.len() != planned_attempts
            || infrastructure_faults != 0
        {
            return Err(EvaluationIsolationError::IncompleteEvaluation);
        }
        let mut by_candidate = BTreeMap::<String, Vec<&EvaluationAttemptInput>>::new();
        let mut workers = BTreeSet::new();
        for attempt in attempts {
            if !valid_id(&attempt.candidate_id) || !valid_id(&attempt.worker_id) {
                return Err(EvaluationIsolationError::InvalidAttemptIdentity);
            }
            workers.insert(attempt.worker_id.clone());
            by_candidate
                .entry(attempt.candidate_id.clone())
                .or_default()
                .push(attempt);
        }
        if by_candidate.is_empty()
            || by_candidate
                .len()
                .checked_mul(repetitions_per_candidate as usize)
                != Some(planned_attempts)
        {
            return Err(EvaluationIsolationError::IncompleteEvaluation);
        }
        let mut admitted_corpora = Vec::new();
        for (candidate_id, candidate_attempts) in &mut by_candidate {
            candidate_attempts.sort_by_key(|attempt| attempt.attempt);
            if candidate_attempts.len() != repetitions_per_candidate as usize
                || candidate_attempts
                    .iter()
                    .enumerate()
                    .any(|(index, attempt)| attempt.attempt != index as u32 + 1)
            {
                return Err(EvaluationIsolationError::IncompleteEvaluation);
            }
            let corpora = candidate_attempts
                .iter()
                .filter_map(|attempt| {
                    attempt
                        .transition_corpus_sha256
                        .map(|digest| (attempt.attempt, digest))
                })
                .collect::<Vec<_>>();
            if corpora.len() > 1 || corpora.first().is_some_and(|(attempt, _)| *attempt != 1) {
                return Err(EvaluationIsolationError::ProofRepetitionLeak);
            }
            if let Some((source_attempt, transition_corpus_sha256)) = corpora.first().copied() {
                if transition_corpus_sha256 == Digest::ZERO {
                    return Err(EvaluationIsolationError::InvalidCorpusIdentity);
                }
                admitted_corpora.push(SealedEvaluationCorpus {
                    candidate_id: candidate_id.clone(),
                    source_attempt,
                    transition_corpus_sha256,
                });
            }
        }
        admitted_corpora.sort_by(|left, right| left.candidate_id.cmp(&right.candidate_id));
        let minimum_training_generation = evaluation_generation
            .checked_add(1)
            .ok_or(EvaluationIsolationError::GenerationOverflow)?;
        let mut seal = Self {
            schema: EVALUATION_GENERATION_SEAL_SCHEMA_V1,
            evaluation_generation,
            minimum_training_generation,
            repetitions_per_candidate,
            planned_attempts,
            completed_attempts,
            evaluation_worker_ids: workers.into_iter().collect(),
            evaluation_worker_role: "evaluation_only",
            training_consumer_role: "post_seal_later_generation_only",
            proof_repetitions_training_eligible: false,
            proof_repetitions: planned_attempts - by_candidate.len(),
            admitted_corpora,
            seal_sha256: Digest::ZERO,
        };
        seal.seal_sha256 = seal.digest();
        Ok(seal)
    }

    pub fn admit_training_generation(
        &self,
        training_generation: u32,
        corpus_digests: &BTreeSet<Digest>,
    ) -> Result<(), EvaluationIsolationError> {
        if self.schema != EVALUATION_GENERATION_SEAL_SCHEMA_V1
            || self.seal_sha256 != self.digest()
            || training_generation < self.minimum_training_generation
            || self.proof_repetitions_training_eligible
        {
            return Err(EvaluationIsolationError::TrainingBeforeSeal);
        }
        let admitted = self
            .admitted_corpora
            .iter()
            .map(|corpus| corpus.transition_corpus_sha256)
            .collect::<BTreeSet<_>>();
        if !corpus_digests.is_subset(&admitted) {
            return Err(EvaluationIsolationError::UnsealedCorpus);
        }
        Ok(())
    }

    fn digest(&self) -> Digest {
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.evaluation-generation-seal/v1\0");
        hasher.update(self.evaluation_generation.to_le_bytes());
        hasher.update(self.minimum_training_generation.to_le_bytes());
        hasher.update(self.repetitions_per_candidate.to_le_bytes());
        hasher.update((self.planned_attempts as u64).to_le_bytes());
        hasher.update((self.completed_attempts as u64).to_le_bytes());
        for worker in &self.evaluation_worker_ids {
            hasher.update((worker.len() as u64).to_le_bytes());
            hasher.update(worker.as_bytes());
        }
        for corpus in &self.admitted_corpora {
            hasher.update((corpus.candidate_id.len() as u64).to_le_bytes());
            hasher.update(corpus.candidate_id.as_bytes());
            hasher.update(corpus.source_attempt.to_le_bytes());
            hasher.update(corpus.transition_corpus_sha256.0);
        }
        Digest(hasher.finalize().into())
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 192
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EvaluationIsolationError {
    IncompleteEvaluation,
    InvalidAttemptIdentity,
    InvalidCorpusIdentity,
    ProofRepetitionLeak,
    GenerationOverflow,
    TrainingBeforeSeal,
    UnsealedCorpus,
}

impl fmt::Display for EvaluationIsolationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "evaluation isolation rejected: {self:?}")
    }
}

impl Error for EvaluationIsolationError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempts(second_has_corpus: bool) -> Vec<EvaluationAttemptInput> {
        (1..=2)
            .map(|attempt| EvaluationAttemptInput {
                candidate_id: "candidate-a".into(),
                attempt,
                worker_id: format!("evaluation-worker-{attempt}"),
                transition_corpus_sha256: (attempt == 1 || second_has_corpus)
                    .then_some(Digest([attempt as u8; 32])),
            })
            .collect()
    }

    #[test]
    fn complete_evaluation_admits_only_first_episode_in_later_generation() {
        let seal = EvaluationGenerationSeal::build(4, 2, 2, 2, 0, &attempts(false)).unwrap();
        assert_eq!(seal.minimum_training_generation, 5);
        assert_eq!(seal.proof_repetitions, 1);
        assert!(!seal.proof_repetitions_training_eligible);
        assert_eq!(seal.evaluation_worker_ids.len(), 2);
        let corpora = BTreeSet::from([Digest([1; 32])]);
        assert_eq!(
            seal.admit_training_generation(4, &corpora),
            Err(EvaluationIsolationError::TrainingBeforeSeal)
        );
        seal.admit_training_generation(5, &corpora).unwrap();
    }

    #[test]
    fn incomplete_or_repeated_corpus_evidence_fails_closed() {
        assert_eq!(
            EvaluationGenerationSeal::build(0, 2, 2, 1, 0, &attempts(false)),
            Err(EvaluationIsolationError::IncompleteEvaluation)
        );
        assert_eq!(
            EvaluationGenerationSeal::build(0, 2, 2, 2, 0, &attempts(true)),
            Err(EvaluationIsolationError::ProofRepetitionLeak)
        );
    }
}
