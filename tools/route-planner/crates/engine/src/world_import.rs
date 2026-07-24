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

pub const EXTRACTED_WORLD_FACTS_SCHEMA: &str = "dusklight.route-planner.extracted-world-facts/v19";
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

impl ExtractedWorldFacts {
    pub fn build(
        content: &ContentIdentity,
        runtime_configuration: &RuntimeConfiguration,
        world_context: &WorldContext,
        inventories: &[WorldInventory],
    ) -> Result<Self, PlannerContractError> {
        content.validate()?;
        runtime_configuration.validate()?;
        world_context
            .validate()
            .map_err(|error| world_error("world_context", error))?;
        let content_sha256 = content.digest()?;
        if runtime_configuration.content_sha256 != content_sha256 {
            return Err(PlannerContractError::new(
                "runtime_configuration.content_sha256",
                "does not name the supplied content identity",
            ));
        }
        if world_context.game_data_sha256 != content.fingerprint.game_data_sha256 {
            return Err(PlannerContractError::new(
                "world_context.game_data_sha256",
                "does not match the supplied content identity",
            ));
        }
        if inventories.len() != world_context.stages.len() {
            return Err(PlannerContractError::new(
                "inventories",
                "does not cover every world-context stage exactly once",
            ));
        }

        let mut inventory_by_stage = BTreeMap::new();
        for inventory in inventories {
            inventory
                .validate()
                .map_err(|error| world_error("inventories", error))?;
            if inventory_by_stage
                .insert(inventory.stage.as_str(), inventory)
                .is_some()
            {
                return Err(PlannerContractError::new(
                    "inventories",
                    "contains a duplicate stage",
                ));
            }
        }

        let stages = world_context
            .stages
            .iter()
            .map(|stage| {
                let inventory = inventory_by_stage
                    .get(stage.stage.as_str())
                    .copied()
                    .ok_or_else(|| {
                        PlannerContractError::new("inventories", "is missing a world-context stage")
                    })?;
                let inventory_sha256 = inventory
                    .digest()
                    .map_err(|error| world_error("inventories", error))?;
                if inventory_sha256 != stage.inventory_sha256 {
                    return Err(PlannerContractError::new(
                        "inventories",
                        "digest does not match the world context",
                    ));
                }
                Ok(WorldImportStage {
                    inventory,
                    inventory_sha256,
                    spatial_index_sha256: Some(stage.spatial_index_sha256),
                })
            })
            .collect::<Result<Vec<_>, PlannerContractError>>()?;
        Self::build_validated(
            content,
            runtime_configuration,
            Some(
                world_context
                    .digest()
                    .map_err(|error| world_error("world_context", error))?,
            ),
            None,
            stages,
            Vec::new(),
        )
    }

    fn build_validated(
        content: &ContentIdentity,
        runtime_configuration: &RuntimeConfiguration,
        world_context_sha256: Option<Digest>,
        native_inventory_set_sha256: Option<Digest>,
        stages: Vec<WorldImportStage<'_>>,
        native_stage_metadata: Vec<NativeStageMetadata>,
    ) -> Result<Self, PlannerContractError> {
        let content_sha256 = content.digest()?;
        let exact_context = ExactContext {
            content_sha256,
            runtime_configuration_sha256: runtime_configuration.digest()?,
        };
        let scope = ContextScope {
            selectors: vec![ContextSelector::Exact {
                context: exact_context.clone(),
            }],
        };
        let mut sources = Vec::with_capacity(stages.len());
        let mut static_world_objects = Vec::new();
        let mut spatial_volumes = Vec::new();
        let mut spatial_planes = Vec::new();
        let mut spawns = Vec::new();
        let mut encoded_exits = Vec::new();
        let mut approach_geometries = Vec::new();
        let mut transitions = Vec::new();
        let mut obligations = Vec::new();

        for stage in stages {
            let inventory = stage.inventory;
            let inventory_sha256 = stage.inventory_sha256;
            sources.push(WorldInventoryFactSource {
                stage: inventory.stage.clone(),
                inventory_sha256,
                spatial_index_sha256: stage.spatial_index_sha256,
            });

            for placement in inventory.placements.iter().chain(&inventory.player_spawns) {
                let object = import_static_object(&inventory.stage, placement)?;
                if placement.kind == PlacementKind::PlayerSpawn {
                    spawns.push(import_spawn(&inventory.stage, placement, &object.id)?);
                }
                static_world_objects.push(object);
            }

            let mut transition_ids_by_exit = BTreeMap::<String, Vec<String>>::new();
            for trigger in &inventory.load_triggers {
                let token = stable_token(
                    "world.load-trigger",
                    &[inventory.stage.as_bytes(), trigger.stable_id.as_bytes()],
                );
                let transition_id = format!("transition.{token}");
                let approach_id = format!("approach.{token}");
                let obligation_id = format!("obligation.reach.{token}");
                let evidence =
                    extracted_evidence(inventory_sha256, &token, trigger.inferred_semantics);
                let collision = inventory
                    .collisions
                    .iter()
                    .find(|collision| collision.prism.authored.stable_id == trigger.collision_id)
                    .expect("validated load trigger references a collision");
                let mut candidate_spawn_ids = inventory
                    .player_spawns
                    .iter()
                    .filter(|spawn| spawn.scope.room == Some(trigger.room))
                    .map(|spawn| {
                        stable_token(
                            "world.spawn",
                            &[inventory.stage.as_bytes(), spawn.stable_id.as_bytes()],
                        )
                    })
                    .collect::<Vec<_>>();
                candidate_spawn_ids.sort();
                let shape = match &collision.prism.reconstruction {
                    KclReconstruction::Reconstructed { plane, triangle } => {
                        let triangle = triangle
                            .map(|point| canonicalize_position([point.x, point.y, point.z]));
                        let (minimum, maximum) = triangle_bounds(&triangle);
                        ExtractedApproachShape::Reconstructed {
                            triangle,
                            plane_normal: canonicalize_position([
                                plane.normal.x,
                                plane.normal.y,
                                plane.normal.z,
                            ]),
                            plane_offset: canonicalize_scalar(plane.d),
                            minimum,
                            maximum,
                        }
                    }
                    KclReconstruction::Degenerate { reason } => {
                        ExtractedApproachShape::Unavailable {
                            reason: reason.clone(),
                        }
                    }
                };
                approach_geometries.push(ExtractedApproachGeometry {
                    id: format!("approach-geometry.{token}"),
                    transition_id: transition_id.clone(),
                    approach_id: approach_id.clone(),
                    source_stage: inventory.stage.clone(),
                    source_room: trigger.room,
                    source_collision_id: trigger.collision_id.clone(),
                    source_inventory_sha256: inventory_sha256,
                    candidate_spawn_ids,
                    shape,
                });
                obligations.push(FeasibilityObligation {
                    id: obligation_id.clone(),
                    label: format!(
                        "Reach collision exit {} in {} room {}",
                        trigger.collision_exit_id, inventory.stage, trigger.room
                    ),
                    scope: scope.clone(),
                    obligation_kind: ObligationKind::Geometry,
                    stage: crate::transition::ObligationStage::Reach,
                    detail: ObligationDetail::Geometry {
                        approach_id: approach_id.clone(),
                        source_region_id: stable_token(
                            "region.collision",
                            &[trigger.collision_id.as_bytes()],
                        ),
                        destination_region_id: stable_token(
                            "region.encoded-exit",
                            &[trigger.scls_id.as_bytes()],
                        ),
                    },
                    evidence: RuleEvidence {
                        truth: TruthStatus::Unknown,
                        records: evidence.records.clone(),
                    },
                });
                let unknown_requirements = trigger
                    .inferred_semantics
                    .then(|| UnknownRequirement {
                        id: "activation-semantics".into(),
                        description: "The collision-code/SCLS activation semantics are inferred and require source or trace confirmation.".into(),
                        evidence: RuleEvidence {
                            truth: TruthStatus::Unknown,
                            records: evidence.records.clone(),
                        },
                    })
                    .into_iter()
                    .collect();
                transitions.push(CandidateTransition {
                    id: transition_id.clone(),
                    label: format!(
                        "{} room {} exit {} to {} room {} point {}",
                        inventory.stage,
                        trigger.room,
                        trigger.collision_exit_id,
                        trigger.destination_stage,
                        trigger.destination_room,
                        trigger.destination_point
                    ),
                    scope: scope.clone(),
                    transition_kind: TransitionKind::EncodedMapExit,
                    approach_id,
                    activation: ActivationContract {
                        hard_guards: source_location_guard(&inventory.stage, trigger.room),
                        physical_obligation_ids: vec![obligation_id],
                        effects: vec![StateOperation::SetLocation {
                            location: SceneLocation {
                                stage: trigger.destination_stage.clone(),
                                room: trigger.destination_room,
                                layer: trigger.destination_layer,
                                spawn: trigger.destination_point,
                            },
                        }],
                        unknown_requirements,
                    },
                    evidence,
                });
                transition_ids_by_exit
                    .entry(trigger.scls_id.clone())
                    .or_default()
                    .push(transition_id);
            }

            if is_source_audited_gz2e01(content) {
                for placement in &inventory.placements {
                    let Some(imported) =
                        import_gz2e01_boss_door(inventory, placement, &scope, inventory_sha256)?
                    else {
                        continue;
                    };
                    transition_ids_by_exit
                        .entry(imported.exit_record_id)
                        .or_default()
                        .push(imported.transition.id.clone());
                    obligations.extend(imported.obligations);
                    spatial_volumes.extend(imported.spatial_volumes);
                    spatial_planes.extend(imported.spatial_planes);
                    transitions.push(imported.transition);
                }
                for placement in &inventory.placements {
                    let Some(imported) = import_gz2e01_keyed_actor_actions(
                        inventory,
                        placement,
                        &scope,
                        inventory_sha256,
                    )?
                    else {
                        continue;
                    };
                    if let Some(exit_record_id) = imported.exit_record_id {
                        transition_ids_by_exit
                            .entry(exit_record_id)
                            .or_default()
                            .extend(
                                imported
                                    .transitions
                                    .iter()
                                    .map(|transition| transition.id.clone()),
                            );
                    }
                    obligations.extend(imported.obligations);
                    transitions.extend(imported.transitions);
                }
                for placement in &inventory.placements {
                    let Some(imported) = import_gz2e01_l7_bridge_demo(
                        inventory,
                        placement,
                        &scope,
                        inventory_sha256,
                    )?
                    else {
                        continue;
                    };
                    for (exit_record_id, transition) in imported.transitions {
                        transition_ids_by_exit
                            .entry(exit_record_id)
                            .or_default()
                            .push(transition.id.clone());
                        transitions.push(transition);
                    }
                    obligations.extend(imported.obligations);
                }
            }

            for exit in &inventory.exits {
                let id = stable_token(
                    "world.encoded-exit",
                    &[inventory.stage.as_bytes(), exit.stable_id.as_bytes()],
                );
                let mut candidate_transition_ids = transition_ids_by_exit
                    .remove(exit.stable_id.as_str())
                    .unwrap_or_default();
                candidate_transition_ids.sort();
                encoded_exits.push(ExtractedEncodedExit {
                    id,
                    source_record_id: exit.stable_id.clone(),
                    source_stage: inventory.stage.clone(),
                    source_room: exit.scope.room,
                    destination: SceneLocation {
                        stage: exit.destination_stage.clone(),
                        room: exit.destination_room,
                        layer: exit.destination_layer,
                        spawn: exit.destination_point,
                    },
                    wipe: exit.wipe,
                    wipe_time: exit.wipe_time,
                    time_hour: exit.time_hour,
                    raw: decode_hex(&exit.raw_hex)?,
                    candidate_transition_ids,
                });
            }
        }

        static_world_objects.sort_by(|left, right| left.id.cmp(&right.id));
        spatial_volumes.sort_by(|left, right| {
            (&left.object_id, &left.volume_id).cmp(&(&right.object_id, &right.volume_id))
        });
        spatial_planes.sort_by(|left, right| left.plane_id.cmp(&right.plane_id));
        spawns.sort_by(|left, right| left.id.cmp(&right.id));
        encoded_exits.sort_by(|left, right| left.id.cmp(&right.id));
        approach_geometries.sort_by(|left, right| left.id.cmp(&right.id));
        obligations.sort_by(|left, right| left.id.cmp(&right.id));
        transitions.sort_by(|left, right| left.id.cmp(&right.id));
        let facts = Self {
            schema: EXTRACTED_WORLD_FACTS_SCHEMA.into(),
            exact_context,
            world_context_sha256,
            native_inventory_set_sha256,
            inventories: sources,
            native_stage_metadata,
            static_world_objects,
            spatial_volumes,
            spatial_planes,
            spawns,
            encoded_exits,
            approach_geometries,
            mechanics: MechanicsCatalog {
                schema: MECHANICS_CATALOG_SCHEMA.into(),
                transitions,
                obligations,
                writers: Vec::new(),
                gates: Vec::new(),
                readers: Vec::new(),
                reconstruction_rules: Vec::new(),
                obstructions: Vec::new(),
                resolvers: Vec::new(),
                techniques: Vec::new(),
                microtraces: Vec::new(),
                goals: Vec::new(),
            },
        };
        facts.validate()?;
        Ok(facts)
    }

    /// Imports planner-native stage records without manufacturing a compatible
    /// world-context or spatial-index identity. Collision-backed transitions
    /// remain absent because the v3 native inventory set marks that domain
    /// unavailable; placement/SCLS-backed actor rules still import normally.
    pub fn build_from_orig_world_inventories(
        content: &ContentIdentity,
        runtime_configuration: &RuntimeConfiguration,
        native: &ExtractedOrigWorldInventories,
    ) -> Result<Self, PlannerContractError> {
        content.validate()?;
        runtime_configuration.validate()?;
        native.validate()?;
        let content_sha256 = content.digest()?;
        if native.content_sha256 != content_sha256
            || native.game_data_sha256 != content.fingerprint.game_data_sha256
        {
            return Err(PlannerContractError::new(
                "native_world.identity",
                "does not match the supplied content identity",
            ));
        }
        if runtime_configuration.content_sha256 != content_sha256 {
            return Err(PlannerContractError::new(
                "runtime_configuration.content_sha256",
                "does not name the supplied content identity",
            ));
        }
        let native_sha256 = native.digest()?;
        let stages = native
            .inventories
            .iter()
            .map(|inventory| {
                Ok(WorldImportStage {
                    inventory,
                    inventory_sha256: inventory.digest()?,
                    spatial_index_sha256: None,
                })
            })
            .collect::<Result<Vec<_>, PlannerContractError>>()?;
        Self::build_validated(
            content,
            runtime_configuration,
            None,
            Some(native_sha256),
            stages,
            native.stage_metadata.clone(),
        )
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != EXTRACTED_WORLD_FACTS_SCHEMA {
            return Err(PlannerContractError::new(
                "extracted_world_facts",
                "has an unsupported schema",
            ));
        }
        ContextSelector::Exact {
            context: self.exact_context.clone(),
        }
        .validate()?;
        if self.inventories.is_empty() || self.inventories.len() > 256 {
            return Err(PlannerContractError::new(
                "inventories",
                "must contain between 1 and 256 exact stage inventories",
            ));
        }
        validate_sorted("inventories", &self.inventories, |value| {
            value.stage.as_str()
        })?;
        let compatible_provenance = matches!(
            (self.world_context_sha256, self.native_inventory_set_sha256),
            (Some(context), None) if context != Digest::ZERO
        );
        let native_provenance = matches!(
            (self.world_context_sha256, self.native_inventory_set_sha256),
            (None, Some(native)) if native != Digest::ZERO
        );
        if !compatible_provenance && !native_provenance {
            return Err(PlannerContractError::new(
                "extracted_world_facts.provenance",
                "must name exactly one nonzero world context or native inventory set",
            ));
        }
        if compatible_provenance && !self.native_stage_metadata.is_empty()
            || native_provenance
                && self.native_stage_metadata.len() != self.inventories.len()
        {
            return Err(PlannerContractError::new(
                "native_stage_metadata",
                "must be empty for compatible provenance and complete for planner-native provenance",
            ));
        }
        for (source, metadata) in self.inventories.iter().zip(&self.native_stage_metadata) {
            if source.stage != metadata.stage {
                return Err(PlannerContractError::new(
                    "native_stage_metadata.stage",
                    "does not match its inventory fact source",
                ));
            }
            metadata.validate_records()?;
        }
        for source in &self.inventories {
            validate_game_name("inventories.stage", &source.stage)?;
            if source.inventory_sha256 == Digest::ZERO
                || compatible_provenance
                    && !matches!(source.spatial_index_sha256, Some(digest) if digest != Digest::ZERO)
                || native_provenance && source.spatial_index_sha256.is_some()
            {
                return Err(PlannerContractError::new(
                    "inventories",
                    "does not match its compatible or planner-native spatial provenance",
                ));
            }
        }
        validate_sorted(
            "static_world_objects",
            &self.static_world_objects,
            |value| value.id.as_str(),
        )?;
        for object in &self.static_world_objects {
            validate_static_object(object)?;
        }
        if self.spatial_volumes.windows(2).any(|pair| {
            (&pair[0].object_id, &pair[0].volume_id) >= (&pair[1].object_id, &pair[1].volume_id)
        }) {
            return Err(PlannerContractError::new(
                "spatial_volumes",
                "must be unique and sorted by object and volume ID",
            ));
        }
        for volume in &self.spatial_volumes {
            validate_spatial_volume(volume)?;
            if !self
                .static_world_objects
                .iter()
                .any(|object| object.id == volume.object_id)
            {
                return Err(PlannerContractError::new(
                    "spatial_volumes.object_id",
                    "does not reference an imported static object",
                ));
            }
        }
        validate_sorted("spatial_planes", &self.spatial_planes, |value| {
            value.plane_id.as_str()
        })?;
        for plane in &self.spatial_planes {
            validate_spatial_plane(plane)?;
        }
        validate_sorted("spawns", &self.spawns, |value| value.id.as_str())?;
        let object_ids = self
            .static_world_objects
            .iter()
            .map(|object| object.id.as_str())
            .collect::<BTreeSet<_>>();
        for spawn in &self.spawns {
            validate_stable_id("spawns.id", &spawn.id)?;
            validate_stable_id("spawns.source_object_id", &spawn.source_object_id)?;
            if !object_ids.contains(spawn.source_object_id.as_str()) {
                return Err(PlannerContractError::new(
                    "spawns.source_object_id",
                    "does not reference an imported static object",
                ));
            }
            spawn.location.validate()?;
            if !canonical_position(spawn.position) {
                return Err(PlannerContractError::new(
                    "spawns.position",
                    "must contain finite canonical coordinates",
                ));
            }
        }
        self.mechanics.validate()?;
        if native_provenance
            && (!self.approach_geometries.is_empty()
                || self
                    .mechanics
                    .transitions
                    .iter()
                    .any(|transition| transition.transition_kind == TransitionKind::EncodedMapExit))
        {
            return Err(PlannerContractError::new(
                "extracted_world_facts.native_collision",
                "native inventory-set provenance cannot contain unavailable collision approaches or encoded-map transitions",
            ));
        }
        validate_sorted("encoded_exits", &self.encoded_exits, |value| {
            value.id.as_str()
        })?;
        let transition_ids = self
            .mechanics
            .transitions
            .iter()
            .map(|transition| transition.id.as_str())
            .collect::<BTreeSet<_>>();
        let mut referenced_transition_ids = BTreeSet::new();
        for exit in &self.encoded_exits {
            validate_stable_id("encoded_exits.id", &exit.id)?;
            validate_game_name("encoded_exits.source_stage", &exit.source_stage)?;
            exit.destination.validate()?;
            if exit.raw.len() != 13
                || !strictly_sorted(&exit.candidate_transition_ids)
                || exit.candidate_transition_ids.iter().any(|id| {
                    !transition_ids.contains(id.as_str())
                        || !referenced_transition_ids.insert(id.as_str())
                })
            {
                return Err(PlannerContractError::new(
                    "encoded_exits",
                    "contains invalid raw data or transition references",
                ));
            }
        }
        validate_sorted("approach_geometries", &self.approach_geometries, |value| {
            value.id.as_str()
        })?;
        let spawn_ids = self
            .spawns
            .iter()
            .map(|spawn| spawn.id.as_str())
            .collect::<BTreeSet<_>>();
        let mut approach_transition_ids = BTreeSet::new();
        for geometry in &self.approach_geometries {
            validate_stable_id("approach_geometries.id", &geometry.id)?;
            validate_stable_id("approach_geometries.transition_id", &geometry.transition_id)?;
            validate_stable_id("approach_geometries.approach_id", &geometry.approach_id)?;
            validate_game_name("approach_geometries.source_stage", &geometry.source_stage)?;
            if geometry.source_collision_id.is_empty()
                || geometry.source_collision_id.len() > 2048
                || geometry.source_inventory_sha256 == Digest::ZERO
                || !strictly_sorted(&geometry.candidate_spawn_ids)
                || geometry
                    .candidate_spawn_ids
                    .iter()
                    .any(|id| !spawn_ids.contains(id.as_str()))
            {
                return Err(PlannerContractError::new(
                    "approach_geometries",
                    "contains an invalid source or spawn reference",
                ));
            }
            if geometry.candidate_spawn_ids.iter().any(|id| {
                self.spawns
                    .iter()
                    .find(|spawn| spawn.id == *id)
                    .is_none_or(|spawn| {
                        spawn.location.stage != geometry.source_stage
                            || spawn.location.room != geometry.source_room
                    })
            }) {
                return Err(PlannerContractError::new(
                    "approach_geometries.candidate_spawn_ids",
                    "must remain in the geometry's exact source stage and room",
                ));
            }
            let Some(transition) = self
                .mechanics
                .transitions
                .iter()
                .find(|transition| transition.id == geometry.transition_id)
            else {
                return Err(PlannerContractError::new(
                    "approach_geometries.transition_id",
                    "references an unknown transition",
                ));
            };
            if transition.transition_kind != TransitionKind::EncodedMapExit
                || transition.approach_id != geometry.approach_id
                || !approach_transition_ids.insert(geometry.transition_id.as_str())
            {
                return Err(PlannerContractError::new(
                    "approach_geometries.transition_id",
                    "must uniquely reference its encoded-map transition and exact approach",
                ));
            }
            validate_approach_shape(&geometry.shape)?;
        }
        let encoded_map_transition_ids = self
            .mechanics
            .transitions
            .iter()
            .filter(|transition| transition.transition_kind == TransitionKind::EncodedMapExit)
            .map(|transition| transition.id.as_str())
            .collect::<BTreeSet<_>>();
        if approach_transition_ids != encoded_map_transition_ids {
            return Err(PlannerContractError::new(
                "approach_geometries",
                "must cover every collision-derived encoded-map transition exactly once",
            ));
        }
        let exit_transition_ids = self
            .mechanics
            .transitions
            .iter()
            .filter(|transition| {
                transition
                    .activation
                    .effects
                    .iter()
                    .any(|effect| matches!(effect, StateOperation::SetLocation { .. }))
            })
            .map(|transition| transition.id.as_str())
            .collect::<BTreeSet<_>>();
        if self.mechanics.transitions.iter().any(|transition| {
            matches!(
                transition.transition_kind,
                TransitionKind::EncodedMapExit | TransitionKind::Door
            ) && !exit_transition_ids.contains(transition.id.as_str())
        }) {
            return Err(PlannerContractError::new(
                "mechanics.transitions",
                "encoded-map and door transitions must contain an encoded location change",
            ));
        }
        if referenced_transition_ids != exit_transition_ids {
            return Err(PlannerContractError::new(
                "mechanics.transitions",
                "every location-changing transition must be referenced exactly once by its encoded exit, while transitions without a location change must not be referenced by one",
            ));
        }
        let expected_scope = ContextScope {
            selectors: vec![ContextSelector::Exact {
                context: self.exact_context.clone(),
            }],
        };
        if self
            .mechanics
            .transitions
            .iter()
            .any(|transition| transition.scope != expected_scope)
            || self
                .mechanics
                .obligations
                .iter()
                .any(|obligation| obligation.scope != expected_scope)
        {
            return Err(PlannerContractError::new(
                "mechanics.scope",
                "does not match the exact imported context",
            ));
        }
        let total = self.static_world_objects.len()
            + self
                .native_stage_metadata
                .iter()
                .map(|metadata| {
                    metadata.room_transforms.len()
                        + metadata.file_lists.len()
                        + metadata.room_reads.len()
                })
                .sum::<usize>()
            + self.spatial_volumes.len()
            + self.spatial_planes.len()
            + self.spawns.len()
            + self.encoded_exits.len()
            + self.approach_geometries.len()
            + self.mechanics.transitions.len()
            + self.mechanics.obligations.len();
        if total > MAX_EXTRACTED_WORLD_RECORDS {
            return Err(PlannerContractError::new(
                "extracted_world_facts",
                "contains too many records",
            ));
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let facts: Self = serde_json::from_slice(bytes)?;
        facts.validate()?;
        if facts.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "extracted_world_facts",
                "is not canonical JSON",
            ));
        }
        Ok(facts)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

