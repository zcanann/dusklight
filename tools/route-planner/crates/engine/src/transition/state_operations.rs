use super::*;

impl StateOperation {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        match self {
            Self::Write { target, value } => {
                validate_field_target(target)?;
                validate_state_value(value)
            }
            Self::WriteFields {
                component_id,
                fields,
            } => {
                validate_stable_id("operation.component_id", component_id)?;
                if fields.is_empty() || fields.len() > 256 {
                    return Err(PlannerContractError::new(
                        "operation.write_fields",
                        "must contain between 1 and 256 fields",
                    ));
                }
                for (field, value) in fields {
                    validate_stable_id("operation.write_fields.field", field)?;
                    validate_state_value(value)?;
                }
                Ok(())
            }
            Self::ReplacePayload {
                component_id,
                payload,
            } => {
                validate_stable_id("operation.component_id", component_id)?;
                payload.validate()
            }
            Self::InvalidatePayloads { selector, .. } => validate_component_selector(selector),
            Self::InvalidateActiveRuntimeSerializedPayloads { selector } => {
                validate_component_selector(selector)
            }
            Self::CopyValue { source, target } | Self::SetBitFromValue { source, target } => {
                validate_field_target(source)?;
                validate_field_target(target)?;
                if source == target {
                    return Err(PlannerContractError::new(
                        "operation.target",
                        "must differ from the source field",
                    ));
                }
                Ok(())
            }
            Self::WriteRaw {
                component_id,
                byte_offset: _,
                mask,
                value,
            } => {
                validate_stable_id("operation.component_id", component_id)?;
                if mask.is_empty()
                    || mask.len() != value.len()
                    || mask.len() > crate::state::MAX_COMPONENT_BYTES
                {
                    return Err(PlannerContractError::new(
                        "operation.write_raw",
                        "mask/value must have equal nonzero bounded lengths",
                    ));
                }
                if mask.iter().all(|byte| *byte == 0) {
                    return Err(PlannerContractError::new(
                        "operation.write_raw.mask",
                        "must select at least one bit",
                    ));
                }
                Ok(())
            }
            Self::WriteBytesField {
                target,
                byte_offset: _,
                mask,
                value,
            } => {
                validate_field_target(target)?;
                validate_raw_write(mask, value, "operation.write_bytes_field")
            }
            Self::WriteBoundRaw {
                component_kind,
                binding,
                mask,
                value,
                ..
            } => {
                validate_bound_raw_target(component_kind, binding)?;
                validate_raw_write(mask, value, "operation.write_bound_raw")
            }
            Self::InvalidateRaw {
                component_id,
                byte_offset: _,
                mask,
            } => {
                validate_stable_id("operation.component_id", component_id)?;
                if mask.is_empty() || mask.len() > crate::state::MAX_COMPONENT_BYTES {
                    return Err(PlannerContractError::new(
                        "operation.invalidate_raw.mask",
                        "must have a nonzero bounded length",
                    ));
                }
                if mask.iter().all(|byte| *byte == 0) {
                    return Err(PlannerContractError::new(
                        "operation.invalidate_raw.mask",
                        "must select at least one bit",
                    ));
                }
                Ok(())
            }
            Self::InvalidateBoundRaw {
                component_kind,
                binding,
                mask,
                ..
            } => {
                validate_bound_raw_target(component_kind, binding)?;
                validate_raw_mask(mask, "operation.invalidate_bound_raw.mask")
            }
            Self::Adjust { target, delta } => {
                validate_field_target(target)?;
                if *delta == 0 {
                    return Err(PlannerContractError::new(
                        "operation.adjust.delta",
                        "must be nonzero",
                    ));
                }
                Ok(())
            }
            Self::DebitUnsigned { target, amount } => {
                validate_field_target(target)?;
                if *amount == 0 {
                    return Err(PlannerContractError::new(
                        "operation.debit_unsigned.amount",
                        "must be nonzero",
                    ));
                }
                Ok(())
            }
            Self::ClampUnsignedMinimum { target, minimum } => {
                validate_field_target(target)?;
                if *minimum == 0 {
                    return Err(PlannerContractError::new(
                        "operation.clamp_unsigned_minimum.minimum",
                        "must be nonzero",
                    ));
                }
                Ok(())
            }
            Self::NormalizeItemSlotsAndLineup {
                component_id,
                inventory_field,
                lineup_field,
                primary_slot,
                secondary_slot,
                single_item,
                combined_item,
                empty_item,
                lineup_order,
            } => {
                validate_stable_id("operation.component_id", component_id)?;
                validate_stable_id("operation.inventory_field", inventory_field)?;
                validate_stable_id("operation.lineup_field", lineup_field)?;
                if inventory_field == lineup_field {
                    return Err(PlannerContractError::new(
                        "operation.normalize_item_slots_and_lineup.lineup_field",
                        "must differ from the inventory field",
                    ));
                }
                if primary_slot == secondary_slot {
                    return Err(PlannerContractError::new(
                        "operation.normalize_item_slots_and_lineup.secondary_slot",
                        "must differ from the primary slot",
                    ));
                }
                if single_item == combined_item
                    || single_item == empty_item
                    || combined_item == empty_item
                {
                    return Err(PlannerContractError::new(
                        "operation.normalize_item_slots_and_lineup.items",
                        "single, combined, and empty item values must be distinct",
                    ));
                }
                if lineup_order.is_empty() {
                    return Err(PlannerContractError::new(
                        "operation.normalize_item_slots_and_lineup.lineup_order",
                        "must not be empty",
                    ));
                }
                let unique = lineup_order.iter().copied().collect::<BTreeSet<_>>();
                if unique.len() != lineup_order.len() {
                    return Err(PlannerContractError::new(
                        "operation.normalize_item_slots_and_lineup.lineup_order",
                        "must contain unique slot indices",
                    ));
                }
                Ok(())
            }
            Self::AdjustBoundRawUnsigned {
                component_kind,
                binding,
                byte_width,
                delta,
                ..
            } => {
                validate_component_kind(component_kind)?;
                validate_binding_reference(binding)?;
                if matches!(
                    binding,
                    ComponentBindingReference::Exact {
                        binding: ComponentBinding::Unbound
                    }
                ) {
                    return Err(PlannerContractError::new(
                        "operation.adjust_bound_raw_unsigned.binding",
                        "must identify an explicit backing-store binding",
                    ));
                }
                if !(1..=8).contains(byte_width) {
                    return Err(PlannerContractError::new(
                        "operation.adjust_bound_raw_unsigned.byte_width",
                        "must be between 1 and 8",
                    ));
                }
                if *delta == 0 {
                    return Err(PlannerContractError::new(
                        "operation.adjust_bound_raw_unsigned.delta",
                        "must be nonzero",
                    ));
                }
                Ok(())
            }
            Self::ClearComponent { selector } | Self::Preserve { selector } => {
                validate_component_selector(selector)
            }
            Self::ClearField { target } | Self::InvalidateField { target } => {
                validate_field_target(target)
            }
            Self::Initialize { component } => component.validate(),
            Self::Copy {
                source,
                destination_component_id,
                binding,
                serialization_owner,
            }
            | Self::Move {
                source,
                destination_component_id,
                binding,
                serialization_owner,
            } => {
                validate_component_selector(source)?;
                validate_stable_id(
                    "operation.destination_component_id",
                    destination_component_id,
                )?;
                validate_binding(binding)?;
                validate_owner(serialization_owner)
            }
            Self::Serialize { selector, owner } => {
                validate_component_selector(selector)?;
                validate_owner(owner)
            }
            Self::Restore {
                owner,
                destination_component_id,
            } => {
                validate_owner(owner)?;
                validate_stable_id(
                    "operation.destination_component_id",
                    destination_component_id,
                )
            }
            Self::ReplaceCustomStore { owner, components } => {
                validate_custom_store_owner("operation.replace_custom_store.owner", owner)?;
                if components.is_empty() || components.len() > 4_096 {
                    return Err(PlannerContractError::new(
                        "operation.replace_custom_store.components",
                        "must contain between 1 and 4096 components",
                    ));
                }
                let mut previous_id = None;
                for component in components {
                    component.validate()?;
                    if component.serialization_owner != *owner {
                        return Err(PlannerContractError::new(
                            "operation.replace_custom_store.components",
                            "every component must name the custom store as its serialization owner",
                        ));
                    }
                    if previous_id.is_some_and(|previous: &str| previous >= component.id.as_str()) {
                        return Err(PlannerContractError::new(
                            "operation.replace_custom_store.components",
                            "must be sorted by unique component ID",
                        ));
                    }
                    previous_id = Some(component.id.as_str());
                }
                Ok(())
            }
            Self::RestorePayloadsFromCustomStore {
                owner,
                component_ids,
            } => {
                validate_custom_store_owner(
                    "operation.restore_payloads_from_custom_store.owner",
                    owner,
                )?;
                validate_id_list(
                    "operation.restore_payloads_from_custom_store.component_ids",
                    component_ids,
                    false,
                )
            }
            Self::CommitLoadStageBank {
                component_id,
                runtime_file_id,
                source_stage,
                destination_stage,
                source_binding,
                destination_binding,
            } => {
                validate_stable_id("operation.component_id", component_id)?;
                validate_stable_id("operation.runtime_file_id", runtime_file_id)?;
                validate_binding(&ComponentBinding::Stage {
                    stage: source_stage.clone(),
                })?;
                validate_binding(&ComponentBinding::Stage {
                    stage: destination_stage.clone(),
                })?;
                validate_binding(source_binding)?;
                validate_binding(destination_binding)?;
                if matches!(source_binding, ComponentBinding::Unbound)
                    || matches!(destination_binding, ComponentBinding::Unbound)
                {
                    return Err(PlannerContractError::new(
                        "operation.commit_load_stage_bank.binding",
                        "source and destination bindings must be explicit",
                    ));
                }
                Ok(())
            }
            Self::ActivateStageBank {
                component_id,
                runtime_file_id,
                stage,
                binding,
            } => {
                validate_stable_id("operation.component_id", component_id)?;
                validate_stable_id("operation.runtime_file_id", runtime_file_id)?;
                validate_binding(&ComponentBinding::Stage {
                    stage: stage.clone(),
                })?;
                validate_binding(binding)?;
                if matches!(binding, ComponentBinding::Unbound) {
                    return Err(PlannerContractError::new(
                        "operation.activate_stage_bank.binding",
                        "must be explicit",
                    ));
                }
                Ok(())
            }
            Self::SaveRuntimeToSlot {
                source_runtime_file_id,
                destination_slot,
                destination_persistent_file_id,
                runtime_component_ids,
                stage_bank_stages,
            } => {
                validate_stable_id("operation.source_runtime_file_id", source_runtime_file_id)?;
                destination_slot.validate("operation.destination_slot")?;
                validate_stable_id(
                    "operation.destination_persistent_file_id",
                    destination_persistent_file_id,
                )?;
                validate_id_list(
                    "operation.runtime_component_ids",
                    runtime_component_ids,
                    false,
                )?;
                validate_stage_list("operation.stage_bank_stages", stage_bank_stages)
            }
            Self::SaveActiveRuntimeToSlot {
                destination_slot,
                destination_id_suffix,
                runtime_component_ids,
                projection_operations,
            } => {
                destination_slot.validate("operation.destination_slot")?;
                validate_stable_id("operation.destination_id_suffix", destination_id_suffix)?;
                validate_id_list(
                    "operation.runtime_component_ids",
                    runtime_component_ids,
                    false,
                )?;
                validate_save_projection_operations(runtime_component_ids, projection_operations)
            }
            Self::LoadRuntimeFromSlot {
                source_runtime_file_id,
                source_slot,
                source_persistent_file_id,
                destination_runtime_file_id,
                destination_allowed_serialization_targets,
                runtime_component_ids,
                stage_bank_stages,
                carried_runtime_component_ids,
            } => {
                validate_stable_id("operation.source_runtime_file_id", source_runtime_file_id)?;
                source_slot.validate("operation.source_slot")?;
                validate_stable_id(
                    "operation.source_persistent_file_id",
                    source_persistent_file_id,
                )?;
                validate_stable_id(
                    "operation.destination_runtime_file_id",
                    destination_runtime_file_id,
                )?;
                if source_runtime_file_id == destination_runtime_file_id {
                    return Err(PlannerContractError::new(
                        "operation.destination_runtime_file_id",
                        "must begin a distinct runtime-file lifetime",
                    ));
                }
                validate_slot_list(
                    "operation.destination_allowed_serialization_targets",
                    destination_allowed_serialization_targets,
                )?;
                validate_id_list(
                    "operation.runtime_component_ids",
                    runtime_component_ids,
                    false,
                )?;
                validate_stage_list("operation.stage_bank_stages", stage_bank_stages)?;
                validate_id_list(
                    "operation.carried_runtime_component_ids",
                    carried_runtime_component_ids,
                    true,
                )?;
                if carried_runtime_component_ids
                    .iter()
                    .any(|id| runtime_component_ids.binary_search(id).is_ok())
                {
                    return Err(PlannerContractError::new(
                        "operation.carried_runtime_component_ids",
                        "must be disjoint from the persistent image manifest",
                    ));
                }
                Ok(())
            }
            Self::LoadActiveRuntimeFromSlot {
                source_slot,
                destination_id_suffix,
                destination_allowed_serialization_targets,
                carried_runtime_component_ids,
            } => {
                source_slot.validate("operation.source_slot")?;
                validate_stable_id("operation.destination_id_suffix", destination_id_suffix)?;
                validate_slot_list(
                    "operation.destination_allowed_serialization_targets",
                    destination_allowed_serialization_targets,
                )?;
                validate_id_list(
                    "operation.carried_runtime_component_ids",
                    carried_runtime_component_ids,
                    true,
                )
            }
            Self::BeginRuntimeFileLifetime {
                destination_id_suffix,
                origin,
                backing,
                allowed_serialization_targets,
            } => {
                validate_stable_id("operation.destination_id_suffix", destination_id_suffix)?;
                RuntimeFile {
                    id: format!("runtime.{destination_id_suffix}"),
                    origin: origin.clone(),
                    backing: backing.clone(),
                    allowed_serialization_targets: allowed_serialization_targets.clone(),
                    lifecycle: RuntimeFileLifecycle::Active,
                }
                .validate()
            }
            Self::Bind { selector, binding } | Self::Rebind { selector, binding } => {
                validate_component_selector(selector)?;
                validate_binding(binding)
            }
            Self::SetActiveRuntimeFile { runtime_file } => {
                runtime_file.validate()?;
                if runtime_file.lifecycle != RuntimeFileLifecycle::Active {
                    return Err(PlannerContractError::new(
                        "operation.runtime_file.lifecycle",
                        "must be active",
                    ));
                }
                Ok(())
            }
            Self::SetExecutionContext { context } => context.validate(),
            Self::CompletePendingWorldLoad => Ok(()),
            Self::SetLocation { location } => location.validate(),
            Self::SetLocationFromFields {
                component_id,
                stage_field,
                room_field,
                spawn_field,
                ..
            }
            | Self::SetPendingWorldLoadFromFields {
                component_id,
                stage_field,
                room_field,
                spawn_field,
                ..
            } => {
                validate_stable_id("operation.component_id", component_id)?;
                for field in [stage_field, room_field, spawn_field] {
                    validate_stable_id("operation.location_field", field)?;
                }
                if stage_field == room_field
                    || stage_field == spawn_field
                    || room_field == spawn_field
                {
                    return Err(PlannerContractError::new(
                        "operation.set_location_from_fields",
                        "must reference three distinct fields",
                    ));
                }
                Ok(())
            }
            Self::SetPlayerForm {
                form: PlayerForm::Other { id },
            } => validate_stable_id("operation.set_player_form.id", id),
            Self::SetPlayerForm { .. } | Self::SetPlayerControl { .. } => Ok(()),
            Self::SetPlayerMount { mount } => {
                if let Some(PlayerMount::Other { id }) = mount {
                    validate_stable_id("operation.set_player_mount.id", id)?;
                }
                Ok(())
            }
            Self::SetPlayerAction { action } => {
                validate_stable_id("operation.set_player_action.action", action)
            }
            Self::ReconstructActor {
                static_object_id,
                instance_id,
                initialization_fields,
                ..
            } => {
                validate_stable_id(
                    "operation.reconstruct_actor.static_object_id",
                    static_object_id,
                )?;
                validate_stable_id("operation.reconstruct_actor.instance_id", instance_id)?;
                validate_state_fields(initialization_fields)
            }
            Self::Project {
                source_runtime_file_id,
                destination_runtime_file_id,
                component_ids,
            } => {
                validate_stable_id("operation.source_runtime_file_id", source_runtime_file_id)?;
                validate_stable_id(
                    "operation.destination_runtime_file_id",
                    destination_runtime_file_id,
                )?;
                validate_id_list("operation.component_ids", component_ids, false)
            }
            Self::Consume {
                pending_operation_id,
            } => validate_stable_id("operation.pending_operation_id", pending_operation_id),
            Self::SetGate { gate_id } | Self::ClearGate { gate_id } => {
                validate_stable_id("operation.gate_id", gate_id)
            }
            Self::AdvanceFlow {
                flow_component_id,
                node_id,
            } => {
                validate_stable_id("operation.flow_component_id", flow_component_id)?;
                validate_stable_id("operation.node_id", node_id)
            }
            Self::BranchFlow {
                flow_component_id,
                edge_id,
                destination_node_id,
            } => {
                validate_stable_id("operation.flow_component_id", flow_component_id)?;
                validate_stable_id("operation.edge_id", edge_id)?;
                validate_stable_id("operation.destination_node_id", destination_node_id)
            }
            Self::ScheduleCleanup { cleanup_id } | Self::CancelCleanup { cleanup_id } => {
                validate_stable_id("operation.cleanup_id", cleanup_id)
            }
            Self::Interrupt { action_id, window } => {
                validate_stable_id("operation.action_id", action_id)?;
                window.validate()
            }
        }
    }
}

