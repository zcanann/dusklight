//! Resolve typed state operations and apply their atomic effects.

use super::*;

impl PlannerExecutionState {
    pub(super) fn affected_component_ids(&self, operation: &StateOperation) -> Vec<String> {
        let mut ids = match operation {
            StateOperation::Write { target, .. }
            | StateOperation::CopyValue { target, .. }
            | StateOperation::SetBitFromValue { target, .. }
            | StateOperation::Adjust { target, .. }
            | StateOperation::DebitUnsigned { target, .. }
            | StateOperation::ClearField { target }
            | StateOperation::InvalidateField { target } => vec![target.component_id.clone()],
            StateOperation::ClampUnsignedMinimum { target, minimum } => self
                .snapshot
                .environment
                .components
                .iter()
                .find(|component| component.id == target.component_id)
                .and_then(|component| match &component.payload {
                    ComponentPayload::Structured { fields } => fields.get(&target.field),
                    _ => None,
                })
                .and_then(|value| match value {
                    StateValue::Unsigned(current) if current < minimum => {
                        Some(vec![target.component_id.clone()])
                    }
                    _ => None,
                })
                .unwrap_or_default(),
            StateOperation::NormalizeItemSlotsAndLineup { component_id, .. } => {
                vec![component_id.clone()]
            }
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
            StateOperation::WriteBytesField { target, .. } => {
                vec![target.component_id.clone()]
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
            } => self.affected_save_component_ids(
                source_runtime_file_id,
                runtime_component_ids,
                Some(stage_bank_stages),
            ),
            StateOperation::SaveActiveRuntimeToSlot {
                runtime_component_ids,
                ..
            } => self.affected_save_component_ids(
                &self.snapshot.environment.active_runtime_file.id,
                runtime_component_ids,
                None,
            ),
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
            | StateOperation::ReconstructActor { .. }
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

    pub(super) fn apply_operation(
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
            StateOperation::WriteBytesField {
                target,
                byte_offset,
                mask,
                value,
            } => {
                let component = self.component_mut(&target.component_id)?;
                let ComponentPayload::Structured { fields } = &mut component.payload else {
                    return Err(PlannerContractError::new(
                        "operation.write_bytes_field",
                        "requires a structured destination component",
                    ));
                };
                let Some(StateValue::Bytes(bytes)) = fields.get_mut(&target.field) else {
                    return Err(PlannerContractError::new(
                        "operation.write_bytes_field",
                        "requires an existing byte-valued destination field",
                    ));
                };
                let offset = usize::try_from(*byte_offset).map_err(|_| {
                    PlannerContractError::new(
                        "operation.write_bytes_field.byte_offset",
                        "does not fit this host",
                    )
                })?;
                let end = offset.checked_add(mask.len()).ok_or_else(|| {
                    PlannerContractError::new("operation.write_bytes_field", "range overflows")
                })?;
                if end > bytes.len() {
                    return Err(PlannerContractError::new(
                        "operation.write_bytes_field",
                        "range exceeds the destination field",
                    ));
                }
                for index in 0..mask.len() {
                    let selected = mask[index];
                    bytes[offset + index] =
                        (bytes[offset + index] & !selected) | (value[index] & selected);
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
            StateOperation::DebitUnsigned { target, amount } => {
                let component = self.component_mut(&target.component_id)?;
                let ComponentPayload::Structured { fields } = &mut component.payload else {
                    return Err(PlannerContractError::new(
                        "operation.debit_unsigned",
                        "requires a structured destination component",
                    ));
                };
                let value = fields.get_mut(&target.field).ok_or_else(|| {
                    PlannerContractError::new(
                        "operation.debit_unsigned",
                        "references an absent field",
                    )
                })?;
                let StateValue::Unsigned(current) = value else {
                    return Err(PlannerContractError::new(
                        "operation.debit_unsigned",
                        "requires an unsigned destination field",
                    ));
                };
                *current = current.saturating_sub(*amount);
                mark_transition(component, application_id);
            }
            StateOperation::ClampUnsignedMinimum { target, minimum } => {
                let component = self.component_mut(&target.component_id)?;
                let ComponentPayload::Structured { fields } = &mut component.payload else {
                    return Err(PlannerContractError::new(
                        "operation.clamp_unsigned_minimum",
                        "requires a structured destination component",
                    ));
                };
                let value = fields.get_mut(&target.field).ok_or_else(|| {
                    PlannerContractError::new(
                        "operation.clamp_unsigned_minimum",
                        "references an absent field",
                    )
                })?;
                let StateValue::Unsigned(current) = value else {
                    return Err(PlannerContractError::new(
                        "operation.clamp_unsigned_minimum",
                        "requires an unsigned destination field",
                    ));
                };
                if *current < *minimum {
                    *current = *minimum;
                    mark_transition(component, application_id);
                }
            }
            StateOperation::NormalizeItemSlotsAndLineup {
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
                let component = self.component_mut(component_id)?;
                let ComponentPayload::Structured { fields } = &mut component.payload else {
                    return Err(PlannerContractError::new(
                        "operation.normalize_item_slots_and_lineup",
                        "requires a structured destination component",
                    ));
                };
                let StateValue::Bytes(mut inventory) =
                    fields.get(inventory_field).cloned().ok_or_else(|| {
                        PlannerContractError::new(
                            "operation.normalize_item_slots_and_lineup.inventory_field",
                            "references an absent inventory field",
                        )
                    })?
                else {
                    return Err(PlannerContractError::new(
                        "operation.normalize_item_slots_and_lineup.inventory_field",
                        "requires a byte-array inventory field",
                    ));
                };
                let StateValue::Bytes(existing_lineup) =
                    fields.get(lineup_field).cloned().ok_or_else(|| {
                        PlannerContractError::new(
                            "operation.normalize_item_slots_and_lineup.lineup_field",
                            "references an absent lineup field",
                        )
                    })?
                else {
                    return Err(PlannerContractError::new(
                        "operation.normalize_item_slots_and_lineup.lineup_field",
                        "requires a byte-array lineup field",
                    ));
                };
                if inventory.len() != existing_lineup.len() {
                    return Err(PlannerContractError::new(
                        "operation.normalize_item_slots_and_lineup.lineup_field",
                        "must have the same length as the inventory field",
                    ));
                }
                let primary = usize::from(*primary_slot);
                let secondary = usize::from(*secondary_slot);
                if primary >= inventory.len()
                    || secondary >= inventory.len()
                    || lineup_order
                        .iter()
                        .any(|slot| usize::from(*slot) >= inventory.len())
                {
                    return Err(PlannerContractError::new(
                        "operation.normalize_item_slots_and_lineup",
                        "slot index exceeds the inventory field",
                    ));
                }
                if inventory[primary] == *combined_item {
                    inventory[secondary] = *combined_item;
                    inventory[primary] = *empty_item;
                }
                if inventory[primary] == *single_item && inventory[secondary] == *combined_item {
                    inventory[primary] = *empty_item;
                }
                let mut lineup = vec![*empty_item; inventory.len()];
                let mut cursor = 0;
                for slot in lineup_order {
                    if inventory[usize::from(*slot)] != *empty_item {
                        lineup[cursor] = *slot;
                        cursor += 1;
                    }
                }
                fields.insert(inventory_field.clone(), StateValue::Bytes(inventory));
                fields.insert(lineup_field.clone(), StateValue::Bytes(lineup));
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
            operation @ StateOperation::SaveActiveRuntimeToSlot { .. } => {
                self.save_active_runtime_to_slot(application_id, operation)?
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
            StateOperation::ReconstructActor {
                static_object_id,
                instance_id,
                required_layer,
                initialization_fields,
            } => {
                if !matches!(
                    self.snapshot.environment.execution_context,
                    ExecutionContext::World
                ) {
                    return Err(PlannerContractError::new(
                        "operation.reconstruct_actor",
                        "requires active world execution",
                    ));
                }
                let location = &self.snapshot.environment.location;
                if location.layer != *required_layer {
                    return Err(PlannerContractError::new(
                        "operation.reconstruct_actor.required_layer",
                        "does not match the loaded layer",
                    ));
                }
                let placement = self
                    .snapshot
                    .environment
                    .static_world_objects
                    .iter()
                    .find(|object| object.id == *static_object_id)
                    .ok_or_else(|| {
                        PlannerContractError::new(
                            "operation.reconstruct_actor.static_object_id",
                            "references an absent static placement",
                        )
                    })?;
                let placement_selected = match &placement.binding {
                    ComponentBinding::Stage { stage } => stage == &location.stage,
                    ComponentBinding::Room { stage, room } => {
                        stage == &location.stage && *room == location.room
                    }
                    _ => false,
                };
                if !placement_selected {
                    return Err(PlannerContractError::new(
                        "operation.reconstruct_actor.static_object_id",
                        "placement is not selected by the current stage and room",
                    ));
                }

                let mut fields = placement.parameters.clone();
                if let Some(control) = self
                    .snapshot
                    .environment
                    .persisted_object_controls
                    .iter()
                    .find(|control| control.object_id == *static_object_id)
                {
                    fields.extend(control.fields.clone());
                }
                fields.extend(initialization_fields.clone());
                let reconstructed = LiveWorldObject {
                    instance_id: instance_id.clone(),
                    static_object_id: Some(static_object_id.clone()),
                    actor_type: placement.actor_type.clone(),
                    lifecycle: ActorLifecycle::Loaded,
                    fields,
                };
                if let Some(index) = self
                    .snapshot
                    .environment
                    .live_world_objects
                    .iter()
                    .position(|object| object.instance_id == *instance_id)
                {
                    let existing = &self.snapshot.environment.live_world_objects[index];
                    if existing.static_object_id.as_deref() != Some(static_object_id)
                        || existing.actor_type != placement.actor_type
                    {
                        return Err(PlannerContractError::new(
                            "operation.reconstruct_actor.instance_id",
                            "existing instance does not match the selected placement",
                        ));
                    }
                    if !matches!(
                        existing.lifecycle,
                        ActorLifecycle::Unloaded | ActorLifecycle::Destroyed
                    ) {
                        return Err(PlannerContractError::new(
                            "operation.reconstruct_actor.instance_id",
                            "existing instance is not at a reconstructable lifecycle boundary",
                        ));
                    }
                    self.snapshot.environment.live_world_objects[index] = reconstructed;
                } else {
                    self.snapshot
                        .environment
                        .live_world_objects
                        .push(reconstructed);
                    self.snapshot
                        .environment
                        .live_world_objects
                        .sort_by(|left, right| left.instance_id.cmp(&right.instance_id));
                }
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
}
