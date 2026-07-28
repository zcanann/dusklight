//! Validate, identify, and transactionally advance execution state.

use super::*;

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

    pub(super) fn push_history(
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

    pub(super) fn affected_save_component_ids(
        &self,
        source_runtime_file_id: &str,
        runtime_component_ids: &[String],
        selected_stage_banks: Option<&[String]>,
    ) -> Vec<String> {
        let stage_is_selected = |stage: &str| {
            selected_stage_banks.is_none_or(|stages| {
                stages
                    .binary_search_by(|candidate| candidate.as_str().cmp(stage))
                    .is_ok()
            })
        };
        let mut ids = runtime_component_ids.to_vec();
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
                                && stage_is_selected(stage)
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
                            if runtime_file_id == source_runtime_file_id && stage_is_selected(stage)
                    );
                    components
                        .iter()
                        .filter(move |_| selected)
                        .map(|component| component.id.clone())
                }),
        );
        ids
    }
}