fn import_static_object(
    stage: &str,
    placement: &PlacementRecord,
) -> Result<StaticWorldObject, PlannerContractError> {
    let raw = decode_hex(&placement.raw_hex)?;
    let id = stable_token(
        "world.object",
        &[stage.as_bytes(), placement.stable_id.as_bytes()],
    );
    let binding = match placement.scope {
        crate::world_data::SourceScope {
            kind: SourceKind::Stage,
            ..
        } => ComponentBinding::Stage {
            stage: stage.into(),
        },
        crate::world_data::SourceScope {
            kind: SourceKind::Room,
            room: Some(room),
        } => ComponentBinding::Room {
            stage: stage.into(),
            room,
        },
        _ => {
            return Err(PlannerContractError::new(
                "placement.scope",
                "has an invalid stage/room binding",
            ));
        }
    };
    let mut parameters = BTreeMap::new();
    parameters.insert("name".into(), StateValue::Text(placement.name.clone()));
    parameters.insert(
        "parameters".into(),
        StateValue::Unsigned(placement.parameters.into()),
    );
    parameters.insert(
        "set_id".into(),
        StateValue::Unsigned(placement.set_id.into()),
    );
    parameters.insert(
        "layer".into(),
        StateValue::Signed(placement.layer.map_or(-1, i64::from)),
    );
    parameters.insert(
        "position_f32_le".into(),
        StateValue::Bytes(f32_bytes([
            placement.position.x,
            placement.position.y,
            placement.position.z,
        ])),
    );
    parameters.insert(
        "angle_i16_le".into(),
        StateValue::Bytes(i16_bytes(placement.angle)),
    );
    parameters.insert("raw_record".into(), StateValue::Bytes(raw.clone()));
    parameters.insert(
        "source_record_id".into(),
        StateValue::Text(placement.stable_id.clone()),
    );
    Ok(StaticWorldObject {
        id,
        actor_type: actor_type(placement),
        placement_sha256: Digest(Sha256::digest(&raw).into()),
        binding,
        parameters,
    })
}

fn import_spawn(
    stage: &str,
    placement: &PlacementRecord,
    source_object_id: &str,
) -> Result<ExtractedSpawn, PlannerContractError> {
    let position = canonicalize_position([
        placement.position.x,
        placement.position.y,
        placement.position.z,
    ]);
    Ok(ExtractedSpawn {
        id: stable_token(
            "world.spawn",
            &[stage.as_bytes(), placement.stable_id.as_bytes()],
        ),
        source_object_id: source_object_id.into(),
        source_record_id: placement.stable_id.clone(),
        location: SceneLocation {
            stage: stage.into(),
            room: placement.scope.room.unwrap_or(-1),
            layer: placement.layer.map_or(-1, |layer| layer as i8),
            spawn: (placement.angle[2] as u16 & 0xff) as i16,
        },
        position,
        rotation: placement.angle,
        parameters: placement.parameters,
    })
}

fn source_location_guard(stage: &str, room: i8) -> PredicateExpression {
    PredicateExpression::All {
        terms: vec![
            PredicateExpression::Compare {
                left: ValueReference::LocationStage,
                operator: ComparisonOperator::Equal,
                right: ValueReference::Literal {
                    value: StateValue::Text(stage.into()),
                },
            },
            PredicateExpression::Compare {
                left: ValueReference::LocationRoom,
                operator: ComparisonOperator::Equal,
                right: ValueReference::Literal {
                    value: StateValue::Signed(room.into()),
                },
            },
        ],
    }
}

struct ImportedBossDoor {
    exit_record_id: String,
    transition: CandidateTransition,
    obligations: Vec<FeasibilityObligation>,
    spatial_volumes: Vec<SpatialVolume>,
    spatial_planes: Vec<SpatialPlane>,
}

#[derive(Clone, Copy)]
enum Gz2e01BossDoorFamily {
    L1,
    L5,
}

fn import_gz2e01_boss_door(
    inventory: &WorldInventory,
    placement: &PlacementRecord,
    scope: &ContextScope,
    inventory_sha256: Digest,
) -> Result<Option<ImportedBossDoor>, PlannerContractError> {
    if !matches!(
        placement.kind,
        PlacementKind::Actor | PlacementKind::ScaledActor
    ) {
        return Ok(None);
    }
    let Some(family) = gz2e01_boss_door_family(&placement.name) else {
        return Ok(None);
    };
    let Some(room) = placement.scope.room else {
        return Ok(None);
    };
    let write_unlock_switch = match family {
        Gz2e01BossDoorFamily::L1 => {
            if (inventory.stage == "D_MN08A" && room == 10)
                || (inventory.stage != "D_MN08A" && room == 50)
            {
                return Ok(None);
            }
            true
        }
        Gz2e01BossDoorFamily::L5 => match inventory.stage.as_str() {
            "D_MN11" => true,
            "D_MN11A" => false,
            _ => return Ok(None),
        },
    };

    let exit_index = ((placement.parameters >> 25) & 0x3f) as usize;
    let matching_exits = inventory
        .exits
        .iter()
        .filter(|exit| exit.record_index == exit_index && exit.scope.room == Some(room))
        .collect::<Vec<_>>();
    let [exit] = matching_exits.as_slice() else {
        return Ok(None);
    };

    // dSv_info_c routes switch IDs below 0x80 into dSv_memBit_c. Other
    // switch domains require their own backing-store import before their writes
    // can be claimed, so this importer deliberately leaves those placements as
    // encoded exits without a boss-door candidate.
    let switch_id = placement.angle[2] as u16 as u8;
    if switch_id >= 0x80 {
        return Ok(None);
    }
    let (switch_byte_offset, switch_mask) = memory_switch_raw_location(switch_id);
    let front_room = ((placement.parameters >> 13) & 0x3f) as u8;
    let back_room = ((placement.parameters >> 19) & 0x3f) as u8;
    let family_token = match family {
        Gz2e01BossDoorFamily::L1 => "l1",
        Gz2e01BossDoorFamily::L5 => "l5",
    };
    let token = stable_token(
        &format!("world.gz2e01.{family_token}-boss-door"),
        &[
            inventory.stage.as_bytes(),
            placement.stable_id.as_bytes(),
            exit.stable_id.as_bytes(),
        ],
    );
    let transition_id = format!("transition.{token}");
    let approach_id = format!("approach.{token}");
    let interaction_obligation_id = format!("obligation.interaction.{token}");
    let actor_obligation_id = format!("obligation.actor-state.{token}");
    let front_obligation_id = format!("obligation.front-side.{token}");
    let facing_obligation_id = format!("obligation.facing.{token}");
    let evidence = gz2e01_boss_door_evidence(family, inventory_sha256, placement, &token);
    let unknown_evidence = RuleEvidence {
        truth: TruthStatus::Unknown,
        records: evidence.records.clone(),
    };
    let boss_key_guard = PredicateExpression::Compare {
        left: ValueReference::BoundRawBits {
            component_kind: ComponentKind::DungeonMemory,
            binding: ComponentBindingReference::CurrentStage,
            byte_offset: 0x1d,
            byte_width: 1,
            mask: 0x04,
        },
        operator: ComparisonOperator::Equal,
        right: ValueReference::Literal {
            value: StateValue::Unsigned(0x04),
        },
    };
    let mut hard_guard_terms = vec![
        source_location_guard(&inventory.stage, room),
        boss_key_guard,
    ];
    if matches!(family, Gz2e01BossDoorFamily::L5) {
        hard_guard_terms.push(PredicateExpression::Compare {
            left: ValueReference::PlayerForm,
            operator: ComparisonOperator::Equal,
            right: ValueReference::Literal {
                value: StateValue::Text("human".into()),
            },
        });
    }
    let destination = SceneLocation {
        stage: exit.destination_stage.clone(),
        room: exit.destination_room,
        layer: exit.destination_layer,
        spawn: exit.destination_point,
    };

    let mut effects = Vec::new();
    if write_unlock_switch {
        effects.push(StateOperation::WriteBoundRaw {
            component_kind: ComponentKind::DungeonMemory,
            binding: ComponentBindingReference::CurrentStage,
            byte_offset: switch_byte_offset,
            mask: vec![switch_mask],
            value: vec![switch_mask],
        });
    }
    effects.push(StateOperation::SetLocation {
        location: destination.clone(),
    });
    let actor_question = match family {
        Gz2e01BossDoorFamily::L1 => {
            "Confirm the loaded boss-door resources and actor/event phases reach INIT, UNLOCK, collision release, CHG_SCENE, and restart handling without an intervening failure or interruption."
        }
        Gz2e01BossDoorFamily::L5 => {
            "Confirm the loaded L5 boss-door resources and event phases reach UNLOCK, key deletion when present, collision release, CHG_SCENE, close/end handling, and the restart-room write without an intervening failure or interruption."
        }
    };

    let object_id = stable_token(
        "world.object",
        &[inventory.stage.as_bytes(), placement.stable_id.as_bytes()],
    );
    let (spatial_volumes, spatial_planes, mut physical_obligation_ids, mut imported_obligations) =
        match family {
            Gz2e01BossDoorFamily::L1 => {
                let spatial_source_sha256 =
                    boss_door_spatial_source_digest(family, inventory_sha256, placement);
                let position = canonicalize_position([
                    placement.position.x,
                    placement.position.y,
                    placement.position.z,
                ]);
                let form_is = |form: &str| PredicateExpression::Compare {
                    left: ValueReference::PlayerForm,
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Text(form.into()),
                    },
                };
                let volume = |volume_id: &str, position: crate::transition::InteractionPosition| {
                    crate::transition::InteractionVolumeTest {
                        position,
                        volume: crate::transition::VolumeReference {
                            object_id: object_id.clone(),
                            volume_id: volume_id.into(),
                        },
                        must_be_inside: true,
                    }
                };
                (
                    vec![
                        SpatialVolume {
                            object_id: object_id.clone(),
                            volume_id: "boss-door-check-area".into(),
                            shape: SpatialVolumeShape::YawOrientedRectangle {
                                origin_xz: [position[0], position[2]],
                                yaw: placement.angle[1],
                                minimum_local_xz: [-200.0, -100.0],
                                maximum_local_xz: [200.0, 100.0],
                            },
                            source_sha256: spatial_source_sha256,
                        },
                        SpatialVolume {
                            object_id: object_id.clone(),
                            volume_id: "boss-door-wolf-current-x".into(),
                            shape: SpatialVolumeShape::YawOrientedStrip {
                                origin_xz: [position[0], position[2]],
                                yaw: placement.angle[1],
                                axis: crate::state::SpatialLocalAxis::X,
                                minimum: -130.0,
                                maximum: 130.0,
                            },
                            source_sha256: spatial_source_sha256,
                        },
                    ],
                    Vec::new(),
                    vec![
                        actor_obligation_id.clone(),
                        interaction_obligation_id.clone(),
                        facing_obligation_id.clone(),
                    ],
                    vec![
                        FeasibilityObligation {
                            id: interaction_obligation_id.clone(),
                            label: format!("Satisfy {} form-specific actor-local area checks", placement.name),
                            scope: scope.clone(),
                            obligation_kind: ObligationKind::Interaction,
                            stage: crate::transition::ObligationStage::Activate,
                            detail: ObligationDetail::CompoundInteraction {
                                actor_instance_id: object_id.clone(),
                                interaction_mode: "door".into(),
                                branches: vec![
                                    crate::transition::InteractionBranch {
                                        when: form_is("human"),
                                        volume_tests: vec![volume(
                                            "boss-door-check-area",
                                            crate::transition::InteractionPosition::Player,
                                        )],
                                        pose_predicate: PredicateExpression::True,
                                    },
                                    crate::transition::InteractionBranch {
                                        when: form_is("wolf"),
                                        volume_tests: vec![
                                            volume(
                                                "boss-door-check-area",
                                                crate::transition::InteractionPosition::PlayerAttention,
                                            ),
                                            volume(
                                                "boss-door-wolf-current-x",
                                                crate::transition::InteractionPosition::Player,
                                            ),
                                        ],
                                        pose_predicate: PredicateExpression::True,
                                    },
                                ],
                                temporal_requirement: None,
                            },
                            evidence: evidence.clone(),
                        },
                        FeasibilityObligation {
                            id: facing_obligation_id.clone(),
                            label: format!("Face {} within binary-angle delta 0x4000", placement.name),
                            scope: scope.clone(),
                            obligation_kind: ObligationKind::Interaction,
                            stage: crate::transition::ObligationStage::Activate,
                            detail: ObligationDetail::Facing {
                                yaw: ValueReference::PlayerRotationY,
                                target_yaw: placement.angle[1].wrapping_sub(0x7fff),
                                maximum_delta: 0x4000,
                            },
                            evidence: evidence.clone(),
                        },
                    ],
                )
            }
            Gz2e01BossDoorFamily::L5 => {
                let spatial_source_sha256 =
                    boss_door_spatial_source_digest(family, inventory_sha256, placement);
                let radians = f64::from(placement.angle[1]) * std::f64::consts::TAU / 65536.0;
                let (sin, cos) = radians.sin_cos();
                let normal = canonicalize_position([sin as f32, 0.0, cos as f32]);
                let position = canonicalize_position([
                    placement.position.x,
                    placement.position.y,
                    placement.position.z,
                ]);
                let offset =
                    canonicalize_scalar(-(normal[0] * position[0] + normal[2] * position[2]));
                let plane_id = format!("plane.front.{token}");
                (
                    vec![SpatialVolume {
                        object_id: object_id.clone(),
                        volume_id: "boss-door-check-area".into(),
                        shape: SpatialVolumeShape::YawOrientedRectangle {
                            origin_xz: [position[0], position[2]],
                            yaw: placement.angle[1],
                            minimum_local_xz: [-200.0, -100.0],
                            maximum_local_xz: [200.0, 100.0],
                        },
                        source_sha256: spatial_source_sha256,
                    }],
                    vec![SpatialPlane {
                        plane_id: plane_id.clone(),
                        normal,
                        offset,
                        source_sha256: spatial_source_sha256,
                    }],
                    vec![
                        actor_obligation_id.clone(),
                        interaction_obligation_id.clone(),
                        front_obligation_id.clone(),
                        facing_obligation_id.clone(),
                    ],
                    vec![
                        FeasibilityObligation {
                            id: interaction_obligation_id.clone(),
                            label: format!(
                                "Stand within {} actor-local checkArea rectangle",
                                placement.name
                            ),
                            scope: scope.clone(),
                            obligation_kind: ObligationKind::Interaction,
                            stage: crate::transition::ObligationStage::Activate,
                            detail: ObligationDetail::Interaction {
                                actor_instance_id: object_id.clone(),
                                interaction_mode: "door".into(),
                                required_volumes: vec![crate::transition::VolumeReference {
                                    object_id: object_id.clone(),
                                    volume_id: "boss-door-check-area".into(),
                                }],
                                excluded_volumes: Vec::new(),
                                pose_predicate: PredicateExpression::True,
                                temporal_requirement: None,
                            },
                            evidence: evidence.clone(),
                        },
                        FeasibilityObligation {
                            id: front_obligation_id.clone(),
                            label: format!(
                                "Approach {} from positive actor-local Z",
                                placement.name
                            ),
                            scope: scope.clone(),
                            obligation_kind: ObligationKind::Geometry,
                            stage: crate::transition::ObligationStage::Reach,
                            detail: ObligationDetail::PlaneSide {
                                plane_id,
                                relation: crate::state::PlaneRelation::Positive,
                            },
                            evidence: evidence.clone(),
                        },
                        FeasibilityObligation {
                            id: facing_obligation_id.clone(),
                            label: format!(
                                "Face {} within binary-angle delta 0x4000",
                                placement.name
                            ),
                            scope: scope.clone(),
                            obligation_kind: ObligationKind::Interaction,
                            stage: crate::transition::ObligationStage::Activate,
                            detail: ObligationDetail::Facing {
                                yaw: ValueReference::PlayerRotationY,
                                target_yaw: placement.angle[1].wrapping_sub(0x7fff),
                                maximum_delta: 0x4000,
                            },
                            evidence: evidence.clone(),
                        },
                    ],
                )
            }
        };
    physical_obligation_ids.sort();
    imported_obligations.push(FeasibilityObligation {
        id: actor_obligation_id.clone(),
        label: format!(
            "Run the loaded {} keyhole, event, collision, and scene-change phases",
            placement.name
        ),
        scope: scope.clone(),
        obligation_kind: ObligationKind::ActorState,
        stage: crate::transition::ObligationStage::Effect,
        detail: ObligationDetail::Unresolved {
            research_question: actor_question.into(),
        },
        evidence: unknown_evidence.clone(),
    });
    imported_obligations.sort_by(|left, right| left.id.cmp(&right.id));

    Ok(Some(ImportedBossDoor {
        exit_record_id: exit.stable_id.clone(),
        transition: CandidateTransition {
            id: transition_id,
            label: format!(
                "{} {} room {} boss door (front {}, back {}, exit {}) to {} room {} point {}",
                inventory.stage,
                placement.name,
                room,
                front_room,
                back_room,
                exit_index,
                destination.stage,
                destination.room,
                destination.spawn
            ),
            scope: scope.clone(),
            transition_kind: TransitionKind::Door,
            approach_id,
            activation: ActivationContract {
                hard_guards: PredicateExpression::All {
                    terms: hard_guard_terms,
                },
                physical_obligation_ids,
                effects,
                unknown_requirements: Vec::new(),
            },
            evidence,
        },
        obligations: imported_obligations,
        spatial_volumes,
        spatial_planes,
    }))
}

fn boss_door_spatial_source_digest(
    family: Gz2e01BossDoorFamily,
    inventory_sha256: Digest,
    placement: &PlacementRecord,
) -> Digest {
    let source_sha256 = match family {
        Gz2e01BossDoorFamily::L1 => {
            static_digest("221c170e034cf90cc43b20dc737bebeb44d6f8b54111d4454024f2fea7069d79")
        }
        Gz2e01BossDoorFamily::L5 => {
            static_digest("9f649b99f027e39f1d39ce066d815a78032b536c4a9a83e0361681af2265102e")
        }
    };
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.route-planner.boss-door-spatial-source/v1");
    hasher.update(inventory_sha256.0);
    hasher.update(source_sha256.0);
    hasher.update(placement.stable_id.as_bytes());
    Digest(hasher.finalize().into())
}

