//! Complete state-local inventory of save/runtime components and boundaries.
//!
//! A catalog is derived from a validated execution state rather than a second
//! handwritten component list. It includes live components, transient backing
//! stores, physical-file images and every supplied boundary's effective action
//! on every live component. Defaults and temporary preserve overrides remain
//! visible so an omitted rule can never look like implicit preservation.

use crate::PlannerContractError;
use crate::artifact::Digest;
use crate::execution::PlannerExecutionState;
use crate::state::{
    BoundaryDisposition, BoundaryKind, BoundaryPolicy, ComponentBinding, ComponentKind,
    ComponentPayload, ComponentSelector, PhysicalSlot, RuntimeFile, SemanticLifetime,
    SerializationOwner, StateComponent,
};
use crate::{canonical_json, require_canonical_json_bytes, validate_stable_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;

pub const COMPONENT_BOUNDARY_CATALOG_SCHEMA: &str =
    "dusklight.route-planner.component-boundary-catalog/v1";

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComponentStorageLocation {
    Live,
    SerializedStore {
        owner: SerializationOwner,
    },
    PersistentImageRuntime {
        image_id: String,
    },
    PersistentImageStageBank {
        image_id: String,
        owner: SerializationOwner,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComponentPayloadCoverage {
    Raw { bytes: u64, known_bytes: u64 },
    Structured { fields: Vec<String> },
    Unknown { expected_bytes: Option<u32> },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentInventoryEntry {
    pub storage: ComponentStorageLocation,
    pub component_id: String,
    pub component_kind: ComponentKind,
    pub binding: ComponentBinding,
    pub lifetime: SemanticLifetime,
    pub serialization_owner: SerializationOwner,
    pub payload_coverage: ComponentPayloadCoverage,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryDispositionSource {
    ExplicitRule,
    DefaultDisposition,
    OneBoundaryPreserveOverride,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveComponentBoundary {
    pub policy_id: String,
    pub boundary: BoundaryKind,
    pub component_id: String,
    pub disposition: BoundaryDisposition,
    pub source: BoundaryDispositionSource,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentBoundaryCatalog {
    pub schema: String,
    pub source_execution_state_sha256: Digest,
    pub active_runtime_file: RuntimeFile,
    pub inactive_runtime_files: Vec<RuntimeFile>,
    pub physical_slots: Vec<PhysicalSlot>,
    pub inventory: Vec<ComponentInventoryEntry>,
    pub preserved_component_ids: BTreeSet<String>,
    pub boundary_policies: Vec<BoundaryPolicy>,
    pub effective_live_boundaries: Vec<EffectiveComponentBoundary>,
    pub content_sha256: Digest,
}

impl ComponentBoundaryCatalog {
    pub fn derive(
        state: &PlannerExecutionState,
        mut policies: Vec<BoundaryPolicy>,
    ) -> Result<Self, PlannerContractError> {
        state.validate()?;
        if policies.is_empty() {
            return Err(PlannerContractError::new(
                "component_boundary_catalog.boundary_policies",
                "must contain at least one reset or transition boundary",
            ));
        }
        policies.sort_by(|left, right| left.id.cmp(&right.id));
        let mut previous = None;
        for policy in &policies {
            policy.validate()?;
            if previous.is_some_and(|prior: &str| prior >= policy.id.as_str()) {
                return Err(PlannerContractError::new(
                    "component_boundary_catalog.boundary_policies",
                    "must have unique policy IDs",
                ));
            }
            previous = Some(policy.id.as_str());
        }

        let mut inventory = Vec::new();
        for component in &state.snapshot.environment.components {
            inventory.push(inventory_entry(ComponentStorageLocation::Live, component)?);
        }
        for (owner, components) in &state.serialized_components {
            for component in components {
                inventory.push(inventory_entry(
                    ComponentStorageLocation::SerializedStore {
                        owner: owner.clone(),
                    },
                    component,
                )?);
            }
        }
        for (image_id, image) in &state.persistent_file_images {
            for component in &image.runtime_components {
                inventory.push(inventory_entry(
                    ComponentStorageLocation::PersistentImageRuntime {
                        image_id: image_id.clone(),
                    },
                    component,
                )?);
            }
            for bank in &image.stage_banks {
                for component in &bank.components {
                    inventory.push(inventory_entry(
                        ComponentStorageLocation::PersistentImageStageBank {
                            image_id: image_id.clone(),
                            owner: bank.owner.clone(),
                        },
                        component,
                    )?);
                }
            }
        }
        inventory.sort_by(|left, right| {
            (&left.storage, left.component_id.as_str())
                .cmp(&(&right.storage, right.component_id.as_str()))
        });
        if inventory.windows(2).any(|pair| {
            pair[0].storage == pair[1].storage && pair[0].component_id == pair[1].component_id
        }) {
            return Err(PlannerContractError::new(
                "component_boundary_catalog.inventory",
                "contains a duplicate component within one storage location",
            ));
        }

        let mut effective_live_boundaries = Vec::new();
        for policy in &policies {
            for component in &state.snapshot.environment.components {
                let (disposition, source) = effective_disposition(state, policy, component)?;
                effective_live_boundaries.push(EffectiveComponentBoundary {
                    policy_id: policy.id.clone(),
                    boundary: policy.boundary.clone(),
                    component_id: component.id.clone(),
                    disposition,
                    source,
                });
            }
        }
        effective_live_boundaries.sort_by(|left, right| {
            (left.policy_id.as_str(), left.component_id.as_str())
                .cmp(&(right.policy_id.as_str(), right.component_id.as_str()))
        });

        let environment = &state.snapshot.environment;
        let mut catalog = Self {
            schema: COMPONENT_BOUNDARY_CATALOG_SCHEMA.into(),
            source_execution_state_sha256: state.digest()?,
            active_runtime_file: environment.active_runtime_file.clone(),
            inactive_runtime_files: environment.inactive_runtime_files.clone(),
            physical_slots: environment.physical_slots.clone(),
            inventory,
            preserved_component_ids: state.preserved_component_ids.clone(),
            boundary_policies: policies,
            effective_live_boundaries,
            content_sha256: Digest::ZERO,
        };
        catalog.content_sha256 = catalog.identity()?;
        catalog.validate()?;
        Ok(catalog)
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != COMPONENT_BOUNDARY_CATALOG_SCHEMA
            || self.source_execution_state_sha256 == Digest::ZERO
            || self.content_sha256 == Digest::ZERO
            || self.inventory.is_empty()
            || self.boundary_policies.is_empty()
        {
            return Err(PlannerContractError::new(
                "component_boundary_catalog",
                "has an unsupported schema or incomplete identity/inventory",
            ));
        }
        self.active_runtime_file.validate()?;
        validate_sorted_unique_runtime_files(&self.inactive_runtime_files)?;
        validate_sorted_unique_slots(&self.physical_slots)?;
        validate_inventory(&self.inventory)?;
        let live_ids = self
            .inventory
            .iter()
            .filter(|entry| entry.storage == ComponentStorageLocation::Live)
            .map(|entry| entry.component_id.as_str())
            .collect::<BTreeSet<_>>();
        for component_id in &self.preserved_component_ids {
            validate_stable_id(
                "component_boundary_catalog.preserved_component_ids",
                component_id,
            )?;
            if !live_ids.contains(component_id.as_str()) {
                return Err(PlannerContractError::new(
                    "component_boundary_catalog.preserved_component_ids",
                    "must name only live components",
                ));
            }
        }
        validate_policies(&self.boundary_policies)?;
        validate_effective_rows(
            &self.inventory,
            &self.preserved_component_ids,
            &self.boundary_policies,
            &self.effective_live_boundaries,
        )?;
        if self.content_sha256 != self.identity()? {
            return Err(PlannerContractError::new(
                "component_boundary_catalog.content_sha256",
                "does not reproduce the canonical catalog",
            ));
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let catalog: Self = serde_json::from_slice(bytes)?;
        catalog.validate()?;
        require_canonical_json_bytes(
            "component_boundary_catalog",
            bytes,
            &catalog.canonical_bytes()?,
        )?;
        Ok(catalog)
    }

    fn identity(&self) -> Result<Digest, PlannerContractError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.route-planner.component-boundary-catalog/v1\0");
        hasher.update(canonical_json(&canonical)?);
        Ok(Digest(hasher.finalize().into()))
    }
}

fn inventory_entry(
    storage: ComponentStorageLocation,
    component: &StateComponent,
) -> Result<ComponentInventoryEntry, PlannerContractError> {
    component.validate()?;
    let payload_coverage = match &component.payload {
        ComponentPayload::Raw { bytes, known_mask } => ComponentPayloadCoverage::Raw {
            bytes: bytes.len() as u64,
            known_bytes: known_mask.iter().filter(|byte| **byte == 0xff).count() as u64,
        },
        ComponentPayload::Structured { fields } => ComponentPayloadCoverage::Structured {
            fields: fields.keys().cloned().collect(),
        },
        ComponentPayload::Unknown { expected_bytes } => ComponentPayloadCoverage::Unknown {
            expected_bytes: *expected_bytes,
        },
    };
    Ok(ComponentInventoryEntry {
        storage,
        component_id: component.id.clone(),
        component_kind: component.component_kind.clone(),
        binding: component.binding.clone(),
        lifetime: component.lifetime,
        serialization_owner: component.serialization_owner.clone(),
        payload_coverage,
    })
}

fn effective_disposition(
    state: &PlannerExecutionState,
    policy: &BoundaryPolicy,
    component: &StateComponent,
) -> Result<(BoundaryDisposition, BoundaryDispositionSource), PlannerContractError> {
    if state.preserved_component_ids.contains(&component.id) {
        return Ok((
            BoundaryDisposition::Preserve,
            BoundaryDispositionSource::OneBoundaryPreserveOverride,
        ));
    }
    let matching = policy
        .component_rules
        .iter()
        .filter(|rule| selector_matches(&rule.selector, component))
        .collect::<Vec<_>>();
    match matching.as_slice() {
        [] => Ok((
            policy.default_disposition.clone(),
            BoundaryDispositionSource::DefaultDisposition,
        )),
        [rule] => Ok((
            rule.disposition.clone(),
            BoundaryDispositionSource::ExplicitRule,
        )),
        _ => Err(PlannerContractError::new(
            "component_boundary_catalog.boundary_policies",
            format!(
                "multiple rules in policy {} match component {}",
                policy.id, component.id
            ),
        )),
    }
}

fn selector_matches(selector: &ComponentSelector, component: &StateComponent) -> bool {
    match selector {
        ComponentSelector::Id { component_id } => component.id == *component_id,
        ComponentSelector::Kind { component_kind } => component.component_kind == *component_kind,
        ComponentSelector::Binding { binding } => component.binding == *binding,
    }
}

fn validate_inventory(entries: &[ComponentInventoryEntry]) -> Result<(), PlannerContractError> {
    let mut previous = None;
    for entry in entries {
        validate_stable_id(
            "component_boundary_catalog.inventory.component_id",
            &entry.component_id,
        )?;
        let key = (&entry.storage, entry.component_id.as_str());
        if previous.is_some_and(|prior| prior >= key) {
            return Err(PlannerContractError::new(
                "component_boundary_catalog.inventory",
                "must be unique and sorted by storage and component ID",
            ));
        }
        match &entry.payload_coverage {
            ComponentPayloadCoverage::Raw { bytes, known_bytes } if known_bytes > bytes => {
                return Err(PlannerContractError::new(
                    "component_boundary_catalog.inventory.payload_coverage",
                    "known raw bytes exceed the payload size",
                ));
            }
            ComponentPayloadCoverage::Structured { fields }
                if fields.windows(2).any(|pair| pair[0] >= pair[1]) =>
            {
                return Err(PlannerContractError::new(
                    "component_boundary_catalog.inventory.payload_coverage",
                    "structured fields must be unique and sorted",
                ));
            }
            _ => {}
        }
        previous = Some(key);
    }
    Ok(())
}

fn validate_policies(policies: &[BoundaryPolicy]) -> Result<(), PlannerContractError> {
    let mut previous = None;
    for policy in policies {
        policy.validate()?;
        if previous.is_some_and(|prior: &str| prior >= policy.id.as_str()) {
            return Err(PlannerContractError::new(
                "component_boundary_catalog.boundary_policies",
                "must be unique and sorted by policy ID",
            ));
        }
        previous = Some(policy.id.as_str());
    }
    Ok(())
}

fn validate_effective_rows(
    inventory: &[ComponentInventoryEntry],
    preserved_component_ids: &BTreeSet<String>,
    policies: &[BoundaryPolicy],
    rows: &[EffectiveComponentBoundary],
) -> Result<(), PlannerContractError> {
    let live_ids = inventory
        .iter()
        .filter(|entry| entry.storage == ComponentStorageLocation::Live)
        .map(|entry| entry.component_id.as_str())
        .collect::<BTreeSet<_>>();
    let expected = live_ids.len().checked_mul(policies.len()).ok_or_else(|| {
        PlannerContractError::new(
            "component_boundary_catalog.effective_live_boundaries",
            "row count overflowed",
        )
    })?;
    if rows.len() != expected {
        return Err(PlannerContractError::new(
            "component_boundary_catalog.effective_live_boundaries",
            "must contain exactly one row per live component and policy",
        ));
    }
    let policy_by_id = policies
        .iter()
        .map(|policy| (policy.id.as_str(), policy))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut previous = None;
    for row in rows {
        let policy = policy_by_id.get(row.policy_id.as_str()).ok_or_else(|| {
            PlannerContractError::new(
                "component_boundary_catalog.effective_live_boundaries.policy_id",
                "references an absent policy",
            )
        })?;
        if row.boundary != policy.boundary || !live_ids.contains(row.component_id.as_str()) {
            return Err(PlannerContractError::new(
                "component_boundary_catalog.effective_live_boundaries",
                "differs from its policy or references a non-live component",
            ));
        }
        let entry = inventory
            .iter()
            .find(|entry| {
                entry.storage == ComponentStorageLocation::Live
                    && entry.component_id == row.component_id
            })
            .ok_or_else(|| {
                PlannerContractError::new(
                    "component_boundary_catalog.effective_live_boundaries.component_id",
                    "references an absent live inventory entry",
                )
            })?;
        let expected = effective_inventory_disposition(preserved_component_ids, policy, entry)?;
        if (row.disposition.clone(), row.source) != expected {
            return Err(PlannerContractError::new(
                "component_boundary_catalog.effective_live_boundaries",
                "does not reproduce the effective policy disposition",
            ));
        }
        let key = (row.policy_id.as_str(), row.component_id.as_str());
        if previous.is_some_and(|prior| prior >= key) {
            return Err(PlannerContractError::new(
                "component_boundary_catalog.effective_live_boundaries",
                "must be unique and sorted by policy and component ID",
            ));
        }
        previous = Some(key);
    }
    Ok(())
}

fn effective_inventory_disposition(
    preserved_component_ids: &BTreeSet<String>,
    policy: &BoundaryPolicy,
    entry: &ComponentInventoryEntry,
) -> Result<(BoundaryDisposition, BoundaryDispositionSource), PlannerContractError> {
    if preserved_component_ids.contains(&entry.component_id) {
        return Ok((
            BoundaryDisposition::Preserve,
            BoundaryDispositionSource::OneBoundaryPreserveOverride,
        ));
    }
    let matching = policy
        .component_rules
        .iter()
        .filter(|rule| match &rule.selector {
            ComponentSelector::Id { component_id } => entry.component_id == *component_id,
            ComponentSelector::Kind { component_kind } => entry.component_kind == *component_kind,
            ComponentSelector::Binding { binding } => entry.binding == *binding,
        })
        .collect::<Vec<_>>();
    match matching.as_slice() {
        [] => Ok((
            policy.default_disposition.clone(),
            BoundaryDispositionSource::DefaultDisposition,
        )),
        [rule] => Ok((
            rule.disposition.clone(),
            BoundaryDispositionSource::ExplicitRule,
        )),
        _ => Err(PlannerContractError::new(
            "component_boundary_catalog.boundary_policies",
            format!(
                "multiple rules in policy {} match component {}",
                policy.id, entry.component_id
            ),
        )),
    }
}

fn validate_sorted_unique_runtime_files(files: &[RuntimeFile]) -> Result<(), PlannerContractError> {
    let mut previous = None;
    for file in files {
        file.validate()?;
        if previous.is_some_and(|prior: &str| prior >= file.id.as_str()) {
            return Err(PlannerContractError::new(
                "component_boundary_catalog.inactive_runtime_files",
                "must be unique and sorted by runtime-file ID",
            ));
        }
        previous = Some(file.id.as_str());
    }
    Ok(())
}

fn validate_sorted_unique_slots(slots: &[PhysicalSlot]) -> Result<(), PlannerContractError> {
    let mut previous = None;
    for slot in slots {
        slot.slot
            .validate("component_boundary_catalog.physical_slots.slot")?;
        if previous.is_some_and(|prior| prior >= slot.slot) {
            return Err(PlannerContractError::new(
                "component_boundary_catalog.physical_slots",
                "must be unique and sorted by physical slot",
            ));
        }
        previous = Some(slot.slot);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::PlannerExecutionState;
    use crate::identity::RuntimeConfiguration;
    use crate::snapshot::{STATE_SNAPSHOT_SCHEMA, StateSnapshot};
    use crate::state::{
        BOUNDARY_POLICY_SCHEMA, BackingAttachment, ComponentBoundaryRule, ComponentProvenance,
        EXECUTION_ENVIRONMENT_SCHEMA, ExecutionContext, PhysicalSlotId, PlayerForm, PlayerState,
        ProvenanceSourceKind, RuntimeFileLifecycle, RuntimeFileOrigin, SceneLocation, StateValue,
    };
    use std::collections::BTreeMap;

    fn component(id: &str, kind: ComponentKind, lifetime: SemanticLifetime) -> StateComponent {
        StateComponent {
            id: id.into(),
            component_kind: kind,
            payload: ComponentPayload::Structured {
                fields: BTreeMap::from([("value".into(), StateValue::Unsigned(1))]),
            },
            binding: ComponentBinding::RuntimeFile {
                runtime_file_id: "runtime-1".into(),
            },
            lifetime,
            serialization_owner: SerializationOwner::RuntimeFile {
                runtime_file_id: "runtime-1".into(),
            },
            provenance: vec![ComponentProvenance {
                source_kind: ProvenanceSourceKind::Initialized,
                source_id: "fixture.component-catalog".into(),
                source_sha256: None,
                transition_id: None,
            }],
        }
    }

    fn state() -> PlannerExecutionState {
        PlannerExecutionState::new(StateSnapshot {
            schema: STATE_SNAPSHOT_SCHEMA.into(),
            id: "snapshot.catalog".into(),
            sequence: 0,
            environment: crate::state::ExecutionEnvironment {
                schema: EXECUTION_ENVIRONMENT_SCHEMA.into(),
                runtime_configuration: RuntimeConfiguration {
                    schema: "dusklight.route-planner.runtime-configuration/v1".into(),
                    content_sha256: "11".repeat(32).parse().unwrap(),
                    language: "en".into(),
                    settings: BTreeMap::new(),
                },
                active_runtime_file: RuntimeFile {
                    id: "runtime-1".into(),
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
                    stage: "F_SP102".into(),
                    room: 0,
                    layer: 10,
                    spawn: 100,
                },
                player: PlayerState {
                    form: PlayerForm::Human,
                    mount: None,
                    position: [0.0; 3],
                    rotation: [0; 3],
                    has_control: Some(true),
                    action: "wait".into(),
                },
                components: vec![
                    component(
                        "inventory",
                        ComponentKind::Inventory,
                        SemanticLifetime::RuntimeFile,
                    ),
                    component(
                        "message",
                        ComponentKind::MessageFlow,
                        SemanticLifetime::Action,
                    ),
                ],
                static_world_objects: Vec::new(),
                live_world_objects: Vec::new(),
                persisted_object_controls: Vec::new(),
                spatial_volumes: Vec::new(),
                spatial_connections: Vec::new(),
                spatial_planes: Vec::new(),
            },
            semantic_observations: Vec::new(),
        })
        .unwrap()
    }

    #[test]
    fn catalogs_defaults_rules_and_preserve_overrides_without_omission() {
        let mut state = state();
        state.preserved_component_ids.insert("message".into());
        let catalog = ComponentBoundaryCatalog::derive(
            &state,
            vec![BoundaryPolicy {
                schema: BOUNDARY_POLICY_SCHEMA.into(),
                id: "boundary.title".into(),
                boundary: BoundaryKind::TitleReturn,
                default_disposition: BoundaryDisposition::Clear,
                component_rules: vec![ComponentBoundaryRule {
                    selector: ComponentSelector::Kind {
                        component_kind: ComponentKind::Inventory,
                    },
                    disposition: BoundaryDisposition::Serialize {
                        owner: SerializationOwner::RuntimeFile {
                            runtime_file_id: "runtime-1".into(),
                        },
                    },
                }],
            }],
        )
        .unwrap();

        assert_eq!(catalog.inventory.len(), 2);
        assert_eq!(catalog.effective_live_boundaries.len(), 2);
        assert_eq!(
            catalog.effective_live_boundaries[0].source,
            BoundaryDispositionSource::ExplicitRule
        );
        assert_eq!(
            catalog.effective_live_boundaries[1].source,
            BoundaryDispositionSource::OneBoundaryPreserveOverride
        );
        ComponentBoundaryCatalog::decode_canonical(&catalog.canonical_bytes().unwrap()).unwrap();
    }

    #[test]
    fn overlapping_selectors_fail_closed() {
        let error = ComponentBoundaryCatalog::derive(
            &state(),
            vec![BoundaryPolicy {
                schema: BOUNDARY_POLICY_SCHEMA.into(),
                id: "boundary.overlap".into(),
                boundary: BoundaryKind::TitleReturn,
                default_disposition: BoundaryDisposition::Unknown,
                component_rules: vec![
                    ComponentBoundaryRule {
                        selector: ComponentSelector::Id {
                            component_id: "inventory".into(),
                        },
                        disposition: BoundaryDisposition::Clear,
                    },
                    ComponentBoundaryRule {
                        selector: ComponentSelector::Kind {
                            component_kind: ComponentKind::Inventory,
                        },
                        disposition: BoundaryDisposition::Preserve,
                    },
                ],
            }],
        )
        .unwrap_err();
        assert_eq!(
            error.field(),
            "component_boundary_catalog.boundary_policies"
        );
    }

    #[test]
    fn resealed_effective_row_drift_is_rejected() {
        let mut catalog = ComponentBoundaryCatalog::derive(
            &state(),
            vec![BoundaryPolicy {
                schema: BOUNDARY_POLICY_SCHEMA.into(),
                id: "boundary.clear".into(),
                boundary: BoundaryKind::ProcessRestart,
                default_disposition: BoundaryDisposition::Clear,
                component_rules: Vec::new(),
            }],
        )
        .unwrap();
        catalog.effective_live_boundaries[0].disposition = BoundaryDisposition::Preserve;
        catalog.content_sha256 = catalog.identity().unwrap();
        assert_eq!(
            catalog.validate().unwrap_err().field(),
            "component_boundary_catalog.effective_live_boundaries"
        );
    }
}
