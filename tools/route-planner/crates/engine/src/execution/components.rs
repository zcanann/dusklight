//! Query and mutate execution-state components and structured values.

use super::*;

impl PlannerExecutionState {
    pub(super) fn structured_value(
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

    pub(super) fn component_mut(
        &mut self,
        id: &str,
    ) -> Result<&mut StateComponent, PlannerContractError> {
        let index = self.component_index(id)?;
        Ok(&mut self.snapshot.environment.components[index])
    }

    pub(super) fn unique_bound_raw_component_id(
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

    pub(super) fn require_absent_component(&self, id: &str) -> Result<(), PlannerContractError> {
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

    pub(super) fn matching_ids(&self, selector: &ComponentSelector) -> BTreeSet<String> {
        self.matching_components(selector)
            .into_iter()
            .map(|component| component.id.clone())
            .collect()
    }

    pub(super) fn matching_ids_including_serialized(
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

    pub(super) fn matching_components(&self, selector: &ComponentSelector) -> Vec<&StateComponent> {
        self.snapshot
            .environment
            .components
            .iter()
            .filter(|component| selector_matches(selector, component))
            .collect()
    }

    pub(super) fn single_component(
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

    pub(super) fn write_flow(
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

    pub(super) fn sort_components(&mut self) {
        self.snapshot
            .environment
            .components
            .sort_by(|left, right| left.id.cmp(&right.id));
    }

    pub(super) fn boundary_disposition(
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