fn gz2e01_boss_door_family(name: &str) -> Option<Gz2e01BossDoorFamily> {
    match name {
        "L1Bdoor" | "L2Bdoor" | "L4Bdoor" | "L6Bdoor" | "L7Bdoor" | "L8Bdoor" | "L9Bdoor" => {
            Some(Gz2e01BossDoorFamily::L1)
        }
        "L5Bdoor" => Some(Gz2e01BossDoorFamily::L5),
        _ => None,
    }
}

fn memory_switch_raw_location(switch_id: u8) -> (u32, u8) {
    let word = u32::from(switch_id / 32);
    let bit_in_word = switch_id % 32;
    let byte_in_word = 3 - u32::from(bit_in_word / 8);
    let byte_offset = 0x08 + word * 4 + byte_in_word;
    (byte_offset, 1_u8 << (bit_in_word % 8))
}

fn is_source_audited_gz2e01(content: &ContentIdentity) -> bool {
    crate::orig_discovery::bundled_supported_build_registry()
        .ok()
        .and_then(|registry| {
            registry
                .identities
                .into_iter()
                .find(|identity| identity.id == "gcn-us-1.0-gz2e01")
        })
        .is_some_and(|identity| identity.fingerprint == content.fingerprint)
}

fn gz2e01_boss_door_evidence(
    family: Gz2e01BossDoorFamily,
    inventory_sha256: Digest,
    placement: &PlacementRecord,
    token: &str,
) -> RuleEvidence {
    let (actor_sha256, actor_note) = match family {
        Gz2e01BossDoorFamily::L1 => (
            "221c170e034cf90cc43b20dc737bebeb44d6f8b54111d4454024f2fea7069d79",
            "d_a_door_bossL1.cpp: boss-key/front/area offer guards; unlock switch, event phases, collision release, and scene-change behavior.",
        ),
        Gz2e01BossDoorFamily::L5 => (
            "9f649b99f027e39f1d39ce066d815a78032b536c4a9a83e0361681af2265102e",
            "d_a_door_bossL5.cpp: human/boss-key/front/area guards; first-unlock switch, keyhole/event phases, collision release, scene change, and restart behavior.",
        ),
    };
    RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![
            EvidenceRecord {
                id: format!("evidence.source.actor.{token}"),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(static_digest(actor_sha256)),
                note: actor_note.into(),
            },
            EvidenceRecord {
                id: format!("evidence.source.name-map.{token}"),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(static_digest(
                    "5c46ffc79e891b59b02455b837d9966d05c147d8d95c91c65cc845dd848d32ad",
                )),
                note: "d_stage.cpp: boss-door placement names map to their exact actor process families.".into(),
            },
            EvidenceRecord {
                id: format!("evidence.source.parameters.{token}"),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(static_digest(
                    "b0dacfc4b9c46786d73a840e55385e535364b9fee7de66cd0e2af18f25d1ca78",
                )),
                note: "d_door_param2.cpp: front/back room, exit number, and unlock-switch parameter decoding.".into(),
            },
            EvidenceRecord {
                id: format!("evidence.source.save-layout.{token}"),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(static_digest(
                    "74a211e5d2ee2c0fe4ce259905fe1f479f373d5b2459d654871cbbd2f61e8756",
                )),
                note: "d_save.h: dSv_memBit_c switch array, key count, and dungeon-item backing layout.".into(),
            },
            EvidenceRecord {
                id: format!("evidence.world.inventory.{token}"),
                kind: EvidenceKind::Extracted,
                source_sha256: Some(inventory_sha256),
                note: format!(
                    "Authenticated world inventory placement {} from resource {}.",
                    placement.stable_id, placement.source_sha256
                ),
            },
        ],
    }
}

struct ImportedKeyedActorActions {
    exit_record_id: Option<String>,
    transitions: Vec<CandidateTransition>,
    obligations: Vec<FeasibilityObligation>,
}

fn import_gz2e01_keyed_actor_actions(
    inventory: &WorldInventory,
    placement: &PlacementRecord,
    scope: &ContextScope,
    inventory_sha256: Digest,
) -> Result<Option<ImportedKeyedActorActions>, PlannerContractError> {
    if !matches!(
        placement.kind,
        PlacementKind::Actor | PlacementKind::ScaledActor
    ) {
        return Ok(None);
    }
    match placement.name.as_str() {
        "L6Mdoor" | "L7door" | "L8Mdoor" => {
            import_gz2e01_keyed_mboss_door(inventory, placement, scope, inventory_sha256)
        }
        "kshtr00" | "L3Bdoor" => {
            import_gz2e01_key_shutter(inventory, placement, scope, inventory_sha256)
        }
        "vshuter" => {
            import_gz2e01_external_switch_shutter(inventory, placement, scope, inventory_sha256)
        }
        "Wchain" => import_gz2e01_wolf_chain_switch(inventory, placement, scope, inventory_sha256),
        "K_Gate" => import_gz2e01_koki_gate(inventory, placement, scope, inventory_sha256),
        "R_Gate" => import_gz2e01_rider_gate(inventory, placement, scope, inventory_sha256),
        "CrvGate" => import_gz2e01_caravan_gate(inventory, placement, scope, inventory_sha256),
        _ => Ok(None),
    }
}

fn import_gz2e01_keyed_mboss_door(
    inventory: &WorldInventory,
    placement: &PlacementRecord,
    scope: &ContextScope,
    inventory_sha256: Digest,
) -> Result<Option<ImportedKeyedActorActions>, PlannerContractError> {
    let Some(room) = placement.scope.room else {
        return Ok(None);
    };
    let front_option = ((placement.parameters >> 8) & 0x03) as u8;
    let front_room = ((placement.parameters >> 13) & 0x3f) as u8;
    if front_option != 2 || room == 51 || room == 52 || front_room != room as u8 {
        return Ok(None);
    }
    let switch_id = placement.angle[2] as u16 as u8;
    if switch_id >= 0x80 {
        return Ok(None);
    }
    let exit_index = ((placement.parameters >> 25) & 0x3f) as usize;
    let matching_exits = inventory
        .exits
        .iter()
        .filter(|exit| exit.record_index == exit_index && exit.scope.room == Some(room))
        .collect::<Vec<_>>();
    let [exit] = matching_exits.as_slice() else {
        return Ok(None);
    };
    let (event_sha256, event_note) = match placement.name.as_str() {
        "L6Mdoor" => (
            "fd5570eca9bd29ee1b433236a10945872930fbf52c2508af9ff2c3f7ea9386fe",
            "L3MBdoor/event_list.dat: DEFAULT_MBS_SHUTTER_L3_F reaches UNLOCK before OPEN and CHG_SCENE.",
        ),
        "L7door" => (
            "7de6bfac10e3ca6c3f6bc88a83815972d3397fd3488b067398cdd8cb0ea0cce4",
            "L7MBdoor/event_list.dat: DEFAULT_MBS_SHUTTER_L7_F reaches UNLOCK before OPEN and CHG_SCENE.",
        ),
        "L8Mdoor" => (
            "b079b8b284208582d9a37b50bd94f13400530abca75db0771147a646a8d83627",
            "L8MBdoor/event_list.dat: DEFAULT_MBS_SHUTTER_L8_F reaches UNLOCK before OPEN and CHG_SCENE.",
        ),
        _ => return Ok(None),
    };
    let family = "keyed-mboss-door";
    let base_token = stable_token(
        "keyed",
        &[
            family.as_bytes(),
            inventory.stage.as_bytes(),
            placement.stable_id.as_bytes(),
        ],
    );
    let evidence = keyed_actor_evidence(
        family,
        inventory_sha256,
        placement,
        "94b00ab791e96a5738a0c2ef94945461c4e930b6128fc5a16d13630da9d1dff2",
        "d_a_door_mbossL1.cpp: front-side option-2 key guard, one-time switch write/key decrement, event/collision phases, and scene change.",
        event_sha256,
        event_note,
        true,
        &base_token,
    );
    let (obligations, obligation_ids) = keyed_actor_obligations(
        scope,
        placement,
        &base_token,
        &evidence,
        "Confirm resources, keyhole when present, the selected retail event cuts, collision release, CHG_SCENE, restart handling, and an uncontended queued key-delta commit complete without interruption.",
        "Reach the authored front side inside |x| <= 130 and |z| <= 110 with the required facing; wolf attention/current-position checks remain part of the physical witness.",
    );
    let location_guard = placement_location_guard(inventory, placement, room);
    let destination = SceneLocation {
        stage: exit.destination_stage.clone(),
        room: exit.destination_room,
        layer: exit.destination_layer,
        spawn: exit.destination_point,
    };
    let first_open = keyed_actor_candidate(
        scope,
        placement,
        family,
        "first-open",
        &format!(
            "{} {} room {} first keyed opening to {} room {} point {}",
            inventory.stage,
            placement.name,
            room,
            destination.stage,
            destination.room,
            destination.spawn
        ),
        TransitionKind::Door,
        PredicateExpression::All {
            terms: vec![
                location_guard.clone(),
                memory_switch_guard(switch_id, false),
                small_key_guard(ComparisonOperator::GreaterThan, 0),
                small_key_guard(ComparisonOperator::LessThanOrEqual, 100),
            ],
        },
        vec![
            memory_switch_write(switch_id),
            small_key_adjust(-1),
            StateOperation::SetLocation {
                location: destination.clone(),
            },
        ],
        &obligation_ids,
        &evidence,
    );
    let first_open_high_key = keyed_actor_candidate(
        scope,
        placement,
        family,
        "first-open-high-key-clamp",
        &format!(
            "{} {} room {} first keyed opening from a high raw key count to {} room {} point {}",
            inventory.stage,
            placement.name,
            room,
            destination.stage,
            destination.room,
            destination.spawn
        ),
        TransitionKind::Door,
        PredicateExpression::All {
            terms: vec![
                location_guard.clone(),
                memory_switch_guard(switch_id, false),
                small_key_guard(ComparisonOperator::GreaterThan, 100),
            ],
        },
        vec![
            memory_switch_write(switch_id),
            small_key_write(99),
            StateOperation::SetLocation {
                location: destination.clone(),
            },
        ],
        &obligation_ids,
        &evidence,
    );
    let reopen = keyed_actor_candidate(
        scope,
        placement,
        family,
        "reopen",
        &format!(
            "{} {} room {} already-unlocked opening to {} room {} point {}",
            inventory.stage,
            placement.name,
            room,
            destination.stage,
            destination.room,
            destination.spawn
        ),
        TransitionKind::Door,
        PredicateExpression::All {
            terms: vec![location_guard, memory_switch_guard(switch_id, true)],
        },
        vec![StateOperation::SetLocation {
            location: destination,
        }],
        &obligation_ids,
        &evidence,
    );
    Ok(Some(ImportedKeyedActorActions {
        exit_record_id: Some(exit.stable_id.clone()),
        transitions: vec![first_open, first_open_high_key, reopen],
        obligations,
    }))
}

fn import_gz2e01_key_shutter(
    inventory: &WorldInventory,
    placement: &PlacementRecord,
    scope: &ContextScope,
    inventory_sha256: Digest,
) -> Result<Option<ImportedKeyedActorActions>, PlannerContractError> {
    let Some(room) = placement.scope.room else {
        return Ok(None);
    };
    let checks_key = placement.parameters >> 31 != 0;
    let authored_type = ((placement.parameters >> 8) & 0xff) as u8;
    let runtime_type = authored_type.wrapping_add(1);
    let supported_type = match (placement.name.as_str(), runtime_type) {
        ("kshtr00", 0) => Some((
            false,
            "8676effbd561ba65f8e4a8b9493aa6b60072d40f72a8e240b2ffa9c5550b40fa",
            "S_shut00/event_list.dat: KEY_JAIL_00 and its wolf variant both contain UNLOCK before OPEN.",
        )),
        ("kshtr00", 2) => Some((
            false,
            "3bff3ce52a0c1660d5ccf0bdcae24b672e50013317b3469698c51e32336c159a",
            "Lv3shut00/event_list.dat: KEY_JAIL_01 and its wolf variant both contain UNLOCK before OPEN.",
        )),
        ("L3Bdoor", 3) => Some((
            true,
            "2184efba5db7b458f01c50534e29ba072fcb58be5e3b6df8f92e35b758726440",
            "K_l3bdoor/event_list.dat: DEFAULT_BS_SHUTTER_L3_F contains UNLOCK before OPEN.",
        )),
        _ => None,
    };
    let Some((uses_boss_key, event_sha256, event_note)) = supported_type else {
        return Ok(None);
    };
    if !checks_key {
        return Ok(None);
    }
    let switch_id = placement.parameters as u8;
    if switch_id >= 0x80 {
        return Ok(None);
    }
    let family = if uses_boss_key {
        "lakebed-boss-key-shutter"
    } else {
        "key-shutter"
    };
    let base_token = stable_token(
        "keyed",
        &[
            family.as_bytes(),
            inventory.stage.as_bytes(),
            placement.stable_id.as_bytes(),
        ],
    );
    let evidence = keyed_actor_evidence(
        family,
        inventory_sha256,
        placement,
        "dca04961403031ef232059f5f9f8997d2f0a3965b111e97d9d72604e0014d14b",
        "d_a_obj_kshutter.cpp: type/check-key decoder, small-key or boss-key offer guard, acceptance switch write, UNLOCK key delta, and collision/open phases.",
        event_sha256,
        event_note,
        false,
        &base_token,
    );
    let (obligations, obligation_ids) = keyed_actor_obligations(
        scope,
        placement,
        &base_token,
        &evidence,
        "Confirm resources, keyhole when present, accepted event, UNLOCK/OPEN cuts, collision release, and an uncontended queued key-delta commit complete without interruption.",
        "Reach the actor's bounded interaction area with the required facing; retain the human/wolf event choice as part of the witness.",
    );
    let location_guard = placement_location_guard(inventory, placement, room);
    let mut transitions = Vec::new();
    if uses_boss_key {
        transitions.push(keyed_actor_candidate(
            scope,
            placement,
            family,
            "open-with-small-key",
            &format!(
                "{} {} room {} boss-key opening with incidental small-key decrement",
                inventory.stage, placement.name, room
            ),
            TransitionKind::ActorDriven,
            PredicateExpression::All {
                terms: vec![
                    location_guard.clone(),
                    memory_switch_guard(switch_id, false),
                    boss_key_guard(),
                    small_key_guard(ComparisonOperator::GreaterThan, 0),
                    small_key_guard(ComparisonOperator::LessThanOrEqual, 100),
                ],
            },
            vec![memory_switch_write(switch_id), small_key_adjust(-1)],
            &obligation_ids,
            &evidence,
        ));
        transitions.push(keyed_actor_candidate(
            scope,
            placement,
            family,
            "open-with-high-small-key-clamp",
            &format!(
                "{} {} room {} boss-key opening with high raw small keys clamped to 99",
                inventory.stage, placement.name, room
            ),
            TransitionKind::ActorDriven,
            PredicateExpression::All {
                terms: vec![
                    location_guard.clone(),
                    memory_switch_guard(switch_id, false),
                    boss_key_guard(),
                    small_key_guard(ComparisonOperator::GreaterThan, 100),
                ],
            },
            vec![memory_switch_write(switch_id), small_key_write(99)],
            &obligation_ids,
            &evidence,
        ));
        transitions.push(keyed_actor_candidate(
            scope,
            placement,
            family,
            "open-with-zero-small-keys",
            &format!(
                "{} {} room {} boss-key opening with clamped zero small keys",
                inventory.stage, placement.name, room
            ),
            TransitionKind::ActorDriven,
            PredicateExpression::All {
                terms: vec![
                    location_guard,
                    memory_switch_guard(switch_id, false),
                    boss_key_guard(),
                    small_key_guard(ComparisonOperator::Equal, 0),
                ],
            },
            vec![memory_switch_write(switch_id)],
            &obligation_ids,
            &evidence,
        ));
    } else {
        transitions.push(keyed_actor_candidate(
            scope,
            placement,
            family,
            "unlock",
            &format!(
                "{} {} room {} keyed shutter unlock",
                inventory.stage, placement.name, room
            ),
            TransitionKind::ActorDriven,
            PredicateExpression::All {
                terms: vec![
                    location_guard,
                    memory_switch_guard(switch_id, false),
                    small_key_guard(ComparisonOperator::GreaterThan, 0),
                    small_key_guard(ComparisonOperator::LessThanOrEqual, 100),
                ],
            },
            vec![memory_switch_write(switch_id), small_key_adjust(-1)],
            &obligation_ids,
            &evidence,
        ));
        transitions.push(keyed_actor_candidate(
            scope,
            placement,
            family,
            "unlock-high-key-clamp",
            &format!(
                "{} {} room {} keyed shutter unlock with high raw keys clamped to 99",
                inventory.stage, placement.name, room
            ),
            TransitionKind::ActorDriven,
            PredicateExpression::All {
                terms: vec![
                    placement_location_guard(inventory, placement, room),
                    memory_switch_guard(switch_id, false),
                    small_key_guard(ComparisonOperator::GreaterThan, 100),
                ],
            },
            vec![memory_switch_write(switch_id), small_key_write(99)],
            &obligation_ids,
            &evidence,
        ));
    }
    Ok(Some(ImportedKeyedActorActions {
        exit_record_id: None,
        transitions,
        obligations,
    }))
}

fn import_gz2e01_external_switch_shutter(
    inventory: &WorldInventory,
    placement: &PlacementRecord,
    scope: &ContextScope,
    inventory_sha256: Digest,
) -> Result<Option<ImportedKeyedActorActions>, PlannerContractError> {
    let Some(room) = placement.scope.room else {
        return Ok(None);
    };
    let switch_id = placement.parameters as u8;
    let runtime_type = ((placement.parameters >> 8) as u8).wrapping_add(1);
    let checks_key = placement.parameters >> 31 != 0;
    if inventory.stage != "R_SP116"
        || room != 6
        || switch_id != 0xef
        || runtime_type != 4
        || checks_key
    {
        return Ok(None);
    }

    let family = "external-switch-shutter";
    let token = stable_token(
        "keyed",
        &[
            family.as_bytes(),
            inventory.stage.as_bytes(),
            placement.stable_id.as_bytes(),
        ],
    );
    let evidence = RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![
            EvidenceRecord {
                id: format!("evidence.source.actor.{token}"),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(static_digest(
                    "dca04961403031ef232059f5f9f8997d2f0a3965b111e97d9d72604e0014d14b",
                )),
                note: "d_a_obj_kshutter.cpp: runtime type 4 has no internal key check or switch writer and opens after its external switch becomes set.".into(),
            },
            EvidenceRecord {
                id: format!("evidence.source.switch-dispatch.{token}"),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(static_digest(
                    "a275457390b8464750adaab345c769afa2dc0b295baba47a617ce6aad6fd26d3",
                )),
                note: "d_save.cpp: switch 0xef resolves through the current room's one-zone switch store.".into(),
            },
            EvidenceRecord {
                id: format!("evidence.world.inventory.{token}"),
                kind: EvidenceKind::Extracted,
                source_sha256: Some(inventory_sha256),
                note: format!(
                    "Authenticated R_SP116 room-6 vshuter placement {} with external one-zone switch 0xef.",
                    placement.stable_id
                ),
            },
        ],
    };
    let unknown_evidence = RuleEvidence {
        truth: TruthStatus::Unknown,
        records: evidence.records.clone(),
    };
    let actor_id = format!("obligation.actor-state.{token}");
    let passage_id = format!("obligation.interaction.{token}.passage");
    let obligations = vec![
        FeasibilityObligation {
            id: actor_id.clone(),
            label: format!("Observe {} respond to external switch 0xef", placement.name),
            scope: scope.clone(),
            obligation_kind: ObligationKind::ActorState,
            stage: crate::transition::ObligationStage::Activate,
            detail: ObligationDetail::Unresolved {
                research_question: "Confirm the loaded type-4 shutter observes the already-set external switch and completes its opening/collision-release phases.".into(),
            },
            evidence: unknown_evidence.clone(),
        },
        FeasibilityObligation {
            id: passage_id.clone(),
            label: format!("Traverse the externally opened {}", placement.name),
            scope: scope.clone(),
            obligation_kind: ObligationKind::Interaction,
            stage: crate::transition::ObligationStage::Activate,
            detail: ObligationDetail::Unresolved {
                research_question: "Witness sufficient shutter opening and background-collision release for passage.".into(),
            },
            evidence: unknown_evidence,
        },
    ];
    let transition = keyed_actor_candidate(
        scope,
        placement,
        family,
        "set-switch-passage",
        "R_SP116 vshuter room 6 externally switched passage",
        TransitionKind::ActorDriven,
        PredicateExpression::All {
            terms: vec![
                placement_location_guard(inventory, placement, room),
                room_switch_label_guard(switch_id, true),
            ],
        },
        Vec::new(),
        &[actor_id, passage_id],
        &evidence,
    );
    Ok(Some(ImportedKeyedActorActions {
        exit_record_id: None,
        transitions: vec![transition],
        obligations,
    }))
}

