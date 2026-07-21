//! Atomic execution of typed planner operations over explicit backing stores.

use crate::artifact::Digest;
use crate::snapshot::StateSnapshot;
use crate::state::{
    BackingAttachment, BoundaryDisposition, BoundaryPolicy, ComponentBinding,
    ComponentBindingReference, ComponentKind, ComponentPayload, ComponentProvenance,
    ComponentSelector, ExecutionContext, PhysicalSlot, ProvenanceSourceKind, RuntimeFile,
    RuntimeFileLifecycle, RuntimeFileOrigin, SceneLocation, SerializationOwner, StateComponent,
    StateValue, validate_serialization_owner,
};
use crate::transition::{StateOperation, TemporalWindow};
use crate::{PlannerContractError, canonical_json, validate_stable_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const PLANNER_EXECUTION_STATE_SCHEMA: &str = "dusklight.route-planner.execution-state/v15";
pub const PERSISTENT_FILE_IMAGE_SCHEMA: &str = "dusklight.route-planner.persistent-file-image/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InterruptionRecord {
    pub action_id: String,
    pub window: TemporalWindow,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutionHistoryKind {
    Operation {
        operation: StateOperation,
        affected_component_ids: Vec<String>,
    },
    BoundaryComponent {
        policy_id: String,
        boundary: crate::state::BoundaryKind,
        component_id: String,
        disposition: BoundaryDisposition,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionHistoryEvent {
    pub event_index: u64,
    pub source_snapshot_sequence: u64,
    pub application_id: String,
    pub result_snapshot_id: String,
    pub operation_index: u32,
    pub event: ExecutionHistoryKind,
}

impl ExecutionHistoryEvent {
    fn validate(&self) -> Result<(), PlannerContractError> {
        match &self.event {
            ExecutionHistoryKind::Operation {
                operation,
                affected_component_ids,
            } => {
                operation.validate()?;
                let mut previous = None;
                for component_id in affected_component_ids {
                    validate_stable_id("execution_history.affected_component_ids", component_id)?;
                    if previous.is_some_and(|prior: &str| prior >= component_id.as_str()) {
                        return Err(PlannerContractError::new(
                            "execution_history.affected_component_ids",
                            "must be unique and sorted",
                        ));
                    }
                    previous = Some(component_id.as_str());
                }
                Ok(())
            }
            ExecutionHistoryKind::BoundaryComponent {
                policy_id,
                boundary,
                component_id,
                disposition,
            } => {
                validate_stable_id("execution_history.policy_id", policy_id)?;
                validate_stable_id("execution_history.component_id", component_id)?;
                if let crate::state::BoundaryKind::Custom { id } = boundary {
                    validate_stable_id("execution_history.boundary.id", id)?;
                }
                match disposition {
                    BoundaryDisposition::Reinitialize { initializer_id } => {
                        validate_stable_id("execution_history.initializer_id", initializer_id)
                    }
                    BoundaryDisposition::Serialize { owner }
                    | BoundaryDisposition::Restore { owner } => validate_serialization_owner(owner),
                    BoundaryDisposition::Preserve
                    | BoundaryDisposition::Clear
                    | BoundaryDisposition::Unknown => Ok(()),
                }
            }
        }
    }
}

impl PersistentFileImage {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != PERSISTENT_FILE_IMAGE_SCHEMA {
            return Err(PlannerContractError::new(
                "persistent_file_image.schema",
                "is unsupported",
            ));
        }
        validate_stable_id("persistent_file_image.id", &self.id)?;
        validate_stable_id(
            "persistent_file_image.source_runtime_file_id",
            &self.source_runtime_file_id,
        )?;
        if self.runtime_components.is_empty() && self.stage_banks.is_empty() {
            return Err(PlannerContractError::new(
                "persistent_file_image",
                "must contain at least one serialized component or stage bank",
            ));
        }
        let runtime_owner = SerializationOwner::RuntimeFile {
            runtime_file_id: self.id.clone(),
        };
        validate_component_store(&runtime_owner, &self.runtime_components, true)?;
        for component in &self.runtime_components {
            validate_persistent_image_binding(&self.id, component)?;
        }
        let mut previous_owner = None;
        for store in &self.stage_banks {
            if previous_owner
                .as_ref()
                .is_some_and(|owner: &SerializationOwner| owner >= &store.owner)
            {
                return Err(PlannerContractError::new(
                    "persistent_file_image.stage_banks",
                    "must be unique and sorted by owner",
                ));
            }
            let SerializationOwner::StageBank {
                runtime_file_id, ..
            } = &store.owner
            else {
                return Err(PlannerContractError::new(
                    "persistent_file_image.stage_banks.owner",
                    "must contain only stage-bank owners",
                ));
            };
            if runtime_file_id != &self.id {
                return Err(PlannerContractError::new(
                    "persistent_file_image.stage_banks.owner",
                    "must be scoped to the persistent file image",
                ));
            }
            validate_component_store(&store.owner, &store.components, false)?;
            for component in &store.components {
                validate_persistent_image_binding(&self.id, component)?;
            }
            previous_owner = Some(store.owner.clone());
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

/// Mutable search state that keeps non-save backing stores separate from the
/// visible execution snapshot. Applying a batch is transactional: a failed
/// operation leaves every store and the snapshot unchanged.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannerExecutionState {
    pub snapshot: StateSnapshot,
    pub gate_states: BTreeMap<String, bool>,
    pub serialized_components: BTreeMap<SerializationOwner, Vec<StateComponent>>,
    pub persistent_file_images: BTreeMap<String, PersistentFileImage>,
    pub preserved_component_ids: BTreeSet<String>,
    pub scheduled_cleanup_ids: BTreeSet<String>,
    pub interruption_log: Vec<InterruptionRecord>,
    pub execution_history: Vec<ExecutionHistoryEvent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationApplication {
    pub source_snapshot_sha256: Digest,
    pub result_snapshot_sha256: Digest,
    pub operation_count: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SerializedComponentStore {
    pub owner: SerializationOwner,
    pub components: Vec<StateComponent>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PersistentFileImage {
    pub schema: String,
    pub id: String,
    pub source_runtime_file_id: String,
    pub runtime_components: Vec<StateComponent>,
    pub stage_banks: Vec<SerializedComponentStore>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlannerExecutionStateDocument {
    pub schema: String,
    pub snapshot: StateSnapshot,
    pub gate_states: BTreeMap<String, bool>,
    pub serialized_component_stores: Vec<SerializedComponentStore>,
    pub persistent_file_images: Vec<PersistentFileImage>,
    pub preserved_component_ids: BTreeSet<String>,
    pub scheduled_cleanup_ids: BTreeSet<String>,
    pub interruption_log: Vec<InterruptionRecord>,
    pub execution_history: Vec<ExecutionHistoryEvent>,
}

#[derive(Serialize)]
struct ExecutionStateIdentity<'a> {
    snapshot_sha256: Digest,
    gate_states: &'a BTreeMap<String, bool>,
    serialized_components: Vec<SerializedOwnerIdentity<'a>>,
    persistent_file_images: Vec<&'a PersistentFileImage>,
    preserved_component_ids: &'a BTreeSet<String>,
    scheduled_cleanup_ids: &'a BTreeSet<String>,
    interruption_log: &'a [InterruptionRecord],
    execution_history: &'a [ExecutionHistoryEvent],
}

#[derive(Serialize)]
struct SerializedOwnerIdentity<'a> {
    owner: &'a SerializationOwner,
    components: &'a [StateComponent],
}

impl PlannerExecutionState {
    pub fn new(snapshot: StateSnapshot) -> Result<Self, PlannerContractError> {
        let state = Self {
            snapshot,
            gate_states: BTreeMap::new(),
            serialized_components: BTreeMap::new(),
            persistent_file_images: BTreeMap::new(),
            preserved_component_ids: BTreeSet::new(),
            scheduled_cleanup_ids: BTreeSet::new(),
            interruption_log: Vec::new(),
            execution_history: Vec::new(),
        };
        state.validate()?;
        Ok(state)
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        self.snapshot.validate()?;
        for id in self.gate_states.keys() {
            validate_stable_id("gate_states.id", id)?;
        }
        for (owner, components) in &self.serialized_components {
            validate_component_store(owner, components, false)?;
        }
        for (id, image) in &self.persistent_file_images {
            if id != &image.id {
                return Err(PlannerContractError::new(
                    "persistent_file_images.id",
                    "map key and image ID disagree",
                ));
            }
            image.validate()?;
        }
        let referenced_images = self
            .snapshot
            .environment
            .physical_slots
            .iter()
            .map(|slot| slot.persistent_file_id.as_str())
            .collect::<BTreeSet<_>>();
        if referenced_images.len() != self.persistent_file_images.len() {
            return Err(PlannerContractError::new(
                "persistent_file_images",
                "must correspond one-to-one with populated physical slots",
            ));
        }
        for slot in &self.snapshot.environment.physical_slots {
            let image = self
                .persistent_file_images
                .get(&slot.persistent_file_id)
                .ok_or_else(|| {
                    PlannerContractError::new(
                        "physical_slots.persistent_file_id",
                        "references an unavailable persistent file image",
                    )
                })?;
            if image.digest()? != slot.serialized_state_sha256 {
                return Err(PlannerContractError::new(
                    "physical_slots.serialized_state_sha256",
                    "does not seal the referenced persistent file image",
                ));
            }
        }
        for id in &self.preserved_component_ids {
            validate_stable_id("preserved_component_ids", id)?;
            if !self
                .snapshot
                .environment
                .components
                .iter()
                .any(|component| &component.id == id)
            {
                return Err(PlannerContractError::new(
                    "preserved_component_ids",
                    "references a component absent from the current snapshot",
                ));
            }
        }
        for id in &self.scheduled_cleanup_ids {
            validate_stable_id("scheduled_cleanup_ids", id)?;
        }
        for interruption in &self.interruption_log {
            validate_stable_id("interruption_log.action_id", &interruption.action_id)?;
            interruption.window.validate()?;
        }
        if self.execution_history.len() > 1_000_000 {
            return Err(PlannerContractError::new(
                "execution_history",
                "must contain at most 1000000 events",
            ));
        }
        let mut previous_group: Option<(u64, &str, &str, u32)> = None;
        for (expected_index, event) in self.execution_history.iter().enumerate() {
            if event.event_index != expected_index as u64 {
                return Err(PlannerContractError::new(
                    "execution_history.event_index",
                    "must be contiguous and zero-based",
                ));
            }
            validate_stable_id("execution_history.application_id", &event.application_id)?;
            validate_stable_id(
                "execution_history.result_snapshot_id",
                &event.result_snapshot_id,
            )?;
            if event.source_snapshot_sequence > self.snapshot.sequence {
                return Err(PlannerContractError::new(
                    "execution_history.source_snapshot_sequence",
                    "cannot exceed the current snapshot sequence",
                ));
            }
            let same_group = previous_group.is_some_and(|(sequence, application, result, _)| {
                sequence == event.source_snapshot_sequence
                    && application == event.application_id
                    && result == event.result_snapshot_id
            });
            if (same_group
                && previous_group.is_some_and(|(_, _, _, operation_index)| {
                    operation_index.checked_add(1) != Some(event.operation_index)
                }))
                || (!same_group && event.operation_index != 0)
            {
                return Err(PlannerContractError::new(
                    "execution_history.operation_index",
                    "must be contiguous and zero-based within each application",
                ));
            }
            event.validate()?;
            previous_group = Some((
                event.source_snapshot_sequence,
                &event.application_id,
                &event.result_snapshot_id,
                event.operation_index,
            ));
        }
        Ok(())
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        self.validate()?;
        let identity = ExecutionStateIdentity {
            snapshot_sha256: self.snapshot.digest()?,
            gate_states: &self.gate_states,
            serialized_components: self
                .serialized_components
                .iter()
                .map(|(owner, components)| SerializedOwnerIdentity { owner, components })
                .collect(),
            persistent_file_images: self.persistent_file_images.values().collect(),
            preserved_component_ids: &self.preserved_component_ids,
            scheduled_cleanup_ids: &self.scheduled_cleanup_ids,
            interruption_log: &self.interruption_log,
            execution_history: &self.execution_history,
        };
        Ok(Digest(Sha256::digest(canonical_json(&identity)?).into()))
    }

    /// Identity used for search dominance. Snapshot labels, sequence counters,
    /// transition provenance, and interruption history explain how a state was
    /// reached but do not make its live game state different.
    pub fn semantic_digest(&self) -> Result<Digest, PlannerContractError> {
        self.validate()?;
        let mut normalized = self.clone();
        normalized.snapshot.id = "search-state".into();
        normalized.snapshot.sequence = 0;
        normalized
            .snapshot
            .environment
            .inactive_runtime_files
            .clear();
        for component in &mut normalized.snapshot.environment.components {
            normalize_provenance(component);
        }
        for components in normalized.serialized_components.values_mut() {
            for component in components {
                normalize_provenance(component);
            }
        }
        for image in normalized.persistent_file_images.values_mut() {
            image.source_runtime_file_id = "search-source".into();
            for component in &mut image.runtime_components {
                normalize_provenance(component);
            }
            for store in &mut image.stage_banks {
                for component in &mut store.components {
                    normalize_provenance(component);
                }
            }
        }
        for slot in &mut normalized.snapshot.environment.physical_slots {
            slot.serialized_state_sha256 =
                normalized.persistent_file_images[&slot.persistent_file_id].digest()?;
        }
        normalized.interruption_log.clear();
        normalized.execution_history.clear();
        normalized.digest()
    }

    pub fn to_document(&self) -> Result<PlannerExecutionStateDocument, PlannerContractError> {
        self.validate()?;
        Ok(PlannerExecutionStateDocument {
            schema: PLANNER_EXECUTION_STATE_SCHEMA.into(),
            snapshot: self.snapshot.clone(),
            gate_states: self.gate_states.clone(),
            serialized_component_stores: self
                .serialized_components
                .iter()
                .map(|(owner, components)| SerializedComponentStore {
                    owner: owner.clone(),
                    components: components.clone(),
                })
                .collect(),
            persistent_file_images: self.persistent_file_images.values().cloned().collect(),
            preserved_component_ids: self.preserved_component_ids.clone(),
            scheduled_cleanup_ids: self.scheduled_cleanup_ids.clone(),
            interruption_log: self.interruption_log.clone(),
            execution_history: self.execution_history.clone(),
        })
    }

    pub fn apply_operations(
        &mut self,
        application_id: &str,
        result_snapshot_id: &str,
        operations: &[StateOperation],
    ) -> Result<OperationApplication, PlannerContractError> {
        validate_stable_id("application_id", application_id)?;
        validate_stable_id("result_snapshot_id", result_snapshot_id)?;
        let source_snapshot_sha256 = self.snapshot.digest()?;
        let mut next = self.clone();
        for (operation_index, operation) in operations.iter().enumerate() {
            operation.validate()?;
            let affected_component_ids = next.affected_component_ids(operation);
            next.apply_operation(application_id, operation)?;
            next.push_history(
                self.snapshot.sequence,
                application_id,
                result_snapshot_id,
                u32::try_from(operation_index).map_err(|_| {
                    PlannerContractError::new(
                        "operations",
                        "contains more operations than can be indexed",
                    )
                })?,
                ExecutionHistoryKind::Operation {
                    operation: operation.clone(),
                    affected_component_ids,
                },
            )?;
        }
        next.snapshot.sequence = next.snapshot.sequence.checked_add(1).ok_or_else(|| {
            PlannerContractError::new("snapshot.sequence", "cannot advance past u64::MAX")
        })?;
        next.snapshot.id = result_snapshot_id.into();
        next.sort_components();
        next.validate()?;
        let result_snapshot_sha256 = next.snapshot.digest()?;
        *self = next;
        Ok(OperationApplication {
            source_snapshot_sha256,
            result_snapshot_sha256,
            operation_count: operations.len(),
        })
    }

    /// Applies a boundary policy to every live component. An explicit
    /// `Preserve` operation is a one-boundary override; otherwise exactly one
    /// component rule or the default disposition controls the component.
    /// `Unknown` fails the entire boundary instead of silently preserving data.
    pub fn apply_boundary(
        &mut self,
        application_id: &str,
        result_snapshot_id: &str,
        policy: &BoundaryPolicy,
        initializers: &BTreeMap<String, StateComponent>,
    ) -> Result<OperationApplication, PlannerContractError> {
        validate_stable_id("application_id", application_id)?;
        validate_stable_id("result_snapshot_id", result_snapshot_id)?;
        policy.validate()?;
        for (id, component) in initializers {
            validate_stable_id("initializers.id", id)?;
            component.validate()?;
        }
        let source_snapshot_sha256 = self.snapshot.digest()?;
        let mut next = self.clone();
        let dispositions = next
            .snapshot
            .environment
            .components
            .iter()
            .map(|component| {
                Ok((
                    component.clone(),
                    next.boundary_disposition(policy, component)?,
                ))
            })
            .collect::<Result<Vec<_>, PlannerContractError>>()?;

        // Serialization completes before restoration so a policy naming both
        // has deterministic writer-then-reader behavior.
        for (component, disposition) in &dispositions {
            if let BoundaryDisposition::Serialize { owner } = disposition {
                let mut serialized = component.clone();
                serialized.serialization_owner = owner.clone();
                mark_transition(&mut serialized, application_id);
                insert_serialized(&mut next.serialized_components, owner, serialized);
            }
        }

        let operation_count = dispositions.len();
        let mut resulting_components = Vec::new();
        for (operation_index, (mut component, disposition)) in dispositions.into_iter().enumerate()
        {
            let component_id = component.id.clone();
            let history_disposition = disposition.clone();
            match disposition {
                BoundaryDisposition::Preserve => {
                    mark_transition(&mut component, application_id);
                    resulting_components.push(component);
                }
                BoundaryDisposition::Clear | BoundaryDisposition::Serialize { .. } => {}
                BoundaryDisposition::Reinitialize { initializer_id } => {
                    let mut initialized =
                        initializers.get(&initializer_id).cloned().ok_or_else(|| {
                            PlannerContractError::new(
                                "boundary.initializer_id",
                                "references an unavailable initializer",
                            )
                        })?;
                    if initialized.id != component.id {
                        return Err(PlannerContractError::new(
                            "boundary.initializer_id",
                            "initializer component ID must match the component it replaces",
                        ));
                    }
                    mark_transition(&mut initialized, application_id);
                    resulting_components.push(initialized);
                }
                BoundaryDisposition::Restore { owner } => {
                    let mut restored =
                        select_serialized(&next.serialized_components, &owner, &component.id)?
                            .clone();
                    restored.id = component.id;
                    mark_transition(&mut restored, application_id);
                    resulting_components.push(restored);
                }
                BoundaryDisposition::Unknown => {
                    return Err(PlannerContractError::new(
                        "boundary.disposition",
                        format!("component {} has unknown boundary behavior", component.id),
                    ));
                }
            }
            next.push_history(
                self.snapshot.sequence,
                application_id,
                result_snapshot_id,
                u32::try_from(operation_index).map_err(|_| {
                    PlannerContractError::new(
                        "boundary",
                        "contains more component dispositions than can be indexed",
                    )
                })?,
                ExecutionHistoryKind::BoundaryComponent {
                    policy_id: policy.id.clone(),
                    boundary: policy.boundary.clone(),
                    component_id,
                    disposition: history_disposition,
                },
            )?;
        }
        next.snapshot.environment.components = resulting_components;
        next.preserved_component_ids.clear();
        next.snapshot.sequence = next.snapshot.sequence.checked_add(1).ok_or_else(|| {
            PlannerContractError::new("snapshot.sequence", "cannot advance past u64::MAX")
        })?;
        next.snapshot.id = result_snapshot_id.into();
        next.sort_components();
        next.validate()?;
        let result_snapshot_sha256 = next.snapshot.digest()?;
        *self = next;
        Ok(OperationApplication {
            source_snapshot_sha256,
            result_snapshot_sha256,
            operation_count,
        })
    }

    pub fn last_field_writer(
        &self,
        component_id: &str,
        field: &str,
    ) -> Option<&ExecutionHistoryEvent> {
        self.execution_history
            .iter()
            .rev()
            .find(|event| history_event_writes_field(event, component_id, field))
    }

    pub fn gate_history(&self, gate_id: &str) -> Vec<&ExecutionHistoryEvent> {
        self.execution_history
            .iter()
            .filter(|event| {
                matches!(
                    &event.event,
                    ExecutionHistoryKind::Operation {
                        operation: StateOperation::SetGate { gate_id: changed }
                            | StateOperation::ClearGate { gate_id: changed },
                        ..
                    } if changed == gate_id
                )
            })
            .collect()
    }

    fn push_history(
        &mut self,
        source_snapshot_sequence: u64,
        application_id: &str,
        result_snapshot_id: &str,
        operation_index: u32,
        event: ExecutionHistoryKind,
    ) -> Result<(), PlannerContractError> {
        let event_index = u64::try_from(self.execution_history.len()).map_err(|_| {
            PlannerContractError::new("execution_history", "event index does not fit in u64")
        })?;
        self.execution_history.push(ExecutionHistoryEvent {
            event_index,
            source_snapshot_sequence,
            application_id: application_id.into(),
            result_snapshot_id: result_snapshot_id.into(),
            operation_index,
            event,
        });
        Ok(())
    }

    fn affected_component_ids(&self, operation: &StateOperation) -> Vec<String> {
        let mut ids = match operation {
            StateOperation::Write { target, .. }
            | StateOperation::CopyValue { target, .. }
            | StateOperation::SetBitFromValue { target, .. }
            | StateOperation::Adjust { target, .. }
            | StateOperation::ClearField { target }
            | StateOperation::InvalidateField { target } => vec![target.component_id.clone()],
            StateOperation::WriteFields { component_id, .. }
            | StateOperation::ReplacePayload { component_id, .. } => {
                vec![component_id.clone()]
            }
            StateOperation::InvalidatePayloads {
                selector,
                include_active_runtime_serialized_stores,
            } => self
                .matching_ids_including_serialized(
                    selector,
                    *include_active_runtime_serialized_stores,
                )
                .into_iter()
                .collect(),
            StateOperation::InvalidateActiveRuntimeSerializedPayloads { selector } => {
                let active_runtime_file_id = &self.snapshot.environment.active_runtime_file.id;
                self.serialized_components
                    .iter()
                    .filter(|(owner, _)| owner_belongs_to_runtime(owner, active_runtime_file_id))
                    .flat_map(|(_, components)| {
                        components
                            .iter()
                            .filter(|component| selector_matches(selector, component))
                            .map(|component| component.id.clone())
                    })
                    .collect()
            }
            StateOperation::WriteRaw { component_id, .. }
            | StateOperation::InvalidateRaw { component_id, .. }
            | StateOperation::CommitLoadStageBank { component_id, .. }
            | StateOperation::ActivateStageBank { component_id, .. } => {
                vec![component_id.clone()]
            }
            StateOperation::WriteBoundRaw {
                component_kind,
                binding,
                ..
            }
            | StateOperation::InvalidateBoundRaw {
                component_kind,
                binding,
                ..
            }
            | StateOperation::AdjustBoundRawUnsigned {
                component_kind,
                binding,
                ..
            } => {
                let resolved_binding = binding.resolve(&self.snapshot.environment);
                self.snapshot
                    .environment
                    .components
                    .iter()
                    .filter(|component| {
                        component.component_kind == *component_kind
                            && resolved_binding
                                .as_ref()
                                .is_some_and(|binding| component.binding == *binding)
                            && matches!(component.payload, ComponentPayload::Raw { .. })
                    })
                    .map(|component| component.id.clone())
                    .collect()
            }
            StateOperation::ClearComponent { selector }
            | StateOperation::Preserve { selector }
            | StateOperation::Serialize { selector, .. }
            | StateOperation::Bind { selector, .. }
            | StateOperation::Rebind { selector, .. } => {
                self.matching_ids(selector).into_iter().collect()
            }
            StateOperation::Initialize { component } => vec![component.id.clone()],
            StateOperation::ReplaceCustomStore { components, .. } => components
                .iter()
                .map(|component| component.id.clone())
                .collect(),
            StateOperation::RestorePayloadsFromCustomStore { component_ids, .. } => {
                component_ids.clone()
            }
            StateOperation::Copy {
                destination_component_id,
                ..
            }
            | StateOperation::Restore {
                destination_component_id,
                ..
            } => vec![destination_component_id.clone()],
            StateOperation::Move {
                source,
                destination_component_id,
                ..
            } => {
                let mut ids = self.matching_ids(source).into_iter().collect::<Vec<_>>();
                ids.push(destination_component_id.clone());
                ids
            }
            StateOperation::Project { component_ids, .. } => component_ids.clone(),
            StateOperation::SaveRuntimeToSlot {
                source_runtime_file_id,
                runtime_component_ids,
                stage_bank_stages,
                ..
            } => {
                let selected_stages = stage_bank_stages
                    .iter()
                    .map(String::as_str)
                    .collect::<BTreeSet<_>>();
                let mut ids = runtime_component_ids.clone();
                ids.extend(
                    self.snapshot
                        .environment
                        .components
                        .iter()
                        .filter(|component| {
                            matches!(
                                &component.serialization_owner,
                                SerializationOwner::StageBank { runtime_file_id, stage }
                                    if runtime_file_id == source_runtime_file_id
                                        && selected_stages.contains(stage.as_str())
                            )
                        })
                        .map(|component| component.id.clone()),
                );
                ids.extend(
                    self.serialized_components
                        .iter()
                        .flat_map(|(owner, components)| {
                            let selected = matches!(
                                owner,
                                SerializationOwner::StageBank { runtime_file_id, stage }
                                    if runtime_file_id == source_runtime_file_id
                                        && selected_stages.contains(stage.as_str())
                            );
                            components
                                .iter()
                                .filter(move |_| selected)
                                .map(|component| component.id.clone())
                        }),
                );
                ids
            }
            StateOperation::LoadRuntimeFromSlot {
                source_runtime_file_id,
                runtime_component_ids,
                ..
            } => {
                let mut ids = runtime_component_ids.clone();
                ids.extend(
                    self.snapshot
                        .environment
                        .components
                        .iter()
                        .filter(|component| {
                            component_belongs_to_runtime(component, source_runtime_file_id)
                        })
                        .map(|component| component.id.clone()),
                );
                ids.extend(
                    self.serialized_components
                        .iter()
                        .filter(|(owner, _)| {
                            owner_belongs_to_runtime(owner, source_runtime_file_id)
                        })
                        .flat_map(|(_, components)| {
                            components.iter().map(|component| component.id.clone())
                        }),
                );
                ids
            }
            StateOperation::LoadActiveRuntimeFromSlot { source_slot, .. } => {
                let source_runtime_file_id = &self.snapshot.environment.active_runtime_file.id;
                let mut ids = self
                    .snapshot
                    .environment
                    .physical_slots
                    .iter()
                    .find(|slot| slot.slot == *source_slot)
                    .and_then(|slot| self.persistent_file_images.get(&slot.persistent_file_id))
                    .into_iter()
                    .flat_map(|image| {
                        image
                            .runtime_components
                            .iter()
                            .chain(
                                image
                                    .stage_banks
                                    .iter()
                                    .flat_map(|store| store.components.iter()),
                            )
                            .map(|component| component.id.clone())
                    })
                    .collect::<Vec<_>>();
                ids.extend(
                    self.snapshot
                        .environment
                        .components
                        .iter()
                        .filter(|component| {
                            component_belongs_to_runtime(component, source_runtime_file_id)
                        })
                        .map(|component| component.id.clone()),
                );
                ids.extend(
                    self.serialized_components
                        .iter()
                        .filter(|(owner, _)| {
                            owner_belongs_to_runtime(owner, source_runtime_file_id)
                        })
                        .flat_map(|(_, components)| {
                            components.iter().map(|component| component.id.clone())
                        }),
                );
                ids
            }
            StateOperation::BeginRuntimeFileLifetime { .. } => {
                let source_runtime_file_id = &self.snapshot.environment.active_runtime_file.id;
                let mut ids = self
                    .snapshot
                    .environment
                    .components
                    .iter()
                    .filter(|component| {
                        component_belongs_to_runtime(component, source_runtime_file_id)
                    })
                    .map(|component| component.id.clone())
                    .collect::<Vec<_>>();
                ids.extend(
                    self.serialized_components
                        .iter()
                        .filter(|(owner, _)| {
                            owner_belongs_to_runtime(owner, source_runtime_file_id)
                        })
                        .flat_map(|(_, components)| {
                            components.iter().map(|component| component.id.clone())
                        }),
                );
                ids
            }
            StateOperation::Consume {
                pending_operation_id,
            } => vec![pending_operation_id.clone()],
            StateOperation::AdvanceFlow {
                flow_component_id, ..
            }
            | StateOperation::BranchFlow {
                flow_component_id, ..
            } => vec![flow_component_id.clone()],
            StateOperation::SetActiveRuntimeFile { .. }
            | StateOperation::SetExecutionContext { .. }
            | StateOperation::CompletePendingWorldLoad
            | StateOperation::SetLocation { .. }
            | StateOperation::SetLocationFromFields { .. }
            | StateOperation::SetPendingWorldLoadFromFields { .. }
            | StateOperation::SetPlayerForm { .. }
            | StateOperation::SetPlayerMount { .. }
            | StateOperation::SetPlayerControl { .. }
            | StateOperation::SetPlayerAction { .. }
            | StateOperation::SetGate { .. }
            | StateOperation::ClearGate { .. }
            | StateOperation::ScheduleCleanup { .. }
            | StateOperation::CancelCleanup { .. }
            | StateOperation::Interrupt { .. } => Vec::new(),
        };
        ids.sort();
        ids.dedup();
        ids
    }

    fn apply_operation(
        &mut self,
        application_id: &str,
        operation: &StateOperation,
    ) -> Result<(), PlannerContractError> {
        match operation {
            StateOperation::Write { target, value } => {
                let component = self.component_mut(&target.component_id)?;
                let ComponentPayload::Structured { fields } = &mut component.payload else {
                    return Err(PlannerContractError::new(
                        "operation.write",
                        "requires a structured destination component",
                    ));
                };
                fields.insert(target.field.clone(), value.clone());
                mark_transition(component, application_id);
            }
            StateOperation::WriteFields {
                component_id,
                fields: replacements,
            } => {
                let component = self.component_mut(component_id)?;
                let ComponentPayload::Structured { fields } = &mut component.payload else {
                    return Err(PlannerContractError::new(
                        "operation.write_fields",
                        "requires a structured destination component",
                    ));
                };
                for (field, value) in replacements {
                    fields.insert(field.clone(), value.clone());
                }
                mark_transition(component, application_id);
            }
            StateOperation::ReplacePayload {
                component_id,
                payload,
            } => {
                let component = self.component_mut(component_id)?;
                component.payload = payload.clone();
                mark_transition(component, application_id);
            }
            StateOperation::InvalidatePayloads {
                selector,
                include_active_runtime_serialized_stores,
            } => {
                let live_ids = self.matching_ids(selector);
                let mut matched = !live_ids.is_empty();
                for id in live_ids {
                    let component = self.component_mut(&id)?;
                    invalidate_payload(component);
                    mark_transition(component, application_id);
                }
                if *include_active_runtime_serialized_stores {
                    let active_runtime_file_id =
                        self.snapshot.environment.active_runtime_file.id.clone();
                    for (owner, components) in &mut self.serialized_components {
                        if !owner_belongs_to_runtime(owner, &active_runtime_file_id) {
                            continue;
                        }
                        for component in components
                            .iter_mut()
                            .filter(|component| selector_matches(selector, component))
                        {
                            matched = true;
                            invalidate_payload(component);
                            mark_transition(component, application_id);
                        }
                    }
                }
                if !matched {
                    return Err(PlannerContractError::new(
                        "operation.invalidate_payloads",
                        "selector matches no live or selected serialized component",
                    ));
                }
            }
            StateOperation::InvalidateActiveRuntimeSerializedPayloads { selector } => {
                let active_runtime_file_id =
                    self.snapshot.environment.active_runtime_file.id.clone();
                for (owner, components) in &mut self.serialized_components {
                    if !owner_belongs_to_runtime(owner, &active_runtime_file_id) {
                        continue;
                    }
                    for component in components
                        .iter_mut()
                        .filter(|component| selector_matches(selector, component))
                    {
                        invalidate_payload(component);
                        mark_transition(component, application_id);
                    }
                }
            }
            StateOperation::CopyValue { source, target } => {
                let value = self.structured_value(source, "operation.copy_value")?;
                let component = self.component_mut(&target.component_id)?;
                let ComponentPayload::Structured { fields } = &mut component.payload else {
                    return Err(PlannerContractError::new(
                        "operation.copy_value",
                        "requires a structured destination component",
                    ));
                };
                fields.insert(target.field.clone(), value);
                mark_transition(component, application_id);
            }
            StateOperation::SetBitFromValue { source, target } => {
                let index = match self.structured_value(source, "operation.set_bit_from_value")? {
                    StateValue::Unsigned(value) => usize::try_from(value).map_err(|_| {
                        PlannerContractError::new(
                            "operation.set_bit_from_value",
                            "source value does not fit this host",
                        )
                    })?,
                    StateValue::Signed(value) if value >= 0 => {
                        usize::try_from(value).map_err(|_| {
                            PlannerContractError::new(
                                "operation.set_bit_from_value",
                                "source value does not fit this host",
                            )
                        })?
                    }
                    _ => {
                        return Err(PlannerContractError::new(
                            "operation.set_bit_from_value",
                            "requires a nonnegative integer source field",
                        ));
                    }
                };
                let component = self.component_mut(&target.component_id)?;
                let ComponentPayload::Structured { fields } = &mut component.payload else {
                    return Err(PlannerContractError::new(
                        "operation.set_bit_from_value",
                        "requires a structured destination component",
                    ));
                };
                let StateValue::Bytes(bits) = fields.get_mut(&target.field).ok_or_else(|| {
                    PlannerContractError::new(
                        "operation.set_bit_from_value",
                        "references an absent destination bit set",
                    )
                })?
                else {
                    return Err(PlannerContractError::new(
                        "operation.set_bit_from_value",
                        "requires a byte-backed destination bit set",
                    ));
                };
                let byte_index = index / 8;
                let Some(byte) = bits.get_mut(byte_index) else {
                    return Err(PlannerContractError::new(
                        "operation.set_bit_from_value",
                        "source index exceeds the destination bit set",
                    ));
                };
                *byte |= 1_u8 << (index % 8);
                mark_transition(component, application_id);
            }
            StateOperation::WriteRaw {
                component_id,
                byte_offset,
                mask,
                value,
            } => {
                let component = self.component_mut(component_id)?;
                let ComponentPayload::Raw { bytes, known_mask } = &mut component.payload else {
                    return Err(PlannerContractError::new(
                        "operation.write_raw",
                        "requires a raw destination component",
                    ));
                };
                let offset = usize::try_from(*byte_offset).map_err(|_| {
                    PlannerContractError::new(
                        "operation.write_raw.byte_offset",
                        "does not fit this host",
                    )
                })?;
                let end = offset.checked_add(mask.len()).ok_or_else(|| {
                    PlannerContractError::new("operation.write_raw", "range overflows")
                })?;
                if end > bytes.len() || end > known_mask.len() {
                    return Err(PlannerContractError::new(
                        "operation.write_raw",
                        "range exceeds the destination component",
                    ));
                }
                for index in 0..mask.len() {
                    let selected = mask[index];
                    bytes[offset + index] =
                        (bytes[offset + index] & !selected) | (value[index] & selected);
                    known_mask[offset + index] |= selected;
                }
                mark_transition(component, application_id);
            }
            StateOperation::WriteBoundRaw {
                component_kind,
                binding,
                byte_offset,
                mask,
                value,
            } => {
                let component_id = self.unique_bound_raw_component_id(
                    component_kind,
                    binding,
                    "operation.write_bound_raw",
                )?;
                let component = self.component_mut(&component_id)?;
                let ComponentPayload::Raw { bytes, known_mask } = &mut component.payload else {
                    unreachable!("bound raw selection accepted a non-raw component")
                };
                let offset = checked_raw_range(
                    *byte_offset,
                    mask.len(),
                    bytes.len(),
                    known_mask.len(),
                    "operation.write_bound_raw",
                )?;
                for index in 0..mask.len() {
                    let selected = mask[index];
                    bytes[offset + index] =
                        (bytes[offset + index] & !selected) | (value[index] & selected);
                    known_mask[offset + index] |= selected;
                }
                mark_transition(component, application_id);
            }
            StateOperation::InvalidateRaw {
                component_id,
                byte_offset,
                mask,
            } => {
                let component = self.component_mut(component_id)?;
                let ComponentPayload::Raw { bytes, known_mask } = &mut component.payload else {
                    return Err(PlannerContractError::new(
                        "operation.invalidate_raw",
                        "requires a raw destination component",
                    ));
                };
                let offset = usize::try_from(*byte_offset).map_err(|_| {
                    PlannerContractError::new(
                        "operation.invalidate_raw.byte_offset",
                        "does not fit this host",
                    )
                })?;
                let end = offset.checked_add(mask.len()).ok_or_else(|| {
                    PlannerContractError::new("operation.invalidate_raw", "range overflows")
                })?;
                if end > bytes.len() || end > known_mask.len() {
                    return Err(PlannerContractError::new(
                        "operation.invalidate_raw",
                        "range exceeds the destination component",
                    ));
                }
                for index in 0..mask.len() {
                    known_mask[offset + index] &= !mask[index];
                }
                mark_transition(component, application_id);
            }
            StateOperation::InvalidateBoundRaw {
                component_kind,
                binding,
                byte_offset,
                mask,
            } => {
                let component_id = self.unique_bound_raw_component_id(
                    component_kind,
                    binding,
                    "operation.invalidate_bound_raw",
                )?;
                let component = self.component_mut(&component_id)?;
                let ComponentPayload::Raw { bytes, known_mask } = &mut component.payload else {
                    unreachable!("bound raw selection accepted a non-raw component")
                };
                let offset = checked_raw_range(
                    *byte_offset,
                    mask.len(),
                    bytes.len(),
                    known_mask.len(),
                    "operation.invalidate_bound_raw",
                )?;
                for index in 0..mask.len() {
                    known_mask[offset + index] &= !mask[index];
                }
                mark_transition(component, application_id);
            }
            StateOperation::Adjust { target, delta } => {
                let component = self.component_mut(&target.component_id)?;
                let ComponentPayload::Structured { fields } = &mut component.payload else {
                    return Err(PlannerContractError::new(
                        "operation.adjust",
                        "requires a structured destination component",
                    ));
                };
                let value = fields.get_mut(&target.field).ok_or_else(|| {
                    PlannerContractError::new("operation.adjust", "references an absent field")
                })?;
                adjust_value(value, *delta)?;
                mark_transition(component, application_id);
            }
            StateOperation::AdjustBoundRawUnsigned {
                component_kind,
                binding,
                byte_offset,
                byte_width,
                delta,
            } => {
                let resolved_binding = binding.resolve(&self.snapshot.environment);
                let matching_ids = self
                    .snapshot
                    .environment
                    .components
                    .iter()
                    .filter(|component| {
                        component.component_kind == *component_kind
                            && resolved_binding
                                .as_ref()
                                .is_some_and(|binding| component.binding == *binding)
                            && matches!(component.payload, ComponentPayload::Raw { .. })
                    })
                    .map(|component| component.id.clone())
                    .collect::<Vec<_>>();
                let [component_id] = matching_ids.as_slice() else {
                    return Err(PlannerContractError::new(
                        "operation.adjust_bound_raw_unsigned",
                        "requires exactly one component with the selected kind and binding",
                    ));
                };
                let component = self.component_mut(component_id)?;
                let ComponentPayload::Raw { bytes, known_mask } = &mut component.payload else {
                    return Err(PlannerContractError::new(
                        "operation.adjust_bound_raw_unsigned",
                        "requires a raw destination component",
                    ));
                };
                let offset = usize::try_from(*byte_offset).map_err(|_| {
                    PlannerContractError::new(
                        "operation.adjust_bound_raw_unsigned.byte_offset",
                        "does not fit this host",
                    )
                })?;
                let width = usize::from(*byte_width);
                let end = offset.checked_add(width).ok_or_else(|| {
                    PlannerContractError::new(
                        "operation.adjust_bound_raw_unsigned",
                        "range overflows",
                    )
                })?;
                if end > bytes.len() || end > known_mask.len() {
                    return Err(PlannerContractError::new(
                        "operation.adjust_bound_raw_unsigned",
                        "range exceeds the destination component",
                    ));
                }
                if known_mask[offset..end].iter().any(|known| *known != 0xff) {
                    return Err(PlannerContractError::new(
                        "operation.adjust_bound_raw_unsigned",
                        "requires every source bit to be known",
                    ));
                }
                let mut current = 0_u64;
                for index in 0..width {
                    current |= u64::from(bytes[offset + index]) << (index * 8);
                }
                let adjusted = if *delta > 0 {
                    current.checked_add(delta.unsigned_abs())
                } else {
                    current.checked_sub(delta.unsigned_abs())
                }
                .ok_or_else(|| {
                    PlannerContractError::new(
                        "operation.adjust_bound_raw_unsigned",
                        "would underflow or overflow",
                    )
                })?;
                let maximum = if *byte_width == 8 {
                    u64::MAX
                } else {
                    (1_u64 << (u32::from(*byte_width) * 8)) - 1
                };
                if adjusted > maximum {
                    return Err(PlannerContractError::new(
                        "operation.adjust_bound_raw_unsigned",
                        "would exceed the selected byte width",
                    ));
                }
                let encoded = adjusted.to_le_bytes();
                bytes[offset..end].copy_from_slice(&encoded[..width]);
                mark_transition(component, application_id);
            }
            StateOperation::ClearComponent { selector } => {
                let ids = self.matching_ids(selector);
                if ids.is_empty() {
                    return Err(no_selector_match("operation.clear_component"));
                }
                self.snapshot
                    .environment
                    .components
                    .retain(|component| !ids.contains(&component.id));
                self.preserved_component_ids
                    .retain(|component_id| !ids.contains(component_id));
            }
            StateOperation::ClearField { target } => {
                let component = self.component_mut(&target.component_id)?;
                let ComponentPayload::Structured { fields } = &mut component.payload else {
                    return Err(PlannerContractError::new(
                        "operation.clear_field",
                        "requires a structured destination component",
                    ));
                };
                if fields.remove(&target.field).is_none() {
                    return Err(PlannerContractError::new(
                        "operation.clear_field",
                        "references an absent field",
                    ));
                }
                mark_transition(component, application_id);
            }
            StateOperation::InvalidateField { target } => {
                let component = self.component_mut(&target.component_id)?;
                let ComponentPayload::Structured { fields } = &mut component.payload else {
                    return Err(PlannerContractError::new(
                        "operation.invalidate_field",
                        "requires a structured destination component",
                    ));
                };
                // Missing already means unknown to structured-field readers, so
                // invalidation is intentionally idempotent.
                fields.remove(&target.field);
                mark_transition(component, application_id);
            }
            StateOperation::Initialize { component } => {
                self.require_absent_component(&component.id)?;
                let mut component = component.clone();
                mark_transition(&mut component, application_id);
                self.snapshot.environment.components.push(component);
            }
            StateOperation::Copy {
                source,
                destination_component_id,
                binding,
                serialization_owner,
            } => {
                self.require_absent_component(destination_component_id)?;
                let source = self.single_component(source, "operation.copy")?.clone();
                let mut destination = source;
                destination.id = destination_component_id.clone();
                destination.binding = binding.clone();
                destination.serialization_owner = serialization_owner.clone();
                mark_transition(&mut destination, application_id);
                self.snapshot.environment.components.push(destination);
            }
            StateOperation::Move {
                source,
                destination_component_id,
                binding,
                serialization_owner,
            } => {
                let source_id = self.single_component(source, "operation.move")?.id.clone();
                if source_id != *destination_component_id {
                    self.require_absent_component(destination_component_id)?;
                }
                let index = self.component_index(&source_id)?;
                let mut destination = self.snapshot.environment.components.remove(index);
                self.preserved_component_ids.remove(&source_id);
                destination.id = destination_component_id.clone();
                destination.binding = binding.clone();
                destination.serialization_owner = serialization_owner.clone();
                mark_transition(&mut destination, application_id);
                self.snapshot.environment.components.push(destination);
            }
            StateOperation::Preserve { selector } => {
                let ids = self.matching_ids(selector);
                if ids.is_empty() {
                    return Err(no_selector_match("operation.preserve"));
                }
                self.preserved_component_ids.extend(ids);
            }
            StateOperation::Serialize { selector, owner } => {
                let matches = self
                    .matching_components(selector)
                    .into_iter()
                    .cloned()
                    .collect::<Vec<_>>();
                if matches.is_empty() {
                    return Err(no_selector_match("operation.serialize"));
                }
                let store = self.serialized_components.entry(owner.clone()).or_default();
                for component in matches {
                    let mut serialized = component;
                    serialized.serialization_owner = owner.clone();
                    mark_transition(&mut serialized, application_id);
                    match store.binary_search_by(|existing| existing.id.cmp(&serialized.id)) {
                        Ok(index) => store[index] = serialized,
                        Err(index) => store.insert(index, serialized),
                    }
                }
            }
            StateOperation::Restore {
                owner,
                destination_component_id,
            } => {
                self.require_absent_component(destination_component_id)?;
                let store = self.serialized_components.get(owner).ok_or_else(|| {
                    PlannerContractError::new(
                        "operation.restore",
                        "references an owner with no serialized components",
                    )
                })?;
                let source = if let Ok(index) = store.binary_search_by(|component| {
                    component.id.as_str().cmp(destination_component_id)
                }) {
                    &store[index]
                } else if let [only] = store.as_slice() {
                    only
                } else {
                    return Err(PlannerContractError::new(
                        "operation.restore",
                        "destination ID is ambiguous within the serialized owner store",
                    ));
                };
                let mut restored = source.clone();
                restored.id = destination_component_id.clone();
                mark_transition(&mut restored, application_id);
                self.snapshot.environment.components.push(restored);
            }
            StateOperation::ReplaceCustomStore { owner, components } => {
                let mut replacement = components.clone();
                for component in &mut replacement {
                    mark_transition(component, application_id);
                }
                self.serialized_components
                    .insert(owner.clone(), replacement);
            }
            StateOperation::RestorePayloadsFromCustomStore {
                owner,
                component_ids,
            } => {
                let sources = self
                    .serialized_components
                    .get(owner)
                    .ok_or_else(|| {
                        PlannerContractError::new(
                            "operation.restore_payloads_from_custom_store.owner",
                            "references an absent custom backing store",
                        )
                    })?
                    .clone();
                let source_ids = sources
                    .iter()
                    .map(|component| component.id.as_str())
                    .collect::<Vec<_>>();
                if source_ids != component_ids.iter().map(String::as_str).collect::<Vec<_>>() {
                    return Err(PlannerContractError::new(
                        "operation.restore_payloads_from_custom_store.component_ids",
                        "must exactly match the custom backing-store manifest",
                    ));
                }
                for source in &sources {
                    let destination = self
                        .snapshot
                        .environment
                        .components
                        .iter()
                        .find(|component| component.id == source.id)
                        .ok_or_else(|| {
                            PlannerContractError::new(
                                "operation.restore_payloads_from_custom_store.component_ids",
                                "references an absent same-ID live destination",
                            )
                        })?;
                    if destination.component_kind != source.component_kind {
                        return Err(PlannerContractError::new(
                            "operation.restore_payloads_from_custom_store.component_ids",
                            "source and destination component kinds must match",
                        ));
                    }
                }
                for source in sources {
                    let destination = self.component_mut(&source.id)?;
                    destination.payload = source.payload;
                    destination.provenance = source.provenance;
                    mark_save_restore(destination, application_id);
                }
            }
            operation @ StateOperation::CommitLoadStageBank { .. } => {
                self.commit_load_stage_bank(application_id, operation)?
            }
            operation @ StateOperation::ActivateStageBank { .. } => {
                self.activate_stage_bank(application_id, operation)?
            }
            operation @ StateOperation::SaveRuntimeToSlot { .. } => {
                self.save_runtime_to_slot(application_id, operation)?
            }
            operation @ StateOperation::LoadRuntimeFromSlot { .. } => {
                self.load_runtime_from_slot(application_id, operation)?
            }
            operation @ StateOperation::LoadActiveRuntimeFromSlot { .. } => {
                self.load_active_runtime_from_slot(application_id, operation)?
            }
            operation @ StateOperation::BeginRuntimeFileLifetime { .. } => {
                self.begin_runtime_file_lifetime(application_id, operation)?
            }
            StateOperation::Bind { selector, binding } => {
                let ids = self.matching_ids(selector);
                if ids.is_empty() {
                    return Err(no_selector_match("operation.bind"));
                }
                for id in ids {
                    let component = self.component_mut(&id)?;
                    if component.binding != ComponentBinding::Unbound {
                        return Err(PlannerContractError::new(
                            "operation.bind",
                            "requires every selected component to be unbound",
                        ));
                    }
                    component.binding = binding.clone();
                    mark_transition(component, application_id);
                }
            }
            StateOperation::Rebind { selector, binding } => {
                let ids = self.matching_ids(selector);
                if ids.is_empty() {
                    return Err(no_selector_match("operation.rebind"));
                }
                for id in ids {
                    let component = self.component_mut(&id)?;
                    component.binding = binding.clone();
                    mark_transition(component, application_id);
                }
            }
            StateOperation::SetActiveRuntimeFile { runtime_file } => {
                self.snapshot.environment.active_runtime_file = runtime_file.clone();
            }
            StateOperation::SetExecutionContext { context } => {
                self.snapshot.environment.execution_context = context.clone();
            }
            StateOperation::CompletePendingWorldLoad => {
                let ExecutionContext::Process {
                    process_name,
                    pending_world_load: Some(location),
                } = &self.snapshot.environment.execution_context
                else {
                    return Err(PlannerContractError::new(
                        "operation.complete_pending_world_load",
                        "requires a process context with one pending world load",
                    ));
                };
                self.snapshot.environment.location = location.clone();
                self.snapshot.environment.execution_context = ExecutionContext::Process {
                    process_name: process_name.clone(),
                    pending_world_load: None,
                };
            }
            StateOperation::SetLocation { location } => {
                self.snapshot.environment.execution_context = ExecutionContext::World;
                self.snapshot.environment.location = location.clone();
            }
            StateOperation::SetLocationFromFields {
                component_id,
                stage_field,
                room_field,
                spawn_field,
                layer,
            } => {
                let component = self
                    .snapshot
                    .environment
                    .components
                    .iter()
                    .find(|component| component.id == *component_id)
                    .ok_or_else(|| {
                        PlannerContractError::new(
                            "operation.set_location_from_fields",
                            "references an absent source component",
                        )
                    })?;
                let ComponentPayload::Structured { fields } = &component.payload else {
                    return Err(PlannerContractError::new(
                        "operation.set_location_from_fields",
                        "requires a structured source component",
                    ));
                };
                let stage = match fields.get(stage_field) {
                    Some(StateValue::Text(stage)) => stage.clone(),
                    _ => {
                        return Err(PlannerContractError::new(
                            "operation.set_location_from_fields.stage",
                            "requires a known text field",
                        ));
                    }
                };
                let room = match fields.get(room_field) {
                    Some(StateValue::Signed(room)) => i8::try_from(*room),
                    Some(StateValue::Unsigned(room)) => i8::try_from(*room),
                    _ => {
                        return Err(PlannerContractError::new(
                            "operation.set_location_from_fields.room",
                            "requires a known integer field",
                        ));
                    }
                }
                .map_err(|_| {
                    PlannerContractError::new(
                        "operation.set_location_from_fields.room",
                        "does not fit an i8 room number",
                    )
                })?;
                let spawn = match fields.get(spawn_field) {
                    Some(StateValue::Signed(spawn)) => i16::try_from(*spawn),
                    Some(StateValue::Unsigned(spawn)) => i16::try_from(*spawn),
                    _ => {
                        return Err(PlannerContractError::new(
                            "operation.set_location_from_fields.spawn",
                            "requires a known integer field",
                        ));
                    }
                }
                .map_err(|_| {
                    PlannerContractError::new(
                        "operation.set_location_from_fields.spawn",
                        "does not fit an i16 spawn number",
                    )
                })?;
                self.snapshot.environment.execution_context = ExecutionContext::World;
                self.snapshot.environment.location = SceneLocation {
                    stage,
                    room,
                    layer: *layer,
                    spawn,
                };
            }
            StateOperation::SetPendingWorldLoadFromFields {
                component_id,
                stage_field,
                room_field,
                spawn_field,
                layer,
            } => {
                let component = self
                    .snapshot
                    .environment
                    .components
                    .iter()
                    .find(|component| component.id == *component_id)
                    .ok_or_else(|| {
                        PlannerContractError::new(
                            "operation.set_pending_world_load_from_fields",
                            "references an absent source component",
                        )
                    })?;
                let ComponentPayload::Structured { fields } = &component.payload else {
                    return Err(PlannerContractError::new(
                        "operation.set_pending_world_load_from_fields",
                        "requires a structured source component",
                    ));
                };
                let stage = match fields.get(stage_field) {
                    Some(StateValue::Text(stage)) => stage.clone(),
                    _ => {
                        return Err(PlannerContractError::new(
                            "operation.set_pending_world_load_from_fields.stage",
                            "requires a known text field",
                        ));
                    }
                };
                let room = match fields.get(room_field) {
                    Some(StateValue::Signed(room)) => i8::try_from(*room),
                    Some(StateValue::Unsigned(room)) => i8::try_from(*room),
                    _ => {
                        return Err(PlannerContractError::new(
                            "operation.set_pending_world_load_from_fields.room",
                            "requires a known integer field",
                        ));
                    }
                }
                .map_err(|_| {
                    PlannerContractError::new(
                        "operation.set_pending_world_load_from_fields.room",
                        "does not fit an i8 room number",
                    )
                })?;
                let spawn = match fields.get(spawn_field) {
                    Some(StateValue::Signed(spawn)) => i16::try_from(*spawn),
                    Some(StateValue::Unsigned(spawn)) => i16::try_from(*spawn),
                    _ => {
                        return Err(PlannerContractError::new(
                            "operation.set_pending_world_load_from_fields.spawn",
                            "requires a known integer field",
                        ));
                    }
                }
                .map_err(|_| {
                    PlannerContractError::new(
                        "operation.set_pending_world_load_from_fields.spawn",
                        "does not fit an i16 spawn number",
                    )
                })?;
                let ExecutionContext::Process { process_name, .. } =
                    &self.snapshot.environment.execution_context
                else {
                    return Err(PlannerContractError::new(
                        "operation.set_pending_world_load_from_fields",
                        "requires an active non-world process",
                    ));
                };
                self.snapshot.environment.execution_context = ExecutionContext::Process {
                    process_name: process_name.clone(),
                    pending_world_load: Some(SceneLocation {
                        stage,
                        room,
                        layer: *layer,
                        spawn,
                    }),
                };
            }
            StateOperation::SetPlayerForm { form } => {
                self.snapshot.environment.player.form = form.clone();
            }
            StateOperation::SetPlayerMount { mount } => {
                self.snapshot.environment.player.mount = mount.clone();
            }
            StateOperation::SetPlayerControl { has_control } => {
                self.snapshot.environment.player.has_control = *has_control;
            }
            StateOperation::SetPlayerAction { action } => {
                self.snapshot.environment.player.action = action.clone();
            }
            StateOperation::Project {
                source_runtime_file_id,
                destination_runtime_file_id,
                component_ids,
            } => {
                for id in component_ids {
                    let component = self.component_mut(id)?;
                    if component.binding
                        != (ComponentBinding::RuntimeFile {
                            runtime_file_id: source_runtime_file_id.clone(),
                        })
                    {
                        return Err(PlannerContractError::new(
                            "operation.project",
                            "selected component is not bound to the declared source runtime file",
                        ));
                    }
                    component.binding = ComponentBinding::RuntimeFile {
                        runtime_file_id: destination_runtime_file_id.clone(),
                    };
                    if component.serialization_owner
                        == (SerializationOwner::RuntimeFile {
                            runtime_file_id: source_runtime_file_id.clone(),
                        })
                    {
                        component.serialization_owner = SerializationOwner::RuntimeFile {
                            runtime_file_id: destination_runtime_file_id.clone(),
                        };
                    }
                    mark_transition(component, application_id);
                }
            }
            StateOperation::Consume {
                pending_operation_id,
            } => {
                let index = self.component_index(pending_operation_id)?;
                if self.snapshot.environment.components[index].component_kind
                    != ComponentKind::PendingOperation
                {
                    return Err(PlannerContractError::new(
                        "operation.consume",
                        "target is not a pending-operation component",
                    ));
                }
                self.snapshot.environment.components.remove(index);
                self.preserved_component_ids.remove(pending_operation_id);
            }
            StateOperation::SetGate { gate_id } => {
                self.gate_states.insert(gate_id.clone(), true);
            }
            StateOperation::ClearGate { gate_id } => {
                self.gate_states.insert(gate_id.clone(), false);
            }
            StateOperation::AdvanceFlow {
                flow_component_id,
                node_id,
            } => self.write_flow(flow_component_id, node_id, None, application_id)?,
            StateOperation::BranchFlow {
                flow_component_id,
                edge_id,
                destination_node_id,
            } => self.write_flow(
                flow_component_id,
                destination_node_id,
                Some(edge_id),
                application_id,
            )?,
            StateOperation::ScheduleCleanup { cleanup_id } => {
                self.scheduled_cleanup_ids.insert(cleanup_id.clone());
            }
            StateOperation::CancelCleanup { cleanup_id } => {
                if !self.scheduled_cleanup_ids.remove(cleanup_id) {
                    return Err(PlannerContractError::new(
                        "operation.cancel_cleanup",
                        "references a cleanup that is not scheduled",
                    ));
                }
            }
            StateOperation::Interrupt { action_id, window } => {
                self.interruption_log.push(InterruptionRecord {
                    action_id: action_id.clone(),
                    window: window.clone(),
                });
            }
        }
        Ok(())
    }

    fn component_index(&self, id: &str) -> Result<usize, PlannerContractError> {
        self.snapshot
            .environment
            .components
            .iter()
            .position(|component| component.id == id)
            .ok_or_else(|| {
                PlannerContractError::new(
                    "operation.component_id",
                    "references an absent component",
                )
            })
    }

    fn commit_load_stage_bank(
        &mut self,
        application_id: &str,
        operation: &StateOperation,
    ) -> Result<(), PlannerContractError> {
        let StateOperation::CommitLoadStageBank {
            component_id,
            runtime_file_id,
            source_stage,
            destination_stage,
            source_binding,
            destination_binding,
        } = operation
        else {
            unreachable!("commit/load helper is called only for its operation variant")
        };
        if self.snapshot.environment.active_runtime_file.id != runtime_file_id.as_str() {
            return Err(PlannerContractError::new(
                "operation.commit_load_stage_bank.runtime_file_id",
                "does not name the active runtime file",
            ));
        }
        if self.snapshot.environment.location.stage != source_stage.as_str() {
            return Err(PlannerContractError::new(
                "operation.commit_load_stage_bank.source_stage",
                "does not match the current scene stage",
            ));
        }
        let source_owner = SerializationOwner::StageBank {
            runtime_file_id: runtime_file_id.into(),
            stage: source_stage.into(),
        };
        let destination_owner = SerializationOwner::StageBank {
            runtime_file_id: runtime_file_id.into(),
            stage: destination_stage.into(),
        };
        let component_index = self.component_index(component_id)?;
        let current = self.snapshot.environment.components[component_index].clone();
        if current.binding != *source_binding
            || current.serialization_owner != source_owner
            || current.lifetime != crate::state::SemanticLifetime::StageLoad
        {
            return Err(PlannerContractError::new(
                "operation.commit_load_stage_bank.source",
                "live component must be stage-load state bound to the exact source backing",
            ));
        }

        let mut committed = current.clone();
        mark_transition(&mut committed, application_id);
        insert_serialized(&mut self.serialized_components, &source_owner, committed);

        let mut restored = select_serialized(
            &self.serialized_components,
            &destination_owner,
            component_id,
        )
        .map_err(|error| {
            PlannerContractError::new(
                "operation.commit_load_stage_bank.destination",
                error.detail(),
            )
        })?
        .clone();
        if restored.id != component_id.as_str()
            || restored.component_kind != current.component_kind
            || restored.binding != *destination_binding
            || restored.serialization_owner != destination_owner
            || restored.lifetime != crate::state::SemanticLifetime::StageLoad
        {
            return Err(PlannerContractError::new(
                "operation.commit_load_stage_bank.destination",
                "stored component does not match the exact destination backing contract",
            ));
        }
        mark_transition(&mut restored, application_id);
        self.snapshot.environment.components[component_index] = restored;
        Ok(())
    }

    fn activate_stage_bank(
        &mut self,
        application_id: &str,
        operation: &StateOperation,
    ) -> Result<(), PlannerContractError> {
        let StateOperation::ActivateStageBank {
            component_id,
            runtime_file_id,
            stage,
            binding,
        } = operation
        else {
            unreachable!("stage activation helper is called only for its operation variant")
        };
        if self.snapshot.environment.active_runtime_file.id != runtime_file_id.as_str() {
            return Err(PlannerContractError::new(
                "operation.activate_stage_bank.runtime_file_id",
                "does not name the active runtime file",
            ));
        }
        self.require_absent_component(component_id)?;
        let owner = SerializationOwner::StageBank {
            runtime_file_id: runtime_file_id.clone(),
            stage: stage.clone(),
        };
        let mut restored = select_serialized(&self.serialized_components, &owner, component_id)
            .map_err(|error| {
                PlannerContractError::new("operation.activate_stage_bank.source", error.detail())
            })?
            .clone();
        if restored.id != component_id.as_str()
            || restored.binding != *binding
            || restored.serialization_owner != owner
            || restored.lifetime != crate::state::SemanticLifetime::StageLoad
        {
            return Err(PlannerContractError::new(
                "operation.activate_stage_bank.source",
                "stored component does not match the exact activation contract",
            ));
        }
        mark_save_restore(&mut restored, application_id);
        self.snapshot.environment.components.push(restored);
        Ok(())
    }

    fn save_runtime_to_slot(
        &mut self,
        application_id: &str,
        operation: &StateOperation,
    ) -> Result<(), PlannerContractError> {
        let StateOperation::SaveRuntimeToSlot {
            source_runtime_file_id,
            destination_slot,
            destination_persistent_file_id,
            runtime_component_ids,
            stage_bank_stages,
        } = operation
        else {
            unreachable!("save helper is called only for its operation variant")
        };
        let active = &self.snapshot.environment.active_runtime_file;
        if active.id != source_runtime_file_id.as_str() {
            return Err(PlannerContractError::new(
                "operation.save_runtime_to_slot.source_runtime_file_id",
                "does not name the active runtime file",
            ));
        }
        if active
            .allowed_serialization_targets
            .binary_search(destination_slot)
            .is_err()
        {
            return Err(PlannerContractError::new(
                "operation.save_runtime_to_slot.destination_slot",
                "is not an allowed serialization target for the active runtime",
            ));
        }

        let mut runtime_components = Vec::with_capacity(runtime_component_ids.len());
        for component_id in runtime_component_ids {
            let component = self
                .snapshot
                .environment
                .components
                .iter()
                .find(|component| &component.id == component_id)
                .ok_or_else(|| {
                    PlannerContractError::new(
                        "operation.save_runtime_to_slot.runtime_component_ids",
                        "references an absent live component",
                    )
                })?;
            if component.serialization_owner
                != (SerializationOwner::RuntimeFile {
                    runtime_file_id: source_runtime_file_id.clone(),
                })
            {
                return Err(PlannerContractError::new(
                    "operation.save_runtime_to_slot.runtime_component_ids",
                    "component is not owned by the active runtime file",
                ));
            }
            let mut serialized = component.clone();
            rekey_component_runtime(
                &mut serialized,
                source_runtime_file_id,
                destination_persistent_file_id,
            );
            serialized.serialization_owner = SerializationOwner::RuntimeFile {
                runtime_file_id: destination_persistent_file_id.clone(),
            };
            mark_save_restore(&mut serialized, application_id);
            runtime_components.push(serialized);
        }

        // Saving performs the normal putSave(current stage) commit for every
        // selected stage-bank component that is presently live.
        let selected_stages = stage_bank_stages
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let live_stage_components = self
            .snapshot
            .environment
            .components
            .iter()
            .filter_map(|component| match &component.serialization_owner {
                SerializationOwner::StageBank {
                    runtime_file_id,
                    stage,
                } if runtime_file_id == source_runtime_file_id
                    && selected_stages.contains(stage.as_str()) =>
                {
                    Some(component.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        for mut component in live_stage_components {
            let owner = component.serialization_owner.clone();
            mark_save_restore(&mut component, application_id);
            insert_serialized(&mut self.serialized_components, &owner, component);
        }

        let mut stage_banks = Vec::with_capacity(stage_bank_stages.len());
        for stage in stage_bank_stages {
            let source_owner = SerializationOwner::StageBank {
                runtime_file_id: source_runtime_file_id.clone(),
                stage: stage.clone(),
            };
            let source = self
                .serialized_components
                .get(&source_owner)
                .ok_or_else(|| {
                    PlannerContractError::new(
                        "operation.save_runtime_to_slot.stage_bank_stages",
                        "references an unavailable stage-bank store",
                    )
                })?;
            let destination_owner = SerializationOwner::StageBank {
                runtime_file_id: destination_persistent_file_id.clone(),
                stage: stage.clone(),
            };
            let mut components = source.clone();
            for component in &mut components {
                rekey_component_runtime(
                    component,
                    source_runtime_file_id,
                    destination_persistent_file_id,
                );
                component.serialization_owner = destination_owner.clone();
                mark_save_restore(component, application_id);
            }
            stage_banks.push(SerializedComponentStore {
                owner: destination_owner,
                components,
            });
        }
        let image = PersistentFileImage {
            schema: PERSISTENT_FILE_IMAGE_SCHEMA.into(),
            id: destination_persistent_file_id.clone(),
            source_runtime_file_id: source_runtime_file_id.clone(),
            runtime_components,
            stage_banks,
        };
        image.validate()?;
        let image_sha256 = image.digest()?;

        if self.snapshot.environment.physical_slots.iter().any(|slot| {
            slot.slot != *destination_slot
                && slot.persistent_file_id == *destination_persistent_file_id
        }) {
            return Err(PlannerContractError::new(
                "operation.save_runtime_to_slot.destination_persistent_file_id",
                "is already attached to a different physical slot",
            ));
        }
        let old_image_id = match self
            .snapshot
            .environment
            .physical_slots
            .binary_search_by_key(&destination_slot.0, |slot| slot.slot.0)
        {
            Ok(index) => {
                let old = self.snapshot.environment.physical_slots[index]
                    .persistent_file_id
                    .clone();
                self.snapshot.environment.physical_slots[index] = PhysicalSlot {
                    slot: *destination_slot,
                    persistent_file_id: destination_persistent_file_id.clone(),
                    serialized_state_sha256: image_sha256,
                };
                Some(old)
            }
            Err(index) => {
                self.snapshot.environment.physical_slots.insert(
                    index,
                    PhysicalSlot {
                        slot: *destination_slot,
                        persistent_file_id: destination_persistent_file_id.clone(),
                        serialized_state_sha256: image_sha256,
                    },
                );
                None
            }
        };
        if let Some(old_image_id) = old_image_id
            && old_image_id != *destination_persistent_file_id
        {
            self.persistent_file_images.remove(&old_image_id);
        }
        self.persistent_file_images
            .insert(destination_persistent_file_id.clone(), image);
        Ok(())
    }

    fn load_runtime_from_slot(
        &mut self,
        application_id: &str,
        operation: &StateOperation,
    ) -> Result<(), PlannerContractError> {
        let StateOperation::LoadRuntimeFromSlot {
            source_runtime_file_id,
            source_slot,
            source_persistent_file_id,
            destination_runtime_file_id,
            destination_allowed_serialization_targets,
            runtime_component_ids,
            stage_bank_stages,
            carried_runtime_component_ids,
        } = operation
        else {
            unreachable!("load helper is called only for its operation variant")
        };
        if self.snapshot.environment.active_runtime_file.id != source_runtime_file_id.as_str() {
            return Err(PlannerContractError::new(
                "operation.load_runtime_from_slot.source_runtime_file_id",
                "does not name the active runtime file",
            ));
        }
        if source_runtime_file_id == destination_runtime_file_id
            || self
                .snapshot
                .environment
                .inactive_runtime_files
                .iter()
                .any(|runtime| runtime.id == *destination_runtime_file_id)
        {
            return Err(PlannerContractError::new(
                "operation.load_runtime_from_slot.destination_runtime_file_id",
                "must name a fresh runtime-file lifetime",
            ));
        }
        let slot = self
            .snapshot
            .environment
            .physical_slots
            .iter()
            .find(|slot| slot.slot == *source_slot)
            .ok_or_else(|| {
                PlannerContractError::new(
                    "operation.load_runtime_from_slot.source_slot",
                    "is not populated",
                )
            })?;
        if slot.persistent_file_id != *source_persistent_file_id {
            return Err(PlannerContractError::new(
                "operation.load_runtime_from_slot.source_persistent_file_id",
                "does not match the selected slot",
            ));
        }
        let image = self
            .persistent_file_images
            .get(source_persistent_file_id)
            .ok_or_else(|| {
                PlannerContractError::new(
                    "operation.load_runtime_from_slot.source_persistent_file_id",
                    "references an unavailable persistent file image",
                )
            })?
            .clone();
        if image.digest()? != slot.serialized_state_sha256 {
            return Err(PlannerContractError::new(
                "operation.load_runtime_from_slot.source_slot",
                "persistent file image fails its slot digest",
            ));
        }
        let image_component_ids = image
            .runtime_components
            .iter()
            .map(|component| component.id.as_str())
            .collect::<Vec<_>>();
        if image_component_ids
            != runtime_component_ids
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        {
            return Err(PlannerContractError::new(
                "operation.load_runtime_from_slot.runtime_component_ids",
                "must exactly match the persistent image manifest",
            ));
        }
        let image_stages = image
            .stage_banks
            .iter()
            .map(|store| match &store.owner {
                SerializationOwner::StageBank { stage, .. } => stage.as_str(),
                _ => unreachable!("validated image contains only stage banks"),
            })
            .collect::<Vec<_>>();
        if image_stages
            != stage_bank_stages
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        {
            return Err(PlannerContractError::new(
                "operation.load_runtime_from_slot.stage_bank_stages",
                "must exactly match the persistent image manifest",
            ));
        }

        let carried_component_ids = carried_runtime_component_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        for component_id in carried_runtime_component_ids {
            let component = self
                .snapshot
                .environment
                .components
                .iter()
                .find(|component| component.id == *component_id)
                .ok_or_else(|| {
                    PlannerContractError::new(
                        "operation.load_runtime_from_slot.carried_runtime_component_ids",
                        "references an absent live component",
                    )
                })?;
            if !component_belongs_to_runtime(component, source_runtime_file_id)
                || component.lifetime != crate::state::SemanticLifetime::RuntimeFile
            {
                return Err(PlannerContractError::new(
                    "operation.load_runtime_from_slot.carried_runtime_component_ids",
                    "must name runtime-lifetime state owned by the active runtime",
                ));
            }
            if matches!(
                component.serialization_owner,
                SerializationOwner::StageBank { .. } | SerializationOwner::PhysicalSlot { .. }
            ) {
                return Err(PlannerContractError::new(
                    "operation.load_runtime_from_slot.carried_runtime_component_ids",
                    "cannot carry stage-bank or physical-slot state as runtime metadata",
                ));
            }
        }

        for component in &mut self.snapshot.environment.components {
            if carried_component_ids.contains(component.id.as_str()) {
                rekey_component_runtime(
                    component,
                    source_runtime_file_id,
                    destination_runtime_file_id,
                );
                rekey_serialization_owner_runtime(
                    &mut component.serialization_owner,
                    source_runtime_file_id,
                    destination_runtime_file_id,
                );
                mark_transition(component, application_id);
            }
        }

        self.snapshot.environment.components.retain(|component| {
            carried_component_ids.contains(component.id.as_str())
                || !component_belongs_to_runtime(component, source_runtime_file_id)
        });
        self.preserved_component_ids.retain(|component_id| {
            self.snapshot
                .environment
                .components
                .iter()
                .any(|component| component.id == *component_id)
        });
        self.serialized_components
            .retain(|owner, _| !owner_belongs_to_runtime(owner, source_runtime_file_id));

        for mut component in image.runtime_components {
            if self
                .snapshot
                .environment
                .components
                .iter()
                .any(|existing| existing.id == component.id)
            {
                return Err(PlannerContractError::new(
                    "operation.load_runtime_from_slot.runtime_component_ids",
                    "collides with a preserved non-file component",
                ));
            }
            rekey_component_runtime(
                &mut component,
                source_persistent_file_id,
                destination_runtime_file_id,
            );
            component.serialization_owner = SerializationOwner::RuntimeFile {
                runtime_file_id: destination_runtime_file_id.clone(),
            };
            mark_save_restore(&mut component, application_id);
            self.snapshot.environment.components.push(component);
        }
        for store in image.stage_banks {
            let SerializationOwner::StageBank { stage, .. } = &store.owner else {
                unreachable!("validated image contains only stage banks")
            };
            let destination_owner = SerializationOwner::StageBank {
                runtime_file_id: destination_runtime_file_id.clone(),
                stage: stage.clone(),
            };
            let mut components = store.components;
            for component in &mut components {
                rekey_component_runtime(
                    component,
                    source_persistent_file_id,
                    destination_runtime_file_id,
                );
                component.serialization_owner = destination_owner.clone();
                mark_save_restore(component, application_id);
            }
            if self
                .serialized_components
                .insert(destination_owner, components)
                .is_some()
            {
                return Err(PlannerContractError::new(
                    "operation.load_runtime_from_slot.stage_bank_stages",
                    "destination runtime already owns a selected stage bank",
                ));
            }
        }

        let mut ended = self.snapshot.environment.active_runtime_file.clone();
        ended.lifecycle = RuntimeFileLifecycle::Ended;
        let insert_at = self
            .snapshot
            .environment
            .inactive_runtime_files
            .binary_search_by(|runtime| runtime.id.cmp(&ended.id))
            .unwrap_err();
        self.snapshot
            .environment
            .inactive_runtime_files
            .insert(insert_at, ended);
        self.snapshot.environment.active_runtime_file = RuntimeFile {
            id: destination_runtime_file_id.clone(),
            origin: RuntimeFileOrigin::LoadedSlot { slot: *source_slot },
            backing: BackingAttachment::CardBacked { slot: *source_slot },
            allowed_serialization_targets: destination_allowed_serialization_targets.clone(),
            lifecycle: RuntimeFileLifecycle::Active,
        };
        Ok(())
    }

    fn load_active_runtime_from_slot(
        &mut self,
        application_id: &str,
        operation: &StateOperation,
    ) -> Result<(), PlannerContractError> {
        let StateOperation::LoadActiveRuntimeFromSlot {
            source_slot,
            destination_id_suffix,
            destination_allowed_serialization_targets,
            carried_runtime_component_ids,
        } = operation
        else {
            unreachable!("active-runtime load helper is called only for its operation variant")
        };
        let source_runtime_file_id = self.snapshot.environment.active_runtime_file.id.clone();
        let destination_runtime_file_id =
            format!("{source_runtime_file_id}.{destination_id_suffix}");
        crate::validate_stable_id(
            "operation.load_active_runtime_from_slot.destination_runtime_file_id",
            &destination_runtime_file_id,
        )?;
        let source_persistent_file_id = self
            .snapshot
            .environment
            .physical_slots
            .iter()
            .find(|slot| slot.slot == *source_slot)
            .ok_or_else(|| {
                PlannerContractError::new(
                    "operation.load_active_runtime_from_slot.source_slot",
                    "is not populated",
                )
            })?
            .persistent_file_id
            .clone();
        let image = self
            .persistent_file_images
            .get(&source_persistent_file_id)
            .ok_or_else(|| {
                PlannerContractError::new(
                    "operation.load_active_runtime_from_slot.source_persistent_file_id",
                    "references an unavailable persistent file image",
                )
            })?;
        let runtime_component_ids: Vec<String> = image
            .runtime_components
            .iter()
            .map(|component| component.id.clone())
            .collect();
        let stage_bank_stages: Vec<String> = image
            .stage_banks
            .iter()
            .map(|store| match &store.owner {
                SerializationOwner::StageBank { stage, .. } => stage.clone(),
                _ => unreachable!("validated persistent image contains only stage banks"),
            })
            .collect();
        self.load_runtime_from_slot(
            application_id,
            &StateOperation::LoadRuntimeFromSlot {
                source_runtime_file_id,
                source_slot: *source_slot,
                source_persistent_file_id,
                destination_runtime_file_id,
                destination_allowed_serialization_targets:
                    destination_allowed_serialization_targets.clone(),
                runtime_component_ids,
                stage_bank_stages,
                carried_runtime_component_ids: carried_runtime_component_ids.clone(),
            },
        )
    }

    fn begin_runtime_file_lifetime(
        &mut self,
        application_id: &str,
        operation: &StateOperation,
    ) -> Result<(), PlannerContractError> {
        let StateOperation::BeginRuntimeFileLifetime {
            destination_id_suffix,
            origin,
            backing,
            allowed_serialization_targets,
        } = operation
        else {
            unreachable!("runtime-lifetime helper is called only for its operation variant")
        };
        let source_runtime = self.snapshot.environment.active_runtime_file.clone();
        let destination_runtime_file_id =
            format!("{}.{}", source_runtime.id, destination_id_suffix);
        crate::validate_stable_id(
            "operation.begin_runtime_file_lifetime.destination_runtime_file_id",
            &destination_runtime_file_id,
        )?;
        if self
            .snapshot
            .environment
            .inactive_runtime_files
            .iter()
            .any(|runtime| runtime.id == destination_runtime_file_id)
        {
            return Err(PlannerContractError::new(
                "operation.begin_runtime_file_lifetime.destination_runtime_file_id",
                "must derive a fresh runtime-file lifetime",
            ));
        }

        for component in &mut self.snapshot.environment.components {
            if !component_belongs_to_runtime(component, &source_runtime.id) {
                continue;
            }
            rekey_component_runtime(component, &source_runtime.id, &destination_runtime_file_id);
            rekey_serialization_owner_runtime(
                &mut component.serialization_owner,
                &source_runtime.id,
                &destination_runtime_file_id,
            );
            mark_transition(component, application_id);
        }

        let source_stores = std::mem::take(&mut self.serialized_components);
        for (mut owner, mut components) in source_stores {
            let owned_by_source = owner_belongs_to_runtime(&owner, &source_runtime.id);
            if owned_by_source {
                rekey_serialization_owner_runtime(
                    &mut owner,
                    &source_runtime.id,
                    &destination_runtime_file_id,
                );
                for component in &mut components {
                    rekey_component_runtime(
                        component,
                        &source_runtime.id,
                        &destination_runtime_file_id,
                    );
                    rekey_serialization_owner_runtime(
                        &mut component.serialization_owner,
                        &source_runtime.id,
                        &destination_runtime_file_id,
                    );
                    mark_transition(component, application_id);
                }
            }
            if self
                .serialized_components
                .insert(owner, components)
                .is_some()
            {
                return Err(PlannerContractError::new(
                    "operation.begin_runtime_file_lifetime.serialized_components",
                    "rekeyed owner collides with an existing store",
                ));
            }
        }

        let mut ended = source_runtime;
        ended.lifecycle = RuntimeFileLifecycle::Ended;
        let insert_at = self
            .snapshot
            .environment
            .inactive_runtime_files
            .binary_search_by(|runtime| runtime.id.cmp(&ended.id))
            .expect_err("validated active runtime is absent from inactive lifetimes");
        self.snapshot
            .environment
            .inactive_runtime_files
            .insert(insert_at, ended);
        self.snapshot.environment.active_runtime_file = RuntimeFile {
            id: destination_runtime_file_id,
            origin: origin.clone(),
            backing: backing.clone(),
            allowed_serialization_targets: allowed_serialization_targets.clone(),
            lifecycle: RuntimeFileLifecycle::Active,
        };
        Ok(())
    }

    fn structured_value(
        &self,
        target: &crate::transition::ComponentFieldTarget,
        field: &str,
    ) -> Result<StateValue, PlannerContractError> {
        let component = self
            .snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == target.component_id)
            .ok_or_else(|| {
                PlannerContractError::new(field, "references an absent source component")
            })?;
        let ComponentPayload::Structured { fields } = &component.payload else {
            return Err(PlannerContractError::new(
                field,
                "requires a structured source component",
            ));
        };
        fields
            .get(&target.field)
            .cloned()
            .ok_or_else(|| PlannerContractError::new(field, "references an absent source field"))
    }

    fn component_mut(&mut self, id: &str) -> Result<&mut StateComponent, PlannerContractError> {
        let index = self.component_index(id)?;
        Ok(&mut self.snapshot.environment.components[index])
    }

    fn unique_bound_raw_component_id(
        &self,
        component_kind: &ComponentKind,
        binding: &ComponentBindingReference,
        field: &str,
    ) -> Result<String, PlannerContractError> {
        let resolved_binding = binding.resolve(&self.snapshot.environment);
        let matches = self
            .snapshot
            .environment
            .components
            .iter()
            .filter(|component| {
                component.component_kind == *component_kind
                    && resolved_binding
                        .as_ref()
                        .is_some_and(|binding| component.binding == *binding)
                    && matches!(component.payload, ComponentPayload::Raw { .. })
            })
            .map(|component| component.id.clone())
            .collect::<Vec<_>>();
        let [component_id] = matches.as_slice() else {
            return Err(PlannerContractError::new(
                field,
                "requires exactly one raw component with the selected kind and binding",
            ));
        };
        Ok(component_id.clone())
    }

    fn require_absent_component(&self, id: &str) -> Result<(), PlannerContractError> {
        if self
            .snapshot
            .environment
            .components
            .iter()
            .any(|component| component.id == id)
        {
            Err(PlannerContractError::new(
                "operation.destination_component_id",
                "already exists",
            ))
        } else {
            Ok(())
        }
    }

    fn matching_ids(&self, selector: &ComponentSelector) -> BTreeSet<String> {
        self.matching_components(selector)
            .into_iter()
            .map(|component| component.id.clone())
            .collect()
    }

    fn matching_ids_including_serialized(
        &self,
        selector: &ComponentSelector,
        include_active_runtime_serialized_stores: bool,
    ) -> BTreeSet<String> {
        let mut ids = self.matching_ids(selector);
        if include_active_runtime_serialized_stores {
            let active_runtime_file_id = &self.snapshot.environment.active_runtime_file.id;
            ids.extend(
                self.serialized_components
                    .iter()
                    .filter(|(owner, _)| owner_belongs_to_runtime(owner, active_runtime_file_id))
                    .flat_map(|(_, components)| components)
                    .filter(|component| selector_matches(selector, component))
                    .map(|component| component.id.clone()),
            );
        }
        ids
    }

    fn matching_components(&self, selector: &ComponentSelector) -> Vec<&StateComponent> {
        self.snapshot
            .environment
            .components
            .iter()
            .filter(|component| selector_matches(selector, component))
            .collect()
    }

    fn single_component(
        &self,
        selector: &ComponentSelector,
        field: &str,
    ) -> Result<&StateComponent, PlannerContractError> {
        let matches = self.matching_components(selector);
        let [component] = matches.as_slice() else {
            return Err(PlannerContractError::new(
                field,
                "source selector must match exactly one component",
            ));
        };
        Ok(component)
    }

    fn write_flow(
        &mut self,
        component_id: &str,
        node_id: &str,
        edge_id: Option<&str>,
        application_id: &str,
    ) -> Result<(), PlannerContractError> {
        let component = self.component_mut(component_id)?;
        if component.component_kind != ComponentKind::MessageFlow {
            return Err(PlannerContractError::new(
                "operation.flow_component_id",
                "target is not a message-flow component",
            ));
        }
        let ComponentPayload::Structured { fields } = &mut component.payload else {
            return Err(PlannerContractError::new(
                "operation.flow_component_id",
                "message-flow target is not structured",
            ));
        };
        fields.insert("node_id".into(), StateValue::Text(node_id.into()));
        if let Some(edge_id) = edge_id {
            fields.insert("last_edge_id".into(), StateValue::Text(edge_id.into()));
        }
        mark_transition(component, application_id);
        Ok(())
    }

    fn sort_components(&mut self) {
        self.snapshot
            .environment
            .components
            .sort_by(|left, right| left.id.cmp(&right.id));
    }

    fn boundary_disposition(
        &self,
        policy: &BoundaryPolicy,
        component: &StateComponent,
    ) -> Result<BoundaryDisposition, PlannerContractError> {
        if self.preserved_component_ids.contains(&component.id) {
            return Ok(BoundaryDisposition::Preserve);
        }
        let matching = policy
            .component_rules
            .iter()
            .filter(|rule| selector_matches(&rule.selector, component))
            .collect::<Vec<_>>();
        match matching.as_slice() {
            [] => Ok(policy.default_disposition.clone()),
            [rule] => Ok(rule.disposition.clone()),
            _ => Err(PlannerContractError::new(
                "boundary.component_rules",
                format!(
                    "multiple rules match component {}; refine the selectors",
                    component.id
                ),
            )),
        }
    }
}

impl PlannerExecutionStateDocument {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != PLANNER_EXECUTION_STATE_SCHEMA {
            return Err(PlannerContractError::new("schema", "is unsupported"));
        }
        self.clone().into_state().map(|_| ())
    }

    pub fn into_state(self) -> Result<PlannerExecutionState, PlannerContractError> {
        if self.schema != PLANNER_EXECUTION_STATE_SCHEMA {
            return Err(PlannerContractError::new("schema", "is unsupported"));
        }
        let mut stores = BTreeMap::new();
        let mut previous = None;
        for store in self.serialized_component_stores {
            if previous
                .as_ref()
                .is_some_and(|owner: &SerializationOwner| owner >= &store.owner)
            {
                return Err(PlannerContractError::new(
                    "serialized_component_stores",
                    "must be unique and sorted by owner",
                ));
            }
            previous = Some(store.owner.clone());
            stores.insert(store.owner, store.components);
        }
        let mut images = BTreeMap::new();
        let mut previous_image_id = None;
        for image in self.persistent_file_images {
            if previous_image_id
                .as_deref()
                .is_some_and(|id: &str| id >= image.id.as_str())
            {
                return Err(PlannerContractError::new(
                    "persistent_file_images",
                    "must be unique and sorted by ID",
                ));
            }
            previous_image_id = Some(image.id.clone());
            images.insert(image.id.clone(), image);
        }
        let state = PlannerExecutionState {
            snapshot: self.snapshot,
            gate_states: self.gate_states,
            serialized_components: stores,
            persistent_file_images: images,
            preserved_component_ids: self.preserved_component_ids,
            scheduled_cleanup_ids: self.scheduled_cleanup_ids,
            interruption_log: self.interruption_log,
            execution_history: self.execution_history,
        };
        state.validate()?;
        Ok(state)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let document: Self = serde_json::from_slice(bytes)?;
        document.validate()?;
        if document.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "planner_execution_state",
                "is not canonical JSON",
            ));
        }
        Ok(document)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

fn selector_matches(selector: &ComponentSelector, component: &StateComponent) -> bool {
    match selector {
        ComponentSelector::Id { component_id } => component.id == *component_id,
        ComponentSelector::Kind { component_kind } => component.component_kind == *component_kind,
        ComponentSelector::Binding { binding } => component.binding == *binding,
    }
}

fn component_belongs_to_runtime(component: &StateComponent, runtime_file_id: &str) -> bool {
    component.binding
        == (ComponentBinding::RuntimeFile {
            runtime_file_id: runtime_file_id.into(),
        })
        || owner_belongs_to_runtime(&component.serialization_owner, runtime_file_id)
}

fn owner_belongs_to_runtime(owner: &SerializationOwner, runtime_file_id: &str) -> bool {
    matches!(
        owner,
        SerializationOwner::RuntimeFile {
            runtime_file_id: owner_runtime
        } | SerializationOwner::StageBank {
            runtime_file_id: owner_runtime,
            ..
        } if owner_runtime == runtime_file_id
    )
}

fn history_event_writes_field(
    event: &ExecutionHistoryEvent,
    component_id: &str,
    field: &str,
) -> bool {
    match &event.event {
        ExecutionHistoryKind::BoundaryComponent {
            component_id: changed,
            disposition,
            ..
        } => {
            changed == component_id
                && !matches!(
                    disposition,
                    BoundaryDisposition::Preserve | BoundaryDisposition::Unknown
                )
        }
        ExecutionHistoryKind::Operation {
            operation,
            affected_component_ids,
        } => match operation {
            StateOperation::Write { target, .. }
            | StateOperation::CopyValue { target, .. }
            | StateOperation::SetBitFromValue { target, .. }
            | StateOperation::Adjust { target, .. }
            | StateOperation::ClearField { target }
            | StateOperation::InvalidateField { target } => {
                target.component_id == component_id && target.field == field
            }
            StateOperation::WriteFields {
                component_id: changed,
                fields,
            } => changed == component_id && fields.contains_key(field),
            StateOperation::ReplacePayload {
                component_id: changed,
                ..
            } => changed == component_id,
            StateOperation::AdvanceFlow {
                flow_component_id, ..
            } => flow_component_id == component_id && field == "node_id",
            StateOperation::BranchFlow {
                flow_component_id, ..
            } => flow_component_id == component_id && matches!(field, "node_id" | "last_edge_id"),
            StateOperation::Initialize { component } => component.id == component_id,
            StateOperation::CommitLoadStageBank {
                component_id: changed,
                ..
            }
            | StateOperation::ActivateStageBank {
                component_id: changed,
                ..
            } => changed == component_id,
            StateOperation::Copy {
                destination_component_id,
                ..
            }
            | StateOperation::Move {
                destination_component_id,
                ..
            }
            | StateOperation::Restore {
                destination_component_id,
                ..
            } => destination_component_id == component_id,
            StateOperation::ClearComponent { .. }
            | StateOperation::InvalidatePayloads { .. }
            | StateOperation::InvalidateActiveRuntimeSerializedPayloads { .. } => {
                affected_component_ids
                    .binary_search_by(|id| id.as_str().cmp(component_id))
                    .is_ok()
            }
            StateOperation::LoadRuntimeFromSlot { .. }
            | StateOperation::LoadActiveRuntimeFromSlot { .. } => affected_component_ids
                .binary_search_by(|id| id.as_str().cmp(component_id))
                .is_ok(),
            StateOperation::RestorePayloadsFromCustomStore { .. } => affected_component_ids
                .binary_search_by(|id| id.as_str().cmp(component_id))
                .is_ok(),
            StateOperation::WriteRaw { .. }
            | StateOperation::WriteBoundRaw { .. }
            | StateOperation::InvalidateRaw { .. }
            | StateOperation::InvalidateBoundRaw { .. }
            | StateOperation::AdjustBoundRawUnsigned { .. }
            | StateOperation::Preserve { .. }
            | StateOperation::Serialize { .. }
            | StateOperation::ReplaceCustomStore { .. }
            | StateOperation::SaveRuntimeToSlot { .. }
            | StateOperation::BeginRuntimeFileLifetime { .. }
            | StateOperation::Bind { .. }
            | StateOperation::Rebind { .. }
            | StateOperation::SetActiveRuntimeFile { .. }
            | StateOperation::SetExecutionContext { .. }
            | StateOperation::CompletePendingWorldLoad
            | StateOperation::SetLocation { .. }
            | StateOperation::SetLocationFromFields { .. }
            | StateOperation::SetPendingWorldLoadFromFields { .. }
            | StateOperation::SetPlayerForm { .. }
            | StateOperation::SetPlayerMount { .. }
            | StateOperation::SetPlayerControl { .. }
            | StateOperation::SetPlayerAction { .. }
            | StateOperation::Project { .. }
            | StateOperation::Consume { .. }
            | StateOperation::SetGate { .. }
            | StateOperation::ClearGate { .. }
            | StateOperation::ScheduleCleanup { .. }
            | StateOperation::CancelCleanup { .. }
            | StateOperation::Interrupt { .. } => false,
        },
    }
}

fn checked_raw_range(
    byte_offset: u32,
    width: usize,
    bytes_len: usize,
    known_mask_len: usize,
    field: &str,
) -> Result<usize, PlannerContractError> {
    let offset = usize::try_from(byte_offset)
        .map_err(|_| PlannerContractError::new(field, "byte offset does not fit this host"))?;
    let end = offset
        .checked_add(width)
        .ok_or_else(|| PlannerContractError::new(field, "range overflows"))?;
    if end > bytes_len || end > known_mask_len {
        return Err(PlannerContractError::new(
            field,
            "range exceeds the destination component",
        ));
    }
    Ok(offset)
}

fn mark_transition(component: &mut StateComponent, application_id: &str) {
    component.provenance.push(ComponentProvenance {
        source_kind: ProvenanceSourceKind::Transition,
        source_id: application_id.into(),
        source_sha256: None,
        transition_id: Some(application_id.into()),
    });
}

fn invalidate_payload(component: &mut StateComponent) {
    let expected_bytes = match &component.payload {
        ComponentPayload::Raw { bytes, .. } => Some(bytes.len() as u32),
        ComponentPayload::Structured { .. } => None,
        ComponentPayload::Unknown { expected_bytes } => *expected_bytes,
    };
    component.payload = ComponentPayload::Unknown { expected_bytes };
}

fn mark_save_restore(component: &mut StateComponent, application_id: &str) {
    component.provenance.push(ComponentProvenance {
        source_kind: ProvenanceSourceKind::SaveRestore,
        source_id: application_id.into(),
        source_sha256: None,
        transition_id: Some(application_id.into()),
    });
}

fn normalize_provenance(component: &mut StateComponent) {
    component.provenance = vec![ComponentProvenance {
        source_kind: ProvenanceSourceKind::Initialized,
        source_id: "search.identity".into(),
        source_sha256: None,
        transition_id: None,
    }];
}

fn no_selector_match(field: &str) -> PlannerContractError {
    PlannerContractError::new(field, "selector did not match any component")
}

fn validate_component_store(
    owner: &SerializationOwner,
    components: &[StateComponent],
    allow_empty: bool,
) -> Result<(), PlannerContractError> {
    validate_serialization_owner(owner)?;
    if *owner == SerializationOwner::None {
        return Err(PlannerContractError::new(
            "serialized_components.owner",
            "cannot use the none owner as a backing store",
        ));
    }
    if components.is_empty() && !allow_empty {
        return Err(PlannerContractError::new(
            "serialized_components",
            "cannot contain an empty owner store",
        ));
    }
    let mut previous = None;
    for component in components {
        component.validate()?;
        if &component.serialization_owner != owner {
            return Err(PlannerContractError::new(
                "serialized_components.owner",
                "store key and component serialization owner disagree",
            ));
        }
        if matches!(owner, SerializationOwner::StageBank { .. })
            && component.lifetime != crate::state::SemanticLifetime::StageLoad
        {
            return Err(PlannerContractError::new(
                "serialized_components.stage_bank",
                "can contain only stage-load-lifetime components",
            ));
        }
        if previous.is_some_and(|id: &str| id >= component.id.as_str()) {
            return Err(PlannerContractError::new(
                "serialized_components",
                "components must be unique and sorted by ID within each owner",
            ));
        }
        previous = Some(component.id.as_str());
    }
    Ok(())
}

fn validate_persistent_image_binding(
    persistent_file_id: &str,
    component: &StateComponent,
) -> Result<(), PlannerContractError> {
    if let ComponentBinding::RuntimeFile { runtime_file_id } = &component.binding
        && runtime_file_id != persistent_file_id
    {
        return Err(PlannerContractError::new(
            "persistent_file_image.binding",
            "runtime-file binding does not name the persistent image",
        ));
    }
    Ok(())
}

fn rekey_component_runtime(component: &mut StateComponent, source: &str, destination: &str) {
    if component.binding
        == (ComponentBinding::RuntimeFile {
            runtime_file_id: source.into(),
        })
    {
        component.binding = ComponentBinding::RuntimeFile {
            runtime_file_id: destination.into(),
        };
    }
}

fn rekey_serialization_owner_runtime(
    owner: &mut SerializationOwner,
    source: &str,
    destination: &str,
) {
    match owner {
        SerializationOwner::RuntimeFile { runtime_file_id }
        | SerializationOwner::StageBank {
            runtime_file_id, ..
        } if runtime_file_id == source => {
            *runtime_file_id = destination.into();
        }
        _ => {}
    }
}

fn insert_serialized(
    stores: &mut BTreeMap<SerializationOwner, Vec<StateComponent>>,
    owner: &SerializationOwner,
    component: StateComponent,
) {
    let store = stores.entry(owner.clone()).or_default();
    match store.binary_search_by(|existing| existing.id.cmp(&component.id)) {
        Ok(index) => store[index] = component,
        Err(index) => store.insert(index, component),
    }
}

fn select_serialized<'a>(
    stores: &'a BTreeMap<SerializationOwner, Vec<StateComponent>>,
    owner: &SerializationOwner,
    destination_component_id: &str,
) -> Result<&'a StateComponent, PlannerContractError> {
    let store = stores.get(owner).ok_or_else(|| {
        PlannerContractError::new(
            "operation.restore",
            "references an owner with no serialized components",
        )
    })?;
    if let Ok(index) =
        store.binary_search_by(|component| component.id.as_str().cmp(destination_component_id))
    {
        Ok(&store[index])
    } else if let [only] = store.as_slice() {
        Ok(only)
    } else {
        Err(PlannerContractError::new(
            "operation.restore",
            "destination ID is ambiguous within the serialized owner store",
        ))
    }
}

fn adjust_value(value: &mut StateValue, delta: i64) -> Result<(), PlannerContractError> {
    match value {
        StateValue::Signed(current) => {
            *current = current.checked_add(delta).ok_or_else(|| {
                PlannerContractError::new("operation.adjust", "signed value overflows")
            })?;
        }
        StateValue::Unsigned(current) if delta > 0 => {
            *current = current.checked_add(delta as u64).ok_or_else(|| {
                PlannerContractError::new("operation.adjust", "unsigned value overflows")
            })?;
        }
        StateValue::Unsigned(current) => {
            *current = current.checked_sub(delta.unsigned_abs()).ok_or_else(|| {
                PlannerContractError::new("operation.adjust", "unsigned value underflows")
            })?;
        }
        _ => {
            return Err(PlannerContractError::new(
                "operation.adjust",
                "requires a signed or unsigned field",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{RUNTIME_CONFIGURATION_SCHEMA, RuntimeConfiguration};
    use crate::snapshot::STATE_SNAPSHOT_SCHEMA;
    use crate::state::{
        BOUNDARY_POLICY_SCHEMA, BackingAttachment, BoundaryKind, ComponentBindingProjection,
        ComponentBindingReference, ComponentBoundaryRule, EXECUTION_ENVIRONMENT_SCHEMA,
        ExecutionEnvironment, PhysicalSlotId, PlayerForm, PlayerMount, PlayerState, RuntimeFile,
        RuntimeFileLifecycle, RuntimeFileOrigin, SceneLocation, SemanticLifetime,
    };
    use crate::transition::ComponentFieldTarget;

    fn provenance() -> Vec<ComponentProvenance> {
        vec![ComponentProvenance {
            source_kind: ProvenanceSourceKind::Initialized,
            source_id: "fixture.initial".into(),
            source_sha256: Some(Digest([7; 32])),
            transition_id: None,
        }]
    }

    fn structured_component(
        id: &str,
        kind: ComponentKind,
        binding: ComponentBinding,
    ) -> StateComponent {
        StateComponent {
            id: id.into(),
            component_kind: kind,
            payload: ComponentPayload::Structured {
                fields: BTreeMap::new(),
            },
            binding,
            lifetime: SemanticLifetime::RuntimeFile,
            serialization_owner: SerializationOwner::RuntimeFile {
                runtime_file_id: "file-0".into(),
            },
            provenance: provenance(),
        }
    }

    fn raw_component() -> StateComponent {
        StateComponent {
            id: "raw.flags".into(),
            component_kind: ComponentKind::PersistentSave,
            payload: ComponentPayload::Raw {
                bytes: vec![0],
                known_mask: vec![0],
            },
            binding: ComponentBinding::Global,
            lifetime: SemanticLifetime::RuntimeFile,
            serialization_owner: SerializationOwner::RuntimeFile {
                runtime_file_id: "file-0".into(),
            },
            provenance: provenance(),
        }
    }

    fn snapshot() -> StateSnapshot {
        let mut flow = structured_component(
            "flow.main",
            ComponentKind::MessageFlow,
            ComponentBinding::Session {
                session_id: "session-1".into(),
            },
        );
        let ComponentPayload::Structured { fields } = &mut flow.payload else {
            unreachable!()
        };
        fields.insert("node_id".into(), StateValue::Text("start".into()));

        StateSnapshot {
            schema: STATE_SNAPSHOT_SCHEMA.into(),
            id: "snapshot.before".into(),
            sequence: 4,
            environment: ExecutionEnvironment {
                schema: EXECUTION_ENVIRONMENT_SCHEMA.into(),
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
                execution_context: ExecutionContext::World,
                location: SceneLocation {
                    stage: "F_SP103".into(),
                    room: 0,
                    layer: 0,
                    spawn: 0,
                },
                player: PlayerState {
                    form: PlayerForm::Human,
                    mount: None,
                    position: [0.0; 3],
                    rotation: [0; 3],
                    has_control: Some(true),
                    action: "idle".into(),
                },
                components: vec![
                    flow,
                    structured_component(
                        "pending.item",
                        ComponentKind::PendingOperation,
                        ComponentBinding::Session {
                            session_id: "session-1".into(),
                        },
                    ),
                    raw_component(),
                    structured_component(
                        "save.main",
                        ComponentKind::PersistentSave,
                        ComponentBinding::RuntimeFile {
                            runtime_file_id: "file-0".into(),
                        },
                    ),
                ],
                static_world_objects: Vec::new(),
                spatial_volumes: Vec::new(),
                spatial_connections: Vec::new(),
                spatial_planes: Vec::new(),
                persisted_object_controls: Vec::new(),
                live_world_objects: Vec::new(),
            },
            semantic_observations: Vec::new(),
        }
    }

    fn id_selector(component_id: &str) -> ComponentSelector {
        ComponentSelector::Id {
            component_id: component_id.into(),
        }
    }

    fn field<'a>(
        state: &'a PlannerExecutionState,
        component_id: &str,
        name: &str,
    ) -> &'a StateValue {
        let component = state
            .snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == component_id)
            .unwrap();
        let ComponentPayload::Structured { fields } = &component.payload else {
            panic!("expected structured component")
        };
        fields.get(name).unwrap()
    }

    #[test]
    fn applies_writes_gates_and_locations_as_one_new_snapshot() {
        let mut state = PlannerExecutionState::new(snapshot()).unwrap();
        let result = state
            .apply_operations(
                "transition.enter-forest",
                "snapshot.after-enter-forest",
                &[
                    StateOperation::Write {
                        target: ComponentFieldTarget {
                            component_id: "save.main".into(),
                            field: "small_keys".into(),
                        },
                        value: StateValue::Unsigned(1),
                    },
                    StateOperation::SetGate {
                        gate_id: "gate.no-teleport".into(),
                    },
                    StateOperation::SetLocation {
                        location: SceneLocation {
                            stage: "D_MN05".into(),
                            room: 1,
                            layer: 0,
                            spawn: 2,
                        },
                    },
                ],
            )
            .unwrap();
        assert_ne!(result.source_snapshot_sha256, result.result_snapshot_sha256);
        assert_eq!(state.snapshot.sequence, 5);
        assert_eq!(state.snapshot.id, "snapshot.after-enter-forest");
        assert_eq!(state.snapshot.environment.location.stage, "D_MN05");
        assert_eq!(
            field(&state, "save.main", "small_keys"),
            &StateValue::Unsigned(1)
        );
        assert_eq!(state.gate_states.get("gate.no-teleport"), Some(&true));
        assert_eq!(state.execution_history.len(), 3);
        let mut without_history = state.clone();
        without_history.execution_history.clear();
        assert_ne!(state.digest().unwrap(), without_history.digest().unwrap());
        assert_eq!(
            state.semantic_digest().unwrap(),
            without_history.semantic_digest().unwrap()
        );
        assert_eq!(
            state
                .last_field_writer("save.main", "small_keys")
                .unwrap()
                .application_id,
            "transition.enter-forest"
        );
        assert_eq!(state.gate_history("gate.no-teleport").len(), 1);
        assert_eq!(
            state
                .snapshot
                .environment
                .components
                .iter()
                .find(|component| component.id == "save.main")
                .unwrap()
                .provenance
                .last()
                .unwrap()
                .transition_id
                .as_deref(),
            Some("transition.enter-forest")
        );
    }

    #[test]
    fn multi_field_write_updates_one_record_atomically_and_tracks_each_field() {
        let mut state = PlannerExecutionState::new(snapshot()).unwrap();
        state
            .apply_operations(
                "writer.savmem-tower",
                "snapshot.after-savmem-tower",
                &[StateOperation::WriteFields {
                    component_id: "save.main".into(),
                    fields: BTreeMap::from([
                        ("return_stage".into(), StateValue::Text("R_SP107".into())),
                        ("return_room".into(), StateValue::Signed(3)),
                        ("return_spawn".into(), StateValue::Unsigned(1)),
                    ]),
                }],
            )
            .unwrap();

        assert_eq!(
            field(&state, "save.main", "return_stage"),
            &StateValue::Text("R_SP107".into())
        );
        assert_eq!(
            field(&state, "save.main", "return_room"),
            &StateValue::Signed(3)
        );
        assert_eq!(
            field(&state, "save.main", "return_spawn"),
            &StateValue::Unsigned(1)
        );
        for name in ["return_stage", "return_room", "return_spawn"] {
            assert_eq!(
                state
                    .last_field_writer("save.main", name)
                    .unwrap()
                    .application_id,
                "writer.savmem-tower"
            );
        }

        state
            .apply_operations(
                "transition.savewarp",
                "snapshot.after-savewarp",
                &[StateOperation::SetLocationFromFields {
                    component_id: "save.main".into(),
                    stage_field: "return_stage".into(),
                    room_field: "return_room".into(),
                    spawn_field: "return_spawn".into(),
                    layer: -1,
                }],
            )
            .unwrap();
        assert_eq!(state.snapshot.environment.location.stage, "R_SP107");
        assert_eq!(state.snapshot.environment.location.room, 3);
        assert_eq!(state.snapshot.environment.location.spawn, 1);
        assert_eq!(state.snapshot.environment.location.layer, -1);
    }

    #[test]
    fn structured_invalidation_removes_known_value_with_distinct_provenance() {
        let mut state = PlannerExecutionState::new(snapshot()).unwrap();
        state
            .apply_operations(
                "cutscene.unknown-suffix",
                "snapshot.after-unknown-suffix",
                &[StateOperation::InvalidateField {
                    target: ComponentFieldTarget {
                        component_id: "save.main".into(),
                        field: "small_keys".into(),
                    },
                }],
            )
            .unwrap();
        let component = state
            .snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "save.main")
            .unwrap();
        let ComponentPayload::Structured { fields } = &component.payload else {
            unreachable!()
        };
        assert!(!fields.contains_key("small_keys"));
        assert!(matches!(
            &state.execution_history.last().unwrap().event,
            ExecutionHistoryKind::Operation {
                operation: StateOperation::InvalidateField { .. },
                ..
            }
        ));
    }

    #[test]
    fn player_state_operations_are_ordered_and_round_trip_in_history() {
        let mut state = PlannerExecutionState::new(snapshot()).unwrap();
        state
            .apply_operations(
                "cutscene.partial-player-state",
                "snapshot.partial-player-state",
                &[
                    StateOperation::SetPlayerForm {
                        form: PlayerForm::Wolf,
                    },
                    StateOperation::SetPlayerMount {
                        mount: Some(PlayerMount::Epona),
                    },
                    StateOperation::SetPlayerControl { has_control: None },
                    StateOperation::SetPlayerAction {
                        action: "cutscene-warp".into(),
                    },
                ],
            )
            .unwrap();

        assert_eq!(state.snapshot.environment.player.form, PlayerForm::Wolf);
        assert_eq!(
            state.snapshot.environment.player.mount,
            Some(PlayerMount::Epona)
        );
        assert_eq!(state.snapshot.environment.player.has_control, None);
        assert_eq!(state.snapshot.environment.player.action, "cutscene-warp");
        assert_eq!(state.execution_history.len(), 4);
        assert!(state.execution_history.iter().all(|event| {
            matches!(
                &event.event,
                ExecutionHistoryKind::Operation {
                    affected_component_ids,
                    ..
                } if affected_component_ids.is_empty()
            )
        }));

        let document = state.to_document().unwrap();
        assert_eq!(document.schema, PLANNER_EXECUTION_STATE_SCHEMA);
        let decoded =
            PlannerExecutionStateDocument::decode_canonical(&document.canonical_bytes().unwrap())
                .unwrap();
        assert_eq!(decoded.into_state().unwrap(), state);
    }

    #[test]
    fn held_writer_value_and_gate_history_remain_queryable_in_order() {
        let mut state = PlannerExecutionState::new(snapshot()).unwrap();
        let return_place = ComponentFieldTarget {
            component_id: "save.main".into(),
            field: "player_return_place".into(),
        };
        state
            .apply_operations(
                "writer.return-place.ordon",
                "snapshot.return-place.ordon",
                &[StateOperation::Write {
                    target: return_place.clone(),
                    value: StateValue::Text("F_SP103:0:0:0".into()),
                }],
            )
            .unwrap();
        state
            .apply_operations(
                "gate.fanadi-lock.set",
                "snapshot.fanadi-lock.set",
                &[StateOperation::SetGate {
                    gate_id: "gate.no-telop".into(),
                }],
            )
            .unwrap();

        assert_eq!(
            field(&state, "save.main", "player_return_place"),
            &StateValue::Text("F_SP103:0:0:0".into())
        );
        assert_eq!(
            state
                .last_field_writer("save.main", "player_return_place")
                .unwrap()
                .application_id,
            "writer.return-place.ordon"
        );
        let gate_history = state.gate_history("gate.no-telop");
        assert_eq!(gate_history.len(), 1);
        assert_eq!(gate_history[0].application_id, "gate.fanadi-lock.set");

        state
            .apply_operations(
                "gate.fanadi-lock.release-and-write",
                "snapshot.fanadi-lock.released",
                &[
                    StateOperation::ClearGate {
                        gate_id: "gate.no-telop".into(),
                    },
                    StateOperation::Write {
                        target: return_place,
                        value: StateValue::Text("R_SP109:0:0:0".into()),
                    },
                ],
            )
            .unwrap();
        assert_eq!(state.gate_history("gate.no-telop").len(), 2);
        let last_writer = state
            .last_field_writer("save.main", "player_return_place")
            .unwrap();
        assert_eq!(
            last_writer.application_id,
            "gate.fanadi-lock.release-and-write"
        );
        assert_eq!(last_writer.operation_index, 1);
    }

    #[test]
    fn recent_item_survives_file_load_and_drives_generic_inventory_grant() {
        // dItemNo_FISHING_ROD_1_e and dItemNo_RAFRELS_MEMO_e.
        const ROD_ITEM_ID: u64 = 0x4a;
        const MEMO_ITEM_ID: u64 = 0x90;

        let mut source = snapshot();
        let mut recent_item = structured_component(
            "event.recent-item",
            ComponentKind::Session,
            ComponentBinding::Session {
                session_id: "session-1".into(),
            },
        );
        recent_item.lifetime = SemanticLifetime::Session;
        recent_item.serialization_owner = SerializationOwner::None;
        let ComponentPayload::Structured { fields } = &mut recent_item.payload else {
            unreachable!()
        };
        fields.insert("get_item_no".into(), StateValue::Unsigned(0));

        let mut handoff = structured_component(
            "event.item-handoff",
            ComponentKind::PendingOperation,
            ComponentBinding::Session {
                session_id: "session-1".into(),
            },
        );
        handoff.lifetime = SemanticLifetime::Action;
        handoff.serialization_owner = SerializationOwner::None;
        let ComponentPayload::Structured { fields } = &mut handoff.payload else {
            unreachable!()
        };
        fields.insert("pre_item_no".into(), StateValue::Unsigned(3));

        let mut inventory_a = structured_component(
            "inventory.active",
            ComponentKind::Inventory,
            ComponentBinding::RuntimeFile {
                runtime_file_id: "file-0".into(),
            },
        );
        let ComponentPayload::Structured { fields } = &mut inventory_a.payload else {
            unreachable!()
        };
        fields.insert("owned_item_ids".into(), StateValue::Bytes(vec![0; 32]));
        source
            .environment
            .components
            .extend([recent_item, handoff, inventory_a]);
        source
            .environment
            .components
            .sort_by(|left, right| left.id.cmp(&right.id));

        let mut state = PlannerExecutionState::new(source).unwrap();
        state
            .apply_operations(
                "writer.file-a-rod-presentation",
                "snapshot.file-a-rod-prepared",
                &[StateOperation::Write {
                    target: ComponentFieldTarget {
                        component_id: "event.recent-item".into(),
                        field: "get_item_no".into(),
                    },
                    value: StateValue::Unsigned(ROD_ITEM_ID),
                }],
            )
            .unwrap();

        let mut inventory_b = structured_component(
            "inventory.active",
            ComponentKind::Inventory,
            ComponentBinding::RuntimeFile {
                runtime_file_id: "file-b".into(),
            },
        );
        inventory_b.serialization_owner = SerializationOwner::RuntimeFile {
            runtime_file_id: "file-b".into(),
        };
        let ComponentPayload::Structured { fields } = &mut inventory_b.payload else {
            unreachable!()
        };
        fields.insert("owned_item_ids".into(), StateValue::Bytes(vec![0; 32]));
        let policy = BoundaryPolicy {
            schema: BOUNDARY_POLICY_SCHEMA.into(),
            id: "boundary.load-file-b".into(),
            boundary: BoundaryKind::LoadPhysicalSlot,
            default_disposition: BoundaryDisposition::Clear,
            component_rules: vec![
                ComponentBoundaryRule {
                    selector: id_selector("event.recent-item"),
                    disposition: BoundaryDisposition::Preserve,
                },
                ComponentBoundaryRule {
                    selector: id_selector("inventory.active"),
                    disposition: BoundaryDisposition::Reinitialize {
                        initializer_id: "inventory.active".into(),
                    },
                },
            ],
        };
        state
            .apply_boundary(
                "boundary.load-file-b",
                "snapshot.file-b-loaded",
                &policy,
                &BTreeMap::from([("inventory.active".into(), inventory_b)]),
            )
            .unwrap();
        assert_eq!(
            field(&state, "event.recent-item", "get_item_no"),
            &StateValue::Unsigned(ROD_ITEM_ID)
        );
        assert!(
            state
                .snapshot
                .environment
                .components
                .iter()
                .all(|component| component.id != "event.item-handoff")
        );

        let loaded = state.clone();
        let file_b = RuntimeFile {
            id: "file-b".into(),
            origin: RuntimeFileOrigin::LoadedSlot {
                slot: PhysicalSlotId(1),
            },
            backing: BackingAttachment::CardBacked {
                slot: PhysicalSlotId(1),
            },
            allowed_serialization_targets: vec![PhysicalSlotId(1)],
            lifecycle: RuntimeFileLifecycle::Active,
        };
        state
            .apply_operations(
                "auru.broken-generic-get-item",
                "snapshot.file-b-rod-granted",
                &[
                    StateOperation::SetActiveRuntimeFile {
                        runtime_file: file_b.clone(),
                    },
                    StateOperation::SetBitFromValue {
                        source: ComponentFieldTarget {
                            component_id: "event.recent-item".into(),
                            field: "get_item_no".into(),
                        },
                        target: ComponentFieldTarget {
                            component_id: "inventory.active".into(),
                            field: "owned_item_ids".into(),
                        },
                    },
                ],
            )
            .unwrap();
        let StateValue::Bytes(items) = field(&state, "inventory.active", "owned_item_ids") else {
            unreachable!()
        };
        assert_ne!(
            items[ROD_ITEM_ID as usize / 8] & (1 << (ROD_ITEM_ID % 8)),
            0
        );

        let mut normal_path = loaded;
        normal_path
            .apply_operations(
                "auru.normal-memo-get-item",
                "snapshot.file-b-memo-granted",
                &[
                    StateOperation::SetActiveRuntimeFile {
                        runtime_file: file_b,
                    },
                    StateOperation::Write {
                        target: ComponentFieldTarget {
                            component_id: "event.recent-item".into(),
                            field: "get_item_no".into(),
                        },
                        value: StateValue::Unsigned(MEMO_ITEM_ID),
                    },
                    StateOperation::SetBitFromValue {
                        source: ComponentFieldTarget {
                            component_id: "event.recent-item".into(),
                            field: "get_item_no".into(),
                        },
                        target: ComponentFieldTarget {
                            component_id: "inventory.active".into(),
                            field: "owned_item_ids".into(),
                        },
                    },
                ],
            )
            .unwrap();
        let StateValue::Bytes(items) = field(&normal_path, "inventory.active", "owned_item_ids")
        else {
            unreachable!()
        };
        assert_ne!(
            items[MEMO_ITEM_ID as usize / 8] & (1 << (MEMO_ITEM_ID % 8)),
            0
        );
        assert_eq!(
            items[ROD_ITEM_ID as usize / 8] & (1 << (ROD_ITEM_ID % 8)),
            0
        );
    }

    #[test]
    fn recent_item_boundary_matrix_is_process_owned_and_last_writer_wins() {
        const ROD_ITEM_ID: u64 = 0x4a;
        const MEMO_ITEM_ID: u64 = 0x90;

        let recent_item_component = |value: u64| {
            let mut component = structured_component(
                "event.recent-item",
                ComponentKind::Session,
                ComponentBinding::Session {
                    session_id: "session-1".into(),
                },
            );
            component.lifetime = SemanticLifetime::Session;
            component.serialization_owner = SerializationOwner::None;
            let ComponentPayload::Structured { fields } = &mut component.payload else {
                unreachable!()
            };
            fields.insert("get_item_no".into(), StateValue::Unsigned(value));
            component
        };
        let state_with_recent_item = || {
            let mut source = snapshot();
            source
                .environment
                .components
                .push(recent_item_component(ROD_ITEM_ID));
            source
                .environment
                .components
                .sort_by(|left, right| left.id.cmp(&right.id));
            PlannerExecutionState::new(source).unwrap()
        };

        let in_process_boundaries = vec![
            ("room-transition", BoundaryKind::RoomTransition),
            ("stage-transition", BoundaryKind::StageTransition),
            ("void-reload", BoundaryKind::VoidReload),
            ("savewarp", BoundaryKind::SaveWarp),
            ("title-return", BoundaryKind::TitleReturn),
            ("load-physical-slot", BoundaryKind::LoadPhysicalSlot),
            ("save-runtime-to-slot", BoundaryKind::SaveRuntimeToSlot),
            ("wrong-state-respawn", BoundaryKind::WrongStateRespawn),
            ("dialogue-interruption", BoundaryKind::DialogueInterruption),
        ];
        for (label, boundary) in in_process_boundaries {
            let mut state = state_with_recent_item();
            let policy = BoundaryPolicy {
                schema: BOUNDARY_POLICY_SCHEMA.into(),
                id: format!("boundary.auru-{label}"),
                boundary,
                default_disposition: BoundaryDisposition::Clear,
                component_rules: vec![ComponentBoundaryRule {
                    selector: id_selector("event.recent-item"),
                    disposition: BoundaryDisposition::Preserve,
                }],
            };
            state
                .apply_boundary(
                    &policy.id,
                    &format!("snapshot.after-{label}"),
                    &policy,
                    &BTreeMap::new(),
                )
                .unwrap();
            assert_eq!(
                field(&state, "event.recent-item", "get_item_no"),
                &StateValue::Unsigned(ROD_ITEM_ID),
                "{label} must not silently clear process-owned mGtItm"
            );
        }

        let mut event_cleanup = state_with_recent_item();
        let mut shown_item = structured_component(
            "event.shown-item",
            ComponentKind::PendingOperation,
            ComponentBinding::Session {
                session_id: "session-1".into(),
            },
        );
        shown_item.lifetime = SemanticLifetime::Action;
        shown_item.serialization_owner = SerializationOwner::None;
        let ComponentPayload::Structured { fields } = &mut shown_item.payload else {
            unreachable!()
        };
        fields.insert("pre_item_no".into(), StateValue::Unsigned(0x91));
        event_cleanup
            .snapshot
            .environment
            .components
            .push(shown_item);
        event_cleanup
            .snapshot
            .environment
            .components
            .sort_by(|left, right| left.id.cmp(&right.id));
        event_cleanup.snapshot.validate().unwrap();
        event_cleanup
            .apply_operations(
                "writer.show-item-x",
                "snapshot.after-show-item-x",
                &[StateOperation::Write {
                    target: ComponentFieldTarget {
                        component_id: "event.shown-item".into(),
                        field: "pre_item_no".into(),
                    },
                    value: StateValue::Unsigned(0x4b),
                }],
            )
            .unwrap();
        assert_eq!(
            field(&event_cleanup, "event.shown-item", "pre_item_no"),
            &StateValue::Unsigned(0x4b)
        );
        assert_eq!(
            field(&event_cleanup, "event.recent-item", "get_item_no"),
            &StateValue::Unsigned(ROD_ITEM_ID),
            "show-item acceptance writes mPreItemNo, not mGtItm"
        );
        let cleanup_policy = BoundaryPolicy {
            schema: BOUNDARY_POLICY_SCHEMA.into(),
            id: "boundary.event-control-remove".into(),
            boundary: BoundaryKind::Custom {
                id: "event-control-remove".into(),
            },
            default_disposition: BoundaryDisposition::Clear,
            component_rules: vec![ComponentBoundaryRule {
                selector: id_selector("event.recent-item"),
                disposition: BoundaryDisposition::Preserve,
            }],
        };
        event_cleanup
            .apply_boundary(
                &cleanup_policy.id,
                "snapshot.after-event-control-remove",
                &cleanup_policy,
                &BTreeMap::new(),
            )
            .unwrap();
        assert!(
            event_cleanup
                .snapshot
                .environment
                .components
                .iter()
                .all(|component| component.id != "event.shown-item")
        );
        assert_eq!(
            field(&event_cleanup, "event.recent-item", "get_item_no"),
            &StateValue::Unsigned(ROD_ITEM_ID)
        );

        event_cleanup
            .apply_operations(
                "writer.auru-normal-memo-presentation",
                "snapshot.after-memo-presentation",
                &[StateOperation::Write {
                    target: ComponentFieldTarget {
                        component_id: "event.recent-item".into(),
                        field: "get_item_no".into(),
                    },
                    value: StateValue::Unsigned(MEMO_ITEM_ID),
                }],
            )
            .unwrap();
        assert_eq!(
            field(&event_cleanup, "event.recent-item", "get_item_no"),
            &StateValue::Unsigned(MEMO_ITEM_ID)
        );
        assert_eq!(
            event_cleanup
                .last_field_writer("event.recent-item", "get_item_no")
                .unwrap()
                .application_id,
            "writer.auru-normal-memo-presentation"
        );

        let process_restart = BoundaryPolicy {
            schema: BOUNDARY_POLICY_SCHEMA.into(),
            id: "boundary.process-restart".into(),
            boundary: BoundaryKind::ProcessRestart,
            default_disposition: BoundaryDisposition::Clear,
            component_rules: vec![ComponentBoundaryRule {
                selector: id_selector("event.recent-item"),
                disposition: BoundaryDisposition::Reinitialize {
                    initializer_id: "event.recent-item".into(),
                },
            }],
        };
        event_cleanup
            .apply_boundary(
                &process_restart.id,
                "snapshot.after-process-restart",
                &process_restart,
                &BTreeMap::from([("event.recent-item".into(), recent_item_component(0))]),
            )
            .unwrap();
        assert_eq!(
            field(&event_cleanup, "event.recent-item", "get_item_no"),
            &StateValue::Unsigned(0)
        );
    }

    #[test]
    fn failed_batches_are_atomic() {
        let mut state = PlannerExecutionState::new(snapshot()).unwrap();
        let before = state.clone();
        let error = state
            .apply_operations(
                "transition.bad",
                "snapshot.never-committed",
                &[
                    StateOperation::Write {
                        target: ComponentFieldTarget {
                            component_id: "save.main".into(),
                            field: "would_have_changed".into(),
                        },
                        value: StateValue::Boolean(true),
                    },
                    StateOperation::ClearComponent {
                        selector: id_selector("missing.component"),
                    },
                ],
            )
            .unwrap_err();
        assert_eq!(error.field(), "operation.clear_component");
        assert_eq!(state, before);
    }

    #[test]
    fn payload_replacement_retains_component_identity_and_is_a_whole_component_writer() {
        let mut state = PlannerExecutionState::new(snapshot()).unwrap();
        let before = state
            .snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "save.main")
            .unwrap()
            .clone();
        let replacement = ComponentPayload::Structured {
            fields: BTreeMap::from([
                ("life".into(), StateValue::Unsigned(12)),
                ("rupees".into(), StateValue::Unsigned(0)),
            ]),
        };

        state
            .apply_operations(
                "initializer.opening-save",
                "snapshot.opening-save-initialized",
                &[StateOperation::ReplacePayload {
                    component_id: "save.main".into(),
                    payload: replacement.clone(),
                }],
            )
            .unwrap();

        let after = state
            .snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "save.main")
            .unwrap();
        assert_eq!(after.payload, replacement);
        assert_eq!(after.id, before.id);
        assert_eq!(after.component_kind, before.component_kind);
        assert_eq!(after.binding, before.binding);
        assert_eq!(after.lifetime, before.lifetime);
        assert_eq!(after.serialization_owner, before.serialization_owner);
        assert_eq!(
            &after.provenance[..before.provenance.len()],
            &before.provenance
        );
        assert_eq!(
            state
                .last_field_writer("save.main", "life")
                .unwrap()
                .application_id,
            "initializer.opening-save"
        );
    }

    #[test]
    fn payload_invalidation_can_include_runtime_stores_but_never_physical_images() {
        let mut state = PlannerExecutionState::new(snapshot()).unwrap();
        state
            .apply_operations(
                "boundary.save-file-0",
                "snapshot.file-0-saved",
                &[StateOperation::SaveRuntimeToSlot {
                    source_runtime_file_id: "file-0".into(),
                    destination_slot: PhysicalSlotId(1),
                    destination_persistent_file_id: "persistent-slot-1".into(),
                    runtime_component_ids: vec!["raw.flags".into(), "save.main".into()],
                    stage_bank_stages: Vec::new(),
                }],
            )
            .unwrap();
        let sealed_image = state.persistent_file_images["persistent-slot-1"].clone();
        let sealed_digest = sealed_image.digest().unwrap();

        let owner = SerializationOwner::StageBank {
            runtime_file_id: "file-0".into(),
            stage: "F_SP103".into(),
        };
        let mut stored = raw_component();
        stored.binding = ComponentBinding::Stage {
            stage: "F_SP103".into(),
        };
        stored.lifetime = SemanticLifetime::StageLoad;
        stored.serialization_owner = owner.clone();
        state
            .serialized_components
            .insert(owner.clone(), vec![stored]);
        let inactive_owner = SerializationOwner::StageBank {
            runtime_file_id: "inactive-file".into(),
            stage: "F_SP103".into(),
        };
        let mut inactive_stored = raw_component();
        inactive_stored.binding = ComponentBinding::Stage {
            stage: "F_SP103".into(),
        };
        inactive_stored.lifetime = SemanticLifetime::StageLoad;
        inactive_stored.serialization_owner = inactive_owner.clone();
        let inactive_payload = inactive_stored.payload.clone();
        state
            .serialized_components
            .insert(inactive_owner.clone(), vec![inactive_stored]);
        state
            .snapshot
            .environment
            .inactive_runtime_files
            .push(RuntimeFile {
                id: "inactive-file".into(),
                origin: RuntimeFileOrigin::NewFile,
                backing: BackingAttachment::MemoryOnly,
                allowed_serialization_targets: Vec::new(),
                lifecycle: RuntimeFileLifecycle::Suspended,
            });
        state.validate().unwrap();

        state
            .apply_operations(
                "initializer.invalidate-runtime-payloads",
                "snapshot.runtime-payloads-invalidated",
                &[StateOperation::InvalidatePayloads {
                    selector: id_selector("raw.flags"),
                    include_active_runtime_serialized_stores: true,
                }],
            )
            .unwrap();

        let live = state
            .snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "raw.flags")
            .unwrap();
        assert_eq!(
            live.payload,
            ComponentPayload::Unknown {
                expected_bytes: Some(1)
            }
        );
        assert_eq!(
            state.serialized_components[&owner][0].payload,
            ComponentPayload::Unknown {
                expected_bytes: Some(1)
            }
        );
        assert_eq!(
            state.serialized_components[&inactive_owner][0].payload,
            inactive_payload
        );
        assert_eq!(
            state.persistent_file_images["persistent-slot-1"],
            sealed_image
        );
        assert_eq!(
            state.persistent_file_images["persistent-slot-1"]
                .digest()
                .unwrap(),
            sealed_digest
        );
        assert!(matches!(
            state
                .snapshot
                .environment
                .components
                .iter()
                .find(|component| component.id == "save.main")
                .unwrap()
                .payload,
            ComponentPayload::Structured { .. }
        ));
        assert_eq!(
            state
                .last_field_writer("raw.flags", "any-field")
                .unwrap()
                .application_id,
            "initializer.invalidate-runtime-payloads"
        );

        let before_failure = state.clone();
        let error = state
            .apply_operations(
                "initializer.missing-payload",
                "snapshot.not-produced",
                &[StateOperation::InvalidatePayloads {
                    selector: id_selector("missing.component"),
                    include_active_runtime_serialized_stores: true,
                }],
            )
            .unwrap_err();
        assert_eq!(error.field(), "operation.invalidate_payloads");
        assert_eq!(state, before_failure);
    }

    #[test]
    fn beginning_runtime_lifetime_rekeys_owned_stores_and_preserves_physical_images() {
        let mut source = snapshot();
        for component in &mut source.environment.components {
            if matches!(component.binding, ComponentBinding::Session { .. }) {
                component.serialization_owner = SerializationOwner::None;
                component.lifetime = SemanticLifetime::Session;
            }
        }
        let session_before = source
            .environment
            .components
            .iter()
            .find(|component| component.id == "flow.main")
            .unwrap()
            .clone();
        let mut state = PlannerExecutionState::new(source).unwrap();
        state
            .apply_operations(
                "boundary.seed-physical-image",
                "snapshot.physical-image-seeded",
                &[StateOperation::SaveRuntimeToSlot {
                    source_runtime_file_id: "file-0".into(),
                    destination_slot: PhysicalSlotId(1),
                    destination_persistent_file_id: "persistent-slot-1".into(),
                    runtime_component_ids: vec!["raw.flags".into(), "save.main".into()],
                    stage_bank_stages: Vec::new(),
                }],
            )
            .unwrap();
        let physical_image_before = state.persistent_file_images["persistent-slot-1"].clone();

        state.snapshot.environment.active_runtime_file = RuntimeFile {
            id: "loaded-a".into(),
            origin: RuntimeFileOrigin::LoadedSlot {
                slot: PhysicalSlotId(1),
            },
            backing: BackingAttachment::CardBacked {
                slot: PhysicalSlotId(1),
            },
            allowed_serialization_targets: vec![PhysicalSlotId(1)],
            lifecycle: RuntimeFileLifecycle::Active,
        };
        for component in &mut state.snapshot.environment.components {
            rekey_component_runtime(component, "file-0", "loaded-a");
            rekey_serialization_owner_runtime(
                &mut component.serialization_owner,
                "file-0",
                "loaded-a",
            );
        }
        let stage_owner = SerializationOwner::StageBank {
            runtime_file_id: "loaded-a".into(),
            stage: "F_SP103".into(),
        };
        let mut stage_component = raw_component();
        stage_component.id = "stage.live".into();
        stage_component.binding = ComponentBinding::Stage {
            stage: "F_SP103".into(),
        };
        stage_component.lifetime = SemanticLifetime::StageLoad;
        stage_component.serialization_owner = stage_owner.clone();
        state
            .snapshot
            .environment
            .components
            .push(stage_component.clone());
        state
            .snapshot
            .environment
            .components
            .sort_by(|left, right| left.id.cmp(&right.id));
        state
            .serialized_components
            .insert(stage_owner, vec![stage_component]);
        let unrelated_owner = SerializationOwner::StageBank {
            runtime_file_id: "suspended".into(),
            stage: "D_MN05".into(),
        };
        let mut unrelated_component = raw_component();
        unrelated_component.id = "stage.unrelated".into();
        unrelated_component.binding = ComponentBinding::Stage {
            stage: "D_MN05".into(),
        };
        unrelated_component.lifetime = SemanticLifetime::StageLoad;
        unrelated_component.serialization_owner = unrelated_owner.clone();
        state
            .serialized_components
            .insert(unrelated_owner.clone(), vec![unrelated_component]);
        state
            .snapshot
            .environment
            .inactive_runtime_files
            .push(RuntimeFile {
                id: "suspended".into(),
                origin: RuntimeFileOrigin::NewFile,
                backing: BackingAttachment::MemoryOnly,
                allowed_serialization_targets: Vec::new(),
                lifecycle: RuntimeFileLifecycle::Suspended,
            });
        state.validate().unwrap();

        let mut colliding = state.clone();
        let colliding_runtime = RuntimeFile {
            id: "loaded-a.title-file-0".into(),
            origin: RuntimeFileOrigin::TitleFile0,
            backing: BackingAttachment::MemoryOnly,
            allowed_serialization_targets: Vec::new(),
            lifecycle: RuntimeFileLifecycle::Ended,
        };
        let insert_at = colliding
            .snapshot
            .environment
            .inactive_runtime_files
            .binary_search_by(|runtime| runtime.id.cmp(&colliding_runtime.id))
            .unwrap_err();
        colliding
            .snapshot
            .environment
            .inactive_runtime_files
            .insert(insert_at, colliding_runtime);
        colliding.validate().unwrap();
        let before_collision = colliding.clone();
        let error = colliding
            .apply_operations(
                "boundary.colliding-title-file-0",
                "snapshot.not-produced",
                &[StateOperation::BeginRuntimeFileLifetime {
                    destination_id_suffix: "title-file-0".into(),
                    origin: RuntimeFileOrigin::TitleFile0,
                    backing: BackingAttachment::MemoryOnly,
                    allowed_serialization_targets: Vec::new(),
                }],
            )
            .unwrap_err();
        assert_eq!(
            error.field(),
            "operation.begin_runtime_file_lifetime.destination_runtime_file_id"
        );
        assert_eq!(colliding, before_collision);

        state
            .apply_operations(
                "boundary.begin-title-file-0",
                "snapshot.title-file-0-active",
                &[StateOperation::BeginRuntimeFileLifetime {
                    destination_id_suffix: "title-file-0".into(),
                    origin: RuntimeFileOrigin::TitleFile0,
                    backing: BackingAttachment::MemoryOnly,
                    allowed_serialization_targets: vec![
                        PhysicalSlotId(1),
                        PhysicalSlotId(2),
                        PhysicalSlotId(3),
                    ],
                }],
            )
            .unwrap();

        let destination = "loaded-a.title-file-0";
        assert_eq!(
            state.snapshot.environment.active_runtime_file.id,
            destination
        );
        assert_eq!(
            state.snapshot.environment.active_runtime_file.origin,
            RuntimeFileOrigin::TitleFile0
        );
        assert_eq!(
            state.snapshot.environment.active_runtime_file.backing,
            BackingAttachment::MemoryOnly
        );
        assert_eq!(
            state
                .snapshot
                .environment
                .inactive_runtime_files
                .iter()
                .find(|runtime| runtime.id == "loaded-a")
                .unwrap()
                .lifecycle,
            RuntimeFileLifecycle::Ended
        );
        let save = state
            .snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "save.main")
            .unwrap();
        assert_eq!(
            save.binding,
            ComponentBinding::RuntimeFile {
                runtime_file_id: destination.into()
            }
        );
        assert_eq!(
            save.serialization_owner,
            SerializationOwner::RuntimeFile {
                runtime_file_id: destination.into()
            }
        );
        let destination_stage_owner = SerializationOwner::StageBank {
            runtime_file_id: destination.into(),
            stage: "F_SP103".into(),
        };
        assert!(
            state
                .serialized_components
                .contains_key(&destination_stage_owner)
        );
        assert!(state.serialized_components.contains_key(&unrelated_owner));
        assert_eq!(
            state
                .snapshot
                .environment
                .components
                .iter()
                .find(|component| component.id == "flow.main")
                .unwrap(),
            &session_before
        );
        assert_eq!(
            state.persistent_file_images["persistent-slot-1"],
            physical_image_before
        );
    }

    #[test]
    fn search_identity_includes_non_snapshot_backing_stores() {
        let state = PlannerExecutionState::new(snapshot()).unwrap();
        let mut gated = state.clone();
        gated.gate_states.insert("gate.no-teleport".into(), true);
        let mut cleanup = state.clone();
        cleanup
            .scheduled_cleanup_ids
            .insert("cleanup.item-handoff".into());
        assert_ne!(state.digest().unwrap(), gated.digest().unwrap());
        assert_ne!(state.digest().unwrap(), cleanup.digest().unwrap());

        let mut history_only = state.clone();
        history_only.snapshot.id = "snapshot.other-history".into();
        history_only.snapshot.sequence = 99;
        mark_transition(
            &mut history_only.snapshot.environment.components[0],
            "transition.history-only",
        );
        assert_ne!(state.digest().unwrap(), history_only.digest().unwrap());
        assert_eq!(
            state.semantic_digest().unwrap(),
            history_only.semantic_digest().unwrap()
        );

        let document = state.to_document().unwrap();
        let bytes = document.canonical_bytes().unwrap();
        let decoded = PlannerExecutionStateDocument::decode_canonical(&bytes).unwrap();
        assert_eq!(decoded.into_state().unwrap(), state);
    }

    #[test]
    fn boundary_policy_clears_unmentioned_components_and_honors_one_shot_preserve() {
        let mut state = PlannerExecutionState::new(snapshot()).unwrap();
        state
            .apply_operations(
                "technique.preserve-save",
                "snapshot.preserve-armed",
                &[StateOperation::Preserve {
                    selector: id_selector("save.main"),
                }],
            )
            .unwrap();
        let policy = BoundaryPolicy {
            schema: BOUNDARY_POLICY_SCHEMA.into(),
            id: "boundary.room-load".into(),
            boundary: BoundaryKind::RoomTransition,
            default_disposition: BoundaryDisposition::Clear,
            component_rules: vec![ComponentBoundaryRule {
                selector: id_selector("flow.main"),
                disposition: BoundaryDisposition::Preserve,
            }],
        };
        state
            .apply_boundary(
                "boundary.room-load",
                "snapshot.after-room-load",
                &policy,
                &BTreeMap::new(),
            )
            .unwrap();
        let ids = state
            .snapshot
            .environment
            .components
            .iter()
            .map(|component| component.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["flow.main", "save.main"]);
        assert!(state.preserved_component_ids.is_empty());
    }

    #[test]
    fn unknown_boundary_behavior_fails_atomically() {
        let mut state = PlannerExecutionState::new(snapshot()).unwrap();
        let before = state.clone();
        let policy = BoundaryPolicy {
            schema: BOUNDARY_POLICY_SCHEMA.into(),
            id: "boundary.unknown".into(),
            boundary: BoundaryKind::WrongStateRespawn,
            default_disposition: BoundaryDisposition::Unknown,
            component_rules: Vec::new(),
        };
        let error = state
            .apply_boundary(
                "boundary.unknown",
                "snapshot.not-produced",
                &policy,
                &BTreeMap::new(),
            )
            .unwrap_err();
        assert_eq!(error.field(), "boundary.disposition");
        assert_eq!(state, before);
    }

    #[test]
    fn boundary_serialization_moves_selected_state_into_its_owner_store() {
        let mut state = PlannerExecutionState::new(snapshot()).unwrap();
        let owner = SerializationOwner::PhysicalSlot {
            slot: PhysicalSlotId(1),
        };
        let policy = BoundaryPolicy {
            schema: BOUNDARY_POLICY_SCHEMA.into(),
            id: "boundary.title-return".into(),
            boundary: BoundaryKind::TitleReturn,
            default_disposition: BoundaryDisposition::Clear,
            component_rules: vec![ComponentBoundaryRule {
                selector: id_selector("save.main"),
                disposition: BoundaryDisposition::Serialize {
                    owner: owner.clone(),
                },
            }],
        };
        state
            .apply_boundary(
                "boundary.title-return",
                "snapshot.at-title",
                &policy,
                &BTreeMap::new(),
            )
            .unwrap();
        assert!(state.snapshot.environment.components.is_empty());
        assert_eq!(state.serialized_components[&owner][0].id, "save.main");
    }

    #[test]
    fn raw_writes_and_invalidation_change_only_selected_knownness_bits() {
        let mut state = PlannerExecutionState::new(snapshot()).unwrap();
        state
            .apply_operations(
                "transition.consume-key",
                "snapshot.after-key",
                &[
                    StateOperation::WriteRaw {
                        component_id: "raw.flags".into(),
                        byte_offset: 0,
                        mask: vec![0x30],
                        value: vec![0x30],
                    },
                    StateOperation::InvalidateRaw {
                        component_id: "raw.flags".into(),
                        byte_offset: 0,
                        mask: vec![0x10],
                    },
                    StateOperation::Write {
                        target: ComponentFieldTarget {
                            component_id: "save.main".into(),
                            field: "small_keys".into(),
                        },
                        value: StateValue::Unsigned(2),
                    },
                    StateOperation::Adjust {
                        target: ComponentFieldTarget {
                            component_id: "save.main".into(),
                            field: "small_keys".into(),
                        },
                        delta: -1,
                    },
                ],
            )
            .unwrap();
        assert_eq!(
            field(&state, "save.main", "small_keys"),
            &StateValue::Unsigned(1)
        );
        let raw = state
            .snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "raw.flags")
            .unwrap();
        assert_eq!(
            raw.payload,
            ComponentPayload::Raw {
                bytes: vec![0x30],
                known_mask: vec![0x20]
            }
        );
    }

    #[test]
    fn bound_raw_writes_follow_current_stage_and_fail_atomically_on_ambiguity() {
        let mut state = PlannerExecutionState::new(snapshot()).unwrap();
        state.snapshot.environment.location.stage = "D_MN05".into();
        let mut component = raw_component();
        component.id = "stage.raw-flags".into();
        component.component_kind = ComponentKind::StageMemory;
        component.binding = ComponentBinding::Stage {
            stage: "D_MN05".into(),
        };
        component.lifetime = SemanticLifetime::StageLoad;
        component.serialization_owner = SerializationOwner::StageBank {
            runtime_file_id: "file-0".into(),
            stage: "D_MN05".into(),
        };
        state.snapshot.environment.components.push(component);
        state
            .snapshot
            .environment
            .components
            .sort_by(|left, right| left.id.cmp(&right.id));
        state.validate().unwrap();

        state
            .apply_operations(
                "transition.write-stage-switch",
                "snapshot.stage-switch-written",
                &[
                    StateOperation::WriteBoundRaw {
                        component_kind: ComponentKind::StageMemory,
                        binding: ComponentBindingReference::CurrentStage,
                        byte_offset: 0,
                        mask: vec![0x30],
                        value: vec![0x20],
                    },
                    StateOperation::InvalidateBoundRaw {
                        component_kind: ComponentKind::StageMemory,
                        binding: ComponentBindingReference::CurrentStage,
                        byte_offset: 0,
                        mask: vec![0x10],
                    },
                ],
            )
            .unwrap();
        let component = state
            .snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "stage.raw-flags")
            .unwrap();
        assert_eq!(
            component.payload,
            ComponentPayload::Raw {
                bytes: vec![0x20],
                known_mask: vec![0x20],
            }
        );

        let mut duplicate = component.clone();
        duplicate.id = "stage.raw-flags.duplicate".into();
        state.snapshot.environment.components.push(duplicate);
        state
            .snapshot
            .environment
            .components
            .sort_by(|left, right| left.id.cmp(&right.id));
        let before = state.clone();
        assert!(
            state
                .apply_operations(
                    "transition.ambiguous-stage-switch",
                    "snapshot.not-produced",
                    &[StateOperation::WriteBoundRaw {
                        component_kind: ComponentKind::StageMemory,
                        binding: ComponentBindingReference::CurrentStage,
                        byte_offset: 0,
                        mask: vec![1],
                        value: vec![1],
                    }],
                )
                .is_err()
        );
        assert_eq!(state, before);
    }

    #[test]
    fn bound_raw_writes_follow_a_binding_projected_from_live_flow_state() {
        let mut snapshot = snapshot();
        snapshot.environment.components.extend([
            StateComponent {
                id: "message-session".into(),
                component_kind: ComponentKind::MessageFlow,
                payload: ComponentPayload::Structured {
                    fields: BTreeMap::from([
                        ("speaker_stage".into(), StateValue::Text("D_MN01".into())),
                        ("speaker_zone".into(), StateValue::Signed(7)),
                    ]),
                },
                binding: ComponentBinding::Global,
                lifetime: SemanticLifetime::Action,
                serialization_owner: SerializationOwner::None,
                provenance: provenance(),
            },
            StateComponent {
                id: "zone.raw".into(),
                component_kind: ComponentKind::ZoneMemory,
                payload: ComponentPayload::Raw {
                    bytes: vec![0],
                    known_mask: vec![0xff],
                },
                binding: ComponentBinding::Zone {
                    stage: "D_MN01".into(),
                    zone: 7,
                },
                lifetime: SemanticLifetime::RoomLoad,
                serialization_owner: SerializationOwner::None,
                provenance: provenance(),
            },
        ]);
        snapshot
            .environment
            .components
            .sort_by(|left, right| left.id.cmp(&right.id));
        let mut state = PlannerExecutionState::new(snapshot).unwrap();
        let operation = StateOperation::WriteBoundRaw {
            component_kind: ComponentKind::ZoneMemory,
            binding: ComponentBindingReference::Projected {
                component_id: "message-session".into(),
                projection: Box::new(ComponentBindingProjection::Zone {
                    stage_field: "speaker_stage".into(),
                    zone_field: "speaker_zone".into(),
                }),
            },
            byte_offset: 0,
            mask: vec![0x20],
            value: vec![0x20],
        };
        state
            .apply_operations(
                "transition.message-zone-write",
                "snapshot.message-zone-written",
                std::slice::from_ref(&operation),
            )
            .unwrap();
        let zone = state
            .snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "zone.raw")
            .unwrap();
        assert!(matches!(
            &zone.payload,
            ComponentPayload::Raw { bytes, .. } if bytes == &[0x20]
        ));

        let flow = state
            .snapshot
            .environment
            .components
            .iter_mut()
            .find(|component| component.id == "message-session")
            .unwrap();
        flow.payload = ComponentPayload::Unknown {
            expected_bytes: None,
        };
        let before = state.clone();
        assert!(
            state
                .apply_operations(
                    "transition.unresolved-message-zone-write",
                    "snapshot.not-produced",
                    &[operation],
                )
                .is_err()
        );
        assert_eq!(state, before);
    }

    #[test]
    fn bound_raw_unsigned_adjusts_only_one_known_stage_bank_atomically() {
        let mut state = PlannerExecutionState::new(snapshot()).unwrap();
        state.snapshot.environment.location.stage = "D_MN05".into();
        let mut bytes = vec![0_u8; 0x20];
        bytes[0x1c] = 2;
        state.snapshot.environment.components.push(StateComponent {
            id: "stage-memory.active".into(),
            component_kind: ComponentKind::DungeonMemory,
            payload: ComponentPayload::Raw {
                bytes,
                known_mask: vec![0xff; 0x20],
            },
            binding: ComponentBinding::Stage {
                stage: "D_MN05".into(),
            },
            lifetime: SemanticLifetime::StageLoad,
            serialization_owner: SerializationOwner::StageBank {
                runtime_file_id: "file-0".into(),
                stage: "D_MN05".into(),
            },
            provenance: provenance(),
        });
        state
            .snapshot
            .environment
            .components
            .sort_by(|left, right| left.id.cmp(&right.id));
        state.validate().unwrap();

        let adjust = |delta| StateOperation::AdjustBoundRawUnsigned {
            component_kind: ComponentKind::DungeonMemory,
            binding: ComponentBindingReference::CurrentStage,
            byte_offset: 0x1c,
            byte_width: 1,
            delta,
        };
        state
            .apply_operations("pickup.small-key", "snapshot.three-keys", &[adjust(1)])
            .unwrap();
        state
            .apply_operations("door.consume-key", "snapshot.two-keys", &[adjust(-1)])
            .unwrap();
        let stage_memory = state
            .snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "stage-memory.active")
            .unwrap();
        let ComponentPayload::Raw { bytes, .. } = &stage_memory.payload else {
            unreachable!()
        };
        assert_eq!(bytes[0x1c], 2);
        assert!(bytes[..0x1c].iter().all(|byte| *byte == 0));
        assert!(bytes[0x1d..].iter().all(|byte| *byte == 0));
        assert_eq!(
            state.execution_history.last().unwrap().event,
            ExecutionHistoryKind::Operation {
                operation: adjust(-1),
                affected_component_ids: vec!["stage-memory.active".into()],
            }
        );

        let before_failure = state.clone();
        let error = state
            .apply_operations(
                "door.wrong-bank",
                "snapshot.not-produced",
                &[StateOperation::AdjustBoundRawUnsigned {
                    component_kind: ComponentKind::DungeonMemory,
                    binding: ComponentBindingReference::Exact {
                        binding: ComponentBinding::Stage {
                            stage: "D_MN04".into(),
                        },
                    },
                    byte_offset: 0x1c,
                    byte_width: 1,
                    delta: -1,
                }],
            )
            .unwrap_err();
        assert_eq!(error.field(), "operation.adjust_bound_raw_unsigned");
        assert_eq!(state, before_failure);

        let before_underflow = state.clone();
        assert!(
            state
                .apply_operations(
                    "door.consume-too-many-keys",
                    "snapshot.no-underflow",
                    &[adjust(-3)],
                )
                .is_err()
        );
        assert_eq!(state, before_underflow);

        let mut duplicate = state
            .snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "stage-memory.active")
            .unwrap()
            .clone();
        duplicate.id = "stage-memory.ambiguous".into();
        state.snapshot.environment.components.push(duplicate);
        state
            .snapshot
            .environment
            .components
            .sort_by(|left, right| left.id.cmp(&right.id));
        let before_ambiguity = state.clone();
        assert!(
            state
                .apply_operations(
                    "door.ambiguous-key-bank",
                    "snapshot.no-ambiguous-write",
                    &[adjust(-1)],
                )
                .is_err()
        );
        assert_eq!(state, before_ambiguity);
        state
            .snapshot
            .environment
            .components
            .retain(|component| component.id != "stage-memory.ambiguous");

        let stage_memory = state
            .snapshot
            .environment
            .components
            .iter_mut()
            .find(|component| component.id == "stage-memory.active")
            .unwrap();
        let ComponentPayload::Raw { known_mask, .. } = &mut stage_memory.payload else {
            unreachable!()
        };
        known_mask[0x1c] = 0;
        let before_unknown = state.clone();
        assert!(
            state
                .apply_operations(
                    "door.unknown-key-count",
                    "snapshot.not-produced-either",
                    &[adjust(-1)],
                )
                .is_err()
        );
        assert_eq!(state, before_unknown);
    }

    #[test]
    fn serialization_clear_and_restore_keep_the_owner_store_independent() {
        let mut state = PlannerExecutionState::new(snapshot()).unwrap();
        let owner = SerializationOwner::PhysicalSlot {
            slot: PhysicalSlotId(1),
        };
        state
            .apply_operations(
                "transition.save-load",
                "snapshot.restored",
                &[
                    StateOperation::Serialize {
                        selector: id_selector("save.main"),
                        owner: owner.clone(),
                    },
                    StateOperation::ClearComponent {
                        selector: id_selector("save.main"),
                    },
                    StateOperation::Restore {
                        owner: owner.clone(),
                        destination_component_id: "save.main".into(),
                    },
                ],
            )
            .unwrap();
        assert_eq!(state.serialized_components[&owner].len(), 1);
        assert!(
            state
                .snapshot
                .environment
                .components
                .iter()
                .any(|component| component.id == "save.main")
        );
        assert_eq!(
            state.serialized_components[&owner][0].serialization_owner,
            owner
        );
    }

    #[test]
    fn serialized_store_keys_and_stage_bank_lifetimes_are_enforced() {
        let mut mismatched_owner = PlannerExecutionState::new(snapshot()).unwrap();
        mismatched_owner.serialized_components.insert(
            SerializationOwner::PhysicalSlot {
                slot: PhysicalSlotId(1),
            },
            vec![structured_component(
                "stored.save",
                ComponentKind::PersistentSave,
                ComponentBinding::RuntimeFile {
                    runtime_file_id: "file-0".into(),
                },
            )],
        );
        assert_eq!(
            mismatched_owner.validate().unwrap_err().field(),
            "serialized_components.owner"
        );

        let mut wrong_lifetime = PlannerExecutionState::new(snapshot()).unwrap();
        let owner = SerializationOwner::StageBank {
            runtime_file_id: "file-0".into(),
            stage: "F_SP103".into(),
        };
        let mut component = structured_component(
            "stage.stored",
            ComponentKind::StageMemory,
            ComponentBinding::Stage {
                stage: "F_SP103".into(),
            },
        );
        component.serialization_owner = owner.clone();
        wrong_lifetime
            .serialized_components
            .insert(owner, vec![component]);
        assert_eq!(
            wrong_lifetime.validate().unwrap_err().field(),
            "serialized_components.stage_bank"
        );
    }

    #[test]
    fn normal_stage_bank_commit_load_is_runtime_scoped_and_atomic() {
        let stage_component = |stage: &str, marker: u64| {
            let mut component = structured_component(
                "stage.live",
                ComponentKind::StageMemory,
                ComponentBinding::Stage {
                    stage: stage.into(),
                },
            );
            component.lifetime = SemanticLifetime::StageLoad;
            component.serialization_owner = SerializationOwner::StageBank {
                runtime_file_id: "file-0".into(),
                stage: stage.into(),
            };
            let ComponentPayload::Structured { fields } = &mut component.payload else {
                unreachable!()
            };
            fields.insert("marker".into(), StateValue::Unsigned(marker));
            component
        };
        let mut source = snapshot();
        source
            .environment
            .components
            .push(stage_component("F_SP103", 11));
        source
            .environment
            .components
            .sort_by(|left, right| left.id.cmp(&right.id));
        let mut state = PlannerExecutionState::new(source).unwrap();
        let destination_owner = SerializationOwner::StageBank {
            runtime_file_id: "file-0".into(),
            stage: "D_MN05".into(),
        };
        state.serialized_components.insert(
            destination_owner.clone(),
            vec![stage_component("D_MN05", 22)],
        );
        let other_file_owner = SerializationOwner::StageBank {
            runtime_file_id: "file-1".into(),
            stage: "D_MN05".into(),
        };
        let mut other_file = stage_component("D_MN05", 99);
        other_file.serialization_owner = other_file_owner.clone();
        state
            .serialized_components
            .insert(other_file_owner, vec![other_file]);
        state.validate().unwrap();

        state
            .apply_operations(
                "boundary.faron-to-forest",
                "snapshot.forest-bank-loaded",
                &[
                    StateOperation::CommitLoadStageBank {
                        component_id: "stage.live".into(),
                        runtime_file_id: "file-0".into(),
                        source_stage: "F_SP103".into(),
                        destination_stage: "D_MN05".into(),
                        source_binding: ComponentBinding::Stage {
                            stage: "F_SP103".into(),
                        },
                        destination_binding: ComponentBinding::Stage {
                            stage: "D_MN05".into(),
                        },
                    },
                    StateOperation::SetLocation {
                        location: SceneLocation {
                            stage: "D_MN05".into(),
                            room: 0,
                            layer: 0,
                            spawn: 0,
                        },
                    },
                ],
            )
            .unwrap();
        assert_eq!(
            field(&state, "stage.live", "marker"),
            &StateValue::Unsigned(22)
        );
        let source_owner = SerializationOwner::StageBank {
            runtime_file_id: "file-0".into(),
            stage: "F_SP103".into(),
        };
        let ComponentPayload::Structured { fields } =
            &state.serialized_components[&source_owner][0].payload
        else {
            unreachable!()
        };
        assert_eq!(fields["marker"], StateValue::Unsigned(11));
        let other_file_owner = SerializationOwner::StageBank {
            runtime_file_id: "file-1".into(),
            stage: "D_MN05".into(),
        };
        let ComponentPayload::Structured { fields } =
            &state.serialized_components[&other_file_owner][0].payload
        else {
            unreachable!()
        };
        assert_eq!(fields["marker"], StateValue::Unsigned(99));
        assert_eq!(state.snapshot.environment.location.stage, "D_MN05");
        assert_eq!(
            state
                .snapshot
                .environment
                .components
                .iter()
                .find(|component| component.id == "stage.live")
                .unwrap()
                .serialization_owner,
            destination_owner
        );

        let before = state.clone();
        let error = state
            .apply_operations(
                "boundary.wrong-file",
                "snapshot.not-produced",
                &[StateOperation::CommitLoadStageBank {
                    component_id: "stage.live".into(),
                    runtime_file_id: "file-1".into(),
                    source_stage: "D_MN05".into(),
                    destination_stage: "F_SP103".into(),
                    source_binding: ComponentBinding::Stage {
                        stage: "D_MN05".into(),
                    },
                    destination_binding: ComponentBinding::Stage {
                        stage: "F_SP103".into(),
                    },
                }],
            )
            .unwrap_err();
        assert_eq!(
            error.field(),
            "operation.commit_load_stage_bank.runtime_file_id"
        );
        assert_eq!(state, before);

        let before = state.clone();
        let error = state
            .apply_operations(
                "boundary.missing-destination",
                "snapshot.not-produced",
                &[StateOperation::CommitLoadStageBank {
                    component_id: "stage.live".into(),
                    runtime_file_id: "file-0".into(),
                    source_stage: "D_MN05".into(),
                    destination_stage: "D_MN06".into(),
                    source_binding: ComponentBinding::Stage {
                        stage: "D_MN05".into(),
                    },
                    destination_binding: ComponentBinding::Stage {
                        stage: "D_MN06".into(),
                    },
                }],
            )
            .unwrap_err();
        assert_eq!(
            error.field(),
            "operation.commit_load_stage_bank.destination"
        );
        assert_eq!(state, before);
    }

    #[test]
    fn file_zero_save_and_load_preserve_nested_stores_and_end_only_the_runtime_lifetime() {
        let stage_component = |stage: &str, marker: u64| {
            let mut component = structured_component(
                "stage.live",
                ComponentKind::StageMemory,
                ComponentBinding::Stage {
                    stage: stage.into(),
                },
            );
            component.lifetime = SemanticLifetime::StageLoad;
            component.serialization_owner = SerializationOwner::StageBank {
                runtime_file_id: "file-0".into(),
                stage: stage.into(),
            };
            let ComponentPayload::Structured { fields } = &mut component.payload else {
                unreachable!()
            };
            fields.insert("marker".into(), StateValue::Unsigned(marker));
            component
        };
        let mut source = snapshot();
        let save = source
            .environment
            .components
            .iter_mut()
            .find(|component| component.id == "save.main")
            .unwrap();
        let ComponentPayload::Structured { fields } = &mut save.payload else {
            unreachable!()
        };
        fields.insert("rupees".into(), StateValue::Unsigned(99));
        let mut raw_counter = raw_component();
        raw_counter.id = "save.raw-counter".into();
        raw_counter.binding = ComponentBinding::RuntimeFile {
            runtime_file_id: "file-0".into(),
        };
        let ComponentPayload::Raw { bytes, known_mask } = &mut raw_counter.payload else {
            unreachable!()
        };
        bytes[0] = 4;
        known_mask[0] = 0xff;
        source.environment.components.push(raw_counter);
        source
            .environment
            .components
            .push(stage_component("F_SP103", 11));
        let mut session = structured_component(
            "session.recent-item",
            ComponentKind::Session,
            ComponentBinding::Session {
                session_id: "process".into(),
            },
        );
        session.lifetime = SemanticLifetime::Session;
        session.serialization_owner = SerializationOwner::None;
        let ComponentPayload::Structured { fields } = &mut session.payload else {
            unreachable!()
        };
        fields.insert("item".into(), StateValue::Unsigned(0x4a));
        source.environment.components.push(session);
        let mut carried = structured_component(
            "runtime.bite-equipment",
            ComponentKind::Custom {
                id: "bite-equipment-transfer".into(),
            },
            ComponentBinding::RuntimeFile {
                runtime_file_id: "file-0".into(),
            },
        );
        let ComponentPayload::Structured { fields } = &mut carried.payload else {
            unreachable!()
        };
        fields.insert("equipped_item".into(), StateValue::Unsigned(0x28));
        source.environment.components.push(carried);
        source.environment.components.push(structured_component(
            "runtime.unselected-metadata",
            ComponentKind::Custom {
                id: "unselected-runtime-metadata".into(),
            },
            ComponentBinding::RuntimeFile {
                runtime_file_id: "file-0".into(),
            },
        ));
        source
            .environment
            .components
            .sort_by(|left, right| left.id.cmp(&right.id));
        let mut state = PlannerExecutionState::new(source).unwrap();
        for (stage, marker) in [("D_MN05", 22), ("F_SP103", 1)] {
            let owner = SerializationOwner::StageBank {
                runtime_file_id: "file-0".into(),
                stage: stage.into(),
            };
            let mut component = stage_component(stage, marker);
            component.serialization_owner = owner.clone();
            state.serialized_components.insert(owner, vec![component]);
        }
        state.validate().unwrap();

        let save_operation = StateOperation::SaveRuntimeToSlot {
            source_runtime_file_id: "file-0".into(),
            destination_slot: PhysicalSlotId(1),
            destination_persistent_file_id: "persistent-slot-1".into(),
            runtime_component_ids: vec!["save.main".into(), "save.raw-counter".into()],
            stage_bank_stages: vec!["D_MN05".into(), "F_SP103".into()],
        };
        state
            .apply_operations(
                "boundary.save-file-0",
                "snapshot.file-0-saved",
                &[save_operation],
            )
            .unwrap();
        assert_eq!(
            state.snapshot.environment.active_runtime_file.origin,
            RuntimeFileOrigin::TitleFile0
        );
        assert_eq!(state.snapshot.environment.physical_slots.len(), 1);
        let image = &state.persistent_file_images["persistent-slot-1"];
        assert_eq!(image.stage_banks.len(), 2);
        let saved_faron = image
            .stage_banks
            .iter()
            .find(|store| {
                matches!(
                    &store.owner,
                    SerializationOwner::StageBank { stage, .. } if stage == "F_SP103"
                )
            })
            .unwrap();
        let ComponentPayload::Structured { fields } = &saved_faron.components[0].payload else {
            unreachable!()
        };
        assert_eq!(fields["marker"], StateValue::Unsigned(11));
        assert_eq!(
            image.runtime_components[0]
                .provenance
                .last()
                .unwrap()
                .source_kind,
            ProvenanceSourceKind::SaveRestore
        );

        let before_failed_load = state.clone();
        let error = state
            .apply_operations(
                "boundary.incomplete-load",
                "snapshot.not-produced",
                &[StateOperation::LoadRuntimeFromSlot {
                    source_runtime_file_id: "file-0".into(),
                    source_slot: PhysicalSlotId(1),
                    source_persistent_file_id: "persistent-slot-1".into(),
                    destination_runtime_file_id: "slot-1-runtime".into(),
                    destination_allowed_serialization_targets: vec![PhysicalSlotId(1)],
                    runtime_component_ids: vec!["save.main".into(), "save.raw-counter".into()],
                    stage_bank_stages: vec!["F_SP103".into()],
                    carried_runtime_component_ids: Vec::new(),
                }],
            )
            .unwrap_err();
        assert_eq!(
            error.field(),
            "operation.load_runtime_from_slot.stage_bank_stages"
        );
        assert_eq!(state, before_failed_load);

        let before_failed_carry = state.clone();
        let error = state
            .apply_operations(
                "boundary.invalid-runtime-carry",
                "snapshot.not-produced",
                &[StateOperation::LoadRuntimeFromSlot {
                    source_runtime_file_id: "file-0".into(),
                    source_slot: PhysicalSlotId(1),
                    source_persistent_file_id: "persistent-slot-1".into(),
                    destination_runtime_file_id: "slot-1-runtime".into(),
                    destination_allowed_serialization_targets: vec![PhysicalSlotId(1)],
                    runtime_component_ids: vec!["save.main".into(), "save.raw-counter".into()],
                    stage_bank_stages: vec!["D_MN05".into(), "F_SP103".into()],
                    carried_runtime_component_ids: vec!["session.recent-item".into()],
                }],
            )
            .unwrap_err();
        assert_eq!(
            error.field(),
            "operation.load_runtime_from_slot.carried_runtime_component_ids"
        );
        assert_eq!(state, before_failed_carry);

        let mut dynamic_load = state.clone();
        let sealed_image_before = dynamic_load.persistent_file_images["persistent-slot-1"].clone();
        dynamic_load
            .apply_operations(
                "boundary.dynamic-load-slot-1",
                "snapshot.dynamic-slot-1-loaded",
                &[StateOperation::LoadActiveRuntimeFromSlot {
                    source_slot: PhysicalSlotId(1),
                    destination_id_suffix: "file-select-slot-1".into(),
                    destination_allowed_serialization_targets: vec![
                        PhysicalSlotId(1),
                        PhysicalSlotId(2),
                        PhysicalSlotId(3),
                    ],
                    carried_runtime_component_ids: vec!["runtime.bite-equipment".into()],
                }],
            )
            .unwrap();
        assert_eq!(
            dynamic_load.snapshot.environment.active_runtime_file.id,
            "file-0.file-select-slot-1"
        );
        assert_eq!(
            dynamic_load.snapshot.environment.active_runtime_file.origin,
            RuntimeFileOrigin::LoadedSlot {
                slot: PhysicalSlotId(1)
            }
        );
        assert_eq!(
            field(&dynamic_load, "save.main", "rupees"),
            &StateValue::Unsigned(99)
        );
        assert_eq!(
            field(&dynamic_load, "runtime.bite-equipment", "equipped_item"),
            &StateValue::Unsigned(0x28)
        );
        assert!(
            dynamic_load
                .snapshot
                .environment
                .components
                .iter()
                .all(|component| component.id != "runtime.unselected-metadata")
        );
        assert_eq!(
            dynamic_load.persistent_file_images["persistent-slot-1"],
            sealed_image_before
        );

        state
            .apply_operations(
                "boundary.load-slot-1",
                "snapshot.slot-1-loaded",
                &[
                    StateOperation::LoadRuntimeFromSlot {
                        source_runtime_file_id: "file-0".into(),
                        source_slot: PhysicalSlotId(1),
                        source_persistent_file_id: "persistent-slot-1".into(),
                        destination_runtime_file_id: "slot-1-runtime".into(),
                        destination_allowed_serialization_targets: vec![PhysicalSlotId(1)],
                        runtime_component_ids: vec!["save.main".into(), "save.raw-counter".into()],
                        stage_bank_stages: vec!["D_MN05".into(), "F_SP103".into()],
                        carried_runtime_component_ids: vec!["runtime.bite-equipment".into()],
                    },
                    StateOperation::ActivateStageBank {
                        component_id: "stage.live".into(),
                        runtime_file_id: "slot-1-runtime".into(),
                        stage: "F_SP103".into(),
                        binding: ComponentBinding::Stage {
                            stage: "F_SP103".into(),
                        },
                    },
                    StateOperation::SetLocation {
                        location: SceneLocation {
                            stage: "F_SP103".into(),
                            room: 0,
                            layer: 0,
                            spawn: 0,
                        },
                    },
                ],
            )
            .unwrap();
        assert_eq!(
            state.snapshot.environment.active_runtime_file,
            RuntimeFile {
                id: "slot-1-runtime".into(),
                origin: RuntimeFileOrigin::LoadedSlot {
                    slot: PhysicalSlotId(1)
                },
                backing: BackingAttachment::CardBacked {
                    slot: PhysicalSlotId(1)
                },
                allowed_serialization_targets: vec![PhysicalSlotId(1)],
                lifecycle: RuntimeFileLifecycle::Active,
            }
        );
        assert_eq!(state.snapshot.environment.inactive_runtime_files.len(), 1);
        assert_eq!(
            state.snapshot.environment.inactive_runtime_files[0].lifecycle,
            RuntimeFileLifecycle::Ended
        );
        assert_eq!(
            field(&state, "save.main", "rupees"),
            &StateValue::Unsigned(99)
        );
        assert_eq!(
            field(&state, "stage.live", "marker"),
            &StateValue::Unsigned(11)
        );
        assert_eq!(
            field(&state, "session.recent-item", "item"),
            &StateValue::Unsigned(0x4a)
        );
        assert_eq!(
            field(&state, "runtime.bite-equipment", "equipped_item"),
            &StateValue::Unsigned(0x28)
        );
        let carried = state
            .snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "runtime.bite-equipment")
            .unwrap();
        assert_eq!(
            carried.binding,
            ComponentBinding::RuntimeFile {
                runtime_file_id: "slot-1-runtime".into(),
            }
        );
        assert_eq!(
            carried.serialization_owner,
            SerializationOwner::RuntimeFile {
                runtime_file_id: "slot-1-runtime".into(),
            }
        );
        assert_eq!(
            carried.provenance.last().unwrap().transition_id.as_deref(),
            Some("boundary.load-slot-1")
        );
        assert!(
            state
                .snapshot
                .environment
                .components
                .iter()
                .all(|component| component.id != "runtime.unselected-metadata")
        );
        assert_eq!(
            state
                .snapshot
                .environment
                .components
                .iter()
                .find(|component| component.id == "save.raw-counter")
                .unwrap()
                .binding,
            ComponentBinding::RuntimeFile {
                runtime_file_id: "slot-1-runtime".into(),
            }
        );
        state
            .apply_operations(
                "transition.increment-loaded-counter",
                "snapshot.loaded-counter-incremented",
                &[StateOperation::AdjustBoundRawUnsigned {
                    component_kind: ComponentKind::PersistentSave,
                    binding: ComponentBindingReference::ActiveRuntimeFile,
                    byte_offset: 0,
                    byte_width: 1,
                    delta: 1,
                }],
            )
            .unwrap();
        let loaded_counter = state
            .snapshot
            .environment
            .components
            .iter()
            .find(|component| component.id == "save.raw-counter")
            .unwrap();
        assert_eq!(
            loaded_counter.binding,
            ComponentBinding::RuntimeFile {
                runtime_file_id: "slot-1-runtime".into(),
            }
        );
        let ComponentPayload::Raw { bytes, .. } = &loaded_counter.payload else {
            unreachable!()
        };
        assert_eq!(bytes[0], 5);
        assert!(
            state
                .serialized_components
                .contains_key(&SerializationOwner::StageBank {
                    runtime_file_id: "slot-1-runtime".into(),
                    stage: "D_MN05".into(),
                })
        );
        assert!(
            !state
                .serialized_components
                .keys()
                .any(|owner| { owner_belongs_to_runtime(owner, "file-0") })
        );

        let sealed_slot_digest =
            state.snapshot.environment.physical_slots[0].serialized_state_sha256;
        state
            .apply_operations(
                "transition.spend-rupees",
                "snapshot.runtime-diverged-from-slot",
                &[StateOperation::Write {
                    target: ComponentFieldTarget {
                        component_id: "save.main".into(),
                        field: "rupees".into(),
                    },
                    value: StateValue::Unsigned(1),
                }],
            )
            .unwrap();
        assert_eq!(
            field(&state, "save.main", "rupees"),
            &StateValue::Unsigned(1)
        );
        let ComponentPayload::Structured { fields } =
            &state.persistent_file_images["persistent-slot-1"].runtime_components[0].payload
        else {
            unreachable!()
        };
        assert_eq!(fields["rupees"], StateValue::Unsigned(99));
        assert_eq!(
            state.snapshot.environment.physical_slots[0].serialized_state_sha256,
            sealed_slot_digest
        );

        let document = state.to_document().unwrap();
        let decoded =
            PlannerExecutionStateDocument::decode_canonical(&document.canonical_bytes().unwrap())
                .unwrap()
                .into_state()
                .unwrap();
        assert_eq!(decoded, state);
        let semantic_with_history = state.semantic_digest().unwrap();
        let mut without_ended_history = state.clone();
        without_ended_history
            .snapshot
            .environment
            .inactive_runtime_files
            .clear();
        assert_eq!(
            semantic_with_history,
            without_ended_history.semantic_digest().unwrap()
        );
    }

    #[test]
    fn copy_move_rebind_and_projection_transform_only_named_components() {
        let mut state = PlannerExecutionState::new(snapshot()).unwrap();
        state
            .apply_operations(
                "technique.component-transfer",
                "snapshot.transferred",
                &[
                    StateOperation::Copy {
                        source: id_selector("save.main"),
                        destination_component_id: "save.copy".into(),
                        binding: ComponentBinding::Unbound,
                        serialization_owner: SerializationOwner::None,
                    },
                    StateOperation::Bind {
                        selector: id_selector("save.copy"),
                        binding: ComponentBinding::Dungeon {
                            dungeon: "forest".into(),
                        },
                    },
                    StateOperation::Move {
                        source: id_selector("save.copy"),
                        destination_component_id: "forest.memory".into(),
                        binding: ComponentBinding::Stage {
                            stage: "D_MN05".into(),
                        },
                        serialization_owner: SerializationOwner::StageBank {
                            runtime_file_id: "file-0".into(),
                            stage: "D_MN05".into(),
                        },
                    },
                    StateOperation::Rebind {
                        selector: id_selector("forest.memory"),
                        binding: ComponentBinding::Stage {
                            stage: "D_MN06".into(),
                        },
                    },
                    StateOperation::Project {
                        source_runtime_file_id: "file-0".into(),
                        destination_runtime_file_id: "file-1".into(),
                        component_ids: vec!["save.main".into()],
                    },
                ],
            )
            .unwrap();

        let components = &state.snapshot.environment.components;
        assert!(
            !components
                .iter()
                .any(|component| component.id == "save.copy")
        );
        assert_eq!(
            components
                .iter()
                .find(|component| component.id == "forest.memory")
                .unwrap()
                .binding,
            ComponentBinding::Stage {
                stage: "D_MN06".into()
            }
        );
        let projected = components
            .iter()
            .find(|component| component.id == "save.main")
            .unwrap();
        assert_eq!(
            projected.binding,
            ComponentBinding::RuntimeFile {
                runtime_file_id: "file-1".into()
            }
        );
        assert_eq!(
            projected.serialization_owner,
            SerializationOwner::RuntimeFile {
                runtime_file_id: "file-1".into()
            }
        );
    }

    #[test]
    fn message_and_pending_operation_state_is_not_collapsed_to_a_boolean() {
        let mut state = PlannerExecutionState::new(snapshot()).unwrap();
        state
            .apply_operations(
                "technique.dialogue-interrupt",
                "snapshot.dialogue-interrupted",
                &[
                    StateOperation::ScheduleCleanup {
                        cleanup_id: "cleanup.item-handoff".into(),
                    },
                    StateOperation::BranchFlow {
                        flow_component_id: "flow.main".into(),
                        edge_id: "edge.reward".into(),
                        destination_node_id: "node.reward".into(),
                    },
                    StateOperation::Interrupt {
                        action_id: "action.sidehop".into(),
                        window: TemporalWindow {
                            earliest_frame: 14,
                            latest_frame: 14,
                            required_input: Some("input.sidehop".into()),
                        },
                    },
                    StateOperation::CancelCleanup {
                        cleanup_id: "cleanup.item-handoff".into(),
                    },
                    StateOperation::Consume {
                        pending_operation_id: "pending.item".into(),
                    },
                ],
            )
            .unwrap();
        assert_eq!(
            field(&state, "flow.main", "node_id"),
            &StateValue::Text("node.reward".into())
        );
        assert_eq!(
            field(&state, "flow.main", "last_edge_id"),
            &StateValue::Text("edge.reward".into())
        );
        assert!(state.scheduled_cleanup_ids.is_empty());
        assert_eq!(state.interruption_log[0].window.earliest_frame, 14);
        assert!(
            !state
                .snapshot
                .environment
                .components
                .iter()
                .any(|component| component.id == "pending.item")
        );
    }
}
