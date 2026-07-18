//! Delta minimization guarded by frozen semantic novelty and replay boundaries.

use super::catalog::{RareStateCombinationReason, SemanticNoveltyAssessment};
use super::{BoundaryFingerprintFact, SemanticNoveltyDescriptor, StateTransitionFact};
use crate::tape::InputTape;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const NOVELTY_MINIMIZATION_SCHEMA: &str = "dusklight-novelty-minimization/v1";
pub const MAX_NOVELTY_MINIMIZATION_ATTEMPTS: usize = 10_000;
pub const MAX_NOVELTY_MINIMIZATION_REPETITIONS: u32 = 64;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct NoveltyReplayBoundary {
    pub simulation_tick: u64,
    pub tape_frame: u64,
    pub fingerprint: BoundaryFingerprintFact,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct NoveltyPreservationPredicate {
    pub descriptor_identity: String,
    pub catalog_observed_episodes_before: u64,
    pub rare_support_episode_ceiling: u64,
    pub required_first_seen_transitions: Vec<StateTransitionFact>,
    pub required_rare_state_combinations: Vec<RareStateCombinationReason>,
    pub replay_boundary: NoveltyReplayBoundary,
    pub identity: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoveltyReplayEvidence {
    pub descriptor: SemanticNoveltyDescriptor,
    pub replay_boundary: NoveltyReplayBoundary,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct NoveltyMinimizationAttempt {
    pub removed_start_frame: usize,
    pub removed_end_frame_exclusive: usize,
    pub frames_before: usize,
    pub frames_after: usize,
    pub accepted: bool,
    pub rejection_reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct NoveltyMinimizationReport {
    pub schema: &'static str,
    pub predicate: NoveltyPreservationPredicate,
    pub source_frames: usize,
    pub minimized_frames: usize,
    pub repetitions: u32,
    pub attempts: Vec<NoveltyMinimizationAttempt>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NoveltyMinimizationConfig {
    pub minimum_frames: usize,
    pub maximum_attempts: usize,
    pub repetitions: u32,
}

impl Default for NoveltyMinimizationConfig {
    fn default() -> Self {
        Self {
            minimum_frames: 1,
            maximum_attempts: 1_000,
            repetitions: 2,
        }
    }
}

#[derive(Debug)]
pub struct NoveltyMinimizationError(String);

impl fmt::Display for NoveltyMinimizationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NoveltyMinimizationError {}

impl NoveltyPreservationPredicate {
    pub fn from_assessment(
        assessment: &SemanticNoveltyAssessment,
        replay_boundary: NoveltyReplayBoundary,
    ) -> Result<Self, NoveltyMinimizationError> {
        if !assessment.semantic_novel
            || assessment.first_seen_transitions.is_empty()
                && assessment.rare_state_combinations.is_empty()
        {
            return Err(NoveltyMinimizationError(
                "novelty minimization requires a positive raw semantic predicate".into(),
            ));
        }
        validate_boundary(&replay_boundary.fingerprint)?;
        let mut predicate = Self {
            descriptor_identity: assessment.descriptor_identity.clone(),
            catalog_observed_episodes_before: assessment.catalog_observed_episodes_before,
            rare_support_episode_ceiling: assessment.rare_support_episode_ceiling,
            required_first_seen_transitions: assessment.first_seen_transitions.clone(),
            required_rare_state_combinations: assessment.rare_state_combinations.clone(),
            replay_boundary,
            identity: String::new(),
        };
        predicate.identity = predicate.compute_identity();
        Ok(predicate)
    }

    pub fn accepts(&self, evidence: &NoveltyReplayEvidence) -> Result<(), String> {
        let transitions = evidence
            .descriptor
            .state_transitions
            .iter()
            .collect::<BTreeSet<_>>();
        if self
            .required_first_seen_transitions
            .iter()
            .any(|required| !transitions.contains(required))
        {
            return Err("candidate lost a required first-seen transition".into());
        }
        let combinations = evidence
            .descriptor
            .state_combinations
            .iter()
            .collect::<BTreeSet<_>>();
        if self
            .required_rare_state_combinations
            .iter()
            .map(|reason| &reason.combination)
            .any(|required| !combinations.contains(required))
        {
            return Err("candidate lost a required rare state combination".into());
        }
        if evidence.replay_boundary != self.replay_boundary
            || !evidence
                .descriptor
                .boundary_fingerprints
                .contains(&self.replay_boundary.fingerprint)
        {
            return Err("candidate changed or lost the exact replay boundary".into());
        }
        Ok(())
    }

    fn compute_identity(&self) -> String {
        let encoded = serde_json::to_vec(&(
            &self.descriptor_identity,
            self.catalog_observed_episodes_before,
            self.rare_support_episode_ceiling,
            &self.required_first_seen_transitions,
            &self.required_rare_state_combinations,
            &self.replay_boundary,
        ))
        .expect("novelty predicate is serializable");
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight-novelty-preservation-predicate/v1\0");
        hasher.update((encoded.len() as u64).to_le_bytes());
        hasher.update(encoded);
        format!("{:x}", hasher.finalize())
    }
}

pub fn minimize_novel_tape<F>(
    source: &InputTape,
    predicate: NoveltyPreservationPredicate,
    config: NoveltyMinimizationConfig,
    mut replay: F,
) -> Result<(InputTape, NoveltyMinimizationReport), NoveltyMinimizationError>
where
    F: FnMut(&InputTape, u32) -> Result<NoveltyReplayEvidence, String>,
{
    source
        .validate()
        .map_err(|error| NoveltyMinimizationError(error.to_string()))?;
    if config.minimum_frames > source.frames.len()
        || config.maximum_attempts == 0
        || config.maximum_attempts > MAX_NOVELTY_MINIMIZATION_ATTEMPTS
        || !(2..=MAX_NOVELTY_MINIMIZATION_REPETITIONS).contains(&config.repetitions)
    {
        return Err(NoveltyMinimizationError(
            "invalid novelty minimization frame or attempt bound".into(),
        ));
    }
    let initial =
        replay_repeated(source, &predicate, config.repetitions, &mut replay).map_err(|error| {
            NoveltyMinimizationError(format!("initial novelty replay failed: {error}"))
        })?;
    if let Err(error) = initial {
        return Err(NoveltyMinimizationError(format!(
            "source does not satisfy frozen novelty: {error}"
        )));
    }

    let source_frames = source.frames.len();
    let mut current = source.clone();
    let mut attempts = Vec::new();
    let mut granularity = 2_usize;
    while current.frames.len() > config.minimum_frames && attempts.len() < config.maximum_attempts {
        let removable = current.frames.len() - config.minimum_frames;
        let partitions = granularity.min(removable.max(1));
        let mut accepted = false;
        for partition in 0..partitions {
            if attempts.len() >= config.maximum_attempts {
                break;
            }
            let start = current.frames.len() * partition / partitions;
            let mut end = current.frames.len() * (partition + 1) / partitions;
            let maximum_remove = current.frames.len() - config.minimum_frames;
            end = end.min(start + maximum_remove);
            if start >= end {
                continue;
            }
            let mut candidate = current.clone();
            candidate.frames.drain(start..end);
            let rejection_reason =
                replay_repeated(&candidate, &predicate, config.repetitions, &mut replay)
                    .unwrap_or_else(|error| Err(format!("replay failed: {error}")))
                    .err();
            let accepted_attempt = rejection_reason.is_none();
            attempts.push(NoveltyMinimizationAttempt {
                removed_start_frame: start,
                removed_end_frame_exclusive: end,
                frames_before: current.frames.len(),
                frames_after: candidate.frames.len(),
                accepted: accepted_attempt,
                rejection_reason,
            });
            if accepted_attempt {
                current = candidate;
                granularity = 2;
                accepted = true;
                break;
            }
        }
        if !accepted {
            if partitions >= removable {
                break;
            }
            granularity = (partitions * 2).min(removable);
        }
    }
    let report = NoveltyMinimizationReport {
        schema: NOVELTY_MINIMIZATION_SCHEMA,
        predicate,
        source_frames,
        minimized_frames: current.frames.len(),
        repetitions: config.repetitions,
        attempts,
    };
    Ok((current, report))
}

fn replay_repeated<F>(
    tape: &InputTape,
    predicate: &NoveltyPreservationPredicate,
    repetitions: u32,
    replay: &mut F,
) -> Result<Result<NoveltyReplayEvidence, String>, String>
where
    F: FnMut(&InputTape, u32) -> Result<NoveltyReplayEvidence, String>,
{
    let mut accepted = None;
    for repetition in 1..=repetitions {
        let evidence = replay(tape, repetition)?;
        if let Err(error) = predicate.accepts(&evidence) {
            return Ok(Err(error));
        }
        if accepted
            .as_ref()
            .is_some_and(|prior: &NoveltyReplayEvidence| prior != &evidence)
        {
            return Ok(Err(
                "cold replay repetitions produced contradictory novelty evidence".into(),
            ));
        }
        accepted = Some(evidence);
    }
    Ok(Ok(accepted.expect("validated repetition count is nonzero")))
}

fn validate_boundary(boundary: &BoundaryFingerprintFact) -> Result<(), NoveltyMinimizationError> {
    if boundary.name.trim().is_empty()
        || boundary.schema.trim().is_empty()
        || boundary.algorithm.trim().is_empty()
        || boundary.canonical_encoding.trim().is_empty()
        || boundary.digest.trim().is_empty()
        || boundary.digest.len() != 64
        || !boundary
            .digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(NoveltyMinimizationError(
            "novelty replay boundary is not fully identified".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic_novelty::catalog::{SemanticNoveltyCatalog, SemanticNoveltyCatalogConfig};
    use crate::tape::InputFrame;
    use crate::tape::TapeBoot;
    use crate::trace::{DecodedTrace, TraceRecord};
    use std::collections::BTreeMap;

    fn boundary(digest: &str) -> BoundaryFingerprintFact {
        BoundaryFingerprintFact {
            name: "novel_terminal".into(),
            schema: "boundary/v1".into(),
            algorithm: "sha256".into(),
            canonical_encoding: "native".into(),
            digest: digest.repeat(32),
        }
    }

    fn descriptor(boundary: BoundaryFingerprintFact, novel: bool) -> SemanticNoveltyDescriptor {
        let procedures = if novel { vec![3, 7] } else { vec![3, 3] };
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
                records: procedures
                    .into_iter()
                    .map(|procedure| TraceRecord {
                        stage_name: "F_SP104".into(),
                        room: 1,
                        player_session_process_id: Some(1),
                        player_proc_id: Some(procedure),
                        ..TraceRecord::default()
                    })
                    .collect(),
            },
            vec![boundary],
        )
        .unwrap()
    }

    fn predicate() -> NoveltyPreservationPredicate {
        let descriptor = descriptor(boundary("ab"), true);
        let assessment = SemanticNoveltyCatalog::default()
            .assess(&descriptor, SemanticNoveltyCatalogConfig::default())
            .unwrap();
        NoveltyPreservationPredicate::from_assessment(
            &assessment,
            NoveltyReplayBoundary {
                simulation_tick: 120,
                tape_frame: 7,
                fingerprint: boundary("ab"),
            },
        )
        .unwrap()
    }

    fn evidence(boundary: BoundaryFingerprintFact, novel: bool) -> NoveltyReplayEvidence {
        NoveltyReplayEvidence {
            descriptor: descriptor(boundary.clone(), novel),
            replay_boundary: NoveltyReplayBoundary {
                simulation_tick: 120,
                tape_frame: 7,
                fingerprint: boundary,
            },
        }
    }

    #[test]
    fn ddmin_accepts_only_replays_preserving_novelty_and_boundary() {
        let tape = InputTape {
            frames: vec![InputFrame::default(); 8],
            ..InputTape::default()
        };
        let (minimized, report) = minimize_novel_tape(
            &tape,
            predicate(),
            NoveltyMinimizationConfig {
                minimum_frames: 2,
                maximum_attempts: 100,
                repetitions: 2,
            },
            |candidate, _| Ok(evidence(boundary("ab"), candidate.frames.len() >= 2)),
        )
        .unwrap();
        assert_eq!(minimized.frames.len(), 2);
        assert_eq!(report.minimized_frames, 2);
        assert!(report.attempts.iter().any(|attempt| attempt.accepted));
    }

    #[test]
    fn changed_boundary_is_rejected_even_when_semantic_novelty_survives() {
        let evidence = evidence(boundary("cd"), true);
        assert_eq!(
            predicate().accepts(&evidence).unwrap_err(),
            "candidate changed or lost the exact replay boundary"
        );
    }

    #[test]
    fn lost_novel_transition_is_rejected_before_artifact_replacement() {
        let evidence = evidence(boundary("ab"), false);
        assert!(
            predicate()
                .accepts(&evidence)
                .unwrap_err()
                .contains("first-seen transition")
        );
    }

    #[test]
    fn contradictory_cold_replays_reject_a_reduction() {
        let tape = InputTape {
            frames: vec![InputFrame::default(); 4],
            ..InputTape::default()
        };
        let (_, report) = minimize_novel_tape(
            &tape,
            predicate(),
            NoveltyMinimizationConfig {
                minimum_frames: 2,
                maximum_attempts: 10,
                repetitions: 2,
            },
            |candidate, repetition| {
                let mut evidence = evidence(boundary("ab"), candidate.frames.len() >= 2);
                if candidate.frames.len() < tape.frames.len() && repetition == 2 {
                    evidence.descriptor.procedure_sequence.push(Some(99));
                }
                Ok(evidence)
            },
        )
        .unwrap();
        assert_eq!(report.minimized_frames, tape.frames.len());
        assert!(report.attempts.iter().all(|attempt| !attempt.accepted));
        assert!(report.attempts.iter().all(|attempt| {
            attempt
                .rejection_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("contradictory novelty evidence"))
        }));
    }
}
