//! Generation seals that keep evaluation repetitions out of online training.

use crate::artifact::Digest;
use crate::episode::EpisodeOutcomeClass;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const EVALUATION_GENERATION_SEAL_SCHEMA_V1: &str = "dusklight-evaluation-generation-seal/v1";
pub const EVALUATION_OUTCOME_COLLECTION_SCHEMA_V1: &str =
    "dusklight-evaluation-outcome-collection/v1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationAttemptInput {
    pub candidate_id: String,
    pub attempt: u32,
    pub worker_id: String,
    /// Present only for the one collection episode retained from evaluation.
    pub transition_corpus_sha256: Option<Digest>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationOutcomeInput {
    pub candidate_id: String,
    pub attempt: u32,
    pub outcome: EpisodeOutcomeClass,
    pub milestone_depth: u16,
    pub goal_reached: bool,
    pub transition_corpus_sha256: Option<Digest>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationOutcomeStratum {
    Success,
    NearMiss,
    OrdinaryFailure,
    OtherTerminal,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SealedEvaluationOutcome {
    pub candidate_id: String,
    pub attempt: u32,
    pub stratum: EvaluationOutcomeStratum,
    pub outcome: EpisodeOutcomeClass,
    pub milestone_depth: u16,
    pub goal_reached: bool,
    pub training_eligible_after_seal: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition_corpus_sha256: Option<Digest>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct EvaluationOutcomeCounts {
    pub successes: usize,
    pub near_misses: usize,
    pub ordinary_failures: usize,
    pub other_terminals: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EvaluationOutcomeCollection {
    pub schema: &'static str,
    pub evaluation_generation: u32,
    pub evaluation_seal_sha256: Digest,
    pub minimum_training_generation: u32,
    pub proof_repetitions_training_eligible: bool,
    pub required_mix_complete: bool,
    pub counts: EvaluationOutcomeCounts,
    pub outcomes: Vec<SealedEvaluationOutcome>,
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
            if !valid_id(&attempt.candidate_id)
                || !valid_id(&attempt.worker_id)
                || !attempt.worker_id.starts_with("evaluation/")
            {
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

impl EvaluationOutcomeCollection {
    pub fn build(
        seal: &EvaluationGenerationSeal,
        inputs: &[EvaluationOutcomeInput],
    ) -> Result<Self, EvaluationIsolationError> {
        let admitted_digests = seal
            .admitted_corpora
            .iter()
            .map(|corpus| corpus.transition_corpus_sha256)
            .collect::<BTreeSet<_>>();
        seal.admit_training_generation(seal.minimum_training_generation, &admitted_digests)?;
        if inputs.len() != seal.planned_attempts {
            return Err(EvaluationIsolationError::SealMismatch);
        }
        let admitted_by_candidate = seal
            .admitted_corpora
            .iter()
            .map(|corpus| {
                (
                    corpus.candidate_id.as_str(),
                    corpus.transition_corpus_sha256,
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut grouped = BTreeMap::<String, Vec<&EvaluationOutcomeInput>>::new();
        for input in inputs {
            if !valid_id(&input.candidate_id) {
                return Err(EvaluationIsolationError::InvalidAttemptIdentity);
            }
            grouped
                .entry(input.candidate_id.clone())
                .or_default()
                .push(input);
        }
        if grouped.is_empty()
            || grouped
                .len()
                .checked_mul(seal.repetitions_per_candidate as usize)
                != Some(inputs.len())
        {
            return Err(EvaluationIsolationError::SealMismatch);
        }

        let mut counts = EvaluationOutcomeCounts::default();
        let mut outcomes = Vec::with_capacity(inputs.len());
        for (candidate_id, candidate_inputs) in &mut grouped {
            candidate_inputs.sort_by_key(|input| input.attempt);
            if candidate_inputs.len() != seal.repetitions_per_candidate as usize
                || candidate_inputs
                    .iter()
                    .enumerate()
                    .any(|(index, input)| input.attempt != index as u32 + 1)
            {
                return Err(EvaluationIsolationError::SealMismatch);
            }
            for input in candidate_inputs {
                let admitted = admitted_by_candidate.get(candidate_id.as_str()).copied();
                if input.attempt != 1 && input.transition_corpus_sha256.is_some() {
                    return Err(EvaluationIsolationError::ProofRepetitionLeak);
                }
                if input.attempt == 1 && input.transition_corpus_sha256 != admitted {
                    return Err(EvaluationIsolationError::SealMismatch);
                }
                let stratum = match (input.outcome, input.goal_reached, input.milestone_depth) {
                    (EpisodeOutcomeClass::Successful, true, _) => {
                        counts.successes += 1;
                        EvaluationOutcomeStratum::Success
                    }
                    (EpisodeOutcomeClass::Failed, false, 1..) => {
                        counts.near_misses += 1;
                        EvaluationOutcomeStratum::NearMiss
                    }
                    (EpisodeOutcomeClass::Failed, false, 0) => {
                        counts.ordinary_failures += 1;
                        EvaluationOutcomeStratum::OrdinaryFailure
                    }
                    (EpisodeOutcomeClass::Successful, false, _)
                    | (EpisodeOutcomeClass::Failed, true, _) => {
                        return Err(EvaluationIsolationError::InvalidOutcome);
                    }
                    (_, true, _) => return Err(EvaluationIsolationError::InvalidOutcome),
                    (_, false, _) => {
                        counts.other_terminals += 1;
                        EvaluationOutcomeStratum::OtherTerminal
                    }
                };
                outcomes.push(SealedEvaluationOutcome {
                    candidate_id: candidate_id.clone(),
                    attempt: input.attempt,
                    stratum,
                    outcome: input.outcome,
                    milestone_depth: input.milestone_depth,
                    goal_reached: input.goal_reached,
                    training_eligible_after_seal: input.attempt == 1 && admitted.is_some(),
                    transition_corpus_sha256: input.transition_corpus_sha256,
                });
            }
        }
        let required_mix_complete =
            counts.successes > 0 && counts.near_misses > 0 && counts.ordinary_failures > 0;
        Ok(Self {
            schema: EVALUATION_OUTCOME_COLLECTION_SCHEMA_V1,
            evaluation_generation: seal.evaluation_generation,
            evaluation_seal_sha256: seal.seal_sha256,
            minimum_training_generation: seal.minimum_training_generation,
            proof_repetitions_training_eligible: false,
            required_mix_complete,
            counts,
            outcomes,
        })
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
    SealMismatch,
    InvalidOutcome,
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
                worker_id: format!("evaluation/worker-{attempt}"),
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

    #[test]
    fn outcome_collection_retains_all_three_strata_without_proof_repetition_leakage() {
        let candidate_ids = ["success", "near-miss", "ordinary-failure"];
        let attempt_inputs = candidate_ids
            .iter()
            .flat_map(|candidate_id| {
                (1..=2).map(move |attempt| EvaluationAttemptInput {
                    candidate_id: (*candidate_id).into(),
                    attempt,
                    worker_id: format!("evaluation/worker-{attempt}"),
                    transition_corpus_sha256: (attempt == 1)
                        .then_some(Digest([candidate_id.as_bytes()[0]; 32])),
                })
            })
            .collect::<Vec<_>>();
        let seal = EvaluationGenerationSeal::build(7, 2, 6, 6, 0, &attempt_inputs).unwrap();
        let outcome_inputs = candidate_ids
            .iter()
            .flat_map(|candidate_id| {
                (1..=2).map(move |attempt| {
                    let (outcome, milestone_depth, goal_reached) = match *candidate_id {
                        "success" => (EpisodeOutcomeClass::Successful, 2, true),
                        "near-miss" => (EpisodeOutcomeClass::Failed, 1, false),
                        _ => (EpisodeOutcomeClass::Failed, 0, false),
                    };
                    EvaluationOutcomeInput {
                        candidate_id: (*candidate_id).into(),
                        attempt,
                        outcome,
                        milestone_depth,
                        goal_reached,
                        transition_corpus_sha256: (attempt == 1)
                            .then_some(Digest([candidate_id.as_bytes()[0]; 32])),
                    }
                })
            })
            .collect::<Vec<_>>();
        let collection = EvaluationOutcomeCollection::build(&seal, &outcome_inputs).unwrap();
        assert!(collection.required_mix_complete);
        assert_eq!(collection.counts.successes, 2);
        assert_eq!(collection.counts.near_misses, 2);
        assert_eq!(collection.counts.ordinary_failures, 2);
        assert_eq!(
            collection
                .outcomes
                .iter()
                .filter(|outcome| outcome.training_eligible_after_seal)
                .count(),
            3
        );
        assert!(
            collection
                .outcomes
                .iter()
                .filter(|outcome| outcome.attempt > 1)
                .all(|outcome| !outcome.training_eligible_after_seal
                    && outcome.transition_corpus_sha256.is_none())
        );
    }
}