fn import_gz2e01_wolf_chain_switch(
    inventory: &WorldInventory,
    placement: &PlacementRecord,
    scope: &ContextScope,
    inventory_sha256: Digest,
) -> Result<Option<ImportedKeyedActorActions>, PlannerContractError> {
    let Some(room) = placement.scope.room else {
        return Ok(None);
    };
    let switch_id = placement.parameters as u8;
    let authored_repeat = ((placement.parameters >> 8) & 0x0f) as u8;
    let repeatable = authored_repeat != 0 && authored_repeat != 0x0f;
    if inventory.stage != "R_SP116"
        || room != 6
        || switch_id != 0xef
        || authored_repeat != 0x0f
        || repeatable
    {
        return Ok(None);
    }

    let family = "wolf-chain-switch";
    let token = stable_token(
        "keyed",
        &[
            family.as_bytes(),
            inventory.stage.as_bytes(),
            placement.stable_id.as_bytes(),
        ],
    );
    let evidence = RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![
            EvidenceRecord {
                id: format!("evidence.source.actor.{token}"),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(static_digest(
                    "e72a2bfcc715f03d1fa934a2033e4360aa22fbfd2ffd4c962cb7a27c949b7fd0",
                )),
                note: "d_a_obj_wchain.cpp: low parameter byte selects the switch; authored repeat nibble 0xf normalizes to one-shot; onNowSwitch writes the clear switch on the next chain execute.".into(),
            },
            EvidenceRecord {
                id: format!("evidence.source.player.{token}"),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(static_digest(
                    "b0c094b0c95144d7c5f89bc1d35d63fcde80f1f032a7772670a8142eb4dc9d8d",
                )),
                note: "d_a_alink_wolf.inc: wolf chain ready/wait state attaches to Wchain and raises onNowSwitch after pull length exceeds the exact 94-unit switch offset.".into(),
            },
            EvidenceRecord {
                id: format!("evidence.source.switch-dispatch.{token}"),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(static_digest(
                    "a275457390b8464750adaab345c769afa2dc0b295baba47a617ce6aad6fd26d3",
                )),
                note: "d_save.cpp: switch 0xef resolves through the current room's one-zone switch store.".into(),
            },
            EvidenceRecord {
                id: format!("evidence.world.inventory.{token}"),
                kind: EvidenceKind::Extracted,
                source_sha256: Some(inventory_sha256),
                note: format!(
                    "Authenticated R_SP116 room-6 Wchain placement {} with parameters 0x00000fef.",
                    placement.stable_id
                ),
            },
        ],
    };
    let unknown_evidence = RuleEvidence {
        truth: TruthStatus::Unknown,
        records: evidence.records.clone(),
    };
    let interaction_id = format!("obligation.interaction.{token}");
    let effect_id = format!("obligation.actor-state.{token}");
    let obligations = vec![
        FeasibilityObligation {
            id: interaction_id.clone(),
            label: "Reach, bite, and pull the R_SP116 room-6 wolf chain".into(),
            scope: scope.clone(),
            obligation_kind: ObligationKind::Interaction,
            stage: crate::transition::ObligationStage::Activate,
            detail: ObligationDetail::Unresolved {
                research_question: "Reach the chain as wolf, acquire its attention target, complete the ready jump without a wall hit, remain attached through the tension wait, and pull past the exact 94-unit switch offset.".into(),
            },
            evidence: unknown_evidence.clone(),
        },
        FeasibilityObligation {
            id: effect_id.clone(),
            label: "Commit the R_SP116 Wchain one-shot switch write".into(),
            scope: scope.clone(),
            obligation_kind: ObligationKind::ActorState,
            stage: crate::transition::ObligationStage::Effect,
            detail: ObligationDetail::Unresolved {
                research_question: "Confirm onNowSwitch survives into the chain actor's next execute, writes one-zone switch 0xef, and completes without an intervening unload.".into(),
            },
            evidence: unknown_evidence,
        },
    ];
    let transition = keyed_actor_candidate(
        scope,
        placement,
        family,
        "wolf-pull-switch",
        "R_SP116 room 6 wolf-chain pull sets one-zone switch 0xef",
        TransitionKind::ActorDriven,
        PredicateExpression::All {
            terms: vec![
                placement_location_guard(inventory, placement, room),
                PredicateExpression::Compare {
                    left: ValueReference::PlayerForm,
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Text("wolf".into()),
                    },
                },
                room_switch_label_guard(switch_id, false),
            ],
        },
        vec![room_switch_label_write(switch_id, true)],
        &[effect_id, interaction_id],
        &evidence,
    );
    Ok(Some(ImportedKeyedActorActions {
        exit_record_id: None,
        transitions: vec![transition],
        obligations,
    }))
}

fn import_gz2e01_koki_gate(
    inventory: &WorldInventory,
    placement: &PlacementRecord,
    scope: &ContextScope,
    inventory_sha256: Digest,
) -> Result<Option<ImportedKeyedActorActions>, PlannerContractError> {
    let Some(room) = placement.scope.room else {
        return Ok(None);
    };
    let name_argument = ((placement.parameters >> 16) & 0x0f) as u8;
    let switch_id = placement.parameters as u8;
    if name_argument != 0 || switch_id >= 0x80 {
        return Ok(None);
    }
    let family = "koki-gate";
    let base_token = stable_token(
        "keyed",
        &[
            family.as_bytes(),
            inventory.stage.as_bytes(),
            placement.stable_id.as_bytes(),
        ],
    );
    let evidence = keyed_actor_evidence(
        family,
        inventory_sha256,
        placement,
        "55696f32a444f9fde4b446442211cc3bed8b2872c8b05d7646001bd3659879e8",
        "d_a_obj_kgate.cpp: type-0 switch/key offer guard, accepted-door key delta and switch write, live push/open behavior, and set-switch reload reconstruction.",
        "c8684156665423d1a133dc0b102098d8ec3be838dd6abe117dbafedf0144ab83",
        "D_KGate00/event_list.dat: KOKI_GATE_OPEN00 contains UNLOCK before OPEN.",
        false,
        &base_token,
    );
    let (obligations, obligation_ids) = keyed_actor_obligations(
        scope,
        placement,
        &base_token,
        &evidence,
        "Confirm resources, accepted door command, event cuts, an uncontended queued key-delta commit, and the unlocked gate's physical open/push behavior complete without interruption.",
        "Reach local x in [-100, 100], z in [0, 100], with the actor/player facing delta required by checkOpen().",
    );
    let transition = keyed_actor_candidate(
        scope,
        placement,
        family,
        "unlock",
        &format!(
            "{} {} room {} keyed gate unlock",
            inventory.stage, placement.name, room
        ),
        TransitionKind::ActorDriven,
        PredicateExpression::All {
            terms: vec![
                placement_location_guard(inventory, placement, room),
                memory_switch_guard(switch_id, false),
                small_key_guard(ComparisonOperator::GreaterThan, 0),
                small_key_guard(ComparisonOperator::LessThanOrEqual, 100),
            ],
        },
        vec![small_key_adjust(-1), memory_switch_write(switch_id)],
        &obligation_ids,
        &evidence,
    );
    let high_key_transition = keyed_actor_candidate(
        scope,
        placement,
        family,
        "unlock-high-key-clamp",
        &format!(
            "{} {} room {} keyed gate unlock with high raw keys clamped to 99",
            inventory.stage, placement.name, room
        ),
        TransitionKind::ActorDriven,
        PredicateExpression::All {
            terms: vec![
                placement_location_guard(inventory, placement, room),
                memory_switch_guard(switch_id, false),
                small_key_guard(ComparisonOperator::GreaterThan, 100),
            ],
        },
        vec![small_key_write(99), memory_switch_write(switch_id)],
        &obligation_ids,
        &evidence,
    );
    Ok(Some(ImportedKeyedActorActions {
        exit_record_id: None,
        transitions: vec![transition, high_key_transition],
        obligations,
    }))
}

fn import_gz2e01_rider_gate(
    inventory: &WorldInventory,
    placement: &PlacementRecord,
    scope: &ContextScope,
    inventory_sha256: Digest,
) -> Result<Option<ImportedKeyedActorActions>, PlannerContractError> {
    let Some(room) = placement.scope.room else {
        return Ok(None);
    };
    let switch_id = placement.parameters as u8;
    let (switch_guard, switch_write, switch_description) =
        match (inventory.stage.as_str(), room, switch_id) {
            ("F_SP109", 0, 0x6b) => (
                memory_switch_guard as fn(u8, bool) -> PredicateExpression,
                memory_switch_write as fn(u8) -> StateOperation,
                "memory switch 0x6b",
            ),
            ("F_SP121", 3, 0x82) | ("F_SP121", 15, 0x81) => (
                dungeon_session_switch_guard as fn(u8, bool) -> PredicateExpression,
                dungeon_session_switch_write as fn(u8) -> StateOperation,
                if switch_id == 0x82 {
                    "dungeon-session switch 0x82"
                } else {
                    "dungeon-session switch 0x81"
                },
            ),
            _ => return Ok(None),
        };
    let family = "rider-gate";
    let token = stable_token(
        "keyed",
        &[
            family.as_bytes(),
            inventory.stage.as_bytes(),
            placement.stable_id.as_bytes(),
        ],
    );
    let mut evidence_records = vec![
        EvidenceRecord {
            id: format!("evidence.source.actor.{token}"),
            kind: EvidenceKind::SourceAudited,
            source_sha256: Some(static_digest(
                "eb644962c9c9596514d552e2f87015f1c68786bf998ff79d41a606276750bffb",
            )),
            note: "d_a_obj_rgate.cpp: key/facing/box offer guard, queued decrement, switch writer, M_035 bypass, event completion, and post-unlock pushing.".into(),
        },
        EvidenceRecord {
            id: format!("evidence.source.event.{token}"),
            kind: EvidenceKind::Extracted,
            source_sha256: Some(static_digest(
                "95582d74d858aeb5b01a9f1beb6c0c1bd6761b619b75f57d1d60d906f73ea856",
            )),
            note: "M_RGate00/event_list.dat: RIDER_GATE_OPEN00 contains UNLOCK before OPEN.".into(),
        },
        EvidenceRecord {
            id: format!("evidence.source.event-label.{token}"),
            kind: EvidenceKind::SourceAudited,
            source_sha256: Some(static_digest(
                "8804d987bb1da08281c143d96d46a2832f02650f4b9777b010f196ed20847a14",
            )),
            note: "d_save_bit_labels.inc: saveBitLabels[68] is M_035 at packed persistent-event coordinate 0x0810.".into(),
        },
        EvidenceRecord {
            id: format!("evidence.source.name-map.{token}"),
            kind: EvidenceKind::SourceAudited,
            source_sha256: Some(static_digest(
                "5c46ffc79e891b59b02455b837d9966d05c147d8d95c91c65cc845dd848d32ad",
            )),
            note: "d_stage.cpp: R_Gate maps to the rider-gate actor process.".into(),
        },
        EvidenceRecord {
            id: format!("evidence.source.switch-dispatch.{token}"),
            kind: EvidenceKind::SourceAudited,
            source_sha256: Some(static_digest(
                "a275457390b8464750adaab345c769afa2dc0b295baba47a617ce6aad6fd26d3",
            )),
            note: "d_save.cpp: switches 0x80 through 0xbf resolve to the 64-bit dungeon-session switch store; lower switches resolve to loaded stage memory.".into(),
        },
        EvidenceRecord {
            id: format!("evidence.world.inventory.{token}"),
            kind: EvidenceKind::Extracted,
            source_sha256: Some(inventory_sha256),
            note: format!(
                "Authenticated {} room-{} layered rider-gate placement {} with {}.",
                inventory.stage, room, placement.stable_id, switch_description
            ),
        },
    ];
    evidence_records.sort_by(|left, right| left.id.cmp(&right.id));
    let evidence = RuleEvidence {
        truth: TruthStatus::Established,
        records: evidence_records,
    };
    let unknown_evidence = RuleEvidence {
        truth: TruthStatus::Unknown,
        records: evidence.records.clone(),
    };
    let actor_id = format!("obligation.actor-state.{token}");
    let interaction_id = format!("obligation.interaction.{token}");
    let passage_id = format!("obligation.interaction.{token}.passage");
    let obligations = vec![
        FeasibilityObligation {
            id: actor_id.clone(),
            label: format!("Complete {} keyed event and committed key delta", placement.name),
            scope: scope.clone(),
            obligation_kind: ObligationKind::ActorState,
            stage: crate::transition::ObligationStage::Effect,
            detail: ObligationDetail::Unresolved {
                research_question: "Confirm the accepted door command, RIDER_GATE_OPEN00 cuts, event reset, and uncontended queued key-delta commit complete without interruption.".into(),
            },
            evidence: unknown_evidence.clone(),
        },
        FeasibilityObligation {
            id: interaction_id.clone(),
            label: format!("Reach and activate {} keyed side", placement.name),
            scope: scope.clone(),
            obligation_kind: ObligationKind::Interaction,
            stage: crate::transition::ObligationStage::Activate,
            detail: ObligationDetail::Unresolved {
                research_question: "Reach actor-local x in [-100, 100] and z in [0, 100] with the required facing while the gate owns its door event.".into(),
            },
            evidence: unknown_evidence.clone(),
        },
        FeasibilityObligation {
            id: passage_id.clone(),
            label: format!("Traverse the physically open {}", placement.name),
            scope: scope.clone(),
            obligation_kind: ObligationKind::Interaction,
            stage: crate::transition::ObligationStage::Activate,
            detail: ObligationDetail::Unresolved {
                research_question: "Witness sufficient leaf opening and collision clearance under either the set-switch push behavior or the M_035 forced-open bypass.".into(),
            },
            evidence: unknown_evidence,
        },
    ];
    let location = placement_location_guard(inventory, placement, room);
    let locked = PredicateExpression::All {
        terms: vec![
            location.clone(),
            switch_guard(switch_id, false),
            persistent_event_bit_guard(0x0810, false),
        ],
    };
    let ordinary = keyed_actor_candidate(
        scope,
        placement,
        family,
        "unlock",
        &format!(
            "{} {} layer {} keyed rider-gate unlock",
            inventory.stage,
            placement.name,
            placement.layer.unwrap_or_default()
        ),
        TransitionKind::ActorDriven,
        PredicateExpression::All {
            terms: vec![
                locked.clone(),
                small_key_guard(ComparisonOperator::GreaterThan, 0),
                small_key_guard(ComparisonOperator::LessThanOrEqual, 100),
            ],
        },
        vec![small_key_adjust(-1), switch_write(switch_id)],
        &[actor_id.clone(), interaction_id.clone()],
        &evidence,
    );
    let high = keyed_actor_candidate(
        scope,
        placement,
        family,
        "unlock-high-key-clamp",
        &format!(
            "{} {} layer {} rider-gate unlock with high raw keys clamped to 99",
            inventory.stage,
            placement.name,
            placement.layer.unwrap_or_default()
        ),
        TransitionKind::ActorDriven,
        PredicateExpression::All {
            terms: vec![
                locked,
                small_key_guard(ComparisonOperator::GreaterThan, 100),
            ],
        },
        vec![small_key_write(99), switch_write(switch_id)],
        &[actor_id, interaction_id],
        &evidence,
    );
    let unlocked_passage = keyed_actor_candidate(
        scope,
        placement,
        family,
        "set-switch-passage",
        &format!(
            "{} {} layer {} set-switch physical passage",
            inventory.stage,
            placement.name,
            placement.layer.unwrap_or_default()
        ),
        TransitionKind::ActorDriven,
        PredicateExpression::All {
            terms: vec![
                location.clone(),
                switch_guard(switch_id, true),
                persistent_event_bit_guard(0x0810, false),
            ],
        },
        Vec::new(),
        std::slice::from_ref(&passage_id),
        &evidence,
    );
    let event_bypass = keyed_actor_candidate(
        scope,
        placement,
        family,
        "m035-forced-open-passage",
        &format!(
            "{} {} layer {} M_035 forced-open passage",
            inventory.stage,
            placement.name,
            placement.layer.unwrap_or_default()
        ),
        TransitionKind::ActorDriven,
        PredicateExpression::All {
            terms: vec![location, persistent_event_bit_guard(0x0810, true)],
        },
        Vec::new(),
        &[passage_id],
        &evidence,
    );
    Ok(Some(ImportedKeyedActorActions {
        exit_record_id: None,
        transitions: vec![ordinary, high, unlocked_passage, event_bypass],
        obligations,
    }))
}

fn import_gz2e01_caravan_gate(
    inventory: &WorldInventory,
    placement: &PlacementRecord,
    scope: &ContextScope,
    inventory_sha256: Digest,
) -> Result<Option<ImportedKeyedActorActions>, PlannerContractError> {
    let Some(room @ (1 | 2)) = placement.scope.room else {
        return Ok(None);
    };
    if inventory.stage != "F_SP118" || placement.parameters != u32::MAX {
        return Ok(None);
    }
    let family = "caravan-gate";
    let token = stable_token(
        "keyed",
        &[
            family.as_bytes(),
            inventory.stage.as_bytes(),
            placement.stable_id.as_bytes(),
        ],
    );
    let mut evidence_records = vec![
        EvidenceRecord {
            id: format!("evidence.source.actor.{token}"),
            kind: EvidenceKind::SourceAudited,
            source_sha256: Some(static_digest(
                "f0916a79d3b157454dd2263307567e472d4f394d61ad8ece9153500d91943697",
            )),
            note: "d_a_obj_crvgate.cpp: parent/child creation, key/facing/distance offer guard, queued decrement, transient paired opening, and boar/event destruction path.".into(),
        },
        EvidenceRecord {
            id: format!("evidence.source.name-map.{token}"),
            kind: EvidenceKind::SourceAudited,
            source_sha256: Some(static_digest(
                "5c46ffc79e891b59b02455b837d9966d05c147d8d95c91c65cc845dd848d32ad",
            )),
            note: "d_stage.cpp: CrvGate maps to the caravan-gate actor process.".into(),
        },
        EvidenceRecord {
            id: format!("evidence.world.inventory.{token}"),
            kind: EvidenceKind::Extracted,
            source_sha256: Some(inventory_sha256),
            note: format!(
                "Authenticated F_SP118 room-{room} caravan-gate parent placement {}.",
                placement.stable_id
            ),
        },
    ];
    evidence_records.sort_by(|left, right| left.id.cmp(&right.id));
    let evidence = RuleEvidence {
        truth: TruthStatus::Established,
        records: evidence_records,
    };
    let unknown_evidence = RuleEvidence {
        truth: TruthStatus::Unknown,
        records: evidence.records.clone(),
    };
    let actor_id = format!("obligation.actor-state.{token}");
    let interaction_id = format!("obligation.interaction.{token}");
    let boar_id = format!("obligation.interaction.{token}.boar-destruction");
    let obligations = vec![
        FeasibilityObligation {
            id: actor_id.clone(),
            label: format!("Complete the paired room-{room} caravan-gate key event"),
            scope: scope.clone(),
            obligation_kind: ObligationKind::ActorState,
            stage: crate::transition::ObligationStage::Effect,
            detail: ObligationDetail::Unresolved {
                research_question: "Confirm parent creation, child lookup, accepted demo command, uncontended queued key-delta commit, camera reset, and transient SetOpen on both leaves complete without interruption.".into(),
            },
            evidence: unknown_evidence.clone(),
        },
        FeasibilityObligation {
            id: interaction_id.clone(),
            label: format!("Reach and activate the room-{room} caravan gate"),
            scope: scope.clone(),
            obligation_kind: ObligationKind::Interaction,
            stage: crate::transition::ObligationStage::Activate,
            detail: ObligationDetail::Unresolved {
                research_question: "Reach within 200 world-XZ units with player/gate facing delta at least 0x5000 while the parent owns its door command.".into(),
            },
            evidence: unknown_evidence.clone(),
        },
        FeasibilityObligation {
            id: boar_id.clone(),
            label: format!("Destroy the room-{room} caravan-gate pair with the running boar"),
            scope: scope.clone(),
            obligation_kind: ObligationKind::Interaction,
            stage: crate::transition::ObligationStage::Activate,
            detail: ObligationDetail::Unresolved {
                research_question: "During a running event, collide the ridden E_WB boar with a gate sphere inside the 490-unit bound at nonzero speed, then witness both paired leaves enter and complete their destruction/open state.".into(),
            },
            evidence: unknown_evidence,
        },
    ];
    let location = placement_location_guard(inventory, placement, room);
    let ordinary = keyed_actor_candidate(
        scope,
        placement,
        family,
        "key-open",
        &format!("F_SP118 room {room} caravan-gate transient key opening"),
        TransitionKind::ActorDriven,
        PredicateExpression::All {
            terms: vec![
                location.clone(),
                small_key_guard(ComparisonOperator::GreaterThan, 0),
                small_key_guard(ComparisonOperator::LessThanOrEqual, 100),
            ],
        },
        vec![small_key_adjust(-1)],
        &[actor_id.clone(), interaction_id.clone()],
        &evidence,
    );
    let high = keyed_actor_candidate(
        scope,
        placement,
        family,
        "key-open-high-key-clamp",
        &format!(
            "F_SP118 room {room} caravan-gate transient opening with high raw keys clamped to 99"
        ),
        TransitionKind::ActorDriven,
        PredicateExpression::All {
            terms: vec![
                location.clone(),
                small_key_guard(ComparisonOperator::GreaterThan, 100),
            ],
        },
        vec![small_key_write(99)],
        &[actor_id, interaction_id],
        &evidence,
    );
    let boar_bypass = keyed_actor_candidate(
        scope,
        placement,
        family,
        "boar-destruction-bypass",
        &format!("F_SP118 room {room} caravan-gate boar destruction bypass"),
        TransitionKind::ActorDriven,
        location,
        Vec::new(),
        &[boar_id],
        &evidence,
    );
    Ok(Some(ImportedKeyedActorActions {
        exit_record_id: None,
        transitions: vec![ordinary, high, boar_bypass],
        obligations,
    }))
}

struct ImportedSceneActorActions {
    transitions: Vec<(String, CandidateTransition)>,
    obligations: Vec<FeasibilityObligation>,
}

