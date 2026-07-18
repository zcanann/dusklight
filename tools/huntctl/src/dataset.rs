//! Episode-grouped dataset splits, leakage checks, and training-only statistics.

use crate::artifact::Digest;
use crate::episode::{EpisodeManifest, EpisodeOutcomeClass};
use crate::tape::InputTape;
use crate::transition_corpus::TransitionCorpus;
use crate::transition_evidence::{EvidenceAvailability, TransitionEvidenceBundle};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

pub const DATASET_MANIFEST_SCHEMA_V1: &str = "dusklight-dataset-manifest/v1";
pub const MAX_DATASET_EPISODES: usize = 4096;
pub const DATASET_SOURCE_SCHEMA_V1: &str = "dusklight-dataset-source/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DatasetSplit {
    Train,
    Validation,
    Test,
    Withheld,
}

#[derive(Clone, Debug)]
pub struct DatasetSource {
    pub source_id: String,
    pub episode: EpisodeManifest,
    pub corpus: TransitionCorpus,
    pub tape: InputTape,
    pub evidence: TransitionEvidenceBundle,
    pub route_family: String,
    pub screenshot_sha256: Vec<Digest>,
    pub checkpoint_sha256: Vec<Digest>,
    pub transition_corpus_path: PathBuf,
    pub gameplay_trace_path: PathBuf,
    pub absolute_tape_path: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetSourceDescriptor {
    pub schema: String,
    pub source_id: String,
    pub episode_manifest: PathBuf,
    pub transition_corpus: PathBuf,
    pub absolute_tape: PathBuf,
    pub transition_evidence: PathBuf,
    pub gameplay_trace: PathBuf,
    pub route_family: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub screenshot_sha256: Vec<Digest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checkpoint_sha256: Vec<Digest>,
}

impl DatasetSourceDescriptor {
    pub fn load(&self, descriptor_directory: &Path) -> Result<DatasetSource, DatasetError> {
        if self.schema != DATASET_SOURCE_SCHEMA_V1 {
            return Err(DatasetError::new("invalid dataset source schema"));
        }
        let resolve = |path: &Path| {
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                descriptor_directory.join(path)
            }
        };
        let episode = serde_json::from_slice(&fs::read(resolve(&self.episode_manifest))?)
            .map_err(|error| DatasetError::new(error.to_string()))?;
        let transition_corpus_path = resolve(&self.transition_corpus);
        let corpus = TransitionCorpus::read_zstd_file(&transition_corpus_path)
            .map_err(|error| DatasetError::new(error.to_string()))?;
        let absolute_tape_path = resolve(&self.absolute_tape);
        let tape = crate::tape::InputTape::decode(&fs::read(&absolute_tape_path)?)
            .map_err(|error| DatasetError::new(error.to_string()))?
            .tape;
        let evidence = serde_json::from_slice(&fs::read(resolve(&self.transition_evidence))?)
            .map_err(|error| DatasetError::new(error.to_string()))?;
        Ok(DatasetSource {
            source_id: self.source_id.clone(),
            episode,
            corpus,
            tape,
            evidence,
            route_family: self.route_family.clone(),
            screenshot_sha256: self.screenshot_sha256.clone(),
            checkpoint_sha256: self.checkpoint_sha256.clone(),
            transition_corpus_path,
            gameplay_trace_path: resolve(&self.gameplay_trace),
            absolute_tape_path,
        })
    }
}

#[derive(Clone, Debug)]
pub struct DatasetBuildConfig {
    pub validation_percent: u8,
    pub test_percent: u8,
    pub withheld_objectives: BTreeSet<String>,
}

