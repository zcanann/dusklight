//! Immutable replay-corpus generations over authenticated native episode shards.
//!
//! The manifest classifies how experience was collected without rewriting the
//! rich `.dseps` payload. Shard/episode identities remain authoritative, and
//! checkpoint, branch, objective, policy-generation, and outcome lineage stay
//! available to later representation and control experiments.

use crate::artifact::Digest;
use dusklight_evidence::native_episode_shard::NativeEpisodeShard;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const NATIVE_REPLAY_CORPUS_SCHEMA_V1: &str = "dusklight-native-replay-corpus/v1";
pub const NATIVE_REPLAY_ENTRY_SCHEMA_V1: &str = "dusklight-native-replay-entry/v1";
const MAX_REPLAY_ENTRIES: usize = 1_000_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayExperienceRole {
    Demonstration,
    PolicyRollout,
    RandomizedCoverage,
    AlternateTerminal,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeReplayEntry {
    pub schema: String,
    pub entry_sha256: Digest,
    pub shard_sha256: Digest,
    pub episode_id: String,
    pub episode_payload_xxh3_128: String,
    pub role: ReplayExperienceRole,
    pub success: bool,
    pub ticks_executed: u32,
    pub first_hit_tick: Option<u32>,
    pub source_frame: u64,
    pub source_boundary_fingerprint: String,
    pub checkpoint_identity: String,
    pub objective: String,
    pub objective_identity: String,
    pub policy_lineage_sha256: Option<Digest>,
    pub parent_entry_sha256: Option<Digest>,
}

impl NativeReplayEntry {
    fn build(source: ReplayEpisodeSource<'_>) -> Result<Self, NativeReplayCorpusError> {
        let episode = source
            .shard
            .episodes
            .get(source.episode_index)
            .ok_or_else(|| NativeReplayCorpusError::new("replay episode index is invalid"))?;
        if source.role == ReplayExperienceRole::Demonstration && !episode.success {
            return Err(NativeReplayCorpusError::new(
                "demonstration replay entry is not a successful episode",
            ));
        }
        if (source.role == ReplayExperienceRole::PolicyRollout)
            != source.policy_lineage_sha256.is_some()
        {
            return Err(NativeReplayCorpusError::new(
                "policy rollout role and policy lineage presence disagree",
            ));
        }
        let mut entry = Self {
            schema: NATIVE_REPLAY_ENTRY_SCHEMA_V1.into(),
            entry_sha256: Digest::ZERO,
            shard_sha256: source.shard.content_sha256,
            episode_id: episode.id.clone(),
            episode_payload_xxh3_128: hex_128(episode.payload_xxh3_128),
            role: source.role,
            success: episode.success,
            ticks_executed: episode.ticks_executed,
            first_hit_tick: episode.first_hit_tick,
            source_frame: source.shard.source_frame,
            source_boundary_fingerprint: source.shard.metadata.source_boundary_fingerprint.clone(),
            checkpoint_identity: source.shard.metadata.checkpoint_identity.clone(),
            objective: source.shard.metadata.objective.clone(),
            objective_identity: source.shard.metadata.objective_identity.clone(),
            policy_lineage_sha256: source.policy_lineage_sha256,
            parent_entry_sha256: source.parent_entry_sha256,
        };
        entry.entry_sha256 = entry.digest()?;
        entry.validate()?;
        Ok(entry)
    }

    pub fn validate(&self) -> Result<(), NativeReplayCorpusError> {
        if self.schema != NATIVE_REPLAY_ENTRY_SCHEMA_V1
            || self.shard_sha256 == Digest::ZERO
            || self.episode_id.is_empty()
            || self.episode_id.len() > 128
            || self.episode_payload_xxh3_128.len() != 32
            || !is_lower_hex(&self.episode_payload_xxh3_128)
            || self.ticks_executed == 0
            || self
                .first_hit_tick
                .is_some_and(|tick| tick.checked_add(1) != Some(self.ticks_executed))
            || self.success != self.first_hit_tick.is_some()
            || self.source_boundary_fingerprint.len() != 32
            || !is_lower_hex(&self.source_boundary_fingerprint)
            || self.checkpoint_identity.is_empty()
            || self.objective.is_empty()
            || self.objective_identity.is_empty()
            || (self.role == ReplayExperienceRole::Demonstration && !self.success)
            || ((self.role == ReplayExperienceRole::PolicyRollout)
                != self.policy_lineage_sha256.is_some())
            || self.entry_sha256 != self.digest()?
        {
            return Err(NativeReplayCorpusError::new(
                "native replay entry is invalid or detached",
            ));
        }
        Ok(())
    }

    fn digest(&self) -> Result<Digest, NativeReplayCorpusError> {
        canonical_digest(
            b"dusklight.native-replay-entry/v1\0",
            &(
                &self.schema,
                self.shard_sha256,
                &self.episode_id,
                &self.episode_payload_xxh3_128,
                self.role,
                self.success,
                self.ticks_executed,
                self.first_hit_tick,
                self.source_frame,
                &self.source_boundary_fingerprint,
                &self.checkpoint_identity,
                &self.objective,
                &self.objective_identity,
                self.policy_lineage_sha256,
                self.parent_entry_sha256,
            ),
        )
    }
}

#[derive(Clone, Copy)]
pub struct ReplayEpisodeSource<'a> {
    pub shard: &'a NativeEpisodeShard,
    pub episode_index: usize,
    pub role: ReplayExperienceRole,
    pub policy_lineage_sha256: Option<Digest>,
    pub parent_entry_sha256: Option<Digest>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeReplayCorpusReport {
    pub entries: usize,
    pub transitions: u64,
    pub successes: usize,
    pub failures: usize,
    pub roles: BTreeMap<ReplayExperienceRole, usize>,
    pub objectives: usize,
    pub checkpoints: usize,
    pub policy_lineages: usize,
    pub branch_edges: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeReplayCorpus {
    pub schema: String,
    pub generation: u32,
    pub parent_corpus_sha256: Option<Digest>,
    pub observation_schema: String,
    pub action_schema: String,
    pub entries: Vec<NativeReplayEntry>,
    pub report: NativeReplayCorpusReport,
    pub corpus_sha256: Digest,
}

impl NativeReplayCorpus {
    pub fn build(
        previous: Option<&Self>,
        additions: &[ReplayEpisodeSource<'_>],
    ) -> Result<Self, NativeReplayCorpusError> {
        if additions.is_empty() {
            return Err(NativeReplayCorpusError::new(
                "native replay generation has no additions",
            ));
        }
        if let Some(previous) = previous {
            previous.validate()?;
        }
        let generation = previous
            .map(|value| value.generation.checked_add(1))
            .unwrap_or(Some(1))
            .ok_or_else(|| NativeReplayCorpusError::new("replay generation overflowed"))?;
        let parent_corpus_sha256 = previous.map(|value| value.corpus_sha256);
        let mut entries = previous
            .map(|value| value.entries.clone())
            .unwrap_or_default();
        let mut observation_schema = previous.map(|value| value.observation_schema.clone());
        let mut action_schema = previous.map(|value| value.action_schema.clone());
        for source in additions {
            let shard = source.shard;
            if observation_schema
                .as_ref()
                .is_some_and(|schema| schema != &shard.metadata.observation_schema)
                || action_schema
                    .as_ref()
                    .is_some_and(|schema| schema != &shard.metadata.action_schema)
            {
                return Err(NativeReplayCorpusError::new(
                    "native replay generation mixes observation or action schemas",
                ));
            }
            observation_schema.get_or_insert_with(|| shard.metadata.observation_schema.clone());
            action_schema.get_or_insert_with(|| shard.metadata.action_schema.clone());
            entries.push(NativeReplayEntry::build(*source)?);
        }
        entries.sort_by_key(|entry| entry.entry_sha256);
        let report = report(&entries)?;
        let mut corpus = Self {
            schema: NATIVE_REPLAY_CORPUS_SCHEMA_V1.into(),
            generation,
            parent_corpus_sha256,
            observation_schema: observation_schema.expect("additions establish schema"),
            action_schema: action_schema.expect("additions establish schema"),
            entries,
            report,
            corpus_sha256: Digest::ZERO,
        };
        corpus.corpus_sha256 = corpus.digest()?;
        corpus.validate()?;
        Ok(corpus)
    }

    pub fn validate(&self) -> Result<(), NativeReplayCorpusError> {
        if self.schema != NATIVE_REPLAY_CORPUS_SCHEMA_V1
            || self.generation == 0
            || (self.generation == 1) != self.parent_corpus_sha256.is_none()
            || self.observation_schema.is_empty()
            || self.action_schema.is_empty()
            || self.entries.is_empty()
            || self.entries.len() > MAX_REPLAY_ENTRIES
            || !self
                .entries
                .windows(2)
                .all(|pair| pair[0].entry_sha256 < pair[1].entry_sha256)
            || self.entries.iter().any(|entry| entry.validate().is_err())
            || self.report != report(&self.entries)?
            || self.corpus_sha256 != self.digest()?
        {
            return Err(NativeReplayCorpusError::new(
                "native replay corpus is invalid or detached",
            ));
        }
        let identities = self
            .entries
            .iter()
            .map(|entry| entry.entry_sha256)
            .collect::<BTreeSet<_>>();
        let source_identities = self
            .entries
            .iter()
            .map(|entry| {
                (
                    entry.shard_sha256,
                    entry.episode_id.as_str(),
                    entry.episode_payload_xxh3_128.as_str(),
                )
            })
            .collect::<BTreeSet<_>>();
        if source_identities.len() != self.entries.len() {
            return Err(NativeReplayCorpusError::new(
                "native replay corpus duplicates an authenticated episode",
            ));
        }
        if self.entries.iter().any(|entry| {
            entry
                .parent_entry_sha256
                .is_some_and(|parent| !identities.contains(&parent) || parent == entry.entry_sha256)
        }) {
            return Err(NativeReplayCorpusError::new(
                "native replay branch parent is absent or self-referential",
            ));
        }
        Ok(())
    }

    fn digest(&self) -> Result<Digest, NativeReplayCorpusError> {
        canonical_digest(
            b"dusklight.native-replay-corpus/v1\0",
            &(
                &self.schema,
                self.generation,
                self.parent_corpus_sha256,
                &self.observation_schema,
                &self.action_schema,
                &self.entries,
                &self.report,
            ),
        )
    }
}

fn report(
    entries: &[NativeReplayEntry],
) -> Result<NativeReplayCorpusReport, NativeReplayCorpusError> {
    let transitions = entries.iter().try_fold(0_u64, |total, entry| {
        total
            .checked_add(u64::from(entry.ticks_executed))
            .ok_or_else(|| {
                NativeReplayCorpusError::new("native replay transition count overflowed")
            })
    })?;
    let mut roles = BTreeMap::new();
    for entry in entries {
        *roles.entry(entry.role).or_default() += 1;
    }
    Ok(NativeReplayCorpusReport {
        entries: entries.len(),
        transitions,
        successes: entries.iter().filter(|entry| entry.success).count(),
        failures: entries.iter().filter(|entry| !entry.success).count(),
        roles,
        objectives: entries
            .iter()
            .map(|entry| entry.objective_identity.as_str())
            .collect::<BTreeSet<_>>()
            .len(),
        checkpoints: entries
            .iter()
            .map(|entry| entry.checkpoint_identity.as_str())
            .collect::<BTreeSet<_>>()
            .len(),
        policy_lineages: entries
            .iter()
            .filter_map(|entry| entry.policy_lineage_sha256)
            .collect::<BTreeSet<_>>()
            .len(),
        branch_edges: entries
            .iter()
            .filter(|entry| entry.parent_entry_sha256.is_some())
            .count(),
    })
}

fn hex_128(value: [u8; 16]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn is_lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn canonical_digest<T: Serialize>(
    domain: &[u8],
    value: &T,
) -> Result<Digest, NativeReplayCorpusError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| NativeReplayCorpusError::new(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeReplayCorpusError(String);

impl NativeReplayCorpusError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for NativeReplayCorpusError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeReplayCorpusError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn shard() -> NativeEpisodeShard {
        NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v14.dseps"
        ))
        .unwrap()
    }

    fn source<'a>(
        shard: &'a NativeEpisodeShard,
        episode_index: usize,
        role: ReplayExperienceRole,
        policy: Option<Digest>,
        parent: Option<Digest>,
    ) -> ReplayEpisodeSource<'a> {
        ReplayEpisodeSource {
            shard,
            episode_index,
            role,
            policy_lineage_sha256: policy,
            parent_entry_sha256: parent,
        }
    }

    #[test]
    fn generations_retain_mixed_experience_and_exact_lineage() {
        let shard = shard();
        let success = shard
            .episodes
            .iter()
            .position(|episode| episode.success)
            .unwrap();
        let failure = shard
            .episodes
            .iter()
            .position(|episode| !episode.success)
            .unwrap();
        let first = NativeReplayCorpus::build(
            None,
            &[source(
                &shard,
                success,
                ReplayExperienceRole::Demonstration,
                None,
                None,
            )],
        )
        .unwrap();
        assert_eq!(first.generation, 1);
        assert_eq!(first.report.successes, 1);
        assert_eq!(first.report.failures, 0);
        let parent = first.entries[0].entry_sha256;
        let second = NativeReplayCorpus::build(
            Some(&first),
            &[source(
                &shard,
                failure,
                ReplayExperienceRole::PolicyRollout,
                Some(Digest([9; 32])),
                Some(parent),
            )],
        )
        .unwrap();
        assert_eq!(second.generation, 2);
        assert_eq!(second.parent_corpus_sha256, Some(first.corpus_sha256));
        assert_eq!(second.report.entries, 2);
        assert_eq!(second.report.failures, 1);
        assert_eq!(second.report.policy_lineages, 1);
        assert_eq!(second.report.branch_edges, 1);
        second.validate().unwrap();

        // Reusing the exact episode under another role is still duplicate
        // experience and must not inflate the replay corpus.
        assert!(
            NativeReplayCorpus::build(
                Some(&second),
                &[source(
                    &shard,
                    failure,
                    ReplayExperienceRole::RandomizedCoverage,
                    None,
                    None,
                )],
            )
            .is_err()
        );
    }

    #[test]
    fn role_schema_and_branch_tampering_fail_closed() {
        let shard = shard();
        let failure = shard
            .episodes
            .iter()
            .position(|episode| !episode.success)
            .unwrap();
        assert!(
            NativeReplayCorpus::build(
                None,
                &[source(
                    &shard,
                    failure,
                    ReplayExperienceRole::Demonstration,
                    None,
                    None,
                )],
            )
            .is_err()
        );
        assert!(
            NativeReplayCorpus::build(
                None,
                &[source(
                    &shard,
                    failure,
                    ReplayExperienceRole::PolicyRollout,
                    None,
                    None,
                )],
            )
            .is_err()
        );
        assert!(
            NativeReplayCorpus::build(
                None,
                &[source(
                    &shard,
                    failure,
                    ReplayExperienceRole::AlternateTerminal,
                    None,
                    Some(Digest([4; 32])),
                )],
            )
            .is_err()
        );
    }
}