fn import_gz2e01_l7_bridge_demo(
    inventory: &WorldInventory,
    placement: &PlacementRecord,
    scope: &ContextScope,
    inventory_sha256: Digest,
) -> Result<Option<ImportedSceneActorActions>, PlannerContractError> {
    if inventory.stage != "D_MN07"
        || placement.name != "dr"
        || placement.kind != PlacementKind::Actor
        || placement.scope.room != Some(6)
        || placement.parameters != 0x18
    {
        return Ok(None);
    }
    let matching_exit = |record_index| {
        inventory
            .exits
            .iter()
            .filter(|exit| exit.scope.room == Some(6) && exit.record_index == record_index)
            .collect::<Vec<_>>()
    };
    let pre_bridge_exits = matching_exit(6);
    let [pre_bridge_exit] = pre_bridge_exits.as_slice() else {
        return Ok(None);
    };
    let post_bridge_exits = matching_exit(7);
    let [post_bridge_exit] = post_bridge_exits.as_slice() else {
        return Ok(None);
    };

    let family = "l7-bridge-demo";
    let token = stable_token(
        "actor-scene",
        &[
            family.as_bytes(),
            inventory.stage.as_bytes(),
            placement.stable_id.as_bytes(),
        ],
    );
    let mut evidence_records = vec![
        EvidenceRecord {
            id: format!("evidence.source.actor.{token}"),
            kind: EvidenceKind::SourceAudited,
            source_sha256: Some(static_digest(
                "7b350f2e3efa4ddb5907b38d4f1f8ceb91d37cc741dce7e4d7de67d436421b02",
            )),
            note: "d_a_L7demo_dr.cpp: layer-sensitive start guards, exact SCLS 6/7 scene requests, switch 0x18 write, and absence of a key decrement.".into(),
        },
        EvidenceRecord {
            id: format!("evidence.source.name-map.{token}"),
            kind: EvidenceKind::SourceAudited,
            source_sha256: Some(static_digest(
                "5c46ffc79e891b59b02455b837d9966d05c147d8d95c91c65cc845dd848d32ad",
            )),
            note: "d_stage.cpp: the exact `dr` placement name maps to the DR/L7 bridge-demo actor process.".into(),
        },
        EvidenceRecord {
            id: format!("evidence.source.save-layout.{token}"),
            kind: EvidenceKind::SourceAudited,
            source_sha256: Some(static_digest(
                "fdac35e3d54a3c496dc20fd2f5e297fa9411a78fb7d09be607a62fa0cfa0c110",
            )),
            note: "d_save.h: current-stage memory switch 0x18 and the small-key count occupy distinct dSv_memBit_c fields.".into(),
        },
        EvidenceRecord {
            id: format!("evidence.world.inventory.{token}"),
            kind: EvidenceKind::Extracted,
            source_sha256: Some(inventory_sha256),
            note: format!(
                "Authenticated D_MN07 room-6 placement {} and its exact room SCLS records.",
                placement.stable_id
            ),
        },
    ];
    evidence_records.sort_by(|left, right| left.id.cmp(&right.id));
    let evidence = RuleEvidence {
        truth: TruthStatus::Established,
        records: evidence_records,
    };
    let unknown_evidence = RuleEvidence {
        truth: TruthStatus::Unknown,
        records: evidence.records.clone(),
    };
    let pre_interaction_id = format!("obligation.interaction.{token}.pre-bridge");
    let pre_effect_id = format!("obligation.actor-state.{token}.pre-bridge");
    let post_effect_id = format!("obligation.actor-state.{token}.post-bridge");
    let obligations = vec![
        FeasibilityObligation {
            id: pre_effect_id.clone(),
            label: "Complete the D_MN07 pre-bridge DR event and SCLS 6 request".into(),
            scope: scope.clone(),
            obligation_kind: ObligationKind::ActorState,
            stage: crate::transition::ObligationStage::Effect,
            detail: ObligationDetail::Unresolved {
                research_question: "Confirm event acceptance, camera/player demo ownership, the fixed walk, and the SCLS 6 scene request complete without interruption; no switch or key write belongs to this branch.".into(),
            },
            evidence: unknown_evidence.clone(),
        },
        FeasibilityObligation {
            id: post_effect_id.clone(),
            label: "Complete the D_MN07 bridge-destruction DR event and SCLS 7 request".into(),
            scope: scope.clone(),
            obligation_kind: ObligationKind::ActorState,
            stage: crate::transition::ObligationStage::Effect,
            detail: ObligationDetail::Unresolved {
                research_question: "Confirm the layer-3 DR event, both bridge-destruction phases, switch 0x18 write, event reset, and SCLS 7 scene request complete without interruption.".into(),
            },
            evidence: unknown_evidence.clone(),
        },
        FeasibilityObligation {
            id: pre_interaction_id.clone(),
            label: "Reach the D_MN07 DR pre-bridge start box with a key".into(),
            scope: scope.clone(),
            obligation_kind: ObligationKind::Interaction,
            stage: crate::transition::ObligationStage::Activate,
            detail: ObligationDetail::Unresolved {
                research_question: "Reach world x in (-4480, -3730) and z in (-12800, -12100) outside layer 3 while the actor can acquire its potential event; the source applies no Y bound.".into(),
            },
            evidence: unknown_evidence,
        },
    ];

    let pre_bridge = keyed_actor_candidate(
        scope,
        placement,
        family,
        "enter-bridge-layer",
        "D_MN07 room 6 key-present DR event to bridge layer via SCLS 6",
        TransitionKind::ActorDriven,
        PredicateExpression::All {
            terms: vec![
                placement_location_guard(inventory, placement, 6),
                location_layer_guard(ComparisonOperator::NotEqual, 3),
                memory_switch_guard(0x18, false),
                small_key_guard(ComparisonOperator::GreaterThan, 0),
            ],
        },
        vec![StateOperation::SetLocation {
            location: scene_location(pre_bridge_exit),
        }],
        &[pre_effect_id, pre_interaction_id],
        &evidence,
    );
    let post_bridge = keyed_actor_candidate(
        scope,
        placement,
        family,
        "destroy-bridge",
        "D_MN07 room 6 layer-3 DR bridge destruction via SCLS 7",
        TransitionKind::ActorDriven,
        PredicateExpression::All {
            terms: vec![
                placement_location_guard(inventory, placement, 6),
                location_layer_guard(ComparisonOperator::Equal, 3),
                memory_switch_guard(0x18, false),
            ],
        },
        vec![
            memory_switch_write(0x18),
            StateOperation::SetLocation {
                location: scene_location(post_bridge_exit),
            },
        ],
        &[post_effect_id],
        &evidence,
    );
    Ok(Some(ImportedSceneActorActions {
        transitions: vec![
            (pre_bridge_exit.stable_id.clone(), pre_bridge),
            (post_bridge_exit.stable_id.clone(), post_bridge),
        ],
        obligations,
    }))
}

fn scene_location(exit: &crate::world_data::StageExitRecord) -> SceneLocation {
    SceneLocation {
        stage: exit.destination_stage.clone(),
        room: exit.destination_room,
        layer: exit.destination_layer,
        spawn: exit.destination_point,
    }
}

fn location_layer_guard(operator: ComparisonOperator, layer: i64) -> PredicateExpression {
    PredicateExpression::Compare {
        left: ValueReference::LocationLayer,
        operator,
        right: ValueReference::Literal {
            value: StateValue::Signed(layer),
        },
    }
}

fn placement_location_guard(
    inventory: &WorldInventory,
    placement: &PlacementRecord,
    room: i8,
) -> PredicateExpression {
    let PredicateExpression::All { mut terms } = source_location_guard(&inventory.stage, room)
    else {
        unreachable!("source_location_guard always returns an all predicate")
    };
    if let Some(layer) = placement.layer {
        terms.push(PredicateExpression::Compare {
            left: ValueReference::LocationLayer,
            operator: ComparisonOperator::Equal,
            right: ValueReference::Literal {
                value: StateValue::Signed(layer.into()),
            },
        });
    }
    PredicateExpression::All { terms }
}

fn small_key_guard(operator: ComparisonOperator, value: u64) -> PredicateExpression {
    PredicateExpression::Compare {
        left: ValueReference::BoundRawBits {
            component_kind: ComponentKind::DungeonMemory,
            binding: ComponentBindingReference::CurrentStage,
            byte_offset: 0x1c,
            byte_width: 1,
            mask: 0xff,
        },
        operator,
        right: ValueReference::Literal {
            value: StateValue::Unsigned(value),
        },
    }
}

fn boss_key_guard() -> PredicateExpression {
    PredicateExpression::Compare {
        left: ValueReference::BoundRawBits {
            component_kind: ComponentKind::DungeonMemory,
            binding: ComponentBindingReference::CurrentStage,
            byte_offset: 0x1d,
            byte_width: 1,
            mask: 0x04,
        },
        operator: ComparisonOperator::Equal,
        right: ValueReference::Literal {
            value: StateValue::Unsigned(0x04),
        },
    }
}

fn memory_switch_guard(switch_id: u8, set: bool) -> PredicateExpression {
    let (byte_offset, mask) = memory_switch_raw_location(switch_id);
    PredicateExpression::Compare {
        left: ValueReference::BoundRawBits {
            component_kind: ComponentKind::DungeonMemory,
            binding: ComponentBindingReference::CurrentStage,
            byte_offset,
            byte_width: 1,
            mask: u64::from(mask),
        },
        operator: ComparisonOperator::Equal,
        right: ValueReference::Literal {
            value: StateValue::Unsigned(if set { u64::from(mask) } else { 0 }),
        },
    }
}

fn dungeon_session_switch_guard(switch_id: u8, set: bool) -> PredicateExpression {
    debug_assert!((0x80..0xc0).contains(&switch_id));
    PredicateExpression::Compare {
        left: ValueReference::BoundRawBits {
            component_kind: ComponentKind::Custom {
                id: DUNGEON_SESSION_SWITCH_LABEL_KIND.into(),
            },
            binding: ComponentBindingReference::CurrentStage,
            byte_offset: u32::from(switch_id - 0x80),
            byte_width: 1,
            mask: 1,
        },
        operator: ComparisonOperator::Equal,
        right: ValueReference::Literal {
            value: StateValue::Unsigned(u64::from(set)),
        },
    }
}

fn room_switch_label_guard(switch_id: u8, set: bool) -> PredicateExpression {
    PredicateExpression::Compare {
        left: ValueReference::BoundRawBits {
            component_kind: ComponentKind::Custom {
                id: ROOM_SWITCH_LABEL_KIND.into(),
            },
            binding: ComponentBindingReference::CurrentRoom,
            byte_offset: u32::from(switch_id),
            byte_width: 1,
            mask: 1,
        },
        operator: ComparisonOperator::Equal,
        right: ValueReference::Literal {
            value: StateValue::Unsigned(u64::from(set)),
        },
    }
}

fn persistent_event_bit_guard(packed_coordinate: u16, set: bool) -> PredicateExpression {
    let byte_offset = u32::from(packed_coordinate >> 8);
    let mask = packed_coordinate as u8;
    PredicateExpression::Compare {
        left: ValueReference::BoundRawBits {
            component_kind: ComponentKind::Custom {
                id: "persistent-event-registers".into(),
            },
            binding: ComponentBindingReference::ActiveRuntimeFile,
            byte_offset,
            byte_width: 1,
            mask: u64::from(mask),
        },
        operator: ComparisonOperator::Equal,
        right: ValueReference::Literal {
            value: StateValue::Unsigned(if set { u64::from(mask) } else { 0 }),
        },
    }
}

fn memory_switch_write(switch_id: u8) -> StateOperation {
    let (byte_offset, mask) = memory_switch_raw_location(switch_id);
    StateOperation::WriteBoundRaw {
        component_kind: ComponentKind::DungeonMemory,
        binding: ComponentBindingReference::CurrentStage,
        byte_offset,
        mask: vec![mask],
        value: vec![mask],
    }
}

fn dungeon_session_switch_write(switch_id: u8) -> StateOperation {
    debug_assert!((0x80..0xc0).contains(&switch_id));
    StateOperation::WriteBoundRaw {
        component_kind: ComponentKind::Custom {
            id: DUNGEON_SESSION_SWITCH_LABEL_KIND.into(),
        },
        binding: ComponentBindingReference::CurrentStage,
        byte_offset: u32::from(switch_id - 0x80),
        mask: vec![1],
        value: vec![1],
    }
}

fn room_switch_label_write(switch_id: u8, set: bool) -> StateOperation {
    StateOperation::WriteBoundRaw {
        component_kind: ComponentKind::Custom {
            id: ROOM_SWITCH_LABEL_KIND.into(),
        },
        binding: ComponentBindingReference::CurrentRoom,
        byte_offset: u32::from(switch_id),
        mask: vec![1],
        value: vec![u8::from(set)],
    }
}

fn small_key_adjust(delta: i64) -> StateOperation {
    StateOperation::AdjustBoundRawUnsigned {
        component_kind: ComponentKind::DungeonMemory,
        binding: ComponentBindingReference::CurrentStage,
        byte_offset: 0x1c,
        byte_width: 1,
        delta,
    }
}

fn small_key_write(value: u8) -> StateOperation {
    StateOperation::WriteBoundRaw {
        component_kind: ComponentKind::DungeonMemory,
        binding: ComponentBindingReference::CurrentStage,
        byte_offset: 0x1c,
        mask: vec![0xff],
        value: vec![value],
    }
}

#[allow(clippy::too_many_arguments)]
fn keyed_actor_candidate(
    scope: &ContextScope,
    placement: &PlacementRecord,
    family: &str,
    branch: &str,
    label: &str,
    transition_kind: TransitionKind,
    hard_guards: PredicateExpression,
    effects: Vec<StateOperation>,
    obligation_ids: &[String],
    evidence: &RuleEvidence,
) -> CandidateTransition {
    let token = stable_token(
        "keyed",
        &[
            family.as_bytes(),
            branch.as_bytes(),
            placement.stable_id.as_bytes(),
        ],
    );
    CandidateTransition {
        id: format!("transition.{token}"),
        label: label.into(),
        scope: scope.clone(),
        transition_kind,
        approach_id: format!("approach.{token}"),
        activation: ActivationContract {
            hard_guards,
            physical_obligation_ids: obligation_ids.to_vec(),
            effects,
            unknown_requirements: Vec::new(),
        },
        evidence: evidence.clone(),
    }
}

fn keyed_actor_obligations(
    scope: &ContextScope,
    placement: &PlacementRecord,
    token: &str,
    evidence: &RuleEvidence,
    actor_question: &str,
    interaction_question: &str,
) -> (Vec<FeasibilityObligation>, Vec<String>) {
    let actor_id = format!("obligation.actor-state.{token}");
    let interaction_id = format!("obligation.interaction.{token}");
    let unknown_evidence = RuleEvidence {
        truth: TruthStatus::Unknown,
        records: evidence.records.clone(),
    };
    (
        vec![
            FeasibilityObligation {
                id: actor_id.clone(),
                label: format!("Run the loaded {} unlock/open phases", placement.name),
                scope: scope.clone(),
                obligation_kind: ObligationKind::ActorState,
                stage: crate::transition::ObligationStage::Effect,
                detail: ObligationDetail::Unresolved {
                    research_question: actor_question.into(),
                },
                evidence: unknown_evidence.clone(),
            },
            FeasibilityObligation {
                id: interaction_id.clone(),
                label: format!("Reach and activate {}", placement.name),
                scope: scope.clone(),
                obligation_kind: ObligationKind::Interaction,
                stage: crate::transition::ObligationStage::Activate,
                detail: ObligationDetail::Unresolved {
                    research_question: interaction_question.into(),
                },
                evidence: unknown_evidence,
            },
        ],
        vec![actor_id, interaction_id],
    )
}

#[allow(clippy::too_many_arguments)]
fn keyed_actor_evidence(
    family: &str,
    inventory_sha256: Digest,
    placement: &PlacementRecord,
    actor_sha256: &str,
    actor_note: &str,
    event_sha256: &str,
    event_note: &str,
    include_mboss_parameters: bool,
    token: &str,
) -> RuleEvidence {
    let mut records = vec![
            EvidenceRecord {
                id: format!("evidence.source.actor.{family}.{token}"),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(static_digest(actor_sha256)),
                note: actor_note.into(),
            },
            EvidenceRecord {
                id: format!("evidence.source.event.{family}.{token}"),
                kind: EvidenceKind::Extracted,
                source_sha256: Some(static_digest(event_sha256)),
                note: event_note.into(),
            },
            EvidenceRecord {
                id: format!("evidence.source.name-map.{family}.{token}"),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(static_digest(
                    "5c46ffc79e891b59b02455b837d9966d05c147d8d95c91c65cc845dd848d32ad",
                )),
                note: "d_stage.cpp: exact placement names map to their distinct actor process families and mini-boss level arguments.".into(),
            },
            EvidenceRecord {
                id: format!("evidence.source.key-commit.{family}.{token}"),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(static_digest(
                    "b58ed135700865df0f0cb9ce0e4115de6ec1f9f6dbb8fff8cc1ff99b437d5569",
                )),
                note: "d_meter2.cpp: queued key deltas clamp to [0, 99], update dSv_memBit_c::mKeyNum, and clear the pending delta.".into(),
            },
            EvidenceRecord {
                id: format!("evidence.source.save-layout.{family}.{token}"),
                kind: EvidenceKind::SourceAudited,
                source_sha256: Some(static_digest(
                    "74a211e5d2ee2c0fe4ce259905fe1f479f373d5b2459d654871cbbd2f61e8756",
                )),
                note: "d_save.h: dSv_memBit_c memory switches, key count byte 0x1c, and dungeon-item byte 0x1d share the active stage bank.".into(),
            },
            EvidenceRecord {
                id: format!("evidence.world.inventory.{family}.{token}"),
                kind: EvidenceKind::Extracted,
                source_sha256: Some(inventory_sha256),
                note: format!(
                    "Authenticated world inventory placement {} from resource {}.",
                    placement.stable_id, placement.source_sha256
                ),
            },
        ];
    if include_mboss_parameters {
        records.push(EvidenceRecord {
            id: format!("evidence.source.parameters.{family}.{token}"),
            kind: EvidenceKind::SourceAudited,
            source_sha256: Some(static_digest(
                "b0dacfc4b9c46786d73a840e55385e535364b9fee7de66cd0e2af18f25d1ca78",
            )),
            note: "d_door_param2.cpp: mini-boss-door front/back room, exit number, option, and unlock-switch parameter decoding.".into(),
        });
        records.sort_by(|left, right| left.id.cmp(&right.id));
    }
    RuleEvidence {
        truth: TruthStatus::Established,
        records,
    }
}

fn static_digest(value: &str) -> Digest {
    value.parse().expect("source-audit digest literal is valid")
}

fn extracted_evidence(inventory_sha256: Digest, token: &str, inferred: bool) -> RuleEvidence {
    RuleEvidence {
        truth: if inferred {
            TruthStatus::Contested
        } else {
            TruthStatus::Established
        },
        records: vec![EvidenceRecord {
            id: format!("evidence.{token}"),
            kind: EvidenceKind::Extracted,
            source_sha256: Some(inventory_sha256),
            note: if inferred {
                "Extracted collision exit code joined to the room SCLS index; activation semantics remain inferred.".into()
            } else {
                "Extracted from an immutable world inventory.".into()
            },
        }],
    }
}

fn actor_type(placement: &PlacementRecord) -> String {
    if placement.kind == PlacementKind::PlayerSpawn {
        return "player-spawn".into();
    }
    let normalized = placement
        .name
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() {
                byte.to_ascii_lowercase() as char
            } else if matches!(byte, b'_' | b'-') {
                byte as char
            } else {
                '-'
            }
        })
        .collect::<String>();
    format!("actor.{normalized}")
}

fn stable_token(domain: &str, parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    for part in parts {
        hasher.update([0]);
        hasher.update(part);
    }
    format!("{domain}.{}", Digest(hasher.finalize().into()))
}

fn decode_hex(value: &str) -> Result<Vec<u8>, PlannerContractError> {
    if !value.len().is_multiple_of(2) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(PlannerContractError::new("raw_hex", "is not canonical hex"));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("ASCII was validated");
            u8::from_str_radix(pair, 16)
                .map_err(|_| PlannerContractError::new("raw_hex", "is not canonical hex"))
        })
        .collect()
}

fn f32_bytes(values: [f32; 3]) -> Vec<u8> {
    canonicalize_position(values)
        .into_iter()
        .flat_map(|value| value.to_bits().to_le_bytes())
        .collect()
}

fn i16_bytes(values: [i16; 3]) -> Vec<u8> {
    values.into_iter().flat_map(i16::to_le_bytes).collect()
}

fn canonicalize_position(mut values: [f32; 3]) -> [f32; 3] {
    for value in &mut values {
        if *value == 0.0 {
            *value = 0.0;
        }
    }
    values
}

fn canonicalize_scalar(value: f32) -> f32 {
    if value == 0.0 { 0.0 } else { value }
}

fn triangle_bounds(triangle: &[[f32; 3]; 3]) -> ([f32; 3], [f32; 3]) {
    let mut minimum = triangle[0];
    let mut maximum = triangle[0];
    for point in &triangle[1..] {
        for axis in 0..3 {
            minimum[axis] = minimum[axis].min(point[axis]);
            maximum[axis] = maximum[axis].max(point[axis]);
        }
    }
    (
        canonicalize_position(minimum),
        canonicalize_position(maximum),
    )
}

fn validate_approach_shape(shape: &ExtractedApproachShape) -> Result<(), PlannerContractError> {
    match shape {
        ExtractedApproachShape::Reconstructed {
            triangle,
            plane_normal,
            plane_offset,
            minimum,
            maximum,
        } => {
            if triangle.iter().any(|point| !canonical_position(*point))
                || !canonical_position(*plane_normal)
                || !plane_offset.is_finite()
                || plane_offset.to_bits() == (-0.0_f32).to_bits()
                || plane_normal.iter().all(|value| *value == 0.0)
                || !canonical_position(*minimum)
                || !canonical_position(*maximum)
                || minimum
                    .iter()
                    .zip(maximum)
                    .any(|(minimum, maximum)| minimum > maximum)
                || triangle_bounds(triangle) != (*minimum, *maximum)
            {
                return Err(PlannerContractError::new(
                    "approach_geometries.shape",
                    "has invalid reconstructed triangle, plane, or exact bounds",
                ));
            }
            Ok(())
        }
        ExtractedApproachShape::Unavailable { reason } => {
            validate_label("approach_geometries.shape.reason", reason)
        }
    }
}

