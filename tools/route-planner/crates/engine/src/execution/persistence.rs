//! Save, load, and rebind runtime and stage-bank component stores.

use super::*;

impl PlannerExecutionState {
    pub(super) fn component_index(&self, id: &str) -> Result<usize, PlannerContractError> {
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

    pub(super) fn commit_load_stage_bank(
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

    pub(super) fn activate_stage_bank(
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

    pub(super) fn save_runtime_to_slot(
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

    pub(super) fn save_active_runtime_to_slot(
        &mut self,
        application_id: &str,
        operation: &StateOperation,
    ) -> Result<(), PlannerContractError> {
        let StateOperation::SaveActiveRuntimeToSlot {
            destination_slot,
            destination_id_suffix,
            runtime_component_ids,
            projection_operations,
        } = operation
        else {
            unreachable!("active-runtime save helper is called only for its operation variant")
        };
        let source_runtime_file_id = self.snapshot.environment.active_runtime_file.id.clone();
        let destination_persistent_file_id =
            format!("{source_runtime_file_id}.{destination_id_suffix}");
        crate::validate_stable_id(
            "operation.save_active_runtime_to_slot.destination_persistent_file_id",
            &destination_persistent_file_id,
        )?;
        let stage_bank_stages = self
            .snapshot
            .environment
            .components
            .iter()
            .map(|component| &component.serialization_owner)
            .chain(self.serialized_components.keys())
            .filter_map(|owner| match owner {
                SerializationOwner::StageBank {
                    runtime_file_id,
                    stage,
                } if runtime_file_id == &source_runtime_file_id => Some(stage.clone()),
                _ => None,
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut projected = self.clone();
        for projection_operation in projection_operations {
            projected
                .apply_operation(application_id, &projection_operation.to_state_operation())?;
        }
        projected.save_runtime_to_slot(
            application_id,
            &StateOperation::SaveRuntimeToSlot {
                source_runtime_file_id,
                destination_slot: *destination_slot,
                destination_persistent_file_id,
                runtime_component_ids: runtime_component_ids.clone(),
                stage_bank_stages,
            },
        )?;
        self.snapshot.environment.physical_slots = projected.snapshot.environment.physical_slots;
        self.persistent_file_images = projected.persistent_file_images;
        Ok(())
    }

    pub(super) fn load_runtime_from_slot(
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

    pub(super) fn load_active_runtime_from_slot(
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

    pub(super) fn begin_runtime_file_lifetime(
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
}
