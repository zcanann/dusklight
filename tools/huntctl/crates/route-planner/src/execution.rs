//! Atomic execution of typed planner operations over explicit backing stores.

use crate::artifact::Digest;
use crate::snapshot::StateSnapshot;
use crate::state::{
    ComponentBinding, ComponentKind, ComponentPayload, ComponentProvenance, ComponentSelector,
    ProvenanceSourceKind, SerializationOwner, StateComponent, StateValue,
    validate_serialization_owner,
};
use crate::transition::{StateOperation, TemporalWindow};
use crate::{PlannerContractError, validate_stable_id};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterruptionRecord {
    pub action_id: String,
    pub window: TemporalWindow,
}

/// Mutable search state that keeps non-save backing stores separate from the
/// visible execution snapshot. Applying a batch is transactional: a failed
/// operation leaves every store and the snapshot unchanged.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannerExecutionState {
    pub snapshot: StateSnapshot,
    pub gate_states: BTreeMap<String, bool>,
    pub serialized_components: BTreeMap<SerializationOwner, Vec<StateComponent>>,
    pub preserved_component_ids: BTreeSet<String>,
    pub scheduled_cleanup_ids: BTreeSet<String>,
    pub interruption_log: Vec<InterruptionRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationApplication {
    pub source_snapshot_sha256: Digest,
    pub result_snapshot_sha256: Digest,
    pub operation_count: usize,
}

impl PlannerExecutionState {
    pub fn new(snapshot: StateSnapshot) -> Result<Self, PlannerContractError> {
        let state = Self {
            snapshot,
            gate_states: BTreeMap::new(),
            serialized_components: BTreeMap::new(),
            preserved_component_ids: BTreeSet::new(),
            scheduled_cleanup_ids: BTreeSet::new(),
            interruption_log: Vec::new(),
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
            validate_serialization_owner(owner)?;
            if *owner == SerializationOwner::None {
                return Err(PlannerContractError::new(
                    "serialized_components.owner",
                    "cannot use the none owner as a backing store",
                ));
            }
            if components.is_empty() {
                return Err(PlannerContractError::new(
                    "serialized_components",
                    "cannot contain an empty owner store",
                ));
            }
            let mut previous = None;
            for component in components {
                component.validate()?;
                if previous.is_some_and(|id: &str| id >= component.id.as_str()) {
                    return Err(PlannerContractError::new(
                        "serialized_components",
                        "components must be unique and sorted by ID within each owner",
                    ));
                }
                previous = Some(component.id.as_str());
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
        Ok(())
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
        for operation in operations {
            operation.validate()?;
            next.apply_operation(application_id, operation)?;
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
            StateOperation::SetLocation { location } => {
                self.snapshot.environment.location = location.clone();
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

    fn component_mut(&mut self, id: &str) -> Result<&mut StateComponent, PlannerContractError> {
        let index = self.component_index(id)?;
        Ok(&mut self.snapshot.environment.components[index])
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
}

fn selector_matches(selector: &ComponentSelector, component: &StateComponent) -> bool {
    match selector {
        ComponentSelector::Id { component_id } => component.id == *component_id,
        ComponentSelector::Kind { component_kind } => component.component_kind == *component_kind,
        ComponentSelector::Binding { binding } => component.binding == *binding,
    }
}

fn mark_transition(component: &mut StateComponent, application_id: &str) {
    component.provenance.push(ComponentProvenance {
        source_kind: ProvenanceSourceKind::Transition,
        source_id: application_id.into(),
        source_sha256: None,
        transition_id: Some(application_id.into()),
    });
}

fn no_selector_match(field: &str) -> PlannerContractError {
    PlannerContractError::new(field, "selector did not match any component")
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
        BackingAttachment, EXECUTION_ENVIRONMENT_SCHEMA, ExecutionEnvironment, PhysicalSlotId,
        PlayerForm, PlayerState, RuntimeFile, RuntimeFileLifecycle, RuntimeFileOrigin,
        SceneLocation, SemanticLifetime,
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
    fn raw_writes_establish_only_selected_bits_and_counters_adjust_relatively() {
        let mut state = PlannerExecutionState::new(snapshot()).unwrap();
        state
            .apply_operations(
                "transition.consume-key",
                "snapshot.after-key",
                &[
                    StateOperation::WriteRaw {
                        component_id: "raw.flags".into(),
                        byte_offset: 0,
                        mask: vec![0x20],
                        value: vec![0x20],
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
                bytes: vec![0x20],
                known_mask: vec![0x20]
            }
        );
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
