//! Content-addressed planner snapshots and raw/semantic boundary diffs.

use crate::artifact::Digest;
use crate::state::{
    BoundaryKind, ComponentBinding, ComponentKind, ComponentPayload, ComponentProvenance,
    ExecutionEnvironment, PhysicalSlot, PhysicalSlotObservation, RuntimeFile, RuntimeFileLifecycle,
    SemanticLifetime, SerializationOwner, StateComponent,
};
use crate::{PlannerContractError, canonical_json, validate_stable_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const STATE_SNAPSHOT_SCHEMA: &str = "dusklight.route-planner.state-snapshot/v6";
pub const STATE_DIFF_SCHEMA: &str = "dusklight.route-planner.state-diff/v6";
pub const SNAPSHOT_CHAIN_SCHEMA: &str = "dusklight.route-planner.snapshot-chain/v6";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationStatus {
    True,
    False,
    Unknown,
    Unsupported,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticObservation {
    pub fact_id: String,
    pub status: ObservationStatus,
    pub evidence_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StateSnapshot {
    pub schema: String,
    pub id: String,
    pub sequence: u64,
    pub environment: ExecutionEnvironment,
    pub semantic_observations: Vec<SemanticObservation>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeltaKind {
    Added,
    Removed,
    Changed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawByteDelta {
    pub offset: u32,
    pub before: Option<u8>,
    pub after: Option<u8>,
    pub before_known_mask: Option<u8>,
    pub after_known_mask: Option<u8>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentDelta {
    pub component_id: String,
    pub delta_kind: DeltaKind,
    pub component_kind_before: Option<ComponentKind>,
    pub component_kind_after: Option<ComponentKind>,
    pub payload_sha256_before: Option<Digest>,
    pub payload_sha256_after: Option<Digest>,
    pub binding_before: Option<ComponentBinding>,
    pub binding_after: Option<ComponentBinding>,
    pub lifetime_before: Option<SemanticLifetime>,
    pub lifetime_after: Option<SemanticLifetime>,
    pub serialization_owner_before: Option<SerializationOwner>,
    pub serialization_owner_after: Option<SerializationOwner>,
    pub provenance_before: Vec<ComponentProvenance>,
    pub provenance_after: Vec<ComponentProvenance>,
    pub raw_byte_deltas: Vec<RawByteDelta>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SlotDelta {
    pub slot: u8,
    pub delta_kind: DeltaKind,
    pub before: Option<PhysicalSlot>,
    pub after: Option<PhysicalSlot>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SlotObservationDelta {
    pub slot: u8,
    pub delta_kind: DeltaKind,
    pub before: Option<PhysicalSlotObservation>,
    pub after: Option<PhysicalSlotObservation>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticDelta {
    pub fact_id: String,
    pub before: Option<ObservationStatus>,
    pub after: Option<ObservationStatus>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StateDiff {
    pub schema: String,
    pub from_snapshot_sha256: Digest,
    pub to_snapshot_sha256: Digest,
    pub boundary: BoundaryKind,
    pub runtime_file_before: RuntimeFile,
    pub runtime_file_after: RuntimeFile,
    pub inactive_runtime_files_before: Vec<RuntimeFile>,
    pub inactive_runtime_files_after: Vec<RuntimeFile>,
    pub location_changed: bool,
    pub player_changed: bool,
    pub static_world_objects_changed: bool,
    pub spatial_volumes_changed: bool,
    pub spatial_connections_changed: bool,
    pub spatial_planes_changed: bool,
    pub persisted_object_controls_changed: bool,
    pub live_world_objects_changed: bool,
    pub slot_deltas: Vec<SlotDelta>,
    pub slot_observation_deltas: Vec<SlotObservationDelta>,
    pub component_deltas: Vec<ComponentDelta>,
    pub semantic_deltas: Vec<SemanticDelta>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotChainEntry {
    pub snapshot_sha256: Digest,
    pub previous_snapshot_sha256: Option<Digest>,
    pub incoming_boundary: Option<BoundaryKind>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotChain {
    pub schema: String,
    pub entries: Vec<SnapshotChainEntry>,
}

impl StateSnapshot {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != STATE_SNAPSHOT_SCHEMA {
            return Err(PlannerContractError::new("schema", "is unsupported"));
        }
        validate_stable_id("id", &self.id)?;
        self.environment.validate()?;
        if self.semantic_observations.len() > 65_536 {
            return Err(PlannerContractError::new(
                "semantic_observations",
                "contains too many records",
            ));
        }
        let mut previous = None;
        for observation in &self.semantic_observations {
            validate_stable_id("semantic_observations.fact_id", &observation.fact_id)?;
            if let Some(evidence_id) = &observation.evidence_id {
                validate_stable_id("semantic_observations.evidence_id", evidence_id)?;
            }
            if previous.is_some_and(|prior: &str| prior >= observation.fact_id.as_str()) {
                return Err(PlannerContractError::new(
                    "semantic_observations",
                    "must be unique and sorted by fact ID",
                ));
            }
            previous = Some(observation.fact_id.as_str());
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let snapshot: Self = serde_json::from_slice(bytes)?;
        snapshot.validate()?;
        if snapshot.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "snapshot",
                "is not canonical JSON",
            ));
        }
        Ok(snapshot)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

impl StateDiff {
    pub fn between(
        before: &StateSnapshot,
        after: &StateSnapshot,
        boundary: BoundaryKind,
    ) -> Result<Self, PlannerContractError> {
        before.validate()?;
        after.validate()?;
        if before.environment.runtime_configuration.content_sha256
            != after.environment.runtime_configuration.content_sha256
        {
            return Err(PlannerContractError::new(
                "snapshots",
                "cannot diff snapshots from different content identities",
            ));
        }
        if after.sequence <= before.sequence {
            return Err(PlannerContractError::new(
                "snapshots",
                "after snapshot sequence must be greater than before sequence",
            ));
        }
        Ok(Self {
            schema: STATE_DIFF_SCHEMA.into(),
            from_snapshot_sha256: before.digest()?,
            to_snapshot_sha256: after.digest()?,
            boundary,
            runtime_file_before: before.environment.active_runtime_file.clone(),
            runtime_file_after: after.environment.active_runtime_file.clone(),
            inactive_runtime_files_before: before.environment.inactive_runtime_files.clone(),
            inactive_runtime_files_after: after.environment.inactive_runtime_files.clone(),
            location_changed: before.environment.location != after.environment.location,
            player_changed: before.environment.player != after.environment.player,
            static_world_objects_changed: before.environment.static_world_objects
                != after.environment.static_world_objects,
            spatial_volumes_changed: before.environment.spatial_volumes
                != after.environment.spatial_volumes,
            spatial_connections_changed: before.environment.spatial_connections
                != after.environment.spatial_connections,
            spatial_planes_changed: before.environment.spatial_planes
                != after.environment.spatial_planes,
            persisted_object_controls_changed: before.environment.persisted_object_controls
                != after.environment.persisted_object_controls,
            live_world_objects_changed: before.environment.live_world_objects
                != after.environment.live_world_objects,
            slot_deltas: diff_slots(
                &before.environment.physical_slots,
                &after.environment.physical_slots,
            ),
            slot_observation_deltas: diff_slot_observations(
                &before.environment.physical_slot_observations,
                &after.environment.physical_slot_observations,
            ),
            component_deltas: diff_components(
                &before.environment.components,
                &after.environment.components,
            )?,
            semantic_deltas: diff_semantics(
                &before.semantic_observations,
                &after.semantic_observations,
            ),
        })
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != STATE_DIFF_SCHEMA
            || self.from_snapshot_sha256 == Digest::ZERO
            || self.to_snapshot_sha256 == Digest::ZERO
            || self.from_snapshot_sha256 == self.to_snapshot_sha256
        {
            return Err(PlannerContractError::new(
                "state_diff",
                "has an invalid schema or snapshot identity",
            ));
        }
        self.runtime_file_before.validate()?;
        self.runtime_file_after.validate()?;
        validate_inactive_runtime_files(
            "inactive_runtime_files_before",
            &self.inactive_runtime_files_before,
        )?;
        validate_inactive_runtime_files(
            "inactive_runtime_files_after",
            &self.inactive_runtime_files_after,
        )?;
        validate_delta_order(
            "slot_deltas",
            self.slot_deltas.iter().map(|delta| delta.slot),
        )?;
        validate_delta_order(
            "slot_observation_deltas",
            self.slot_observation_deltas.iter().map(|delta| delta.slot),
        )?;
        validate_delta_order(
            "component_deltas",
            self.component_deltas
                .iter()
                .map(|delta| delta.component_id.as_str()),
        )?;
        validate_delta_order(
            "semantic_deltas",
            self.semantic_deltas
                .iter()
                .map(|delta| delta.fact_id.as_str()),
        )
    }
}

impl SnapshotChain {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != SNAPSHOT_CHAIN_SCHEMA || self.entries.is_empty() {
            return Err(PlannerContractError::new(
                "snapshot_chain",
                "has an invalid schema or no entries",
            ));
        }
        let mut seen = BTreeSet::new();
        for (index, entry) in self.entries.iter().enumerate() {
            if entry.snapshot_sha256 == Digest::ZERO || !seen.insert(entry.snapshot_sha256) {
                return Err(PlannerContractError::new(
                    "snapshot_chain.entries",
                    "contains a zero or duplicate snapshot digest",
                ));
            }
            if index == 0 {
                if entry.previous_snapshot_sha256.is_some() || entry.incoming_boundary.is_some() {
                    return Err(PlannerContractError::new(
                        "snapshot_chain.entries",
                        "the first entry cannot have a predecessor or incoming boundary",
                    ));
                }
            } else if entry.previous_snapshot_sha256
                != Some(self.entries[index - 1].snapshot_sha256)
                || entry.incoming_boundary.is_none()
            {
                return Err(PlannerContractError::new(
                    "snapshot_chain.entries",
                    "must form a contiguous boundary-labelled digest chain",
                ));
            }
        }
        Ok(())
    }
}

fn validate_inactive_runtime_files(
    field: &str,
    runtimes: &[RuntimeFile],
) -> Result<(), PlannerContractError> {
    let mut previous = None;
    for runtime in runtimes {
        runtime.validate()?;
        if runtime.lifecycle == RuntimeFileLifecycle::Active {
            return Err(PlannerContractError::new(
                field,
                "cannot contain an active runtime-file lifetime",
            ));
        }
        if previous.is_some_and(|id: &str| id >= runtime.id.as_str()) {
            return Err(PlannerContractError::new(
                field,
                "must be unique and sorted by runtime-file ID",
            ));
        }
        previous = Some(runtime.id.as_str());
    }
    Ok(())
}

fn diff_slots(before: &[PhysicalSlot], after: &[PhysicalSlot]) -> Vec<SlotDelta> {
    let before = before
        .iter()
        .map(|slot| (slot.slot.0, slot))
        .collect::<BTreeMap<_, _>>();
    let after = after
        .iter()
        .map(|slot| (slot.slot.0, slot))
        .collect::<BTreeMap<_, _>>();
    before
        .keys()
        .chain(after.keys())
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|slot| match (before.get(&slot), after.get(&slot)) {
            (Some(left), Some(right)) if left == right => None,
            (Some(left), Some(right)) => Some(SlotDelta {
                slot,
                delta_kind: DeltaKind::Changed,
                before: Some((*left).clone()),
                after: Some((*right).clone()),
            }),
            (Some(left), None) => Some(SlotDelta {
                slot,
                delta_kind: DeltaKind::Removed,
                before: Some((*left).clone()),
                after: None,
            }),
            (None, Some(right)) => Some(SlotDelta {
                slot,
                delta_kind: DeltaKind::Added,
                before: None,
                after: Some((*right).clone()),
            }),
            (None, None) => None,
        })
        .collect()
}

fn diff_slot_observations(
    before: &[PhysicalSlotObservation],
    after: &[PhysicalSlotObservation],
) -> Vec<SlotObservationDelta> {
    let before = before
        .iter()
        .map(|slot| (slot.slot.0, slot))
        .collect::<BTreeMap<_, _>>();
    let after = after
        .iter()
        .map(|slot| (slot.slot.0, slot))
        .collect::<BTreeMap<_, _>>();
    before
        .keys()
        .chain(after.keys())
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|slot| match (before.get(&slot), after.get(&slot)) {
            (Some(left), Some(right)) if left == right => None,
            (Some(left), Some(right)) => Some(SlotObservationDelta {
                slot,
                delta_kind: DeltaKind::Changed,
                before: Some((*left).clone()),
                after: Some((*right).clone()),
            }),
            (Some(left), None) => Some(SlotObservationDelta {
                slot,
                delta_kind: DeltaKind::Removed,
                before: Some((*left).clone()),
                after: None,
            }),
            (None, Some(right)) => Some(SlotObservationDelta {
                slot,
                delta_kind: DeltaKind::Added,
                before: None,
                after: Some((*right).clone()),
            }),
            (None, None) => None,
        })
        .collect()
}

fn diff_components(
    before: &[StateComponent],
    after: &[StateComponent],
) -> Result<Vec<ComponentDelta>, PlannerContractError> {
    let before = before
        .iter()
        .map(|component| (component.id.as_str(), component))
        .collect::<BTreeMap<_, _>>();
    let after = after
        .iter()
        .map(|component| (component.id.as_str(), component))
        .collect::<BTreeMap<_, _>>();
    let mut deltas = Vec::new();
    for id in before
        .keys()
        .chain(after.keys())
        .copied()
        .collect::<BTreeSet<_>>()
    {
        match (before.get(id), after.get(id)) {
            (Some(left), Some(right)) if left == right => {}
            (Some(left), Some(right)) => deltas.push(component_delta(id, Some(left), Some(right))?),
            (Some(left), None) => deltas.push(component_delta(id, Some(left), None)?),
            (None, Some(right)) => deltas.push(component_delta(id, None, Some(right))?),
            (None, None) => {}
        }
    }
    Ok(deltas)
}

fn component_delta(
    id: &str,
    before: Option<&StateComponent>,
    after: Option<&StateComponent>,
) -> Result<ComponentDelta, PlannerContractError> {
    Ok(ComponentDelta {
        component_id: id.into(),
        delta_kind: match (before, after) {
            (None, Some(_)) => DeltaKind::Added,
            (Some(_), None) => DeltaKind::Removed,
            _ => DeltaKind::Changed,
        },
        component_kind_before: before.map(|value| value.component_kind.clone()),
        component_kind_after: after.map(|value| value.component_kind.clone()),
        payload_sha256_before: before
            .map(|value| payload_digest(&value.payload))
            .transpose()?,
        payload_sha256_after: after
            .map(|value| payload_digest(&value.payload))
            .transpose()?,
        binding_before: before.map(|value| value.binding.clone()),
        binding_after: after.map(|value| value.binding.clone()),
        lifetime_before: before.map(|value| value.lifetime),
        lifetime_after: after.map(|value| value.lifetime),
        serialization_owner_before: before.map(|value| value.serialization_owner.clone()),
        serialization_owner_after: after.map(|value| value.serialization_owner.clone()),
        provenance_before: before.map_or_else(Vec::new, |value| value.provenance.clone()),
        provenance_after: after.map_or_else(Vec::new, |value| value.provenance.clone()),
        raw_byte_deltas: diff_raw_payloads(
            before.map(|value| &value.payload),
            after.map(|value| &value.payload),
        ),
    })
}

fn payload_digest(payload: &ComponentPayload) -> Result<Digest, PlannerContractError> {
    Ok(Digest(Sha256::digest(serde_json::to_vec(payload)?).into()))
}

fn diff_raw_payloads(
    before: Option<&ComponentPayload>,
    after: Option<&ComponentPayload>,
) -> Vec<RawByteDelta> {
    let (before, after) = (raw_payload(before), raw_payload(after));
    if before.is_none() && after.is_none() {
        return Vec::new();
    }
    let length = before
        .map_or(0, |(bytes, _)| bytes.len())
        .max(after.map_or(0, |(bytes, _)| bytes.len()));
    (0..length)
        .filter_map(|offset| {
            let before_byte = before.and_then(|(bytes, _)| bytes.get(offset)).copied();
            let after_byte = after.and_then(|(bytes, _)| bytes.get(offset)).copied();
            let before_mask = before.and_then(|(_, mask)| mask.get(offset)).copied();
            let after_mask = after.and_then(|(_, mask)| mask.get(offset)).copied();
            if before_byte == after_byte && before_mask == after_mask {
                None
            } else {
                Some(RawByteDelta {
                    offset: offset as u32,
                    before: before_byte,
                    after: after_byte,
                    before_known_mask: before_mask,
                    after_known_mask: after_mask,
                })
            }
        })
        .collect()
}

fn raw_payload(payload: Option<&ComponentPayload>) -> Option<(&[u8], &[u8])> {
    match payload {
        Some(ComponentPayload::Raw { bytes, known_mask }) => Some((bytes, known_mask)),
        _ => None,
    }
}

fn diff_semantics(
    before: &[SemanticObservation],
    after: &[SemanticObservation],
) -> Vec<SemanticDelta> {
    let before = before
        .iter()
        .map(|value| (value.fact_id.as_str(), value.status))
        .collect::<BTreeMap<_, _>>();
    let after = after
        .iter()
        .map(|value| (value.fact_id.as_str(), value.status))
        .collect::<BTreeMap<_, _>>();
    before
        .keys()
        .chain(after.keys())
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|id| {
            let left = before.get(id).copied();
            let right = after.get(id).copied();
            (left != right).then(|| SemanticDelta {
                fact_id: id.into(),
                before: left,
                after: right,
            })
        })
        .collect()
}

fn validate_delta_order<T: Ord>(
    field: &str,
    values: impl IntoIterator<Item = T>,
) -> Result<(), PlannerContractError> {
    let mut previous = None;
    for value in values {
        if previous.as_ref().is_some_and(|prior| prior >= &value) {
            return Err(PlannerContractError::new(
                field,
                "must be unique and sorted",
            ));
        }
        previous = Some(value);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{RUNTIME_CONFIGURATION_SCHEMA, RuntimeConfiguration};
    use crate::state::{
        BackingAttachment, CaptureStatus, ComponentPayload, PhysicalSlotId,
        PhysicalSlotObservation, PlayerForm, PlayerState, ProvenanceSourceKind,
        RuntimeFileLifecycle, RuntimeFileOrigin, SceneLocation, StateComponent,
    };

    fn component(bytes: Vec<u8>, known_mask: Vec<u8>, binding: ComponentBinding) -> StateComponent {
        StateComponent {
            id: "stage-memory".into(),
            component_kind: ComponentKind::StageMemory,
            payload: ComponentPayload::Raw { bytes, known_mask },
            binding,
            lifetime: SemanticLifetime::StageLoad,
            serialization_owner: SerializationOwner::StageBank {
                runtime_file_id: "file-0".into(),
                stage: "F_SP103".into(),
            },
            provenance: vec![ComponentProvenance {
                source_kind: ProvenanceSourceKind::TraceObservation,
                source_id: "trace.boundary".into(),
                source_sha256: Some(Digest([8; 32])),
                transition_id: None,
            }],
        }
    }

    fn snapshot(sequence: u64, component: StateComponent) -> StateSnapshot {
        StateSnapshot {
            schema: STATE_SNAPSHOT_SCHEMA.into(),
            id: format!("snapshot.{sequence}"),
            sequence,
            environment: ExecutionEnvironment {
                schema: crate::state::EXECUTION_ENVIRONMENT_SCHEMA.into(),
                runtime_configuration: RuntimeConfiguration {
                    schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
                    content_sha256: Digest([1; 32]),
                    language: "en".into(),
                    settings: BTreeMap::new(),
                },
                active_runtime_file: RuntimeFile {
                    id: "file-0".into(),
                    origin: RuntimeFileOrigin::TitleFile0,
                    backing: BackingAttachment::MemoryOnly,
                    allowed_serialization_targets: vec![PhysicalSlotId(1)],
                    lifecycle: RuntimeFileLifecycle::Active,
                },
                inactive_runtime_files: Vec::new(),
                physical_slots: Vec::new(),
                physical_slot_observations: Vec::new(),
                location: SceneLocation {
                    stage: "F_SP103".into(),
                    room: 0,
                    layer: 0,
                    spawn: 0,
                },
                player: PlayerState {
                    form: PlayerForm::Human,
                    mount: None,
                    position: [0.0, 0.0, 0.0],
                    rotation: [0, 0, 0],
                    has_control: Some(true),
                    action: "idle".into(),
                },
                components: vec![component],
                static_world_objects: Vec::new(),
                spatial_volumes: Vec::new(),
                spatial_connections: Vec::new(),
                spatial_planes: Vec::new(),
                persisted_object_controls: Vec::new(),
                live_world_objects: Vec::new(),
            },
            semantic_observations: vec![SemanticObservation {
                fact_id: "story.faron.twilight".into(),
                status: ObservationStatus::Unknown,
                evidence_id: None,
            }],
        }
    }

    #[test]
    fn diff_records_raw_knownness_binding_and_provenance_changes() {
        let before = snapshot(
            1,
            component(
                vec![0x10, 0x20],
                vec![0xff, 0x00],
                ComponentBinding::Stage {
                    stage: "F_SP103".into(),
                },
            ),
        );
        let mut after = snapshot(
            2,
            component(
                vec![0x10, 0x21],
                vec![0xff, 0xff],
                ComponentBinding::Stage {
                    stage: "D_MN09".into(),
                },
            ),
        );
        after.semantic_observations[0].status = ObservationStatus::True;
        after.environment.components[0]
            .provenance
            .push(ComponentProvenance {
                source_kind: ProvenanceSourceKind::Theorycraft,
                source_id: "rebind.forest-to-tot".into(),
                source_sha256: None,
                transition_id: Some("technique.rebind".into()),
            });

        let diff = StateDiff::between(&before, &after, BoundaryKind::WrongStateRespawn).unwrap();
        diff.validate().unwrap();
        assert_eq!(diff.component_deltas[0].raw_byte_deltas.len(), 1);
        assert_ne!(
            diff.component_deltas[0].binding_before,
            diff.component_deltas[0].binding_after
        );
        assert_eq!(diff.component_deltas[0].provenance_after.len(), 2);
        assert_eq!(diff.semantic_deltas[0].after, Some(ObservationStatus::True));
    }

    #[test]
    fn runtime_file_and_physical_slots_are_diffed_independently() {
        let component = component(
            vec![0],
            vec![0],
            ComponentBinding::RuntimeFile {
                runtime_file_id: "file-0".into(),
            },
        );
        let mut before = snapshot(1, component.clone());
        before
            .environment
            .physical_slot_observations
            .push(PhysicalSlotObservation {
                slot: PhysicalSlotId(1),
                content_status: CaptureStatus::NotSampled,
                attached_to_active_runtime: false,
            });
        let mut after = snapshot(2, component);
        after.environment.physical_slots.push(PhysicalSlot {
            slot: PhysicalSlotId(1),
            persistent_file_id: "save-1".into(),
            serialized_state_sha256: Digest([9; 32]),
        });
        after.environment.active_runtime_file.backing = BackingAttachment::CardBacked {
            slot: PhysicalSlotId(1),
        };
        let mut ended_runtime = before.environment.active_runtime_file.clone();
        ended_runtime.id = "file-previous".into();
        ended_runtime.lifecycle = RuntimeFileLifecycle::Ended;
        after
            .environment
            .inactive_runtime_files
            .push(ended_runtime.clone());
        after
            .environment
            .physical_slot_observations
            .push(PhysicalSlotObservation {
                slot: PhysicalSlotId(1),
                content_status: CaptureStatus::Present,
                attached_to_active_runtime: true,
            });
        let diff = StateDiff::between(&before, &after, BoundaryKind::SaveRuntimeToSlot).unwrap();
        assert_ne!(diff.runtime_file_before, diff.runtime_file_after);
        assert!(diff.inactive_runtime_files_before.is_empty());
        assert_eq!(diff.inactive_runtime_files_after, vec![ended_runtime]);
        assert_eq!(diff.slot_deltas.len(), 1);
        assert_eq!(diff.slot_deltas[0].delta_kind, DeltaKind::Added);
        assert_eq!(diff.slot_observation_deltas.len(), 1);
        assert_eq!(
            diff.slot_observation_deltas[0].delta_kind,
            DeltaKind::Changed
        );
    }

    #[test]
    fn unknown_and_unsupported_are_not_defaulted_to_false() {
        let component = component(vec![0], vec![0], ComponentBinding::Global);
        let mut snapshot = snapshot(1, component);
        snapshot.semantic_observations = vec![
            SemanticObservation {
                fact_id: "fact.geometry".into(),
                status: ObservationStatus::Unknown,
                evidence_id: None,
            },
            SemanticObservation {
                fact_id: "fact.message-flow".into(),
                status: ObservationStatus::Unsupported,
                evidence_id: None,
            },
        ];
        snapshot.validate().unwrap();
        assert!(
            snapshot
                .semantic_observations
                .iter()
                .all(|observation| { !matches!(observation.status, ObservationStatus::False) })
        );
    }

    #[test]
    fn snapshot_chain_requires_contiguous_digest_links_and_boundaries() {
        let component = component(vec![0], vec![0], ComponentBinding::Global);
        let first = snapshot(1, component.clone()).digest().unwrap();
        let second = snapshot(2, component).digest().unwrap();
        let chain = SnapshotChain {
            schema: SNAPSHOT_CHAIN_SCHEMA.into(),
            entries: vec![
                SnapshotChainEntry {
                    snapshot_sha256: first,
                    previous_snapshot_sha256: None,
                    incoming_boundary: None,
                },
                SnapshotChainEntry {
                    snapshot_sha256: second,
                    previous_snapshot_sha256: Some(first),
                    incoming_boundary: Some(BoundaryKind::RoomTransition),
                },
            ],
        };
        chain.validate().unwrap();

        let mut broken = chain;
        broken.entries[1].previous_snapshot_sha256 = Some(Digest([7; 32]));
        assert_eq!(
            broken.validate().unwrap_err().field(),
            "snapshot_chain.entries"
        );
    }
}