fn validate_raw_write(mask: &[u8], value: &[u8], field: &str) -> Result<(), PlannerContractError> {
    if mask.is_empty()
        || mask.len() != value.len()
        || mask.len() > crate::state::MAX_COMPONENT_BYTES
    {
        return Err(PlannerContractError::new(
            field,
            "mask/value must have equal nonzero bounded lengths",
        ));
    }
    validate_raw_mask(mask, &format!("{field}.mask"))
}

fn validate_raw_mask(mask: &[u8], field: &str) -> Result<(), PlannerContractError> {
    if mask.is_empty() || mask.len() > crate::state::MAX_COMPONENT_BYTES {
        return Err(PlannerContractError::new(
            field,
            "must have a nonzero bounded length",
        ));
    }
    if mask.iter().all(|byte| *byte == 0) {
        return Err(PlannerContractError::new(
            field,
            "must select at least one bit",
        ));
    }
    Ok(())
}

impl SaveProjectionOperation {
    pub(crate) fn to_state_operation(&self) -> StateOperation {
        match self {
            Self::Write { target, value } => StateOperation::Write {
                target: target.clone(),
                value: value.clone(),
            },
            Self::WriteFields {
                component_id,
                fields,
            } => StateOperation::WriteFields {
                component_id: component_id.clone(),
                fields: fields.clone(),
            },
            Self::CopyValue { source, target } => StateOperation::CopyValue {
                source: source.clone(),
                target: target.clone(),
            },
            Self::WriteRaw {
                component_id,
                byte_offset,
                mask,
                value,
            } => StateOperation::WriteRaw {
                component_id: component_id.clone(),
                byte_offset: *byte_offset,
                mask: mask.clone(),
                value: value.clone(),
            },
            Self::WriteBytesField {
                target,
                byte_offset,
                mask,
                value,
            } => StateOperation::WriteBytesField {
                target: target.clone(),
                byte_offset: *byte_offset,
                mask: mask.clone(),
                value: value.clone(),
            },
            Self::InvalidateRaw {
                component_id,
                byte_offset,
                mask,
            } => StateOperation::InvalidateRaw {
                component_id: component_id.clone(),
                byte_offset: *byte_offset,
                mask: mask.clone(),
            },
            Self::InvalidateField { target } => StateOperation::InvalidateField {
                target: target.clone(),
            },
        }
    }

