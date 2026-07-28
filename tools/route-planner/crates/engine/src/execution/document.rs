//! Convert execution documents and validate serialized store invariants.

use super::*;

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

pub(super) fn selector_matches(selector: &ComponentSelector, component: &StateComponent) -> bool {
    match selector {
        ComponentSelector::Id { component_id } => component.id == *component_id,
        ComponentSelector::Kind { component_kind } => component.component_kind == *component_kind,
        ComponentSelector::Binding { binding } => component.binding == *binding,
    }
}

pub(super) fn component_belongs_to_runtime(
    component: &StateComponent,
    runtime_file_id: &str,
) -> bool {
    component.binding
        == (ComponentBinding::RuntimeFile {
            runtime_file_id: runtime_file_id.into(),
        })
        || owner_belongs_to_runtime(&component.serialization_owner, runtime_file_id)
}

pub(super) fn owner_belongs_to_runtime(owner: &SerializationOwner, runtime_file_id: &str) -> bool {
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

pub(super) fn history_event_writes_field(
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
            | StateOperation::DebitUnsigned { target, .. }
            | StateOperation::ClearField { target }
            | StateOperation::InvalidateField { target } => {
                target.component_id == component_id && target.field == field
            }
            StateOperation::ClampUnsignedMinimum { target, .. } => {
                target.component_id == component_id
                    && target.field == field
                    && affected_component_ids
                        .binary_search_by(|id| id.as_str().cmp(component_id))
                        .is_ok()
            }
            StateOperation::NormalizeItemSlotsAndLineup {
                component_id: changed,
                inventory_field,
                lineup_field,
                ..
            } => changed == component_id && (field == inventory_field || field == lineup_field),
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
            | StateOperation::WriteBytesField { .. }
            | StateOperation::WriteBoundRaw { .. }
            | StateOperation::InvalidateRaw { .. }
            | StateOperation::InvalidateBoundRaw { .. }
            | StateOperation::AdjustBoundRawUnsigned { .. }
            | StateOperation::Preserve { .. }
            | StateOperation::Serialize { .. }
            | StateOperation::ReplaceCustomStore { .. }
            | StateOperation::SaveRuntimeToSlot { .. }
            | StateOperation::SaveActiveRuntimeToSlot { .. }
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
            | StateOperation::ReconstructActor { .. }
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

pub(super) fn checked_raw_range(
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

pub(super) fn mark_transition(component: &mut StateComponent, application_id: &str) {
    component.provenance.push(ComponentProvenance {
        source_kind: ProvenanceSourceKind::Transition,
        source_id: application_id.into(),
        source_sha256: None,
        transition_id: Some(application_id.into()),
    });
}

pub(super) fn invalidate_payload(component: &mut StateComponent) {
    let expected_bytes = match &component.payload {
        ComponentPayload::Raw { bytes, .. } => Some(bytes.len() as u32),
        ComponentPayload::Structured { .. } => None,
        ComponentPayload::Unknown { expected_bytes } => *expected_bytes,
    };
    component.payload = ComponentPayload::Unknown { expected_bytes };
}

pub(super) fn mark_save_restore(component: &mut StateComponent, application_id: &str) {
    component.provenance.push(ComponentProvenance {
        source_kind: ProvenanceSourceKind::SaveRestore,
        source_id: application_id.into(),
        source_sha256: None,
        transition_id: Some(application_id.into()),
    });
}

pub(super) fn normalize_provenance(component: &mut StateComponent) {
    component.provenance = vec![ComponentProvenance {
        source_kind: ProvenanceSourceKind::Initialized,
        source_id: "search.identity".into(),
        source_sha256: None,
        transition_id: None,
    }];
}

pub(super) fn no_selector_match(field: &str) -> PlannerContractError {
    PlannerContractError::new(field, "selector did not match any component")
}

pub(super) fn validate_component_store(
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

pub(super) fn validate_persistent_image_binding(
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

pub(super) fn rekey_component_runtime(
    component: &mut StateComponent,
    source: &str,
    destination: &str,
) {
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

pub(super) fn rekey_serialization_owner_runtime(
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

pub(super) fn insert_serialized(
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

pub(super) fn select_serialized<'a>(
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

pub(super) fn adjust_value(value: &mut StateValue, delta: i64) -> Result<(), PlannerContractError> {
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
