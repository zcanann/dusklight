//! Scenario/fidelity-partitioned, multi-outcome autonomous discovery archive.

use crate::semantic_novelty::SemanticNoveltyDescriptor;
use crate::semantic_novelty::catalog::SemanticNoveltyAssessment;
use serde::Serialize;
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

pub const DISCOVERY_ARCHIVE_SCHEMA: &str = "dusklight-discovery-archive/v1";
pub const DEFAULT_OUTCOMES_PER_BEHAVIOR_CELL: usize = 4;
pub const MAX_OUTCOMES_PER_BEHAVIOR_CELL: usize = 8;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryFidelity {
    Headless,
    Headful,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct DiscoveryArchivePartitionKey {
    pub scenario_name: String,
    pub scenario_identity: String,
    pub fidelity: DiscoveryFidelity,
    /// Binds exact renderer, timing, trace-channel, and build configuration.
    pub fidelity_identity: String,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "label")]
pub enum DiscoveryOutcomeKind {
    Goal,
    Crash,
    Hang,
    OutOfBoundsRoute,
    Corruption,
    EventSequence,
    Other(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DiscoveryRetentionQuality {
    /// Native evidence strength; zero is explicitly unsupported.
    pub evidence_rank: u16,
    pub cold_replay_passes: u16,
    pub milestone_depth: u16,
    pub minimized_tape_frames: u64,
}

impl DiscoveryRetentionQuality {
    fn ordering_key(&self) -> (u16, u16, u16, Reverse<u64>) {
        (
            self.evidence_rank,
            self.cold_replay_passes,
            self.milestone_depth,
            Reverse(self.minimized_tape_frames),
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DiscoveryArchiveEntry {
    pub artifact_identity: String,
    pub outcome: DiscoveryOutcomeKind,
    pub quality: DiscoveryRetentionQuality,
    pub descriptor: SemanticNoveltyDescriptor,
    pub novelty: SemanticNoveltyAssessment,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscoveryArchiveDecision {
    InsertedNewCell,
    InsertedDistinctOutcome,
    ReplacedSameOutcome,
    RejectedUnsupported,
    RejectedDuplicate,
    RejectedLowerQuality,
    RejectedAtCapacity,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DiscoveryArchiveCellSnapshot {
    pub descriptor_identity: String,
    pub outcomes: Vec<DiscoveryArchiveEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DiscoveryArchivePartitionSnapshot {
    pub key: DiscoveryArchivePartitionKey,
    pub cells: Vec<DiscoveryArchiveCellSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DiscoveryArchiveSnapshot {
    pub schema: &'static str,
    pub outcomes_per_behavior_cell: usize,
    pub partitions: Vec<DiscoveryArchivePartitionSnapshot>,
}

#[derive(Clone, Debug)]
pub struct DiscoveryArchive {
    outcomes_per_behavior_cell: usize,
    partitions:
        BTreeMap<DiscoveryArchivePartitionKey, BTreeMap<String, Vec<DiscoveryArchiveEntry>>>,
}

#[derive(Debug)]
pub struct DiscoveryArchiveError(String);

impl fmt::Display for DiscoveryArchiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for DiscoveryArchiveError {}

impl Default for DiscoveryArchive {
    fn default() -> Self {
        Self {
            outcomes_per_behavior_cell: DEFAULT_OUTCOMES_PER_BEHAVIOR_CELL,
            partitions: BTreeMap::new(),
        }
    }
}

impl DiscoveryArchive {
    pub fn new(outcomes_per_behavior_cell: usize) -> Result<Self, DiscoveryArchiveError> {
        if !(2..=MAX_OUTCOMES_PER_BEHAVIOR_CELL).contains(&outcomes_per_behavior_cell) {
            return Err(DiscoveryArchiveError(format!(
                "outcomes per discovery cell must be between 2 and {MAX_OUTCOMES_PER_BEHAVIOR_CELL}"
            )));
        }
        Ok(Self {
            outcomes_per_behavior_cell,
            partitions: BTreeMap::new(),
        })
    }

    pub fn consider(
        &mut self,
        partition: DiscoveryArchivePartitionKey,
        entry: DiscoveryArchiveEntry,
    ) -> Result<DiscoveryArchiveDecision, DiscoveryArchiveError> {
        validate_partition(&partition)?;
        validate_entry(&entry)?;
        if entry.quality.evidence_rank == 0 {
            return Ok(DiscoveryArchiveDecision::RejectedUnsupported);
        }
        let descriptor_identity = entry.descriptor.identity();
        let partition_cells = self.partitions.entry(partition).or_default();
        let new_cell = !partition_cells.contains_key(&descriptor_identity);
        let cell = partition_cells.entry(descriptor_identity).or_default();
        if cell
            .iter()
            .any(|existing| existing.artifact_identity == entry.artifact_identity)
        {
            return Ok(DiscoveryArchiveDecision::RejectedDuplicate);
        }
        if let Some(index) = cell
            .iter()
            .position(|existing| existing.outcome == entry.outcome)
        {
            if compare_entries(&entry, &cell[index]).is_gt() {
                cell[index] = entry;
                sort_cell(cell);
                return Ok(DiscoveryArchiveDecision::ReplacedSameOutcome);
            }
            return Ok(DiscoveryArchiveDecision::RejectedLowerQuality);
        }
        if cell.len() >= self.outcomes_per_behavior_cell {
            return Ok(DiscoveryArchiveDecision::RejectedAtCapacity);
        }
        cell.push(entry);
        sort_cell(cell);
        Ok(if new_cell {
            DiscoveryArchiveDecision::InsertedNewCell
        } else {
            DiscoveryArchiveDecision::InsertedDistinctOutcome
        })
    }

    pub fn partition_count(&self) -> usize {
        self.partitions.len()
    }

    pub fn snapshot(&self) -> DiscoveryArchiveSnapshot {
        DiscoveryArchiveSnapshot {
            schema: DISCOVERY_ARCHIVE_SCHEMA,
            outcomes_per_behavior_cell: self.outcomes_per_behavior_cell,
            partitions: self
                .partitions
                .iter()
                .map(|(key, cells)| DiscoveryArchivePartitionSnapshot {
                    key: key.clone(),
                    cells: cells
                        .iter()
                        .map(
                            |(descriptor_identity, outcomes)| DiscoveryArchiveCellSnapshot {
                                descriptor_identity: descriptor_identity.clone(),
                                outcomes: outcomes.clone(),
                            },
                        )
                        .collect(),
                })
                .collect(),
        }
    }
}

fn compare_entries(
    left: &DiscoveryArchiveEntry,
    right: &DiscoveryArchiveEntry,
) -> std::cmp::Ordering {
    left.quality
        .ordering_key()
        .cmp(&right.quality.ordering_key())
        .then_with(|| right.artifact_identity.cmp(&left.artifact_identity))
}

fn sort_cell(cell: &mut [DiscoveryArchiveEntry]) {
    cell.sort_by(|left, right| {
        left.outcome
            .cmp(&right.outcome)
            .then_with(|| compare_entries(right, left))
    });
}

fn validate_partition(
    partition: &DiscoveryArchivePartitionKey,
) -> Result<(), DiscoveryArchiveError> {
    if partition.scenario_name.trim().is_empty() {
        return Err(DiscoveryArchiveError("scenario name is empty".into()));
    }
    validate_sha256("scenario identity", &partition.scenario_identity)?;
    validate_sha256("fidelity identity", &partition.fidelity_identity)
}

fn validate_entry(entry: &DiscoveryArchiveEntry) -> Result<(), DiscoveryArchiveError> {
    validate_sha256("artifact identity", &entry.artifact_identity)?;
    if entry.novelty.descriptor_identity != entry.descriptor.identity() {
        return Err(DiscoveryArchiveError(
            "novelty assessment does not bind the archived descriptor".into(),
        ));
    }
    if matches!(&entry.outcome, DiscoveryOutcomeKind::Other(label) if label.trim().is_empty()) {
        return Err(DiscoveryArchiveError("outcome label is empty".into()));
    }
    Ok(())
}

fn validate_sha256(label: &str, digest: &str) -> Result<(), DiscoveryArchiveError> {
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(DiscoveryArchiveError(format!(
            "discovery archive {label} is not lowercase SHA-256"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic_novelty::catalog::{SemanticNoveltyCatalog, SemanticNoveltyCatalogConfig};
    use crate::tape::TapeBoot;
    use crate::trace::{DecodedTrace, TraceRecord};

    fn descriptor() -> SemanticNoveltyDescriptor {
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
                records: vec![TraceRecord {
                    stage_name: "F_SP104".into(),
                    room: 1,
                    player_session_process_id: Some(1),
                    player_proc_id: Some(3),
                    ..TraceRecord::default()
                }],
            },
            Vec::new(),
        )
        .unwrap()
    }

    fn partition(
        fidelity: DiscoveryFidelity,
        scenario: u8,
        fidelity_id: u8,
    ) -> DiscoveryArchivePartitionKey {
        DiscoveryArchivePartitionKey {
            scenario_name: format!("scenario-{scenario}"),
            scenario_identity: format!("{scenario:02x}").repeat(32),
            fidelity,
            fidelity_identity: format!("{fidelity_id:02x}").repeat(32),
        }
    }

    fn entry(
        artifact: u8,
        outcome: DiscoveryOutcomeKind,
        evidence_rank: u16,
    ) -> DiscoveryArchiveEntry {
        let descriptor = descriptor();
        let novelty = SemanticNoveltyCatalog::default()
            .assess(&descriptor, SemanticNoveltyCatalogConfig::default())
            .unwrap();
        DiscoveryArchiveEntry {
            artifact_identity: format!("{artifact:02x}").repeat(32),
            outcome,
            quality: DiscoveryRetentionQuality {
                evidence_rank,
                cold_replay_passes: 2,
                milestone_depth: 3,
                minimized_tape_frames: 100,
            },
            descriptor,
            novelty,
        }
    }

    #[test]
    fn scenario_and_fidelity_form_hard_archive_partitions() {
        let mut archive = DiscoveryArchive::default();
        let candidate = entry(1, DiscoveryOutcomeKind::Goal, 2);
        archive
            .consider(
                partition(DiscoveryFidelity::Headless, 1, 10),
                candidate.clone(),
            )
            .unwrap();
        archive
            .consider(
                partition(DiscoveryFidelity::Headful, 1, 11),
                candidate.clone(),
            )
            .unwrap();
        archive
            .consider(partition(DiscoveryFidelity::Headless, 2, 10), candidate)
            .unwrap();
        assert_eq!(archive.partition_count(), 3);
    }

    #[test]
    fn one_cell_retains_several_distinct_useful_outcomes() {
        let mut archive = DiscoveryArchive::default();
        let partition = partition(DiscoveryFidelity::Headless, 1, 10);
        assert_eq!(
            archive
                .consider(partition.clone(), entry(1, DiscoveryOutcomeKind::Goal, 2))
                .unwrap(),
            DiscoveryArchiveDecision::InsertedNewCell
        );
        for (artifact, outcome) in [
            (2, DiscoveryOutcomeKind::Crash),
            (3, DiscoveryOutcomeKind::OutOfBoundsRoute),
        ] {
            assert_eq!(
                archive
                    .consider(partition.clone(), entry(artifact, outcome, 2))
                    .unwrap(),
                DiscoveryArchiveDecision::InsertedDistinctOutcome
            );
        }
        assert_eq!(archive.snapshot().partitions[0].cells[0].outcomes.len(), 3);
    }

    #[test]
    fn lower_quality_same_outcome_cannot_stomp_the_cell() {
        let mut archive = DiscoveryArchive::default();
        let partition = partition(DiscoveryFidelity::Headless, 1, 10);
        archive
            .consider(partition.clone(), entry(1, DiscoveryOutcomeKind::Crash, 3))
            .unwrap();
        assert_eq!(
            archive
                .consider(partition, entry(2, DiscoveryOutcomeKind::Crash, 1))
                .unwrap(),
            DiscoveryArchiveDecision::RejectedLowerQuality
        );
        assert_eq!(
            archive.snapshot().partitions[0].cells[0].outcomes[0].artifact_identity,
            "01".repeat(32)
        );
    }
}