    fn target_component_id(&self) -> &str {
        match self {
            Self::Write { target, .. }
            | Self::WriteBytesField { target, .. }
            | Self::InvalidateField { target } => &target.component_id,
            Self::WriteFields { component_id, .. }
            | Self::WriteRaw { component_id, .. }
            | Self::InvalidateRaw { component_id, .. } => component_id,
            Self::CopyValue { target, .. } => &target.component_id,
        }
    }
}

fn validate_save_projection_operations(
    runtime_component_ids: &[String],
    operations: &[SaveProjectionOperation],
) -> Result<(), PlannerContractError> {
    if operations.len() > 256 {
        return Err(PlannerContractError::new(
            "operation.save_active_runtime_to_slot.projection_operations",
            "must contain at most 256 operations",
        ));
    }
    for operation in operations {
        let target_component_id = operation.target_component_id();
        if runtime_component_ids
            .binary_search_by(|candidate| candidate.as_str().cmp(target_component_id))
            .is_err()
        {
            return Err(PlannerContractError::new(
                "operation.save_active_runtime_to_slot.projection_operations",
                "targets a component outside the persistent runtime projection",
            ));
        }
        operation.to_state_operation().validate()?;
    }
    Ok(())
}

fn validate_bound_raw_target(
    component_kind: &ComponentKind,
    binding: &ComponentBindingReference,
) -> Result<(), PlannerContractError> {
    validate_component_kind(component_kind)?;
    validate_binding_reference(binding)?;
    if matches!(
        binding,
        ComponentBindingReference::Exact {
            binding: ComponentBinding::Unbound
        }
    ) {
        return Err(PlannerContractError::new(
            "operation.bound_raw.binding",
            "must identify an explicit backing-store binding",
        ));
    }
    Ok(())
}
