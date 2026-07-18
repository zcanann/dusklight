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

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct NoveltyPreservationPredicate {
    pub descriptor_identity: String,
    pub catalog_observed_episodes_before: u64,
    pub rare_support_episode_ceiling: u64,
    pub required_first_seen_transitions: Vec<StateTransitionFact>,
    pub required_rare_state_combinations: Vec<RareStateCombinationReason>,
    pub replay_boundary: BoundaryFingerprintFact,
    pub identity: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoveltyReplayEvidence {
    pub descriptor: SemanticNoveltyDescriptor,
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
    pub attempts: Vec<NoveltyMinimizationAttempt>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NoveltyMinimizationConfig {
    pub minimum_frames: usize,
    pub maximum_attempts: usize,
}

impl Default for NoveltyMinimizationConfig {
    fn default() -> Self {
        Self {
            minimum_frames: 1,
            maximum_attempts: 1_000,
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
        replay_boundary: BoundaryFingerprintFact,
    ) -> Result<Self, NoveltyMinimizationError> {
        if !assessment.semantic_novel
            || assessment.first_seen_transitions.is_empty()
                && assessment.rare_state_combinations.is_empty()
        {
            return Err(NoveltyMinimizationError(
                "novelty minimization requires a positive raw semantic predicate".into(),
            ));
        }
        validate_boundary(&replay_boundary)?;
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
        if !evidence
            .descriptor
            .boundary_fingerprints
            .contains(&self.replay_boundary)
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
    F: FnMut(&InputTape) -> Result<NoveltyReplayEvidence, String>,
{
    source
        .validate()
        .map_err(|error| NoveltyMinimizationError(error.to_string()))?;
    if config.minimum_frames > source.frames.len()
        || config.maximum_attempts == 0
        || config.maximum_attempts > MAX_NOVELTY_MINIMIZATION_ATTEMPTS
    {
        return Err(NoveltyMinimizationError(
            "invalid novelty minimization frame or attempt bound".into(),
        ));
    }
    let initial = replay(source).map_err(|error| {
        NoveltyMinimizationError(format!("initial novelty replay failed: {error}"))
    })?;
    predicate.accepts(&initial).map_err(|error| {
        NoveltyMinimizationError(format!("source does not satisfy frozen novelty: {error}"))
    })?;

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
            let replay_result = replay(&candidate);
            let rejection_reason = match replay_result {
                Ok(evidence) => predicate.accepts(&evidence).err(),
                Err(error) => Some(format!("replay failed: {error}")),
            };
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
        attempts,
    };
    Ok((current, report))
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
        NoveltyPreservationPredicate::from_assessment(&assessment, boundary("ab")).unwrap()
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
            },
            |candidate| {
                Ok(NoveltyReplayEvidence {
                    descriptor: descriptor(boundary("ab"), candidate.frames.len() >= 2),
                })
            },
        )
        .unwrap();
        assert_eq!(minimized.frames.len(), 2);
        assert_eq!(report.minimized_frames, 2);
        assert!(report.attempts.iter().any(|attempt| attempt.accepted));
    }

    #[test]
    fn changed_boundary_is_rejected_even_when_semantic_novelty_survives() {
        let evidence = NoveltyReplayEvidence {
            descriptor: descriptor(boundary("cd"), true),
        };
        assert_eq!(
            predicate().accepts(&evidence).unwrap_err(),
            "candidate changed or lost the exact replay boundary"
        );
    }

    #[test]
    fn lost_novel_transition_is_rejected_before_artifact_replacement() {
        let evidence = NoveltyReplayEvidence {
            descriptor: descriptor(boundary("ab"), false),
        };
        assert!(
            predicate()
                .accepts(&evidence)
                .unwrap_err()
                .contains("first-seen transition")
        );
    }
}
