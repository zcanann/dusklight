//! Planner-owned input contracts for immutable extracted world data.
//!
//! These schemas intentionally remain wire-compatible with existing world
//! inventory artifacts. Compatibility is a data boundary, not a Rust
//! dependency on the producer that happened to emit them.

use crate::artifact::Digest;
use crate::{PlannerContractError, canonical_json};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

pub const WORLD_CONTEXT_SCHEMA: &str = "dusklight-world-context/v1";
pub const WORLD_INVENTORY_SCHEMA: &str = "dusklight-world-inventory/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorldContextStage {
    pub stage: String,
    pub inventory_sha256: Digest,
    pub spatial_index_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorldContext {
    pub schema: String,
    pub game_data_sha256: Digest,
    pub stages: Vec<WorldContextStage>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KclSourceIndices {
    pub position: u16,
    pub face_normal: u16,
    pub edge_normal_1: u16,
    pub edge_normal_2: u16,
    pub edge_normal_3: u16,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CollisionCode {
    pub raw: [u32; 5],
    pub exit_id: u8,
    pub polygon_color: u8,
    pub special_code: u8,
    pub link_no: u8,
    pub wall_code: u8,
    pub attribute_0: u8,
    pub attribute_1: u8,
    pub ground_code: u8,
    pub camera_move_background: u8,
    pub room_camera: u8,
    pub room_path: u8,
    pub room_path_point: u8,
    pub room_info: u8,
    pub sound_id: u8,
    pub room: u8,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CollisionPlane {
    pub anchor: Vec3,
    pub normal: Vec3,
    pub d: f32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KclAuthoredPrism {
    pub stable_id: String,
    pub prism_index: u16,
    pub height: f32,
    pub source_indices: KclSourceIndices,
    pub attribute: u16,
    pub code: CollisionCode,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum KclReconstruction {
    Reconstructed {
        plane: CollisionPlane,
        triangle: [Vec3; 3],
    },
    Degenerate {
        reason: String,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KclInventoryPrism {
    pub authored: KclAuthoredPrism,
    pub reconstruction: KclReconstruction,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Stage,
    Room,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceScope {
    pub kind: SourceKind,
    pub room: Option<i8>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorldSource {
    pub scope: SourceScope,
    pub archive_sha256: Digest,
    pub stage_data_path: String,
    pub stage_data_sha256: Digest,
    pub kcl_path: Option<String>,
    pub kcl_sha256: Option<Digest>,
    pub plc_path: Option<String>,
    pub plc_sha256: Option<Digest>,
    pub addressable_prisms: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageChunkSummary {
    pub source_sha256: Digest,
    pub scope: SourceScope,
    pub tag: String,
    pub record_count: usize,
    pub data_offset: usize,
    pub recognized_record_size: Option<usize>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlacementKind {
    Actor,
    ScaledActor,
    Treasure,
    PlayerSpawn,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlacementRecord {
    pub stable_id: String,
    pub source_sha256: Digest,
    pub scope: SourceScope,
    pub chunk_tag: String,
    pub record_index: usize,
    pub layer: Option<u8>,
    pub kind: PlacementKind,
    pub name: String,
    pub parameters: u32,
    pub position: Vec3,
    pub angle: [i16; 3],
    pub set_id: u16,
    pub scale_raw: Option<[u8; 3]>,
    pub raw_hex: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageExitRecord {
    pub stable_id: String,
    pub source_sha256: Digest,
    pub scope: SourceScope,
    pub chunk_tag: String,
    pub record_index: usize,
    pub destination_stage: String,
    pub destination_point: i16,
    pub destination_room: i8,
    pub destination_layer: i8,
    pub wipe: u8,
    pub wipe_time: u8,
    pub time_hour: i8,
    pub raw_start: u8,
    pub raw_field_a: u8,
    pub raw_field_b: u8,
    pub raw_wipe: u8,
    pub raw_hex: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CollisionInventoryRecord {
    pub room: i8,
    pub prism: KclInventoryPrism,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CollisionLoadTrigger {
    pub stable_id: String,
    pub room: i8,
    pub collision_id: String,
    pub collision_exit_id: u8,
    pub scls_id: String,
    pub destination_stage: String,
    pub destination_room: i8,
    pub destination_layer: i8,
    pub destination_point: i16,
    pub inferred_semantics: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorldInventory {
    pub schema: String,
    pub stage: String,
    pub sources: Vec<WorldSource>,
    pub chunks: Vec<StageChunkSummary>,
    pub placements: Vec<PlacementRecord>,
    pub player_spawns: Vec<PlacementRecord>,
    pub exits: Vec<StageExitRecord>,
    pub collisions: Vec<CollisionInventoryRecord>,
    pub load_triggers: Vec<CollisionLoadTrigger>,
}

impl WorldContext {
    pub fn build(
        game_data_sha256: Digest,
        inventories: &[WorldInventory],
    ) -> Result<Self, PlannerContractError> {
        let mut stages = inventories
            .iter()
            .map(|inventory| {
                Ok(WorldContextStage {
                    stage: inventory.stage.clone(),
                    inventory_sha256: inventory.digest()?,
                    // Spatial data is not interpreted by the planner, but its
                    // identity remains mandatory in imported contexts.
                    spatial_index_sha256: inventory.digest()?,
                })
            })
            .collect::<Result<Vec<_>, PlannerContractError>>()?;
        stages.sort_by(|left, right| left.stage.cmp(&right.stage));
        let context = Self {
            schema: WORLD_CONTEXT_SCHEMA.into(),
            game_data_sha256,
            stages,
        };
        context.validate()?;
        Ok(context)
    }

    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != WORLD_CONTEXT_SCHEMA || self.game_data_sha256 == Digest::ZERO {
            return Err(PlannerContractError::new(
                "world_context",
                "has an unsupported schema or zero game-data digest",
            ));
        }
        if self.stages.is_empty() || self.stages.len() > 256 {
            return Err(PlannerContractError::new(
                "world_context.stages",
                "must contain between 1 and 256 stages",
            ));
        }
        let mut prior = None;
        let mut names = BTreeSet::new();
        for stage in &self.stages {
            validate_stage(&stage.stage)?;
            if stage.inventory_sha256 == Digest::ZERO
                || stage.spatial_index_sha256 == Digest::ZERO
                || !names.insert(stage.stage.as_str())
                || prior.is_some_and(|value: &str| value >= stage.stage.as_str())
            {
                return Err(PlannerContractError::new(
                    "world_context.stages",
                    "must be sorted, unique, and content-addressed",
                ));
            }
            prior = Some(stage.stage.as_str());
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        canonical_json(self)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let context: Self = serde_json::from_slice(bytes)?;
        context.validate()?;
        if context.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "world_context",
                "is not canonical JSON",
            ));
        }
        Ok(context)
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

impl WorldInventory {
    pub fn validate(&self) -> Result<(), PlannerContractError> {
        if self.schema != WORLD_INVENTORY_SCHEMA {
            return Err(PlannerContractError::new(
                "world_inventory.schema",
                "is unsupported",
            ));
        }
        validate_stage(&self.stage)?;
        if self.sources.is_empty() {
            return Err(PlannerContractError::new(
                "world_inventory.sources",
                "must not be empty",
            ));
        }
        if self.player_spawns.iter().any(|placement| {
            placement.kind != PlacementKind::PlayerSpawn || !finite(placement.position)
        }) || self
            .placements
            .iter()
            .any(|placement| !finite(placement.position))
        {
            return Err(PlannerContractError::new(
                "world_inventory.placements",
                "contains an invalid placement",
            ));
        }
        for exit in &self.exits {
            validate_stage(&exit.destination_stage)?;
        }
        for trigger in &self.load_triggers {
            validate_stage(&trigger.destination_stage)?;
            if !self
                .exits
                .iter()
                .any(|exit| exit.stable_id == trigger.scls_id)
                || !self
                    .collisions
                    .iter()
                    .any(|collision| collision.prism.authored.stable_id == trigger.collision_id)
            {
                return Err(PlannerContractError::new(
                    "world_inventory.load_triggers",
                    "references an unknown exit or collision",
                ));
            }
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PlannerContractError> {
        self.validate()?;
        Ok(serde_json::to_vec(self)?)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, PlannerContractError> {
        let inventory: Self = serde_json::from_slice(bytes)?;
        inventory.validate()?;
        if inventory.canonical_bytes()? != bytes {
            return Err(PlannerContractError::new(
                "world_inventory",
                "is not canonical JSON",
            ));
        }
        Ok(inventory)
    }

    pub fn read_canonical(path: &Path) -> Result<Self, PlannerContractError> {
        Self::decode_canonical(
            &fs::read(path)
                .map_err(|error| PlannerContractError::new("world_inventory", error.to_string()))?,
        )
    }

    pub fn digest(&self) -> Result<Digest, PlannerContractError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

fn validate_stage(stage: &str) -> Result<(), PlannerContractError> {
    if stage.is_empty()
        || stage.len() > 8
        || !stage
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(PlannerContractError::new(
            "stage",
            "must contain 1-8 ASCII letters, digits, or underscores",
        ));
    }
    Ok(())
}

fn finite(value: Vec3) -> bool {
    value.x.is_finite() && value.y.is_finite() && value.z.is_finite()
}
