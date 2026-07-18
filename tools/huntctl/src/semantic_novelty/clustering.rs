//! Stable semantic symptom clustering for autonomous discovery incidents.

use super::{EventFact, SemanticNoveltyDescriptor, SemanticState};
use crate::semantic_novelty::archive::DiscoveryArchivePartitionKey;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

pub const SYMPTOM_CLUSTER_INDEX_SCHEMA: &str = "dusklight-symptom-cluster-index/v1";
pub const MAX_SYMPTOM_CLUSTER_PARTITIONS: usize = 64;
pub const MAX_SYMPTOM_CLUSTERS: usize = 4_096;
pub const MAX_RETAINED_CLUSTER_EXAMPLES: usize = 8;
pub const MAX_CRASH_FRAMES: usize = 16;
pub const MAX_EVENT_TAIL: usize = 32;
pub const MAX_SYMPTOM_LABEL_BYTES: usize = 256;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct StableCrashFrame {
    pub module: String,
    pub symbol: String,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum DiscoverySymptomKind {
    Crash {
        category: String,
        frames: Vec<StableCrashFrame>,
    },
    Hang {
        watchdog: String,
    },
    OutOfBoundsRoute {
        oracle: String,
    },
    Corruption {
        oracle: String,
    },
    EventSequence,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct DiscoverySymptomDescriptor {
    pub kind: DiscoverySymptomKind,
    pub terminal_state: Option<SemanticState>,
    pub event_tail: Vec<EventFact>,
    pub contact_identity: Option<String>,
    pub terminal_boundary_identity: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DiscoverySymptomObservation {
    pub artifact_identity: String,
    pub generation: u32,
    pub symptom: DiscoverySymptomDescriptor,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SymptomCluster {
    pub cluster_identity: String,
    pub representative: DiscoverySymptomObservation,
    pub occurrences: u64,
    pub first_seen_generation: u32,
    pub last_seen_generation: u32,
    pub example_artifact_identities: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SymptomClusterPartitionSnapshot {
    pub partition: DiscoveryArchivePartitionKey,
    pub clusters: Vec<SymptomCluster>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SymptomClusterIndexSnapshot {
    pub schema: &'static str,
    pub partitions: Vec<SymptomClusterPartitionSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SymptomClusterDecision {
    NewCluster {
        cluster_identity: String,
    },
    ExistingCluster {
        cluster_identity: String,
        occurrences: u64,
    },
}

#[derive(Clone, Debug, Default)]
pub struct SymptomClusterIndex {
    partitions: BTreeMap<DiscoveryArchivePartitionKey, BTreeMap<String, SymptomCluster>>,
}

#[derive(Debug)]
pub struct SymptomClusterError(String);

impl fmt::Display for SymptomClusterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for SymptomClusterError {}

impl DiscoverySymptomDescriptor {
    pub fn from_semantic_descriptor(
        kind: DiscoverySymptomKind,
        descriptor: &SemanticNoveltyDescriptor,
    ) -> Result<Self, SymptomClusterError> {
        validate_kind(&kind)?;
        let event_start = descriptor
            .event_sequence
            .len()
            .saturating_sub(MAX_EVENT_TAIL);
        let terminal_boundary_identity = descriptor
            .boundary_fingerprints
            .last()
            .map(|boundary| boundary.digest.clone());
        Ok(Self {
            kind,
            terminal_state: descriptor
                .state_combinations
                .last()
                .map(|combination| combination.state.clone()),
            event_tail: descriptor.event_sequence[event_start..].to_vec(),
            contact_identity: descriptor.axis_identities().contacts,
            terminal_boundary_identity,
        })
    }

    pub fn identity(&self) -> String {
        let encoded = serde_json::to_vec(self).expect("symptom descriptor is serializable");
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight-discovery-symptom/v1\0");
        hasher.update((encoded.len() as u64).to_le_bytes());
        hasher.update(encoded);
        format!("{:x}", hasher.finalize())
    }
}

impl SymptomClusterIndex {
    pub fn consider(
        &mut self,
        partition: DiscoveryArchivePartitionKey,
        observation: DiscoverySymptomObservation,
    ) -> Result<SymptomClusterDecision, SymptomClusterError> {
        validate_partition(&partition)?;
        validate_sha256("artifact identity", &observation.artifact_identity)?;
        validate_kind(&observation.symptom.kind)?;
        let cluster_identity = observation.symptom.identity();
        if !self.partitions.contains_key(&partition)
            && self.partitions.len() >= MAX_SYMPTOM_CLUSTER_PARTITIONS
        {
            return Err(SymptomClusterError(format!(
                "symptom index exceeds {MAX_SYMPTOM_CLUSTER_PARTITIONS} partitions"
            )));
        }
        let clusters = self.partitions.entry(partition).or_default();
        if let Some(cluster) = clusters.get_mut(&cluster_identity) {
            cluster.occurrences = cluster.occurrences.saturating_add(1);
            cluster.last_seen_generation = cluster.last_seen_generation.max(observation.generation);
            if cluster.example_artifact_identities.len() < MAX_RETAINED_CLUSTER_EXAMPLES
                && !cluster
                    .example_artifact_identities
                    .contains(&observation.artifact_identity)
            {
                cluster
                    .example_artifact_identities
                    .push(observation.artifact_identity);
                cluster.example_artifact_identities.sort();
            }
            return Ok(SymptomClusterDecision::ExistingCluster {
                cluster_identity,
                occurrences: cluster.occurrences,
            });
        }
        if clusters.len() >= MAX_SYMPTOM_CLUSTERS {
            return Err(SymptomClusterError(format!(
                "symptom partition exceeds {MAX_SYMPTOM_CLUSTERS} clusters"
            )));
        }
        clusters.insert(
            cluster_identity.clone(),
            SymptomCluster {
                cluster_identity: cluster_identity.clone(),
                representative: observation.clone(),
                occurrences: 1,
                first_seen_generation: observation.generation,
                last_seen_generation: observation.generation,
                example_artifact_identities: vec![observation.artifact_identity],
            },
        );
        Ok(SymptomClusterDecision::NewCluster { cluster_identity })
    }

    pub fn snapshot(&self) -> SymptomClusterIndexSnapshot {
        SymptomClusterIndexSnapshot {
            schema: SYMPTOM_CLUSTER_INDEX_SCHEMA,
            partitions: self
                .partitions
                .iter()
                .map(|(partition, clusters)| SymptomClusterPartitionSnapshot {
                    partition: partition.clone(),
                    clusters: clusters.values().cloned().collect(),
                })
                .collect(),
        }
    }
}

fn validate_kind(kind: &DiscoverySymptomKind) -> Result<(), SymptomClusterError> {
    let invalid = match kind {
        DiscoverySymptomKind::Crash { category, frames } => {
            invalid_label(category)
                || frames.is_empty()
                || frames.len() > MAX_CRASH_FRAMES
                || frames
                    .iter()
                    .any(|frame| invalid_label(&frame.module) || invalid_label(&frame.symbol))
        }
        DiscoverySymptomKind::Hang { watchdog }
        | DiscoverySymptomKind::OutOfBoundsRoute { oracle: watchdog }
        | DiscoverySymptomKind::Corruption { oracle: watchdog } => invalid_label(watchdog),
        DiscoverySymptomKind::EventSequence => false,
    };
    if invalid {
        return Err(SymptomClusterError(
            "symptom kind is empty or exceeds its bounded stable frame set".into(),
        ));
    }
    Ok(())
}

fn invalid_label(value: &str) -> bool {
    value.trim().is_empty() || value.len() > MAX_SYMPTOM_LABEL_BYTES
}

fn validate_partition(partition: &DiscoveryArchivePartitionKey) -> Result<(), SymptomClusterError> {
    if partition.scenario_name.trim().is_empty() {
        return Err(SymptomClusterError("symptom scenario name is empty".into()));
    }
    validate_sha256("scenario identity", &partition.scenario_identity)?;
    validate_sha256("fidelity identity", &partition.fidelity_identity)
}

fn validate_sha256(label: &str, digest: &str) -> Result<(), SymptomClusterError> {
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(SymptomClusterError(format!(
            "symptom {label} is not lowercase SHA-256"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic_novelty::archive::{DiscoveryArchivePartitionKey, DiscoveryFidelity};
    use crate::tape::TapeBoot;
    use crate::trace::{DecodedTrace, TraceRecord};

    fn partition() -> DiscoveryArchivePartitionKey {
        DiscoveryArchivePartitionKey {
            scenario_name: "intro".into(),
            scenario_identity: "11".repeat(32),
            fidelity: DiscoveryFidelity::Headless,
            fidelity_identity: "22".repeat(32),
        }
    }

    fn descriptor(event: i16) -> SemanticNoveltyDescriptor {
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
                    event_id: event,
                    ..TraceRecord::default()
                }],
            },
            Vec::new(),
        )
        .unwrap()
    }

    fn crash() -> DiscoverySymptomKind {
        DiscoverySymptomKind::Crash {
            category: "segmentation_fault".into(),
            frames: vec![StableCrashFrame {
                module: "dusklight".into(),
                symbol: "dBgS::ChkPolySafe".into(),
            }],
        }
    }

    #[test]
    fn equivalent_crashes_join_one_cluster_despite_distinct_artifacts() {
        let symptom =
            DiscoverySymptomDescriptor::from_semantic_descriptor(crash(), &descriptor(1)).unwrap();
        let mut index = SymptomClusterIndex::default();
        assert!(matches!(
            index
                .consider(
                    partition(),
                    DiscoverySymptomObservation {
                        artifact_identity: "31".repeat(32),
                        generation: 1,
                        symptom: symptom.clone(),
                    },
                )
                .unwrap(),
            SymptomClusterDecision::NewCluster { .. }
        ));
        assert!(matches!(
            index
                .consider(
                    partition(),
                    DiscoverySymptomObservation {
                        artifact_identity: "32".repeat(32),
                        generation: 2,
                        symptom,
                    },
                )
                .unwrap(),
            SymptomClusterDecision::ExistingCluster { occurrences: 2, .. }
        ));
        assert_eq!(index.snapshot().partitions[0].clusters.len(), 1);
    }

    #[test]
    fn changed_event_tail_or_symptom_kind_forms_a_distinct_cluster() {
        let mut index = SymptomClusterIndex::default();
        for (artifact, kind, event) in [
            (1, crash(), 1),
            (
                2,
                DiscoverySymptomKind::Hang {
                    watchdog: "simulation_tick_stall".into(),
                },
                1,
            ),
            (3, crash(), 2),
        ] {
            let symptom =
                DiscoverySymptomDescriptor::from_semantic_descriptor(kind, &descriptor(event))
                    .unwrap();
            index
                .consider(
                    partition(),
                    DiscoverySymptomObservation {
                        artifact_identity: format!("{artifact:02x}").repeat(32),
                        generation: 1,
                        symptom,
                    },
                )
                .unwrap();
        }
        assert_eq!(index.snapshot().partitions[0].clusters.len(), 3);
    }

    #[test]
    fn every_supported_incident_class_has_a_stable_identity() {
        let descriptor = descriptor(1);
        let kinds = [
            DiscoverySymptomKind::Hang {
                watchdog: "tick_stall".into(),
            },
            DiscoverySymptomKind::OutOfBoundsRoute {
                oracle: "void_survival".into(),
            },
            DiscoverySymptomKind::Corruption {
                oracle: "invalid_actor_state".into(),
            },
            DiscoverySymptomKind::EventSequence,
        ];
        let identities = kinds
            .into_iter()
            .map(|kind| {
                DiscoverySymptomDescriptor::from_semantic_descriptor(kind, &descriptor)
                    .unwrap()
                    .identity()
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(identities.len(), 4);
    }

    #[test]
    fn unbounded_crash_frames_are_rejected() {
        let invalid = DiscoverySymptomKind::Crash {
            category: "segmentation_fault".into(),
            frames: vec![StableCrashFrame {
                module: "m".repeat(MAX_SYMPTOM_LABEL_BYTES + 1),
                symbol: "0x7ffee123".into(),
            }],
        };
        assert!(
            DiscoverySymptomDescriptor::from_semantic_descriptor(invalid, &descriptor(1)).is_err()
        );
    }
}