fn canonical_position(values: [f32; 3]) -> bool {
    values
        .iter()
        .all(|value| value.is_finite() && value.to_bits() != (-0.0_f32).to_bits())
}

fn validate_game_name(field: &str, value: &str) -> Result<(), PlannerContractError> {
    if value.is_empty()
        || value.len() > 8
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(PlannerContractError::new(field, "is not a valid game name"));
    }
    Ok(())
}

fn validate_sorted<T>(
    field: &str,
    values: &[T],
    key: impl Fn(&T) -> &str,
) -> Result<(), PlannerContractError> {
    if values.windows(2).any(|pair| key(&pair[0]) >= key(&pair[1])) {
        return Err(PlannerContractError::new(
            field,
            "must be unique and sorted",
        ));
    }
    Ok(())
}

fn strictly_sorted(values: &[String]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn world_error(field: &str, error: impl std::fmt::Display) -> PlannerContractError {
    PlannerContractError::new(field, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{
        CONTENT_IDENTITY_SCHEMA, ContentFingerprint, GamePlatform, GameRegion,
        RUNTIME_CONFIGURATION_SCHEMA,
    };
    use crate::orig_discovery::bundled_supported_build_registry;
    use crate::world_data::{
        CollisionCode, CollisionInventoryRecord, CollisionLoadTrigger, KclAuthoredPrism,
        KclInventoryPrism, KclReconstruction, KclSourceIndices, PlacementKind, PlacementRecord,
        SourceKind, SourceScope, StageExitRecord, Vec3, WORLD_INVENTORY_SCHEMA, WorldContext,
        WorldInventory, WorldSource,
    };

    fn content() -> ContentIdentity {
        ContentIdentity {
            schema: CONTENT_IDENTITY_SCHEMA.into(),
            id: "gcn-us-test".into(),
            fingerprint: ContentFingerprint {
                platform: GamePlatform::GameCube,
                region: GameRegion::Usa,
                revision: "test".into(),
                product_id: "GZ2E01".into(),
                executable_sha256: Digest([1; 32]),
                game_data_sha256: Digest([2; 32]),
                resource_manifest_sha256: Digest([3; 32]),
            },
        }
    }

    fn audited_content() -> ContentIdentity {
        bundled_supported_build_registry()
            .unwrap()
            .identities
            .into_iter()
            .find(|identity| identity.id == "gcn-us-1.0-gz2e01")
            .unwrap()
    }

    fn runtime(content: &ContentIdentity) -> RuntimeConfiguration {
        RuntimeConfiguration {
            schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
            content_sha256: content.digest().unwrap(),
            language: "en".into(),
            settings: BTreeMap::new(),
        }
    }

    fn placement(
        stable_id: &str,
        kind: PlacementKind,
        name: &str,
        angle_z: i16,
    ) -> PlacementRecord {
        PlacementRecord {
            stable_id: stable_id.into(),
            source_sha256: Digest([12; 32]),
            scope: SourceScope {
                kind: SourceKind::Room,
                room: Some(0),
            },
            chunk_tag: if kind == PlacementKind::PlayerSpawn {
                "PLYR".into()
            } else {
                "ACTR".into()
            },
            record_index: 0,
            layer: None,
            kind,
            name: name.into(),
            parameters: 0x1234,
            position: Vec3 {
                x: 10.0,
                y: 20.0,
                z: 30.0,
            },
            angle: [0, 0x1000, angle_z],
            set_id: 7,
            scale_raw: None,
            raw_hex: "00".repeat(32),
        }
    }

    fn inventory(linked: bool) -> WorldInventory {
        let kcl = Digest([14; 32]);
        let plc = Digest([15; 32]);
        let collision_id = format!("kcl-sha256:{kcl}/plc-sha256:{plc}/prism/1");
        let exit_id = "dzr-sha256:fixture/chunk/SCLS/record/0".to_string();
        let raw_code = [if linked { 0 } else { 0x3f }, 0, 0, 0, 0];
        let collision = CollisionInventoryRecord {
            room: 0,
            prism: KclInventoryPrism {
                authored: KclAuthoredPrism {
                    stable_id: collision_id.clone(),
                    prism_index: 1,
                    height: 1.0,
                    source_indices: KclSourceIndices {
                        position: 0,
                        face_normal: 0,
                        edge_normal_1: 0,
                        edge_normal_2: 0,
                        edge_normal_3: 0,
                    },
                    attribute: 0,
                    code: CollisionCode {
                        raw: raw_code,
                        exit_id: if linked { 0 } else { 0x3f },
                        polygon_color: 0,
                        special_code: 0,
                        link_no: 0,
                        wall_code: 0,
                        attribute_0: 0,
                        attribute_1: 0,
                        ground_code: 0,
                        camera_move_background: 0,
                        room_camera: 0,
                        room_path: 0,
                        room_path_point: 0,
                        room_info: 0,
                        sound_id: 0,
                        room: 0,
                    },
                },
                reconstruction: KclReconstruction::Degenerate {
                    reason: "fixture".into(),
                },
            },
        };
        let exit = StageExitRecord {
            stable_id: exit_id.clone(),
            source_sha256: Digest([12; 32]),
            scope: SourceScope {
                kind: SourceKind::Room,
                room: Some(0),
            },
            chunk_tag: "SCLS".into(),
            record_index: 0,
            destination_stage: "F_SP104".into(),
            destination_point: 2,
            destination_room: 1,
            destination_layer: -1,
            wipe: 0,
            wipe_time: 0,
            time_hour: -1,
            raw_start: 2,
            raw_field_a: 0,
            raw_field_b: 0x0f,
            raw_wipe: 15,
            raw_hex: "00".repeat(13),
        };
        let load_triggers = if linked {
            let mut hasher = Sha256::new();
            hasher.update(b"dusklight.collision-load-trigger/v1\0");
            hasher.update(collision_id.as_bytes());
            hasher.update([0]);
            hasher.update(exit_id.as_bytes());
            let digest = Digest(hasher.finalize().into());
            vec![CollisionLoadTrigger {
                stable_id: format!("load-trigger-sha256:{digest}"),
                room: 0,
                collision_id,
                collision_exit_id: 0,
                scls_id: exit_id,
                destination_stage: "F_SP104".into(),
                destination_room: 1,
                destination_layer: -1,
                destination_point: 2,
                inferred_semantics: true,
            }]
        } else {
            Vec::new()
        };
        WorldInventory {
            schema: WORLD_INVENTORY_SCHEMA.into(),
            stage: "F_SP103".into(),
            sources: vec![
                WorldSource {
                    scope: SourceScope {
                        kind: SourceKind::Stage,
                        room: None,
                    },
                    archive_sha256: Digest([10; 32]),
                    stage_data_path: "stage.dzs".into(),
                    stage_data_sha256: Digest([11; 32]),
                    kcl_path: None,
                    kcl_sha256: None,
                    plc_path: None,
                    plc_sha256: None,
                    addressable_prisms: 0,
                },
                WorldSource {
                    scope: SourceScope {
                        kind: SourceKind::Room,
                        room: Some(0),
                    },
                    archive_sha256: Digest([13; 32]),
                    stage_data_path: "room.dzr".into(),
                    stage_data_sha256: Digest([12; 32]),
                    kcl_path: Some("room.kcl".into()),
                    kcl_sha256: Some(kcl),
                    plc_path: Some("room.plc".into()),
                    plc_sha256: Some(plc),
                    addressable_prisms: 1,
                },
            ],
            chunks: Vec::new(),
            placements: vec![placement(
                "actor-record",
                PlacementKind::Actor,
                "kytag14",
                0,
            )],
            player_spawns: vec![placement(
                "spawn-record",
                PlacementKind::PlayerSpawn,
                "start",
                5,
            )],
            exits: vec![exit],
            collisions: vec![collision],
            load_triggers,
        }
    }

    fn boss_door_inventory(room: i8) -> WorldInventory {
        let room_resource =
            static_digest("9336aabaee513b635d6d0d3db3f5f3b67f5c6bd6643581ebd1a8f7b779fa8e7a");
        let exit_id = "dzr-sha256:boss-room/chunk/SCLS/record/0".to_string();
        WorldInventory {
            schema: WORLD_INVENTORY_SCHEMA.into(),
            stage: "D_MN05".into(),
            sources: vec![
                WorldSource {
                    scope: SourceScope {
                        kind: SourceKind::Stage,
                        room: None,
                    },
                    archive_sha256: Digest([10; 32]),
                    stage_data_path: "stage.dzs".into(),
                    stage_data_sha256: Digest([11; 32]),
                    kcl_path: None,
                    kcl_sha256: None,
                    plc_path: None,
                    plc_sha256: None,
                    addressable_prisms: 0,
                },
                WorldSource {
                    scope: SourceScope {
                        kind: SourceKind::Room,
                        room: Some(room),
                    },
                    archive_sha256: static_digest(
                        "5b495a915c1539b92f57e84f7cbcf0b5662a8caeaf7ecf0503ac15af7a6e6a77",
                    ),
                    stage_data_path: "room.dzr".into(),
                    stage_data_sha256: room_resource,
                    kcl_path: None,
                    kcl_sha256: None,
                    plc_path: None,
                    plc_sha256: None,
                    addressable_prisms: 0,
                },
            ],
            chunks: Vec::new(),
            placements: vec![PlacementRecord {
                stable_id: "dzr-sha256:boss-room/chunk/ACTR/record/0".into(),
                source_sha256: room_resource,
                scope: SourceScope {
                    kind: SourceKind::Room,
                    room: Some(room),
                },
                chunk_tag: "ACTR".into(),
                record_index: 0,
                layer: None,
                kind: PlacementKind::ScaledActor,
                name: "L1Bdoor".into(),
                parameters: 0x0191_8000,
                position: Vec3 {
                    x: 7283.0,
                    y: 3302.0,
                    z: -16430.0,
                },
                angle: [-211, 0, 0x1717],
                set_id: 0xff,
                scale_raw: Some([10, 10, 10]),
                raw_hex: "4c3142646f6f72000191800045e39800454e6000c6805c00ff2d0000171700ff0a0a0aff"
                    .into(),
            }],
            player_spawns: Vec::new(),
            exits: vec![StageExitRecord {
                stable_id: exit_id,
                source_sha256: room_resource,
                scope: SourceScope {
                    kind: SourceKind::Room,
                    room: Some(room),
                },
                chunk_tag: "SCLS".into(),
                record_index: 0,
                destination_stage: "D_MN05A".into(),
                destination_point: 0,
                destination_room: 50,
                destination_layer: -1,
                wipe: 0,
                wipe_time: 0,
                time_hour: -1,
                raw_start: 0,
                raw_field_a: 0,
                raw_field_b: 0,
                raw_wipe: 0,
                raw_hex: "00".repeat(13),
            }],
            collisions: Vec::new(),
            load_triggers: Vec::new(),
        }
    }

    fn l5_boss_door_inventory(boss_room: bool) -> WorldInventory {
        let room = if boss_room { 50 } else { 4 };
        let mut inventory = boss_door_inventory(room);
        inventory.stage = if boss_room { "D_MN11A" } else { "D_MN11" }.into();
        let (archive_sha256, resource_sha256, parameters, position, destination) = if boss_room {
            (
                static_digest("4acd3b8ce5ac24820364314c1cbec9569bf0faad2d4f0e6688e974616d8c7889"),
                static_digest("106533086f77371b6abd4cfea2d0d2c14fd88f5ce1a2569bfc8020848d2519a6"),
                0x0390_8200,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 2100.0,
                },
                ("F_SP114", 1, 2, 11, 1),
            )
        } else {
            (
                static_digest("6ee1274731222f3abe62c50de686fbae60663ba11e911f79c54fe886e221cd55"),
                static_digest("cd32b1ac737b8cfe6f92fa35a18ba4e24a5ccbfe8b466ae403b6c269dcbfc5c3"),
                0x0590_8200,
                Vec3 {
                    x: 0.0,
                    y: 2109.0,
                    z: -5237.0,
                },
                ("D_MN11A", 50, -1, 1, 2),
            )
        };
        inventory.sources[1].archive_sha256 = archive_sha256;
        inventory.sources[1].stage_data_sha256 = resource_sha256;
        let placement = &mut inventory.placements[0];
        placement.stable_id = format!("dzr-sha256:l5-{room}/chunk/Door/record/0");
        placement.source_sha256 = resource_sha256;
        placement.chunk_tag = "Door".into();
        placement.name = "L5Bdoor".into();
        placement.parameters = parameters;
        placement.position = position;
        placement.angle = [-1, 0, -248];
        placement.raw_hex = if boss_room {
            "4c3542646f6f720003908200000000000000000045034000ffff0000ff0800ff0a0a0aff"
        } else {
            "4c3542646f6f720005908200000000004503d000c5a3a800ffff0000ff0800ff0a0a0aff"
        }
        .into();
        let exit = &mut inventory.exits[0];
        exit.stable_id = format!("dzr-sha256:l5-{room}/chunk/SCLS/record/{}", destination.4);
        exit.source_sha256 = resource_sha256;
        exit.record_index = destination.4;
        exit.destination_stage = destination.0.into();
        exit.destination_room = destination.1;
        exit.destination_layer = destination.2;
        exit.destination_point = destination.3;
        inventory
    }

    fn replace_room_actor(
        mut inventory: WorldInventory,
        stage: &str,
        room: i8,
        placement: PlacementRecord,
        keep_exit: bool,
    ) -> WorldInventory {
        inventory.stage = stage.into();
        inventory.sources[1].scope.room = Some(room);
        inventory.sources[1].stage_data_sha256 = placement.source_sha256;
        inventory.placements = vec![placement];
        if keep_exit {
            inventory.exits[0].scope.room = Some(room);
            inventory.exits[0].source_sha256 = inventory.sources[1].stage_data_sha256;
        } else {
            inventory.exits.clear();
        }
        inventory
    }

    fn keyed_mboss_inventory() -> WorldInventory {
        let source =
            static_digest("2756f041cd797b24e2794983d3c6e0b370aa1dd50a57a08a2e437585688a268c");
        let mut inventory = replace_room_actor(
            boss_door_inventory(7),
            "D_MN06",
            7,
            PlacementRecord {
                stable_id: format!("dzr-sha256:{source}/chunk/Door/record/0"),
                source_sha256: source,
                scope: SourceScope {
                    kind: SourceKind::Room,
                    room: Some(7),
                },
                chunk_tag: "Door".into(),
                record_index: 0,
                layer: None,
                kind: PlacementKind::ScaledActor,
                name: "L6Mdoor".into(),
                parameters: 0x01b0_e600,
                position: Vec3 {
                    x: 1580.0,
                    y: 8250.0,
                    z: 700.0,
                },
                angle: [-1, 0x4000, -227],
                set_id: 0xff,
                scale_raw: Some([10, 10, 10]),
                raw_hex: "4c364d646f6f720001b0e60044c580004600e800442f0000ffff4000ff1d00ff0a0a0aff"
                    .into(),
            },
            true,
        );
        let exit = &mut inventory.exits[0];
        exit.stable_id = format!("dzr-sha256:{source}/chunk/SCLS/record/0");
        exit.destination_stage = "D_MN06B".into();
        exit.destination_room = 51;
        exit.destination_layer = -1;
        exit.destination_point = 0;
        inventory
    }

    fn regular_key_shutter_inventory() -> WorldInventory {
        let source =
            static_digest("f4a05b52105afd1dacb6b5b4b8e51706a922be24ea1b15fc198b47fd8aefb578");
        replace_room_actor(
            boss_door_inventory(9),
            "D_MN01",
            9,
            PlacementRecord {
                stable_id: format!("dzr-sha256:{source}/chunk/ACTR/record/4"),
                source_sha256: source,
                scope: SourceScope {
                    kind: SourceKind::Room,
                    room: Some(9),
                },
                chunk_tag: "ACTR".into(),
                record_index: 4,
                layer: None,
                kind: PlacementKind::Actor,
                name: "kshtr00".into(),
                parameters: 0x80ff_0123,
                position: Vec3 {
                    x: 15185.0,
                    y: -50.0,
                    z: -570.0,
                },
                angle: [0, -5461, 255],
                set_id: 0xffff,
                scale_raw: None,
                raw_hex: "6b7368747230300080ff0123466d4400c2480000c40e80000000eaab00ffffff".into(),
            },
            false,
        )
    }

    fn lakebed_boss_key_shutter_inventory() -> WorldInventory {
        let source =
            static_digest("9336aabaee513b635d6d0d3db3f5f3b67f5c6bd6643581ebd1a8f7b779fa8e7a");
        replace_room_actor(
            boss_door_inventory(3),
            "D_MN01",
            3,
            PlacementRecord {
                stable_id: format!("dzr-sha256:{source}/chunk/Door/record/0"),
                source_sha256: source,
                scope: SourceScope {
                    kind: SourceKind::Room,
                    room: Some(3),
                },
                chunk_tag: "Door".into(),
                record_index: 0,
                layer: None,
                kind: PlacementKind::ScaledActor,
                name: "L3Bdoor".into(),
                parameters: 0x80ff_0255,
                position: Vec3 {
                    x: 0.0,
                    y: -320.0,
                    z: 325.31067,
                },
                angle: [0, 0, 0],
                set_id: 0xff,
                scale_raw: Some([10, 10, 10]),
                raw_hex: "4c3342646f6f720080ff025500000000c3a0000043a2a7c400000000000000ff0a0a0aff"
                    .into(),
            },
            false,
        )
    }

    fn koki_gate_inventory(switch_id: u8) -> WorldInventory {
        let source =
            static_digest("5c2208b4088c8ac55dabca200f7bd7eedac3cf2c93364eb49fcfae2216513e21");
        let parameters = 0x0ff0_ff00 | u32::from(switch_id);
        let mut raw = "4b5f4761746500000ff0ff0cc688e00043480000c62eec00000026660000ffff".to_owned();
        raw.replace_range(22..24, &format!("{switch_id:02x}"));
        replace_room_actor(
            boss_door_inventory(3),
            "F_SP108",
            3,
            PlacementRecord {
                stable_id: format!("dzr-sha256:{source}/chunk/ACT0/record/36"),
                source_sha256: source,
                scope: SourceScope {
                    kind: SourceKind::Room,
                    room: Some(3),
                },
                chunk_tag: "ACT0".into(),
                record_index: 36,
                layer: Some(0),
                kind: PlacementKind::Actor,
                name: "K_Gate".into(),
                parameters,
                position: Vec3 {
                    x: -17520.0,
                    y: 200.0,
                    z: -11195.0,
                },
                angle: [0, 9830, 0],
                set_id: 0xffff,
                scale_raw: None,
                raw_hex: raw,
            },
            false,
        )
    }

    fn rider_gate_inventory() -> WorldInventory {
        let source =
            static_digest("22482b0344bcbb4a068562e088684590a3c27190ebfcc2d5c41a9a51c7b109f6");
        replace_room_actor(
            boss_door_inventory(0),
            "F_SP109",
            0,
            PlacementRecord {
                stable_id: format!("dzr-sha256:{source}/chunk/ACT0/record/10"),
                source_sha256: source,
                scope: SourceScope {
                    kind: SourceKind::Room,
                    room: Some(0),
                },
                chunk_tag: "ACT0".into(),
                record_index: 10,
                layer: Some(0),
                kind: PlacementKind::Actor,
                name: "R_Gate".into(),
                parameters: 0x0ff2_ff6b,
                position: Vec3 {
                    x: -8055.0,
                    y: 780.0,
                    z: -8235.0,
                },
                angle: [0, -16384, 0],
                set_id: 0xffff,
                scale_raw: None,
                raw_hex: "525f4761746500000ff2ff6bc5fbb80044430000c600ac000000c0000000ffff".into(),
            },
            false,
        )
    }

    fn caravan_gate_inventory() -> WorldInventory {
        let source =
            static_digest("1f60355fcaab8b2b0c4d32b62ac638049952d36f5fb3bc7a81472708402639a4");
        replace_room_actor(
            boss_door_inventory(1),
            "F_SP118",
            1,
            PlacementRecord {
                stable_id: format!("dzr-sha256:{source}/chunk/ACT0/record/23"),
                source_sha256: source,
                scope: SourceScope {
                    kind: SourceKind::Room,
                    room: Some(1),
                },
                chunk_tag: "ACT0".into(),
                record_index: 23,
                layer: Some(0),
                kind: PlacementKind::Actor,
                name: "CrvGate".into(),
                parameters: u32::MAX,
                position: Vec3 {
                    x: 2150.0,
                    y: 0.0,
                    z: -450.0,
                },
                angle: [0, -32768, 0],
                set_id: 0xffff,
                scale_raw: None,
                raw_hex: "4372764761746500ffffffff4506600000000000c3e10000000080000000ffff".into(),
            },
            false,
        )
    }

    fn l7_bridge_demo_inventory() -> WorldInventory {
        let source =
            static_digest("a7014eb0a33bb9a57af75caff72605f6725273909f5bd2cf61c465c140fe6a6e");
        let placement = PlacementRecord {
            stable_id: format!("dzr-sha256:{source}/chunk/ACTR/record/15"),
            source_sha256: source,
            scope: SourceScope {
                kind: SourceKind::Room,
                room: Some(6),
            },
            chunk_tag: "ACTR".into(),
            record_index: 15,
            layer: None,
            kind: PlacementKind::Actor,
            name: "dr".into(),
            parameters: 0x18,
            position: Vec3 {
                x: -7075.0,
                y: -200.0,
                z: -11809.403,
            },
            angle: [0, -32768, 0],
            set_id: 0xffff,
            scale_raw: None,
            raw_hex: "647200000000000000000018c5dd1800c3480000c638859d000080000000ffff".into(),
        };
        let mut inventory =
            replace_room_actor(boss_door_inventory(6), "D_MN07", 6, placement, false);
        inventory.exits = [
            (6, 7, 3, "445f4d4e303700000706f03301"),
            (7, 8, -1, "445f4d4e303700000806f03f00"),
        ]
        .into_iter()
        .map(|(record_index, spawn, layer, raw_hex)| StageExitRecord {
            stable_id: format!("dzr-sha256:{source}/chunk/SCLS/record/{record_index}"),
            source_sha256: source,
            scope: SourceScope {
                kind: SourceKind::Room,
                room: Some(6),
            },
            chunk_tag: "SCLS".into(),
            record_index,
            destination_stage: "D_MN07".into(),
            destination_point: spawn,
            destination_room: 6,
            destination_layer: layer,
            wipe: if record_index == 6 { 1 } else { 0 },
            wipe_time: 1,
            time_hour: -1,
            raw_start: spawn as u8,
            raw_field_a: 0xf0,
            raw_field_b: if record_index == 6 { 0x33 } else { 0x3f },
            raw_wipe: if record_index == 6 { 1 } else { 0 },
            raw_hex: raw_hex.into(),
        })
        .collect();
        inventory
    }

    fn has_small_key_comparison(
        transition: &CandidateTransition,
        expected_operator: ComparisonOperator,
        expected_value: u64,
    ) -> bool {
        let PredicateExpression::All { terms } = &transition.activation.hard_guards else {
            return false;
        };
        terms.iter().any(|term| {
            matches!(
                term,
                PredicateExpression::Compare {
                    left: ValueReference::BoundRawBits {
                        byte_offset: 0x1c,
                        byte_width: 1,
                        mask: 0xff,
                        ..
                    },
                    operator,
                    right: ValueReference::Literal {
                        value: StateValue::Unsigned(value),
                    },
                } if *operator == expected_operator && *value == expected_value
            )
        })
    }

    fn predicate_has_layer_comparison(
        predicate: &PredicateExpression,
        expected_operator: ComparisonOperator,
        expected_layer: i64,
    ) -> bool {
        match predicate {
            PredicateExpression::Compare {
                left: ValueReference::LocationLayer,
                operator,
                right:
                    ValueReference::Literal {
                        value: StateValue::Signed(layer),
                    },
            } => *operator == expected_operator && *layer == expected_layer,
            PredicateExpression::All { terms } | PredicateExpression::Any { terms } => {
                terms.iter().any(|term| {
                    predicate_has_layer_comparison(term, expected_operator, expected_layer)
                })
            }
            PredicateExpression::Not { term } => {
                predicate_has_layer_comparison(term, expected_operator, expected_layer)
            }
            _ => false,
        }
    }

    fn predicate_has_persistent_event_bit(predicate: &PredicateExpression, set: bool) -> bool {
        match predicate {
            PredicateExpression::Compare {
                left:
                    ValueReference::BoundRawBits {
                        component_kind: ComponentKind::Custom { id },
                        binding: ComponentBindingReference::ActiveRuntimeFile,
                        byte_offset: 0x08,
                        byte_width: 1,
                        mask: 0x10,
                    },
                operator: ComparisonOperator::Equal,
                right:
                    ValueReference::Literal {
                        value: StateValue::Unsigned(value),
                    },
            } => id == "persistent-event-registers" && *value == if set { 0x10 } else { 0 },
            PredicateExpression::All { terms } | PredicateExpression::Any { terms } => terms
                .iter()
                .any(|term| predicate_has_persistent_event_bit(term, set)),
            PredicateExpression::Not { term } => predicate_has_persistent_event_bit(term, set),
            _ => false,
        }
    }

    fn writes_small_key(transition: &CandidateTransition, expected_value: u8) -> bool {
        transition.activation.effects.iter().any(|effect| {
            matches!(
                effect,
                StateOperation::WriteBoundRaw {
                    component_kind: ComponentKind::DungeonMemory,
                    binding: ComponentBindingReference::CurrentStage,
                    byte_offset: 0x1c,
                    mask,
                    value,
                } if mask == &[0xff] && value == &[expected_value]
            )
        })
    }

    fn world_context(game_data_sha256: Digest, inventory: &WorldInventory) -> WorldContext {
        let context = WorldContext {
            schema: crate::world_data::WORLD_CONTEXT_SCHEMA.into(),
            game_data_sha256,
            stages: vec![crate::world_data::WorldContextStage {
                stage: inventory.stage.clone(),
                inventory_sha256: inventory.digest().unwrap(),
                spatial_index_sha256: Digest([99; 32]),
            }],
        };
        context.validate().unwrap();
        context
    }

    #[test]
    fn imports_joined_exit_as_obstructed_candidate_without_claiming_feasibility() {
        let content = content();
        let runtime = runtime(&content);
        let inventory = inventory(true);
        inventory.validate().unwrap();
        let context = world_context(Digest([2; 32]), &inventory);
        let facts = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&inventory),
        )
        .unwrap();
        assert_eq!(facts.static_world_objects.len(), 2);
        assert_eq!(facts.spawns[0].location.spawn, 5);
        assert_eq!(facts.mechanics.transitions.len(), 1);
        assert_eq!(facts.mechanics.obligations.len(), 1);
        assert_eq!(facts.approach_geometries.len(), 1);
        assert!(matches!(
            facts.approach_geometries[0].shape,
            ExtractedApproachShape::Unavailable { .. }
        ));
        assert_eq!(
            facts.approach_geometries[0].candidate_spawn_ids,
            vec![facts.spawns[0].id.clone()]
        );
        assert_eq!(facts.encoded_exits[0].candidate_transition_ids.len(), 1);
        let transition = &facts.mechanics.transitions[0];
        assert_eq!(transition.evidence.truth, TruthStatus::Contested);
        assert_eq!(transition.activation.unknown_requirements.len(), 1);
        assert_eq!(transition.activation.physical_obligation_ids.len(), 1);
        assert!(matches!(
            transition.activation.effects.as_slice(),
            [StateOperation::SetLocation { location }]
                if location.stage == "F_SP104" && location.room == 1 && location.spawn == 2
        ));
        let bytes = facts.canonical_bytes().unwrap();
        assert_eq!(
            ExtractedWorldFacts::decode_canonical(&bytes).unwrap(),
            facts
        );
        assert_ne!(facts.digest().unwrap(), Digest::ZERO);
    }

    #[test]
    fn derives_collision_triangle_bounds_and_same_room_spawn_candidates() {
        let content = content();
        let runtime = runtime(&content);
        let mut inventory = inventory(true);
        inventory.collisions[0].prism.reconstruction = KclReconstruction::Reconstructed {
            plane: crate::world_data::CollisionPlane {
                anchor: Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                normal: Vec3 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                },
                d: 0.0,
            },
            triangle: [
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                Vec3 {
                    x: 2.0,
                    y: 0.0,
                    z: 0.0,
                },
                Vec3 {
                    x: 0.0,
                    y: 3.0,
                    z: 0.0,
                },
            ],
        };
        inventory.validate().unwrap();
        let context = world_context(Digest([2; 32]), &inventory);
        let facts = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&inventory),
        )
        .unwrap();
        let geometry = &facts.approach_geometries[0];
        assert_eq!(geometry.transition_id, facts.mechanics.transitions[0].id);
        assert_eq!(
            geometry.approach_id,
            facts.mechanics.transitions[0].approach_id
        );
        assert_eq!(
            geometry.candidate_spawn_ids,
            vec![facts.spawns[0].id.clone()]
        );
        assert!(matches!(
            geometry.shape,
            ExtractedApproachShape::Reconstructed {
                minimum: [0.0, 0.0, 0.0],
                maximum: [2.0, 3.0, 0.0],
                plane_normal: [0.0, 1.0, 0.0],
                plane_offset: 0.0,
                ..
            }
        ));

        let mut tampered = facts;
        let ExtractedApproachShape::Reconstructed { maximum, .. } =
            &mut tampered.approach_geometries[0].shape
        else {
            unreachable!();
        };
        maximum[0] = 3.0;
        assert_eq!(
            tampered.validate().unwrap_err().field(),
            "approach_geometries.shape"
        );
    }

    #[test]
    fn imports_audited_gz2e01_boss_door_guard_write_and_destination() {
        assert_eq!(memory_switch_raw_location(0x00), (0x0b, 0x01));
        assert_eq!(memory_switch_raw_location(0x17), (0x09, 0x80));
        assert_eq!(memory_switch_raw_location(0x1f), (0x08, 0x80));
        assert_eq!(memory_switch_raw_location(0x20), (0x0f, 0x01));
        assert_eq!(memory_switch_raw_location(0x7f), (0x14, 0x80));
        let content = audited_content();
        let runtime = runtime(&content);
        let inventory = boss_door_inventory(12);
        inventory.validate().unwrap();
        let context = world_context(content.fingerprint.game_data_sha256, &inventory);
        let facts = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&inventory),
        )
        .unwrap();

        assert_eq!(facts.mechanics.transitions.len(), 1);
        assert_eq!(facts.mechanics.obligations.len(), 3);
        assert_eq!(facts.spatial_volumes.len(), 2);
        assert_eq!(facts.encoded_exits[0].candidate_transition_ids.len(), 1);
        let transition = &facts.mechanics.transitions[0];
        assert_eq!(transition.transition_kind, TransitionKind::Door);
        assert_eq!(transition.evidence.truth, TruthStatus::Established);
        assert_eq!(transition.activation.physical_obligation_ids.len(), 3);
        assert!(facts.mechanics.obligations.iter().any(|obligation| {
            matches!(
                &obligation.detail,
                ObligationDetail::CompoundInteraction { branches, .. }
                    if branches.len() == 2
                        && branches[1].volume_tests.iter().any(|test| {
                            test.position == crate::transition::InteractionPosition::PlayerAttention
                        })
                        && branches[1].volume_tests.iter().any(|test| {
                            test.position == crate::transition::InteractionPosition::Player
                                && test.volume.volume_id == "boss-door-wolf-current-x"
                        })
            )
        }));
        assert!(matches!(
            facts.spatial_volumes[1].shape,
            SpatialVolumeShape::YawOrientedStrip {
                axis: crate::state::SpatialLocalAxis::X,
                minimum: -130.0,
                maximum: 130.0,
                ..
            }
        ));
        let PredicateExpression::All { terms } = &transition.activation.hard_guards else {
            panic!("boss door must retain source location and boss-key guards")
        };
        assert!(terms.iter().any(|term| {
            matches!(
                term,
                PredicateExpression::Compare {
                    left: ValueReference::BoundRawBits {
                        component_kind: ComponentKind::DungeonMemory,
                        binding: ComponentBindingReference::CurrentStage,
                        byte_offset: 0x1d,
                        byte_width: 1,
                        mask: 0x04,
                    },
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Unsigned(0x04),
                    },
                }
            )
        }));
        assert!(matches!(
            transition.activation.effects.as_slice(),
            [
                StateOperation::WriteBoundRaw {
                    component_kind: ComponentKind::DungeonMemory,
                    binding: ComponentBindingReference::CurrentStage,
                    byte_offset: 0x09,
                    mask,
                    value,
                },
                StateOperation::SetLocation { location },
            ] if mask == &[0x80]
                && value == &[0x80]
                && location.stage == "D_MN05A"
                && location.room == 50
                && location.spawn == 0
        ));
    }

    #[test]
    fn does_not_generalize_boss_door_source_semantics_or_reverse_side() {
        let inventory = boss_door_inventory(12);
        let mut unaudited_content = audited_content();
        unaudited_content.fingerprint.executable_sha256 = Digest([0x55; 32]);
        let unaudited_runtime = runtime(&unaudited_content);
        let context = world_context(unaudited_content.fingerprint.game_data_sha256, &inventory);
        let facts = ExtractedWorldFacts::build(
            &unaudited_content,
            &unaudited_runtime,
            &context,
            std::slice::from_ref(&inventory),
        )
        .unwrap();
        assert!(facts.mechanics.transitions.is_empty());
        assert!(facts.encoded_exits[0].candidate_transition_ids.is_empty());

        let content = audited_content();
        let runtime = runtime(&content);
        let reverse_inventory = boss_door_inventory(50);
        let context = world_context(content.fingerprint.game_data_sha256, &reverse_inventory);
        let facts = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&reverse_inventory),
        )
        .unwrap();
        assert!(facts.mechanics.transitions.is_empty());
        assert!(facts.encoded_exits[0].candidate_transition_ids.is_empty());
    }

    #[test]
    fn imports_l5_human_guard_and_distinguishes_dungeon_from_boss_room_unlock() {
        let content = audited_content();
        let runtime = runtime(&content);
        let dungeon_inventory = l5_boss_door_inventory(false);
        let context = world_context(content.fingerprint.game_data_sha256, &dungeon_inventory);
        let dungeon = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&dungeon_inventory),
        )
        .unwrap();
        let transition = &dungeon.mechanics.transitions[0];
        let PredicateExpression::All { terms } = &transition.activation.hard_guards else {
            panic!("L5 boss door must retain location, boss-key, and form guards")
        };
        assert!(terms.iter().any(|term| {
            matches!(
                term,
                PredicateExpression::Compare {
                    left: ValueReference::PlayerForm,
                    operator: ComparisonOperator::Equal,
                    right: ValueReference::Literal {
                        value: StateValue::Text(form),
                    },
                } if form == "human"
            )
        }));
        assert!(matches!(
            transition.activation.effects.as_slice(),
            [
                StateOperation::WriteBoundRaw {
                    component_kind: ComponentKind::DungeonMemory,
                    binding: ComponentBindingReference::CurrentStage,
                    byte_offset: 0x0a,
                    mask,
                    value,
                },
                StateOperation::SetLocation { location },
            ] if mask == &[0x01]
                && value == &[0x01]
                && location.stage == "D_MN11A"
                && location.room == 50
                && location.spawn == 1
        ));
        assert!(transition.evidence.records.iter().any(|record| {
            record.source_sha256
                == Some(static_digest(
                    "9f649b99f027e39f1d39ce066d815a78032b536c4a9a83e0361681af2265102e",
                ))
        }));
        assert_eq!(dungeon.spatial_volumes.len(), 1);
        assert_eq!(dungeon.spatial_planes.len(), 1);
        assert!(matches!(
            &dungeon.spatial_volumes[0].shape,
            SpatialVolumeShape::YawOrientedRectangle {
                origin_xz,
                yaw: 0,
                minimum_local_xz,
                maximum_local_xz,
            } if origin_xz == &[0.0, -5237.0]
                && minimum_local_xz == &[-200.0, -100.0]
                && maximum_local_xz == &[200.0, 100.0]
        ));
        assert_eq!(dungeon.spatial_planes[0].normal, [0.0, 0.0, 1.0]);
        assert_eq!(dungeon.spatial_planes[0].offset, 5237.0);
        assert_eq!(transition.activation.physical_obligation_ids.len(), 4);
        assert!(dungeon.mechanics.obligations.iter().any(|obligation| {
            matches!(
                &obligation.detail,
                ObligationDetail::Interaction {
                    required_volumes,
                    ..
                } if required_volumes[0].volume_id == "boss-door-check-area"
            )
        }));
        assert!(dungeon.mechanics.obligations.iter().any(|obligation| {
            matches!(
                &obligation.detail,
                ObligationDetail::PlaneSide {
                    plane_id,
                    relation: crate::state::PlaneRelation::Positive,
                } if plane_id == &dungeon.spatial_planes[0].plane_id
            )
        }));

        let boss_room_inventory = l5_boss_door_inventory(true);
        let context = world_context(content.fingerprint.game_data_sha256, &boss_room_inventory);
        let boss_room = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&boss_room_inventory),
        )
        .unwrap();
        assert!(matches!(
            boss_room.mechanics.transitions[0]
                .activation
                .effects
                .as_slice(),
            [StateOperation::SetLocation { location }]
                if location.stage == "F_SP114"
                    && location.room == 1
                    && location.layer == 2
                    && location.spawn == 11
        ));
    }

    #[test]
    fn imports_keyed_mboss_first_open_and_reopen_as_distinct_branches() {
        let content = audited_content();
        let runtime = runtime(&content);
        let inventory = keyed_mboss_inventory();
        inventory.validate().unwrap();
        let context = world_context(content.fingerprint.game_data_sha256, &inventory);
        let facts = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&inventory),
        )
        .unwrap();

        assert_eq!(facts.schema, EXTRACTED_WORLD_FACTS_SCHEMA);
        assert_eq!(facts.mechanics.transitions.len(), 3);
        assert_eq!(facts.mechanics.obligations.len(), 2);
        assert_eq!(facts.encoded_exits[0].candidate_transition_ids.len(), 3);
        let first_open =
            facts
                .mechanics
                .transitions
                .iter()
                .find(|transition| {
                    transition.activation.effects.iter().any(|effect| {
                        matches!(effect, StateOperation::AdjustBoundRawUnsigned { .. })
                    })
                })
                .expect("first keyed opening branch");
        assert_eq!(first_open.transition_kind, TransitionKind::Door);
        assert!(matches!(
            first_open.activation.effects.as_slice(),
            [
                StateOperation::WriteBoundRaw {
                    component_kind: ComponentKind::DungeonMemory,
                    binding: ComponentBindingReference::CurrentStage,
                    byte_offset: 0x08,
                    mask,
                    value,
                },
                StateOperation::AdjustBoundRawUnsigned {
                    component_kind: ComponentKind::DungeonMemory,
                    binding: ComponentBindingReference::CurrentStage,
                    byte_offset: 0x1c,
                    byte_width: 1,
                    delta: -1,
                },
                StateOperation::SetLocation { location },
            ] if mask == &[0x20]
                && value == &[0x20]
                && location.stage == "D_MN06B"
                && location.room == 51
        ));
        let PredicateExpression::All { terms } = &first_open.activation.hard_guards else {
            panic!("first opening must retain location, switch, and key guards")
        };
        assert!(terms.iter().any(|term| matches!(
            term,
            PredicateExpression::Compare {
                left: ValueReference::BoundRawBits {
                    byte_offset: 0x1c,
                    byte_width: 1,
                    mask: 0xff,
                    ..
                },
                operator: ComparisonOperator::GreaterThan,
                right: ValueReference::Literal {
                    value: StateValue::Unsigned(0),
                },
            }
        )));
        assert!(has_small_key_comparison(
            first_open,
            ComparisonOperator::LessThanOrEqual,
            100,
        ));
        let reopen = facts
            .mechanics
            .transitions
            .iter()
            .find(|transition| transition.activation.effects.len() == 1)
            .expect("already-unlocked reopening branch");
        assert!(matches!(
            reopen.activation.effects.as_slice(),
            [StateOperation::SetLocation { location }]
                if location.stage == "D_MN06B" && location.room == 51
        ));
        let high = facts
            .mechanics
            .transitions
            .iter()
            .find(|transition| writes_small_key(transition, 99))
            .expect("high raw key-count clamp branch");
        assert!(has_small_key_comparison(
            high,
            ComparisonOperator::GreaterThan,
            100,
        ));
    }

    #[test]
    fn imports_regular_key_shutter_switch_and_small_key_mutation() {
        let content = audited_content();
        let runtime = runtime(&content);
        let inventory = regular_key_shutter_inventory();
        inventory.validate().unwrap();
        let context = world_context(content.fingerprint.game_data_sha256, &inventory);
        let facts = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&inventory),
        )
        .unwrap();

        assert_eq!(facts.mechanics.transitions.len(), 2);
        assert_eq!(facts.mechanics.obligations.len(), 2);
        let transition =
            facts
                .mechanics
                .transitions
                .iter()
                .find(|transition| {
                    transition.activation.effects.iter().any(|effect| {
                        matches!(effect, StateOperation::AdjustBoundRawUnsigned { .. })
                    })
                })
                .expect("ordinary key decrement branch");
        assert_eq!(transition.transition_kind, TransitionKind::ActorDriven);
        assert!(matches!(
            transition.activation.effects.as_slice(),
            [
                StateOperation::WriteBoundRaw {
                    byte_offset: 0x0f,
                    mask,
                    value,
                    ..
                },
                StateOperation::AdjustBoundRawUnsigned {
                    byte_offset: 0x1c,
                    byte_width: 1,
                    delta: -1,
                    ..
                },
            ] if mask == &[0x08] && value == &[0x08]
        ));
        assert!(has_small_key_comparison(
            transition,
            ComparisonOperator::LessThanOrEqual,
            100,
        ));
        assert!(transition.evidence.records.iter().any(|record| {
            record.source_sha256
                == Some(static_digest(
                    "3bff3ce52a0c1660d5ccf0bdcae24b672e50013317b3469698c51e32336c159a",
                ))
        }));
        let high = facts
            .mechanics
            .transitions
            .iter()
            .find(|transition| writes_small_key(transition, 99))
            .expect("high raw key-count clamp branch");
        assert!(has_small_key_comparison(
            high,
            ComparisonOperator::GreaterThan,
            100,
        ));
    }

    #[test]
    fn imports_lakebed_boss_shutter_zero_normal_and_high_small_key_outcomes() {
        let content = audited_content();
        let runtime = runtime(&content);
        let inventory = lakebed_boss_key_shutter_inventory();
        inventory.validate().unwrap();
        let context = world_context(content.fingerprint.game_data_sha256, &inventory);
        let facts = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&inventory),
        )
        .unwrap();

        assert_eq!(facts.mechanics.transitions.len(), 3);
        assert_eq!(facts.mechanics.obligations.len(), 2);
        let positive =
            facts
                .mechanics
                .transitions
                .iter()
                .find(|transition| {
                    transition.activation.effects.iter().any(|effect| {
                        matches!(effect, StateOperation::AdjustBoundRawUnsigned { .. })
                    })
                })
                .expect("boss-key opening with a small key");
        assert!(matches!(
            positive.activation.effects.as_slice(),
            [
                StateOperation::WriteBoundRaw {
                    byte_offset: 0x11,
                    mask,
                    value,
                    ..
                },
                StateOperation::AdjustBoundRawUnsigned {
                    byte_offset: 0x1c,
                    byte_width: 1,
                    delta: -1,
                    ..
                },
            ] if mask == &[0x20] && value == &[0x20]
        ));
        assert!(has_small_key_comparison(
            positive,
            ComparisonOperator::LessThanOrEqual,
            100,
        ));
        let zero = facts
            .mechanics
            .transitions
            .iter()
            .find(|transition| transition.activation.effects.len() == 1)
            .expect("boss-key opening with zero small keys");
        let PredicateExpression::All { terms } = &zero.activation.hard_guards else {
            panic!("zero-key branch must retain boss-key and zero-key guards")
        };
        assert!(terms.iter().any(|term| matches!(
            term,
            PredicateExpression::Compare {
                left: ValueReference::BoundRawBits {
                    byte_offset: 0x1d,
                    mask: 0x04,
                    ..
                },
                right: ValueReference::Literal {
                    value: StateValue::Unsigned(0x04),
                },
                ..
            }
        )));
        let high = facts
            .mechanics
            .transitions
            .iter()
            .find(|transition| writes_small_key(transition, 99))
            .expect("high raw key-count clamp branch");
        assert!(has_small_key_comparison(
            high,
            ComparisonOperator::GreaterThan,
            100,
        ));
        assert!(terms.iter().any(|term| matches!(
            term,
            PredicateExpression::Compare {
                left: ValueReference::BoundRawBits {
                    byte_offset: 0x1c,
                    mask: 0xff,
                    ..
                },
                operator: ComparisonOperator::Equal,
                right: ValueReference::Literal {
                    value: StateValue::Unsigned(0),
                },
            }
        )));
    }

    #[test]
    fn imports_only_memory_switch_backed_type_zero_koki_gate_unlocks() {
        let content = audited_content();
        let runtime = runtime(&content);
        let inventory = koki_gate_inventory(0x0c);
        inventory.validate().unwrap();
        let context = world_context(content.fingerprint.game_data_sha256, &inventory);
        let facts = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&inventory),
        )
        .unwrap();

        assert_eq!(facts.mechanics.transitions.len(), 2);
        let ordinary =
            facts
                .mechanics
                .transitions
                .iter()
                .find(|transition| {
                    transition.activation.effects.iter().any(|effect| {
                        matches!(effect, StateOperation::AdjustBoundRawUnsigned { .. })
                    })
                })
                .expect("ordinary key decrement branch");
        assert!(matches!(
            ordinary.activation.effects.as_slice(),
            [
                StateOperation::AdjustBoundRawUnsigned {
                    byte_offset: 0x1c,
                    byte_width: 1,
                    delta: -1,
                    ..
                },
                StateOperation::WriteBoundRaw {
                    byte_offset: 0x0a,
                    mask,
                    value,
                    ..
                },
            ] if mask == &[0x10] && value == &[0x10]
        ));
        assert!(has_small_key_comparison(
            ordinary,
            ComparisonOperator::LessThanOrEqual,
            100,
        ));
        let PredicateExpression::All { terms } = &ordinary.activation.hard_guards else {
            panic!("gate must retain location, switch, and key guards")
        };
        assert!(terms.iter().any(|term| matches!(
            term,
            PredicateExpression::All { terms }
                if terms.iter().any(|nested| matches!(
                    nested,
                    PredicateExpression::Compare {
                        left: ValueReference::LocationLayer,
                        right: ValueReference::Literal {
                            value: StateValue::Signed(0),
                        },
                        ..
                    }
                ))
        )));
        let high = facts
            .mechanics
            .transitions
            .iter()
            .find(|transition| writes_small_key(transition, 99))
            .expect("high raw key-count clamp branch");
        assert!(has_small_key_comparison(
            high,
            ComparisonOperator::GreaterThan,
            100,
        ));

        let absent_switch_inventory = koki_gate_inventory(0xff);
        absent_switch_inventory.validate().unwrap();
        let context = world_context(
            content.fingerprint.game_data_sha256,
            &absent_switch_inventory,
        );
        let absent_switch = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&absent_switch_inventory),
        )
        .unwrap();
        assert!(absent_switch.mechanics.transitions.is_empty());
        assert!(absent_switch.mechanics.obligations.is_empty());
    }

    #[test]
    fn imports_fsp109_rider_gate_unlock_and_m035_bypass_without_conflating_them() {
        let content = audited_content();
        let runtime = runtime(&content);
        let inventory = rider_gate_inventory();
        inventory.validate().unwrap();
        let context = world_context(content.fingerprint.game_data_sha256, &inventory);
        let facts = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&inventory),
        )
        .unwrap();

        assert_eq!(facts.mechanics.transitions.len(), 4);
        assert_eq!(facts.mechanics.obligations.len(), 3);
        let ordinary = facts
            .mechanics
            .transitions
            .iter()
            .find(|transition| {
                transition.label.contains("keyed rider-gate unlock")
                    && transition.activation.effects.iter().any(|effect| {
                        matches!(effect, StateOperation::AdjustBoundRawUnsigned { .. })
                    })
            })
            .unwrap();
        assert!(predicate_has_persistent_event_bit(
            &ordinary.activation.hard_guards,
            false,
        ));
        assert!(has_small_key_comparison(
            ordinary,
            ComparisonOperator::GreaterThan,
            0,
        ));
        assert!(matches!(
            ordinary.activation.effects.as_slice(),
            [
                StateOperation::AdjustBoundRawUnsigned {
                    byte_offset: 0x1c,
                    delta: -1,
                    ..
                },
                StateOperation::WriteBoundRaw {
                    byte_offset: 0x16,
                    mask,
                    value,
                    ..
                },
            ] if mask == &[0x08] && value == &[0x08]
        ));

        let event_bypass = facts
            .mechanics
            .transitions
            .iter()
            .find(|transition| transition.label.contains("M_035 forced-open"))
            .unwrap();
        assert!(event_bypass.activation.effects.is_empty());
        assert!(predicate_has_persistent_event_bit(
            &event_bypass.activation.hard_guards,
            true,
        ));
        assert!(!has_small_key_comparison(
            event_bypass,
            ComparisonOperator::GreaterThan,
            0,
        ));

        let unlocked = facts
            .mechanics
            .transitions
            .iter()
            .find(|transition| transition.label.contains("set-switch physical passage"))
            .unwrap();
        assert!(unlocked.activation.effects.is_empty());
        assert!(predicate_has_persistent_event_bit(
            &unlocked.activation.hard_guards,
            false,
        ));

        let mut fsp121 = inventory;
        fsp121.stage = "F_SP121".into();
        fsp121.sources[1].scope.room = Some(3);
        fsp121.placements[0].scope.room = Some(3);
        fsp121.placements[0].parameters = (fsp121.placements[0].parameters & !0xff) | 0x82;
        let context = world_context(content.fingerprint.game_data_sha256, &fsp121);
        let imported =
            ExtractedWorldFacts::build(&content, &runtime, &context, std::slice::from_ref(&fsp121))
                .unwrap();
        assert_eq!(imported.mechanics.transitions.len(), 4);
        assert_eq!(imported.mechanics.obligations.len(), 3);
        let unlock = imported
            .mechanics
            .transitions
            .iter()
            .find(|transition| transition.label.contains("keyed rider-gate unlock"))
            .unwrap();
        assert!(unlock.activation.effects.iter().any(|effect| matches!(
            effect,
            StateOperation::WriteBoundRaw {
                component_kind: ComponentKind::Custom { id },
                binding: ComponentBindingReference::CurrentStage,
                byte_offset: 2,
                mask,
                value,
            } if id == DUNGEON_SESSION_SWITCH_LABEL_KIND && mask == &[1] && value == &[1]
        )));

        let mut wrong_room = fsp121;
        wrong_room.sources[1].scope.room = Some(4);
        wrong_room.placements[0].scope.room = Some(4);
        let context = world_context(content.fingerprint.game_data_sha256, &wrong_room);
        let excluded = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&wrong_room),
        )
        .unwrap();
        assert!(excluded.mechanics.transitions.is_empty());
    }

    #[test]
    fn imports_rsp116_wolf_chain_writer_and_vshuter_consumer_as_causal_pair() {
        let content = audited_content();
        let runtime = runtime(&content);
        let mut inventory = regular_key_shutter_inventory();
        inventory.stage = "R_SP116".into();
        inventory.sources[1].scope.room = Some(6);
        let placement = &mut inventory.placements[0];
        placement.name = "vshuter".into();
        placement.scope.room = Some(6);
        placement.parameters = 0x00ff_03ef;
        let mut chain = placement.clone();
        chain.stable_id = format!("{}/wchain", chain.stable_id);
        chain.name = "Wchain".into();
        chain.parameters = 0x0000_0fef;
        chain.raw_hex = "57636861696e000000000fefbefd6d7fc4fa0000c4a71af4000000000000ffff".into();
        inventory.placements.push(chain);
        let context = world_context(content.fingerprint.game_data_sha256, &inventory);
        let facts = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&inventory),
        )
        .unwrap();

        assert_eq!(facts.mechanics.transitions.len(), 2);
        assert_eq!(facts.mechanics.obligations.len(), 4);
        let writer = facts
            .mechanics
            .transitions
            .iter()
            .find(|transition| transition.label.contains("wolf-chain pull"))
            .unwrap();
        assert!(matches!(
            writer.activation.effects.as_slice(),
            [StateOperation::WriteBoundRaw {
                component_kind: ComponentKind::Custom { id },
                binding: ComponentBindingReference::CurrentRoom,
                byte_offset: 0xef,
                mask,
                value,
            }] if id == ROOM_SWITCH_LABEL_KIND && mask == &[1] && value == &[1]
        ));
        assert!(matches!(
            &writer.activation.hard_guards,
            PredicateExpression::All { terms }
                if terms.iter().any(|term| matches!(
                    term,
                    PredicateExpression::Compare {
                        left: ValueReference::PlayerForm,
                        operator: ComparisonOperator::Equal,
                        right: ValueReference::Literal {
                            value: StateValue::Text(form),
                        },
                    } if form == "wolf"
                ))
        ));

        let passage = facts
            .mechanics
            .transitions
            .iter()
            .find(|transition| transition.label.contains("externally switched passage"))
            .unwrap();
        assert!(passage.activation.effects.is_empty());
        assert!(matches!(
            &passage.activation.hard_guards,
            PredicateExpression::All { terms }
                if terms.iter().any(|term| matches!(
                    term,
                    PredicateExpression::Compare {
                        left: ValueReference::BoundRawBits {
                            component_kind: ComponentKind::Custom { id },
                            binding: ComponentBindingReference::CurrentRoom,
                            byte_offset: 0xef,
                            byte_width: 1,
                            mask: 1,
                        },
                        operator: ComparisonOperator::Equal,
                        right: ValueReference::Literal {
                            value: StateValue::Unsigned(1),
                        },
                    } if id == ROOM_SWITCH_LABEL_KIND
                ))
        ));
    }

    #[test]
    fn imports_fsp118_caravan_key_opening_and_boar_bypass_as_transient_branches() {
        let content = audited_content();
        let runtime = runtime(&content);
        let inventory = caravan_gate_inventory();
        inventory.validate().unwrap();
        let context = world_context(content.fingerprint.game_data_sha256, &inventory);
        let facts = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&inventory),
        )
        .unwrap();

        assert_eq!(facts.mechanics.transitions.len(), 3);
        assert_eq!(facts.mechanics.obligations.len(), 3);
        let ordinary = facts
            .mechanics
            .transitions
            .iter()
            .find(|transition| transition.label.ends_with("transient key opening"))
            .unwrap();
        assert!(has_small_key_comparison(
            ordinary,
            ComparisonOperator::GreaterThan,
            0,
        ));
        assert!(matches!(
            ordinary.activation.effects.as_slice(),
            [StateOperation::AdjustBoundRawUnsigned {
                byte_offset: 0x1c,
                delta: -1,
                ..
            }]
        ));
        assert!(
            !ordinary
                .activation
                .effects
                .iter()
                .any(|effect| matches!(effect, StateOperation::WriteBoundRaw { .. }))
        );

        let high = facts
            .mechanics
            .transitions
            .iter()
            .find(|transition| transition.label.contains("clamped to 99"))
            .unwrap();
        assert!(writes_small_key(high, 99));
        let boar = facts
            .mechanics
            .transitions
            .iter()
            .find(|transition| transition.label.contains("boar destruction bypass"))
            .unwrap();
        assert!(boar.activation.effects.is_empty());
        assert!(!has_small_key_comparison(
            boar,
            ComparisonOperator::GreaterThan,
            0,
        ));
        assert_eq!(boar.activation.physical_obligation_ids.len(), 1);

        let mut unrelated = inventory;
        unrelated.stage = "F_SP117".into();
        let context = world_context(content.fingerprint.game_data_sha256, &unrelated);
        let excluded = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&unrelated),
        )
        .unwrap();
        assert!(excluded.mechanics.transitions.is_empty());
    }

    #[test]
    fn imports_l7_bridge_demo_as_two_distinct_scls_backed_actor_transitions() {
        let content = audited_content();
        let runtime = runtime(&content);
        let inventory = l7_bridge_demo_inventory();
        inventory.validate().unwrap();
        let context = world_context(content.fingerprint.game_data_sha256, &inventory);
        let facts = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&inventory),
        )
        .unwrap();

        assert_eq!(facts.mechanics.transitions.len(), 2);
        assert_eq!(facts.mechanics.obligations.len(), 3);
        let enter = facts
            .mechanics
            .transitions
            .iter()
            .find(|transition| transition.label.contains("SCLS 6"))
            .unwrap();
        assert_eq!(enter.transition_kind, TransitionKind::ActorDriven);
        assert!(has_small_key_comparison(
            enter,
            ComparisonOperator::GreaterThan,
            0,
        ));
        assert!(!enter.activation.effects.iter().any(|effect| matches!(
            effect,
            StateOperation::AdjustBoundRawUnsigned { .. }
                | StateOperation::WriteBoundRaw {
                    byte_offset: 0x1c,
                    ..
                }
        )));
        assert!(enter.activation.effects.iter().any(|effect| matches!(
            effect,
            StateOperation::SetLocation { location }
                if location.stage == "D_MN07"
                    && location.room == 6
                    && location.layer == 3
                    && location.spawn == 7
        )));
        assert!(predicate_has_layer_comparison(
            &enter.activation.hard_guards,
            ComparisonOperator::NotEqual,
            3,
        ));

        let destroy = facts
            .mechanics
            .transitions
            .iter()
            .find(|transition| transition.label.contains("SCLS 7"))
            .unwrap();
        assert!(!has_small_key_comparison(
            destroy,
            ComparisonOperator::GreaterThan,
            0,
        ));
        assert!(predicate_has_layer_comparison(
            &destroy.activation.hard_guards,
            ComparisonOperator::Equal,
            3,
        ));
        assert!(matches!(
            destroy.activation.effects.as_slice(),
            [
                StateOperation::WriteBoundRaw {
                    byte_offset: 0x08,
                    mask,
                    value,
                    ..
                },
                StateOperation::SetLocation { location },
            ] if mask == &[0x01]
                && value == &[0x01]
                && location.stage == "D_MN07"
                && location.room == 6
                && location.layer == -1
                && location.spawn == 8
        ));
        for record_index in [6, 7] {
            let exit = facts
                .encoded_exits
                .iter()
                .find(|exit| {
                    exit.source_record_id
                        .ends_with(&format!("record/{record_index}"))
                })
                .unwrap();
            assert_eq!(exit.candidate_transition_ids.len(), 1);
        }
    }

    #[test]
    fn imports_wrapped_type_zero_shutter_and_each_mboss_event_resource() {
        let scope = ContextScope {
            selectors: vec![ContextSelector::Exact {
                context: ExactContext {
                    content_sha256: Digest([0x11; 32]),
                    runtime_configuration_sha256: Digest([0x22; 32]),
                },
            }],
        };
        let inventory = regular_key_shutter_inventory();
        let mut shutter = inventory.placements[0].clone();
        shutter.parameters = 0x80ff_ff2b;
        let imported =
            import_gz2e01_keyed_actor_actions(&inventory, &shutter, &scope, Digest([0x33; 32]))
                .unwrap()
                .expect("authored 0xff wraps to the supported runtime type zero");
        assert_eq!(imported.transitions.len(), 2);
        assert!(
            imported.transitions[0]
                .evidence
                .records
                .iter()
                .any(|record| {
                    record.source_sha256
                        == Some(static_digest(
                            "8676effbd561ba65f8e4a8b9493aa6b60072d40f72a8e240b2ffa9c5550b40fa",
                        ))
                })
        );

        let inventory = keyed_mboss_inventory();
        for (name, event_sha256) in [
            (
                "L7door",
                "7de6bfac10e3ca6c3f6bc88a83815972d3397fd3488b067398cdd8cb0ea0cce4",
            ),
            (
                "L8Mdoor",
                "b079b8b284208582d9a37b50bd94f13400530abca75db0771147a646a8d83627",
            ),
        ] {
            let mut placement = inventory.placements[0].clone();
            placement.name = name.into();
            let imported = import_gz2e01_keyed_actor_actions(
                &inventory,
                &placement,
                &scope,
                Digest([0x33; 32]),
            )
            .unwrap()
            .expect("audited keyed mini-boss alias");
            assert_eq!(imported.transitions.len(), 3);
            assert!(
                imported.transitions[0]
                    .evidence
                    .records
                    .iter()
                    .any(|record| record.source_sha256 == Some(static_digest(event_sha256)))
            );
        }
    }

    #[test]
    fn excludes_unaudited_keyed_family_bypasses_and_non_memory_switches() {
        let scope = ContextScope {
            selectors: vec![ContextSelector::Exact {
                context: ExactContext {
                    content_sha256: Digest([0x11; 32]),
                    runtime_configuration_sha256: Digest([0x22; 32]),
                },
            }],
        };
        let inventory = regular_key_shutter_inventory();
        let mut placement = inventory.placements[0].clone();
        for name in ["vshuter"] {
            placement.name = name.into();
            assert!(
                import_gz2e01_keyed_actor_actions(
                    &inventory,
                    &placement,
                    &scope,
                    Digest([0x33; 32]),
                )
                .unwrap()
                .is_none()
            );
        }

        placement = inventory.placements[0].clone();
        placement.parameters &= 0x7fff_ffff;
        assert!(
            import_gz2e01_keyed_actor_actions(&inventory, &placement, &scope, Digest([0x33; 32]),)
                .unwrap()
                .is_none()
        );

        let gate_inventory = koki_gate_inventory(0x82);
        let mut gate = gate_inventory.placements[0].clone();
        assert!(
            import_gz2e01_keyed_actor_actions(&gate_inventory, &gate, &scope, Digest([0x33; 32]),)
                .unwrap()
                .is_none()
        );
        gate.parameters = (gate.parameters & !(0x0f << 16)) | (1 << 16);
        assert!(
            import_gz2e01_keyed_actor_actions(&gate_inventory, &gate, &scope, Digest([0x33; 32]),)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn encoded_exits_reference_every_location_changing_world_transition() {
        let content = audited_content();
        let runtime = runtime(&content);

        let gate_inventory = koki_gate_inventory(0x0c);
        let context = world_context(content.fingerprint.game_data_sha256, &gate_inventory);
        let mut actor_facts = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&gate_inventory),
        )
        .unwrap();
        actor_facts.mechanics.transitions[0].transition_kind = TransitionKind::Door;
        assert_eq!(
            actor_facts.validate().unwrap_err().field(),
            "mechanics.transitions"
        );

        let door_inventory = keyed_mboss_inventory();
        let context = world_context(content.fingerprint.game_data_sha256, &door_inventory);
        let mut door_facts = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&door_inventory),
        )
        .unwrap();
        door_facts.mechanics.transitions[0].transition_kind = TransitionKind::ActorDriven;
        door_facts.validate().unwrap();
        let transition_id = door_facts.mechanics.transitions[0].id.clone();
        door_facts
            .encoded_exits
            .iter_mut()
            .find(|exit| exit.candidate_transition_ids.contains(&transition_id))
            .unwrap()
            .candidate_transition_ids
            .retain(|id| id != &transition_id);
        assert_eq!(
            door_facts.validate().unwrap_err().field(),
            "mechanics.transitions"
        );
    }

    #[test]
    fn keeps_unjoined_scls_as_encoded_fact_without_inventing_transition() {
        let content = content();
        let runtime = runtime(&content);
        let inventory = inventory(false);
        inventory.validate().unwrap();
        let context = world_context(Digest([2; 32]), &inventory);
        let facts = ExtractedWorldFacts::build(
            &content,
            &runtime,
            &context,
            std::slice::from_ref(&inventory),
        )
        .unwrap();
        assert!(facts.mechanics.transitions.is_empty());
        assert!(facts.mechanics.obligations.is_empty());
        assert!(facts.encoded_exits[0].candidate_transition_ids.is_empty());
    }

    #[test]
    fn exact_content_and_world_context_must_agree() {
        let content = content();
        let runtime = runtime(&content);
        let inventory = inventory(false);
        let context = world_context(Digest([9; 32]), &inventory);
        assert_eq!(
            ExtractedWorldFacts::build(
                &content,
                &runtime,
                &context,
                std::slice::from_ref(&inventory),
            )
            .unwrap_err()
            .field(),
            "world_context.game_data_sha256"
        );
    }
}
