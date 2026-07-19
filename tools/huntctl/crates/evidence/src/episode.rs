//! Complete, reproducible identities for collected transition episodes.

use crate::artifact::{ArtifactIdentity, Digest};
use crate::tape::TapeBoot;
use crate::transition_corpus::{StateReferenceKind, TransitionCorpus};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::PathBuf;

pub const EPISODE_CONTEXT_SCHEMA_V1: &str = "dusklight-episode-context/v1";
pub const EPISODE_MANIFEST_SCHEMA_V1: &str = "dusklight-episode-manifest/v1";
pub const EPISODE_LEDGER_SCHEMA_V1: &str = "dusklight-episode-ledger/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeContext {
    pub schema: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_identity: Option<ArtifactIdentity>,
    pub run_build: RunBuildIdentity,
    pub objective: EpisodeObjectiveIdentity,
    pub producer: EpisodeProducerIdentity,
    pub seed: EpisodeSeed,
    pub worker_id: String,
    pub lineage: EpisodeLineage,
    pub outcome: EpisodeOutcome,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunBuildIdentity {
    pub executable_sha256: Digest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dusklight_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aurora_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feature_digest: Option<Digest>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeObjectiveIdentity {
    pub id: String,
    pub digest: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeProducerKind {
    ManualImport,
    Seed,
    Evolution,
    FittedQ,
    SystematicProbe,
    RandomProbe,
    LatinHypercube,
    StructuredCounterfactual,
    ArchiveNovelty,
    BlindCoverage,
    Replay,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeProducerIdentity {
    pub kind: EpisodeProducerKind,
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EpisodeSeed {
    NotApplicable,
    Deterministic { value: u64 },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeLineage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_candidate_id: Option<String>,
    pub generation: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intervention: Option<EpisodeIntervention>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeIntervention {
    pub start_frame: u64,
    pub end_frame_exclusive: u64,
    pub parent_end_frame_exclusive: u64,
    pub description: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeOutcomeClass {
    Successful,
    Failed,
    Crashed,
    TimedOut,
    Desynced,
    Unsupported,
    Truncated,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeOutcome {
    pub class: EpisodeOutcomeClass,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeScenarioIdentity {
    pub id: String,
    pub digest: Digest,
    pub boot: TapeBoot,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeReferenceKind {
    Boundary,
    Snapshot,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeBoundaryIdentity {
    pub kind: EpisodeReferenceKind,
    pub digest: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeQueryViewIdentity {
    pub id: String,
    pub schema_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeArtifactIdentity {
    pub absolute_tape_sha256: Digest,
    pub gameplay_trace_sha256: Digest,
    pub transition_corpus_sha256: Digest,
    pub transition_evidence_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeManifest {
    pub schema: String,
    pub input_identity_sha256: Digest,
    pub episode_sha256: Digest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_identity: Option<ArtifactIdentity>,
    pub scenario: EpisodeScenarioIdentity,
    pub parent_boundary: EpisodeBoundaryIdentity,
    pub artifacts: EpisodeArtifactIdentity,
    pub run_build: RunBuildIdentity,
    pub query_view: EpisodeQueryViewIdentity,
    pub action_schema_sha256: Digest,
    pub objective: EpisodeObjectiveIdentity,
    pub producer: EpisodeProducerIdentity,
    pub seed: EpisodeSeed,
    pub worker_id: String,
    pub lineage: EpisodeLineage,
    pub outcome: EpisodeOutcome,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeLedger {
    pub schema: String,
    pub groups: BTreeMap<String, EpisodeGroup>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeGroup {
    pub input_identity_sha256: Digest,
    pub episodes: Vec<EpisodeOccurrence>,
    pub proof_repetitions: Vec<ProofRepetition>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeOccurrence {
    pub episode_sha256: Digest,
    pub occurrences: u32,
    pub manifest_paths: Vec<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProofRepetition {
    pub proof_sha256: Digest,
    pub occurrences: u32,
    pub proof_paths: Vec<PathBuf>,
    pub worker_id: String,
    pub attempt: u32,
    pub outcome: EpisodeOutcome,
}

pub struct EpisodeManifestBuild<'a> {
    pub context: &'a EpisodeContext,
    pub boot: &'a TapeBoot,
    pub corpus: &'a TransitionCorpus,
    pub query_view_id: &'a str,
    pub tape_sha256: Digest,
    pub trace_sha256: Digest,
    pub transition_evidence_sha256: Digest,
}

impl EpisodeContext {
    pub fn validate(&self) -> Result<(), EpisodeError> {
        if let Some(identity) = &self.run_identity {
            identity
                .validate()
                .map_err(|error| EpisodeError::new(error.to_string()))?;
        }
        if self.schema != EPISODE_CONTEXT_SCHEMA_V1
            || self.run_build.executable_sha256 == Digest::ZERO
            || self.objective.digest == Digest::ZERO
            || !valid_name(&self.objective.id, 192)
            || !valid_name(&self.producer.name, 192)
            || !valid_name(&self.producer.version, 96)
            || !valid_name(&self.worker_id, 192)
            || self
                .run_build
                .dusklight_commit
                .as_deref()
                .is_some_and(|value| !valid_name(value, 192))
            || self
                .run_build
                .aurora_commit
                .as_deref()
                .is_some_and(|value| !valid_name(value, 192))
            || self
                .run_build
                .target
                .as_deref()
                .is_some_and(|value| !valid_name(value, 192))
            || self
                .run_build
                .profile
                .as_deref()
                .is_some_and(|value| !valid_name(value, 192))
            || !valid_text(&self.outcome.reason, 2048)
            || self
                .lineage
                .candidate_id
                .as_deref()
                .is_some_and(|value| !valid_name(value, 192))
            || self
                .lineage
                .parent_candidate_id
                .as_deref()
                .is_some_and(|value| !valid_name(value, 192))
            || self.lineage.intervention.as_ref().is_some_and(|value| {
                (value.end_frame_exclusive <= value.start_frame
                    && value.parent_end_frame_exclusive <= value.start_frame)
                    || !valid_text(&value.description, 2048)
            })
        {
            return Err(EpisodeError::new(
                "episode context is incomplete or invalid",
            ));
        }
        Ok(())
    }
}

impl EpisodeManifest {
    pub fn build(input: EpisodeManifestBuild<'_>) -> Result<Self, EpisodeError> {
        input.context.validate()?;
        input
            .corpus
            .validate()
            .map_err(|error| EpisodeError::new(error.to_string()))?;
        let first = input
            .corpus
            .transitions
            .first()
            .ok_or_else(|| EpisodeError::new("episode corpus is empty"))?;
        let scenario = scenario_identity(input.boot)?;
        let parent_boundary = EpisodeBoundaryIdentity {
            kind: match first.source.kind {
                StateReferenceKind::Boundary => EpisodeReferenceKind::Boundary,
                StateReferenceKind::Snapshot => EpisodeReferenceKind::Snapshot,
            },
            digest: first.source.digest,
        };
        let artifacts = EpisodeArtifactIdentity {
            absolute_tape_sha256: input.tape_sha256,
            gameplay_trace_sha256: input.trace_sha256,
            transition_corpus_sha256: input
                .corpus
                .content_digest()
                .map_err(|error| EpisodeError::new(error.to_string()))?,
            transition_evidence_sha256: input.transition_evidence_sha256,
        };
        let query_view = EpisodeQueryViewIdentity {
            id: input.query_view_id.into(),
            schema_sha256: input.corpus.feature_schema,
        };
        let mut manifest = Self {
            schema: EPISODE_MANIFEST_SCHEMA_V1.into(),
            input_identity_sha256: Digest::ZERO,
            episode_sha256: Digest::ZERO,
            run_identity: input.context.run_identity.clone(),
            scenario,
            parent_boundary,
            artifacts,
            run_build: input.context.run_build.clone(),
            query_view,
            action_schema_sha256: input.corpus.action_schema,
            objective: input.context.objective.clone(),
            producer: input.context.producer.clone(),
            seed: input.context.seed.clone(),
            worker_id: input.context.worker_id.clone(),
            lineage: input.context.lineage.clone(),
            outcome: input.context.outcome.clone(),
        };
        manifest.input_identity_sha256 = manifest.compute_input_identity()?;
        manifest.episode_sha256 = manifest.compute_episode_identity()?;
        manifest.validate(input.corpus)?;
        Ok(manifest)
    }

    pub fn validate(&self, corpus: &TransitionCorpus) -> Result<(), EpisodeError> {
        let first = corpus
            .transitions
            .first()
            .ok_or_else(|| EpisodeError::new("episode corpus is empty"))?;
        let expected_parent = EpisodeBoundaryIdentity {
            kind: match first.source.kind {
                StateReferenceKind::Boundary => EpisodeReferenceKind::Boundary,
                StateReferenceKind::Snapshot => EpisodeReferenceKind::Snapshot,
            },
            digest: first.source.digest,
        };
        if self.schema != EPISODE_MANIFEST_SCHEMA_V1
            || self.artifacts.absolute_tape_sha256 == Digest::ZERO
            || self.artifacts.gameplay_trace_sha256 == Digest::ZERO
            || self.artifacts.transition_evidence_sha256 == Digest::ZERO
            || self.parent_boundary != expected_parent
            || !valid_name(&self.scenario.id, 512)
            || !valid_name(&self.query_view.id, 192)
            || self.query_view.schema_sha256 != corpus.feature_schema
            || self.action_schema_sha256 != corpus.action_schema
            || self.artifacts.transition_corpus_sha256
                != corpus
                    .content_digest()
                    .map_err(|error| EpisodeError::new(error.to_string()))?
            || self.scenario != scenario_identity(&self.scenario.boot)?
            || self.input_identity_sha256 != self.compute_input_identity()?
            || self.episode_sha256 != self.compute_episode_identity()?
        {
            return Err(EpisodeError::new("episode manifest identity mismatch"));
        }
        let context = EpisodeContext {
            schema: EPISODE_CONTEXT_SCHEMA_V1.into(),
            run_identity: self.run_identity.clone(),
            run_build: self.run_build.clone(),
            objective: self.objective.clone(),
            producer: self.producer.clone(),
            seed: self.seed.clone(),
            worker_id: self.worker_id.clone(),
            lineage: self.lineage.clone(),
            outcome: self.outcome.clone(),
        };
        context.validate()?;
        if self.outcome.class == EpisodeOutcomeClass::Successful
            && !corpus
                .transitions
                .last()
                .is_some_and(|transition| transition.terminal)
        {
            return Err(EpisodeError::new(
                "successful episode must end in a terminal transition",
            ));
        }
        Ok(())
    }

    fn compute_input_identity(&self) -> Result<Digest, EpisodeError> {
        canonical_digest(
            b"dusklight.episode-input-identity/v1\0",
            &(
                &self.run_identity,
                &self.scenario,
                &self.parent_boundary,
                self.artifacts.absolute_tape_sha256,
                &self.run_build,
                &self.query_view,
                self.action_schema_sha256,
                &self.objective,
                &self.producer,
                &self.seed,
                &self.lineage,
            ),
        )
    }

    fn compute_episode_identity(&self) -> Result<Digest, EpisodeError> {
        canonical_digest(
            b"dusklight.episode-identity/v1\0",
            &(
                self.input_identity_sha256,
                &self.worker_id,
                &self.outcome,
                self.artifacts.gameplay_trace_sha256,
                self.artifacts.transition_corpus_sha256,
                self.artifacts.transition_evidence_sha256,
            ),
        )
    }
}

impl EpisodeLedger {
    pub fn new() -> Self {
        Self {
            schema: EPISODE_LEDGER_SCHEMA_V1.into(),
            groups: BTreeMap::new(),
        }
    }

    pub fn ingest_episode(&mut self, manifest: &EpisodeManifest, path: PathBuf) {
        let group = self
            .groups
            .entry(manifest.input_identity_sha256.to_string())
            .or_insert_with(|| EpisodeGroup {
                input_identity_sha256: manifest.input_identity_sha256,
                episodes: Vec::new(),
                proof_repetitions: Vec::new(),
            });
        if let Some(existing) = group
            .episodes
            .iter_mut()
            .find(|episode| episode.episode_sha256 == manifest.episode_sha256)
        {
            existing.occurrences = existing.occurrences.saturating_add(1);
            if !existing.manifest_paths.contains(&path) {
                existing.manifest_paths.push(path);
                existing.manifest_paths.sort();
            }
        } else {
            group.episodes.push(EpisodeOccurrence {
                episode_sha256: manifest.episode_sha256,
                occurrences: 1,
                manifest_paths: vec![path],
            });
            group
                .episodes
                .sort_by_key(|episode| episode.episode_sha256.to_string());
        }
    }

    pub fn ingest_proof(
        &mut self,
        input_identity_sha256: Digest,
        proof_sha256: Digest,
        path: PathBuf,
        worker_id: String,
        attempt: u32,
        outcome: EpisodeOutcome,
    ) -> Result<(), EpisodeError> {
        let group = self
            .groups
            .get_mut(&input_identity_sha256.to_string())
            .ok_or_else(|| EpisodeError::new("proof repetition has no episode input group"))?;
        if let Some(existing) = group
            .proof_repetitions
            .iter_mut()
            .find(|proof| proof.proof_sha256 == proof_sha256)
        {
            existing.occurrences = existing.occurrences.saturating_add(1);
            if !existing.proof_paths.contains(&path) {
                existing.proof_paths.push(path);
                existing.proof_paths.sort();
            }
        } else {
            group.proof_repetitions.push(ProofRepetition {
                proof_sha256,
                occurrences: 1,
                proof_paths: vec![path],
                worker_id,
                attempt,
                outcome,
            });
            group
                .proof_repetitions
                .sort_by_key(|proof| proof.proof_sha256.to_string());
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), EpisodeError> {
        if self.schema != EPISODE_LEDGER_SCHEMA_V1 {
            return Err(EpisodeError::new("invalid episode ledger schema"));
        }
        for (key, group) in &self.groups {
            if key != &group.input_identity_sha256.to_string()
                || group.episodes.is_empty()
                || group.episodes.iter().any(|episode| {
                    episode.episode_sha256 == Digest::ZERO
                        || episode.occurrences == 0
                        || episode.manifest_paths.is_empty()
                })
                || group.proof_repetitions.iter().any(|proof| {
                    proof.proof_sha256 == Digest::ZERO
                        || proof.occurrences == 0
                        || proof.proof_paths.is_empty()
                        || !valid_name(&proof.worker_id, 192)
                        || !valid_text(&proof.outcome.reason, 2048)
                })
            {
                return Err(EpisodeError::new("episode ledger entry is invalid"));
            }
        }
        Ok(())
    }
}

impl Default for EpisodeLedger {
    fn default() -> Self {
        Self::new()
    }
}

fn scenario_identity(boot: &TapeBoot) -> Result<EpisodeScenarioIdentity, EpisodeError> {
    let id = match boot {
        TapeBoot::Process => "process-boot".into(),
        TapeBoot::Stage {
            stage,
            room,
            point,
            layer,
            save_slot,
            fixture,
        } => format!(
            "stage:{stage}:room:{room}:point:{point}:layer:{layer}:save:{}:fixture:{}",
            save_slot.map_or_else(|| "none".into(), |value| value.to_string()),
            fixture.as_ref().map_or("none", |value| value.name.as_str())
        ),
    };
    Ok(EpisodeScenarioIdentity {
        id,
        digest: canonical_digest(b"dusklight.episode-scenario/v1\0", boot)?,
        boot: boot.clone(),
    })
}

fn canonical_digest<T: Serialize>(domain: &[u8], value: &T) -> Result<Digest, EpisodeError> {
    let encoded =
        serde_json::to_vec(value).map_err(|error| EpisodeError::new(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((encoded.len() as u64).to_le_bytes());
    hasher.update(encoded);
    Ok(Digest(hasher.finalize().into()))
}

fn valid_name(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':' | b'/')
        })
}

fn valid_text(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value
            .chars()
            .all(|character| !character.is_control() || matches!(character, '\n' | '\t'))
}

#[derive(Debug)]
pub struct EpisodeError(String);

impl EpisodeError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for EpisodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for EpisodeError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transition_corpus::{MacroAction, StateReference, Transition};

    fn corpus() -> TransitionCorpus {
        TransitionCorpus::new(
            Digest([1; 32]),
            Digest([2; 32]),
            1,
            vec![Transition {
                source: StateReference {
                    kind: StateReferenceKind::Boundary,
                    digest: Digest([3; 32]),
                },
                state: vec![0.0],
                action: MacroAction {
                    action_id: 1,
                    macro_kind: 1,
                    parameters: Vec::new(),
                },
                duration_ticks: 1,
                reward: -1.0,
                next: StateReference {
                    kind: StateReferenceKind::Boundary,
                    digest: Digest([4; 32]),
                },
                next_state: vec![1.0],
                terminal: true,
            }],
        )
        .unwrap()
    }

    fn context(worker: &str) -> EpisodeContext {
        EpisodeContext {
            schema: EPISODE_CONTEXT_SCHEMA_V1.into(),
            run_identity: None,
            run_build: RunBuildIdentity {
                executable_sha256: Digest([5; 32]),
                dusklight_commit: Some("abc123".into()),
                aurora_commit: None,
                target: Some("aarch64-apple-darwin".into()),
                profile: Some("debug".into()),
                feature_digest: None,
            },
            objective: EpisodeObjectiveIdentity {
                id: "intro-route".into(),
                digest: Digest([6; 32]),
            },
            producer: EpisodeProducerIdentity {
                kind: EpisodeProducerKind::Evolution,
                name: "huntctl".into(),
                version: "1".into(),
            },
            seed: EpisodeSeed::Deterministic { value: 7 },
            worker_id: worker.into(),
            lineage: EpisodeLineage {
                candidate_id: Some("candidate-a".into()),
                parent_candidate_id: Some("candidate-parent".into()),
                generation: 2,
                intervention: Some(EpisodeIntervention {
                    start_frame: 10,
                    end_frame_exclusive: 12,
                    parent_end_frame_exclusive: 12,
                    description: "replace-action".into(),
                }),
            },
            outcome: EpisodeOutcome {
                class: EpisodeOutcomeClass::Successful,
                reason: "objective reached".into(),
            },
        }
    }

    fn manifest(worker: &str) -> EpisodeManifest {
        EpisodeManifest::build(EpisodeManifestBuild {
            context: &context(worker),
            boot: &TapeBoot::Process,
            corpus: &corpus(),
            query_view_id: "movement-state/v2",
            tape_sha256: Digest([7; 32]),
            trace_sha256: Digest([8; 32]),
            transition_evidence_sha256: Digest([9; 32]),
        })
        .unwrap()
    }

    #[test]
    fn complete_identity_round_trips_and_validates() {
        let corpus = corpus();
        let manifest = EpisodeManifest::build(EpisodeManifestBuild {
            context: &context("worker-0"),
            boot: &TapeBoot::Process,
            corpus: &corpus,
            query_view_id: "movement-state/v2",
            tape_sha256: Digest([7; 32]),
            trace_sha256: Digest([8; 32]),
            transition_evidence_sha256: Digest([9; 32]),
        })
        .unwrap();
        let encoded = serde_json::to_vec(&manifest).unwrap();
        let decoded: EpisodeManifest = serde_json::from_slice(&encoded).unwrap();
        decoded.validate(&corpus).unwrap();
        assert_eq!(decoded, manifest);
    }

    #[test]
    fn repetition_worker_changes_episode_but_not_input_identity() {
        let first = manifest("worker-0");
        let second = manifest("worker-1");
        assert_eq!(first.input_identity_sha256, second.input_identity_sha256);
        assert_ne!(first.episode_sha256, second.episode_sha256);
    }

    #[test]
    fn rejects_missing_build_and_unbounded_intervention() {
        let corpus = corpus();
        let mut context = context("worker-0");
        context.run_build.executable_sha256 = Digest::ZERO;
        assert!(
            EpisodeManifest::build(EpisodeManifestBuild {
                context: &context,
                boot: &TapeBoot::Process,
                corpus: &corpus,
                query_view_id: "movement-state/v2",
                tape_sha256: Digest([7; 32]),
                trace_sha256: Digest([8; 32]),
                transition_evidence_sha256: Digest([9; 32]),
            })
            .is_err()
        );
    }

    #[test]
    fn ledger_deduplicates_exact_episodes_and_retains_repetition_proofs() {
        let first = manifest("worker-0");
        let second = manifest("worker-1");
        let mut ledger = EpisodeLedger::new();
        ledger.ingest_episode(&first, "first/episode.json".into());
        ledger.ingest_episode(&first, "copy/episode.json".into());
        ledger.ingest_episode(&second, "second/episode.json".into());
        ledger
            .ingest_proof(
                first.input_identity_sha256,
                Digest([10; 32]),
                "first/attempt.json".into(),
                "worker-0".into(),
                1,
                first.outcome.clone(),
            )
            .unwrap();
        ledger
            .ingest_proof(
                first.input_identity_sha256,
                Digest([11; 32]),
                "second/attempt.json".into(),
                "worker-1".into(),
                2,
                second.outcome.clone(),
            )
            .unwrap();
        ledger.validate().unwrap();
        let group = ledger.groups.values().next().unwrap();
        assert_eq!(group.episodes.len(), 2);
        assert_eq!(
            group
                .episodes
                .iter()
                .map(|item| item.occurrences)
                .sum::<u32>(),
            3
        );
        assert_eq!(group.proof_repetitions.len(), 2);
    }
}
