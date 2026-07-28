//! Conservative import of immutable world inventories into planner facts.
//!
//! Authored SCLS destinations remain addressable even when no activation is
//! known. A candidate transition is emitted only for an extracted collision /
//! SCLS join, and that candidate still carries explicit activation-semantics
//! and physical-reachability obligations.

use crate::artifact::Digest;
use crate::identity::{ContentIdentity, ContextSelector, ExactContext, RuntimeConfiguration};
use crate::logic::{
    ComparisonOperator, ContextScope, EvidenceKind, EvidenceRecord, PredicateExpression,
    RuleEvidence, TruthStatus, ValueReference,
};
use crate::orig_world::{ExtractedOrigWorldInventories, NativeStageMetadata};
use crate::state::{
    ComponentBinding, ComponentBindingReference, ComponentKind, SceneLocation, SpatialPlane,
    SpatialVolume, SpatialVolumeShape, StateValue, StaticWorldObject, validate_spatial_plane,
    validate_spatial_volume, validate_static_object,
};
use crate::transition::{
    ActivationContract, CandidateTransition, FeasibilityObligation, MECHANICS_CATALOG_SCHEMA,
    MechanicsCatalog, ObligationDetail, ObligationKind, StateOperation, TransitionKind,
    UnknownRequirement,
};
use crate::world_data::{
    KclReconstruction, PlacementKind, PlacementRecord, SourceKind, WorldContext, WorldInventory,
};
use crate::{PlannerContractError, canonical_json, validate_label, validate_stable_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const EXTRACTED_WORLD_FACTS_SCHEMA: &str = "dusklight.route-planner.extracted-world-facts/v21";
pub const MAX_EXTRACTED_WORLD_RECORDS: usize = 2_000_000;

const DUNGEON_SESSION_SWITCH_LABEL_KIND: &str = "observed-dungeon-session-switch-labels";
const ROOM_SWITCH_LABEL_KIND: &str = "observed-room-switch-labels";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorldInventoryFactSource {
    pub stage: String,
    pub inventory_sha256: Digest,
    pub spatial_index_sha256: Option<Digest>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedSpawn {
    pub id: String,
    pub source_object_id: String,
    pub source_record_id: String,
    pub location: SceneLocation,
    pub position: [f32; 3],
    pub rotation: [i16; 3],
    pub parameters: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedEncodedExit {
    pub id: String,
    pub source_record_id: String,
    pub source_stage: String,
    pub source_room: Option<i8>,
    pub destination: SceneLocation,
    pub wipe: u8,
    pub wipe_time: u8,
    pub time_hour: i8,
    pub raw: Vec<u8>,
    pub candidate_transition_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExtractedApproachShape {
    Reconstructed {
        triangle: [[f32; 3]; 3],
        plane_normal: [f32; 3],
        plane_offset: f32,
        minimum: [f32; 3],
        maximum: [f32; 3],
    },
    Unavailable {
        reason: String,
    },
}

/// Geometry attached to one imported collision/SCLS candidate. Same-room
/// spawns are candidates for later reachability work, never proof of a path.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedApproachGeometry {
    pub id: String,
    pub transition_id: String,
    pub approach_id: String,
    pub source_stage: String,
    pub source_room: i8,
    pub source_collision_id: String,
    pub source_inventory_sha256: Digest,
    pub candidate_spawn_ids: Vec<String>,
    pub shape: ExtractedApproachShape,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractedWorldFacts {
    pub schema: String,
    pub exact_context: ExactContext,
    pub world_context_sha256: Option<Digest>,
    pub native_inventory_set_sha256: Option<Digest>,
    pub inventories: Vec<WorldInventoryFactSource>,
    pub native_stage_metadata: Vec<NativeStageMetadata>,
    pub static_world_objects: Vec<StaticWorldObject>,
    pub spatial_volumes: Vec<SpatialVolume>,
    pub spatial_planes: Vec<SpatialPlane>,
    pub spawns: Vec<ExtractedSpawn>,
    pub encoded_exits: Vec<ExtractedEncodedExit>,
    pub approach_geometries: Vec<ExtractedApproachGeometry>,
    pub mechanics: MechanicsCatalog,
}

#[derive(Clone, Copy)]
struct WorldImportStage<'a> {
    inventory: &'a WorldInventory,
    inventory_sha256: Digest,
    spatial_index_sha256: Option<Digest>,
}

mod boss_door;
mod core;
mod generic;
mod guards;
mod keyed_actors;
mod scene_actors;
mod utilities;

use boss_door::*;
use generic::*;
use guards::*;
use keyed_actors::*;
use scene_actors::*;
use utilities::*;

#[cfg(test)]
mod tests;
