//! Atomic execution of typed planner operations over explicit backing stores.

use crate::artifact::Digest;
use crate::snapshot::StateSnapshot;
use crate::state::{
    ActorLifecycle, BackingAttachment, BoundaryDisposition, BoundaryPolicy, ComponentBinding,
    ComponentBindingReference, ComponentKind, ComponentPayload, ComponentProvenance,
    ComponentSelector, ExecutionContext, LiveWorldObject, PhysicalSlot, ProvenanceSourceKind,
    RuntimeFile, RuntimeFileLifecycle, RuntimeFileOrigin, SceneLocation, SerializationOwner,
    StateComponent, StateValue, validate_serialization_owner,
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

mod components;
mod document;
mod operations;
mod persistence;
mod state;

use document::*;

#[cfg(test)]
mod tests;