impl Default for DatasetBuildConfig {
    fn default() -> Self {
        Self {
            validation_percent: 10,
            test_percent: 10,
            withheld_objectives: BTreeSet::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetManifest {
    pub schema: String,
    pub dataset_sha256: Digest,
    pub frozen_withheld_sha256: Digest,
    pub entries: Vec<DatasetEntry>,
    pub report: DatasetReport,
    pub normalization: Vec<NormalizationStatistics>,
    pub leakage: LeakageAudit,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetEntry {
    pub source_id: String,
    pub split: DatasetSplit,
    pub input_identity_sha256: Digest,
    pub episode_sha256: Digest,
    pub scenario_sha256: Digest,
    pub parent_boundary_sha256: Digest,
    pub tape_sha256: Digest,
    pub corpus_sha256: Digest,
    pub feature_schema_sha256: Digest,
    pub action_schema_sha256: Digest,
    pub objective_id: String,
    pub route_family: String,
    pub candidate_id: Option<String>,
    pub parent_candidate_id: Option<String>,
    pub outcome: EpisodeOutcomeClass,
    pub transitions: u64,
    pub transition_corpus: PathBuf,
    pub leakage_group_sha256: Digest,
    pub screenshot_sha256: Vec<Digest>,
    pub checkpoint_sha256: Vec<Digest>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetReport {
    pub unique_episodes: usize,
    pub unique_inputs: usize,
    pub effective_decisions: u64,
    pub split_episodes: BTreeMap<DatasetSplit, u64>,
    pub action_support: BTreeMap<u32, u64>,
    pub state_coverage_bins: usize,
    pub missingness: BTreeMap<String, u64>,
    pub outcome_imbalance: BTreeMap<String, u64>,
    pub boundary_diversity: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizationStatistics {
    pub schema: String,
    pub feature_schema_sha256: Digest,
    pub training_episode_sha256: Vec<Digest>,
    pub sample_count: u64,
    pub means: Vec<f64>,
    pub standard_deviations: Vec<f64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LeakageAudit {
    pub compared_episode_pairs: u64,
    pub grouped_by_scenario: u64,
    pub grouped_by_parent_boundary: u64,
    pub grouped_by_route_family: u64,
    pub grouped_by_exact_tape: u64,
    pub grouped_by_tape_prefix: u64,
    pub grouped_by_checkpoint: u64,
    pub grouped_by_screenshot: u64,
    pub grouped_by_continuation_ancestry: u64,
    pub cross_split_violations: u64,
}

impl DatasetManifest {
    pub fn build(
        sources: &[DatasetSource],
        config: &DatasetBuildConfig,
    ) -> Result<Self, DatasetError> {
        validate_inputs(sources, config)?;
        let mut groups = DisjointGroups::new(sources.len());
        let mut leakage = LeakageAudit {
            compared_episode_pairs: 0,
            grouped_by_scenario: 0,
            grouped_by_parent_boundary: 0,
            grouped_by_route_family: 0,
            grouped_by_exact_tape: 0,
            grouped_by_tape_prefix: 0,
            grouped_by_checkpoint: 0,
            grouped_by_screenshot: 0,
            grouped_by_continuation_ancestry: 0,
            cross_split_violations: 0,
        };
        for left in 0..sources.len() {
            for right in left + 1..sources.len() {
                leakage.compared_episode_pairs += 1;
                let a = &sources[left];
                let b = &sources[right];
                let scenario = a.episode.scenario.digest == b.episode.scenario.digest;
                let parent = a.parent_digest() == b.parent_digest();
                let route = a.route_family == b.route_family;
                let exact_tape = a.episode.artifacts.absolute_tape_sha256
                    == b.episode.artifacts.absolute_tape_sha256;
                let prefix = tape_is_prefix(&a.tape, &b.tape) || tape_is_prefix(&b.tape, &a.tape);
                let checkpoint = intersects(&a.checkpoint_sha256, &b.checkpoint_sha256);
                let screenshot = intersects(&a.screenshot_sha256, &b.screenshot_sha256);
                let ancestry = continuation_related(&a.episode, &b.episode);
                leakage.grouped_by_scenario += u64::from(scenario);
                leakage.grouped_by_parent_boundary += u64::from(parent);
                leakage.grouped_by_route_family += u64::from(route);
                leakage.grouped_by_exact_tape += u64::from(exact_tape);
                leakage.grouped_by_tape_prefix += u64::from(prefix);
                leakage.grouped_by_checkpoint += u64::from(checkpoint);
                leakage.grouped_by_screenshot += u64::from(screenshot);
                leakage.grouped_by_continuation_ancestry += u64::from(ancestry);
                if scenario
                    || parent
                    || route
                    || exact_tape
                    || prefix
                    || checkpoint
                    || screenshot
                    || ancestry
                {
                    groups.union(left, right);
                }
            }
        }
        let mut components = BTreeMap::<usize, Vec<usize>>::new();
        for index in 0..sources.len() {
            components
                .entry(groups.find(index))
                .or_default()
                .push(index);
        }
        let mut splits = vec![DatasetSplit::Train; sources.len()];
        let mut group_ids = vec![Digest::ZERO; sources.len()];
        for component in components.values() {
            let group_id = component_identity(component, sources);
            let withheld = component.iter().any(|index| {
                config
                    .withheld_objectives
                    .contains(&sources[*index].episode.objective.id)
            });
            let split = if withheld {
                DatasetSplit::Withheld
            } else {
                component_split(group_id, config)
            };
            for index in component {
                splits[*index] = split;
                group_ids[*index] = group_id;
            }
        }
        for left in 0..sources.len() {
            for right in left + 1..sources.len() {
                if groups.find(left) == groups.find(right) && splits[left] != splits[right] {
                    leakage.cross_split_violations += 1;
                }
            }
        }
        if leakage.cross_split_violations != 0 {
            return Err(DatasetError::new("related episodes crossed dataset splits"));
        }
        let mut entries = Vec::with_capacity(sources.len());
        for (index, (source, split)) in sources.iter().zip(&splits).enumerate() {
            entries.push(DatasetEntry {
                source_id: source.source_id.clone(),
                split: *split,
                input_identity_sha256: source.episode.input_identity_sha256,
                episode_sha256: source.episode.episode_sha256,
                scenario_sha256: source.episode.scenario.digest,
                parent_boundary_sha256: source.parent_digest(),
                tape_sha256: source.episode.artifacts.absolute_tape_sha256,
                corpus_sha256: source.episode.artifacts.transition_corpus_sha256,
                feature_schema_sha256: source.episode.query_view.schema_sha256,
                action_schema_sha256: source.episode.action_schema_sha256,
                objective_id: source.episode.objective.id.clone(),
                route_family: source.route_family.clone(),
                candidate_id: source.episode.lineage.candidate_id.clone(),
                parent_candidate_id: source.episode.lineage.parent_candidate_id.clone(),
                outcome: source.episode.outcome.class,
                transitions: source.corpus.transitions.len() as u64,
                transition_corpus: source.transition_corpus_path.clone(),
                leakage_group_sha256: group_ids[index],
                screenshot_sha256: source.screenshot_sha256.clone(),
                checkpoint_sha256: source.checkpoint_sha256.clone(),
            });
        }
        entries.sort_by_key(|entry| entry.episode_sha256.to_string());
        let report = dataset_report(sources, &splits);
        let normalization = normalization_statistics(sources, &splits)?;
        let frozen_withheld_sha256 = canonical_digest(
            b"dusklight.frozen-withheld/v1\0",
            &entries
                .iter()
                .filter(|entry| entry.split == DatasetSplit::Withheld)
                .map(|entry| entry.episode_sha256)
                .collect::<Vec<_>>(),
        )?;
        let mut manifest = Self {
            schema: DATASET_MANIFEST_SCHEMA_V1.into(),
            dataset_sha256: Digest::ZERO,
            frozen_withheld_sha256,
            entries,
            report,
            normalization,
            leakage,
        };
        manifest.dataset_sha256 = manifest.compute_identity()?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), DatasetError> {
        if self.schema != DATASET_MANIFEST_SCHEMA_V1
            || self.entries.is_empty()
            || self.entries.len() > MAX_DATASET_EPISODES
            || self.leakage.cross_split_violations != 0
            || self.dataset_sha256 != self.compute_identity()?
            || self
                .normalization
                .iter()
                .any(|stats| stats.schema != "dusklight-normalization/v1")
        {
            return Err(DatasetError::new("dataset manifest is invalid"));
        }
        let mut episode_ids = BTreeSet::new();
        let mut source_ids = BTreeSet::new();
        for entry in &self.entries {
            if entry.leakage_group_sha256 == Digest::ZERO
                || !episode_ids.insert(entry.episode_sha256)
                || !source_ids.insert(entry.source_id.as_str())
            {
                return Err(DatasetError::new("dataset entry identity is invalid"));
            }
        }
        for left in 0..self.entries.len() {
            for right in left + 1..self.entries.len() {
                let a = &self.entries[left];
                let b = &self.entries[right];
                let related = a.leakage_group_sha256 == b.leakage_group_sha256
                    || a.scenario_sha256 == b.scenario_sha256
                    || a.parent_boundary_sha256 == b.parent_boundary_sha256
                    || a.route_family == b.route_family
                    || a.tape_sha256 == b.tape_sha256
                    || intersects(&a.checkpoint_sha256, &b.checkpoint_sha256)
                    || intersects(&a.screenshot_sha256, &b.screenshot_sha256)
                    || entry_continuation_related(a, b);
                if related && a.split != b.split {
                    return Err(DatasetError::new("dataset contains cross-split leakage"));
                }
            }
        }
        for statistics in &self.normalization {
            let expected: BTreeSet<_> = self
                .entries
                .iter()
                .filter(|entry| {
                    entry.split == DatasetSplit::Train
                        && entry.feature_schema_sha256 == statistics.feature_schema_sha256
                })
                .map(|entry| entry.episode_sha256)
                .collect();
            if expected != statistics.training_episode_sha256.iter().copied().collect() {
                return Err(DatasetError::new(
                    "normalization statistics include non-training episodes",
                ));
            }
        }
        let withheld = self
            .entries
            .iter()
            .filter(|entry| entry.split == DatasetSplit::Withheld)
            .map(|entry| entry.episode_sha256)
            .collect::<Vec<_>>();
        if self.frozen_withheld_sha256
            != canonical_digest(b"dusklight.frozen-withheld/v1\0", &withheld)?
        {
            return Err(DatasetError::new("frozen withheld identity mismatch"));
        }
        Ok(())
    }

    fn compute_identity(&self) -> Result<Digest, DatasetError> {
        canonical_digest(
            b"dusklight.dataset-manifest/v1\0",
            &(
                &self.schema,
                self.frozen_withheld_sha256,
                &self.entries,
                &self.report,
                &self.normalization,
                &self.leakage,
            ),
        )
    }
}

impl DatasetSource {
    fn parent_digest(&self) -> Digest {
        self.episode.parent_boundary.digest
    }
}

fn validate_inputs(
    sources: &[DatasetSource],
    config: &DatasetBuildConfig,
) -> Result<(), DatasetError> {
    if sources.is_empty()
        || sources.len() > MAX_DATASET_EPISODES
        || config.validation_percent > 100
        || config.test_percent > 100
        || u16::from(config.validation_percent) + u16::from(config.test_percent) >= 100
    {
        return Err(DatasetError::new("invalid dataset build configuration"));
    }
    let mut source_ids = BTreeSet::new();
    for source in sources {
        source
            .episode
            .validate(&source.corpus)
            .map_err(|error| DatasetError::new(error.to_string()))?;
        source
            .evidence
            .validate(&source.corpus)
            .map_err(|error| DatasetError::new(error.to_string()))?;
        let tape_bytes = source
            .tape
            .encode()
            .map_err(|error| DatasetError::new(error.to_string()))?;
        if source.source_id.is_empty()
            || source.route_family.is_empty()
            || !source_ids.insert(source.source_id.as_str())
            || Digest(Sha256::digest(&tape_bytes).into())
                != source.episode.artifacts.absolute_tape_sha256
            || source.evidence.corpus_sha256 != source.episode.artifacts.transition_corpus_sha256
        {
            return Err(DatasetError::new("dataset source identity mismatch"));
        }
    }
    Ok(())
}

fn component_identity(component: &[usize], sources: &[DatasetSource]) -> Digest {
    let mut identities: Vec<_> = component
        .iter()
        .map(|index| sources[*index].episode.input_identity_sha256.to_string())
        .collect();
    identities.sort();
    Digest(Sha256::digest(identities.join("\0").as_bytes()).into())
}

fn component_split(component_identity: Digest, config: &DatasetBuildConfig) -> DatasetSplit {
    let bucket = u16::from_le_bytes([
        component_identity.as_bytes()[0],
        component_identity.as_bytes()[1],
    ]) % 100;
    if bucket < u16::from(config.test_percent) {
        DatasetSplit::Test
    } else if bucket < u16::from(config.test_percent + config.validation_percent) {
        DatasetSplit::Validation
    } else {
        DatasetSplit::Train
    }
}

fn dataset_report(sources: &[DatasetSource], splits: &[DatasetSplit]) -> DatasetReport {
    let mut inputs = BTreeSet::new();
    let mut split_episodes = BTreeMap::new();
    let mut action_support = BTreeMap::new();
    let mut state_bins = BTreeSet::new();
    let mut missingness = BTreeMap::new();
    let mut outcomes = BTreeMap::new();
    let mut boundaries = BTreeSet::new();
    let mut effective_decisions = 0_u64;
    for (source, split) in sources.iter().zip(splits) {
        inputs.insert(source.episode.input_identity_sha256);
        *split_episodes.entry(*split).or_default() += 1;
        boundaries.insert(source.parent_digest());
        *outcomes
            .entry(format!("{:?}", source.episode.outcome.class).to_lowercase())
            .or_default() += 1;
        for transition in &source.corpus.transitions {
            effective_decisions += 1;
            *action_support
                .entry(transition.action.action_id)
                .or_default() += 1;
            let mut hasher = Sha256::new();
            for value in transition.state.iter().take(24) {
                hasher.update(((value * 16.0).round() as i32).to_le_bytes());
            }
            state_bins.insert(Digest(hasher.finalize().into()));
        }
        for transition in &source.evidence.transitions {
            for availability in [
                source.evidence.event_side_table[transition.event.pre_action as usize].availability,
                source.evidence.event_side_table[transition.event.post_action as usize]
                    .availability,
                source.evidence.entity_side_table[transition.entities.pre_action as usize]
                    .availability,
                source.evidence.entity_side_table[transition.entities.post_action as usize]
                    .availability,
                transition.predicate.pre_action.availability,
                transition.predicate.post_action.availability,
            ] {
                *missingness
                    .entry(availability_name(availability).into())
                    .or_default() += 1;
            }
        }
    }
    DatasetReport {
        unique_episodes: sources.len(),
        unique_inputs: inputs.len(),
        effective_decisions,
        split_episodes,
        action_support,
        state_coverage_bins: state_bins.len(),
        missingness,
        outcome_imbalance: outcomes,
        boundary_diversity: boundaries.len(),
    }
}

fn normalization_statistics(
    sources: &[DatasetSource],
    splits: &[DatasetSplit],
) -> Result<Vec<NormalizationStatistics>, DatasetError> {
    let mut groups = BTreeMap::<Digest, Vec<usize>>::new();
    for (index, (source, split)) in sources.iter().zip(splits).enumerate() {
        if *split == DatasetSplit::Train {
            groups
                .entry(source.corpus.feature_schema)
                .or_default()
                .push(index);
        }
    }
    let mut output = Vec::new();
    for (schema, indices) in groups {
        let width = sources[indices[0]].corpus.feature_count as usize;
        let mut count = 0_u64;
        let mut means = vec![0.0_f64; width];
        let mut m2 = vec![0.0_f64; width];
        let mut episodes = Vec::new();
        for index in indices {
            let source = &sources[index];
            episodes.push(source.episode.episode_sha256);
            for state in source
                .corpus
                .transitions
                .iter()
                .flat_map(|transition| [&transition.state, &transition.next_state])
            {
                count += 1;
                for (feature, value) in state.iter().enumerate() {
                    let value = f64::from(*value);
                    let delta = value - means[feature];
                    means[feature] += delta / count as f64;
                    m2[feature] += delta * (value - means[feature]);
                }
            }
        }
        episodes.sort_by_key(ToString::to_string);
        let standard_deviations = m2
            .into_iter()
            .map(|sum| {
                if count > 1 {
                    (sum / (count - 1) as f64).sqrt()
                } else {
                    0.0
                }
            })
            .collect();
        output.push(NormalizationStatistics {
            schema: "dusklight-normalization/v1".into(),
            feature_schema_sha256: schema,
            training_episode_sha256: episodes,
            sample_count: count,
            means,
            standard_deviations,
        });
    }
    Ok(output)
}

fn tape_is_prefix(prefix: &InputTape, full: &InputTape) -> bool {
    prefix.boot == full.boot
        && prefix.tick_rate_numerator == full.tick_rate_numerator
        && prefix.tick_rate_denominator == full.tick_rate_denominator
        && prefix.frames.len() <= full.frames.len()
        && prefix.frames == full.frames[..prefix.frames.len()]
}

fn intersects(left: &[Digest], right: &[Digest]) -> bool {
    left.iter().any(|item| right.contains(item))
}

fn continuation_related(left: &EpisodeManifest, right: &EpisodeManifest) -> bool {
    let ids = [
        left.lineage.candidate_id.as_deref(),
        left.lineage.parent_candidate_id.as_deref(),
    ];
    [
        right.lineage.candidate_id.as_deref(),
        right.lineage.parent_candidate_id.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|candidate| ids.into_iter().flatten().any(|left| left == candidate))
}

fn entry_continuation_related(left: &DatasetEntry, right: &DatasetEntry) -> bool {
    let left_ids = [
        left.candidate_id.as_deref(),
        left.parent_candidate_id.as_deref(),
    ];
    [
        right.candidate_id.as_deref(),
        right.parent_candidate_id.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|candidate| left_ids.into_iter().flatten().any(|left| left == candidate))
}

fn availability_name(value: EvidenceAvailability) -> &'static str {
    match value {
        EvidenceAvailability::Present => "present",
        EvidenceAvailability::Absent => "absent",
        EvidenceAvailability::Unavailable => "unavailable",
        EvidenceAvailability::Truncated => "truncated",
        EvidenceAvailability::Unrequested => "unrequested",
    }
}

fn canonical_digest<T: Serialize>(domain: &[u8], value: &T) -> Result<Digest, DatasetError> {
    let bytes = serde_json::to_vec(value).map_err(|error| DatasetError::new(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

struct DisjointGroups {
    parents: Vec<usize>,
}

impl DisjointGroups {
    fn new(count: usize) -> Self {
        Self {
            parents: (0..count).collect(),
        }
    }

    fn find(&mut self, value: usize) -> usize {
        let mut root = value;
        while self.parents[root] != root {
            root = self.parents[root];
        }
        let mut current = value;
        while self.parents[current] != current {
            let next = self.parents[current];
            self.parents[current] = root;
            current = next;
        }
        root
    }

    fn union(&mut self, left: usize, right: usize) {
        let left = self.find(left);
        let right = self.find(right);
        if left != right {
            self.parents[right] = left;
        }
    }
}

#[derive(Debug)]
pub struct DatasetError(String);

impl DatasetError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for DatasetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for DatasetError {}

impl From<std::io::Error> for DatasetError {
    fn from(value: std::io::Error) -> Self {
        Self(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::episode::{
        EPISODE_CONTEXT_SCHEMA_V1, EpisodeContext, EpisodeLineage, EpisodeManifestBuild,
        EpisodeObjectiveIdentity, EpisodeOutcome, EpisodeProducerIdentity, EpisodeProducerKind,
        EpisodeSeed, RunBuildIdentity,
    };
    use crate::tape::{InputFrame, TapeBoot};
    use crate::transition_corpus::{MacroAction, StateReference, StateReferenceKind, Transition};
    use crate::transition_evidence::{
        AlignedTransitionEvidence, EntityFactsEvidence, EventFactsEvidence, EvidencePhase,
        EvidenceReferenceKind, ExactActionEvidence, ObservationBoundaryEvidence,
        PredicateFactsEvidence, PredicateTransitionEvidence, RewardComponentEvidence,
        RewardEvidence, SideTableTransitionEvidence, TRANSITION_EVIDENCE_SCHEMA_V1,
        TerminalReasonEvidence,
    };

    fn source(
        id: &str,
        stage: &str,
        route_family: &str,
        objective: &str,
        frames: usize,
        byte: u8,
    ) -> DatasetSource {
        let tape = InputTape {
            boot: TapeBoot::Stage {
                stage: stage.into(),
                room: 0,
                point: 0,
                layer: 0,
                save_slot: None,
                fixture: None,
            },
            tick_rate_numerator: 30,
            tick_rate_denominator: 1,
            frames: vec![InputFrame::default(); frames],
        };
        let tape_bytes = tape.encode().unwrap();
        let source_reference = StateReference {
            kind: StateReferenceKind::Boundary,
            digest: Digest([byte; 32]),
        };
        let next_reference = StateReference {
            kind: StateReferenceKind::Boundary,
            digest: Digest([byte.wrapping_add(1); 32]),
        };
        let corpus = TransitionCorpus::new(
            Digest([0x11; 32]),
            Digest([0x22; 32]),
            2,
            vec![Transition {
                source: source_reference.clone(),
                state: vec![byte as f32, 0.0],
                action: MacroAction {
                    action_id: u32::from(byte),
                    macro_kind: 1,
                    parameters: Vec::new(),
                },
                duration_ticks: 1,
                reward: 1.0,
                next: next_reference.clone(),
                next_state: vec![byte as f32 + 1.0, 0.0],
                terminal: true,
            }],
        )
        .unwrap();
        let corpus_sha256 = corpus.content_digest().unwrap();
        let availability = EventFactsEvidence {
            availability: EvidenceAvailability::Unavailable,
            running: false,
            event_id: -1,
            mode: 0,
            status: 0,
            map_tool_id: 0,
            name_hash: None,
        };
        let entities = EntityFactsEvidence {
            availability: EvidenceAvailability::Unavailable,
            observed_count: 0,
            truncated: false,
            actors: Vec::new(),
        };
        let predicate = PredicateFactsEvidence {
            availability: EvidenceAvailability::Unavailable,
            configured: false,
            reached: false,
            authored: false,
            goal_name_hash: None,
            requested_count: 0,
            hit_count: 0,
            stable_ticks: 0,
            consecutive_ticks: 0,
            sequence_steps: 0,
            sequence_next_step: 0,
            sequence_within_ticks: 0,
            sequence_elapsed_ticks: 0,
            first_hit_tick: None,
        };
        let evidence = TransitionEvidenceBundle {
            schema: TRANSITION_EVIDENCE_SCHEMA_V1.into(),
            corpus_sha256,
            trace_sha256: Digest([0x44; 32]),
            tape_sha256: Digest(Sha256::digest(&tape_bytes).into()),
            event_side_table: vec![availability],
            entity_side_table: vec![entities],
            transitions: vec![AlignedTransitionEvidence {
                corpus_transition_index: 0,
                pre_action: ObservationBoundaryEvidence {
                    reference_kind: EvidenceReferenceKind::Boundary,
                    reference_sha256: source_reference.digest,
                    boundary_index: 1,
                    simulation_tick: 0,
                    tape_frame: Some(0),
                    phase: EvidencePhase::PostSimulation,
                },
                action: ExactActionEvidence::PadFrame {
                    tape_frame: 1,
                    frame: tape.frames[1].clone(),
                },
                duration_ticks: 1,
                post_action: ObservationBoundaryEvidence {
                    reference_kind: EvidenceReferenceKind::Boundary,
                    reference_sha256: next_reference.digest,
                    boundary_index: 2,
                    simulation_tick: 1,
                    tape_frame: Some(1),
                    phase: EvidencePhase::PostSimulation,
                },
                event: SideTableTransitionEvidence {
                    pre_action: 0,
                    post_action: 0,
                },
                entities: SideTableTransitionEvidence {
                    pre_action: 0,
                    post_action: 0,
                },
                reward: RewardEvidence {
                    training_reward: 1.0,
                    components: vec![RewardComponentEvidence {
                        name: "objective".into(),
                        value: 1.0,
                        source_fact: "test".into(),
                    }],
                },
                predicate: PredicateTransitionEvidence {
                    pre_action: predicate.clone(),
                    post_action: predicate,
                },
                terminal_reason: Some(TerminalReasonEvidence::ObjectiveReached),
            }],
        };
        evidence.validate(&corpus).unwrap();
        let context = EpisodeContext {
            schema: EPISODE_CONTEXT_SCHEMA_V1.into(),
            run_build: RunBuildIdentity {
                executable_sha256: Digest([0x55; 32]),
                dusklight_commit: None,
                aurora_commit: None,
                target: None,
                profile: None,
                feature_digest: None,
            },
            objective: EpisodeObjectiveIdentity {
                id: objective.into(),
                digest: Digest([0x66; 32]),
            },
            producer: EpisodeProducerIdentity {
                kind: EpisodeProducerKind::SystematicProbe,
                name: "test".into(),
                version: "1".into(),
            },
            seed: EpisodeSeed::Deterministic { value: 1 },
            worker_id: format!("worker-{id}"),
            lineage: EpisodeLineage {
                candidate_id: Some(format!("candidate-{id}")),
                parent_candidate_id: None,
                generation: 1,
                intervention: None,
            },
            outcome: EpisodeOutcome {
                class: EpisodeOutcomeClass::Successful,
                reason: "objective reached".into(),
            },
        };
        let episode = EpisodeManifest::build(EpisodeManifestBuild {
            context: &context,
            boot: &tape.boot,
            corpus: &corpus,
            query_view_id: "test-view",
            tape_sha256: Digest(Sha256::digest(&tape_bytes).into()),
            trace_sha256: evidence.trace_sha256,
            transition_evidence_sha256: Digest([0x77; 32]),
        })
        .unwrap();
        DatasetSource {
            source_id: id.into(),
            episode,
            corpus,
            tape,
            evidence,
            route_family: route_family.into(),
            screenshot_sha256: Vec::new(),
            checkpoint_sha256: Vec::new(),
            transition_corpus_path: PathBuf::from(format!("{id}.dtcz")),
            gameplay_trace_path: PathBuf::from(format!("{id}.trace")),
            absolute_tape_path: PathBuf::from(format!("{id}.tape")),
        }
    }

    #[test]
    fn grouped_splits_freeze_withheld_and_fit_normalization_on_training_only() {
        let sources = vec![
            source("a", "STAGE_A", "family-a", "normal", 2, 1),
            source("b", "STAGE_A", "family-a", "normal", 3, 2),
            source("held", "STAGE_B", "family-held", "frozen", 2, 3),
        ];
        let manifest = DatasetManifest::build(
            &sources,
            &DatasetBuildConfig {
                validation_percent: 0,
                test_percent: 0,
                withheld_objectives: BTreeSet::from(["frozen".into()]),
            },
        )
        .unwrap();
        manifest.validate().unwrap();
        assert_eq!(manifest.entries[0].split, DatasetSplit::Train);
        assert_eq!(manifest.entries[1].split, DatasetSplit::Train);
        assert_eq!(
            manifest
                .entries
                .iter()
                .find(|entry| entry.objective_id == "frozen")
                .unwrap()
                .split,
            DatasetSplit::Withheld
        );
        assert_eq!(manifest.normalization.len(), 1);
        assert_eq!(manifest.normalization[0].training_episode_sha256.len(), 2);
        assert_eq!(manifest.report.unique_episodes, 3);
        assert_eq!(manifest.report.effective_decisions, 3);
        assert!(manifest.leakage.grouped_by_scenario > 0);
        assert_eq!(manifest.leakage.cross_split_violations, 0);
    }
}
