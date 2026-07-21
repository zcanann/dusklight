//! Offline local-geometry views derived from native learning episodes.
//!
//! Static triangles remain in content-addressed [`WorldInventory`] artifacts.
//! This module joins an authenticated native episode to those immutable worlds
//! and retains only bounded, per-observation proximity probes. It never calls a
//! live game collision system or changes gameplay state.

use super::model_representation::{LocalGeometryProbe, MAX_LOCAL_GEOMETRY_PROBES};
use crate::artifact::Digest;
use crate::world_spatial::{WorldPointQueryRequest, WorldSpatialIndex, WorldSurfaceFilter};
use dusklight_evidence::native_episode_shard::{
    NativeEpisodeShard, NativeLearningObservation, NativeObservationPhase,
};
use dusklight_world::world_context::WorldContext;
use dusklight_world::world_geometry::Vec3;
use dusklight_world::world_inventory::{PlacementKind, SourceKind, WorldInventory};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const NATIVE_GEOMETRY_VIEW_SCHEMA_V3: &str = "dusklight-native-geometry-view/v3";
pub const MAX_GEOMETRY_QUERY_DISTANCE: f32 = 65_536.0;
pub const MAX_STATIC_PLACEMENTS_PER_WORLD: usize = 1_000_000;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeGeometryViewConfiguration {
    pub maximum_distance: f32,
    pub surface_limit: usize,
}

impl Default for NativeGeometryViewConfiguration {
    fn default() -> Self {
        Self {
            maximum_distance: 512.0,
            surface_limit: MAX_LOCAL_GEOMETRY_PROBES,
        }
    }
}

impl NativeGeometryViewConfiguration {
    fn validate(self) -> Result<(), NativeGeometryViewError> {
        if !self.maximum_distance.is_finite()
            || !(0.0..=MAX_GEOMETRY_QUERY_DISTANCE).contains(&self.maximum_distance)
            || self.maximum_distance == 0.0
            || !(1..=MAX_LOCAL_GEOMETRY_PROBES).contains(&self.surface_limit)
        {
            return Err(NativeGeometryViewError::new(
                "native geometry view requires a positive bounded distance and surface limit",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GeometryObservationPhase {
    PreInput,
    PostSimulation,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GeometryObservationStatus {
    Present,
    PlayerAbsent,
    RoomUnavailable,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeometryWorldReference {
    pub stage: String,
    pub inventory_sha256: Digest,
    pub spatial_index_sha256: Digest,
    /// Complete semantic ACT*/SCO*/TRES/PLY placement population from the
    /// immutable inventory. It is stored once per world, never copied per tick
    /// or proximity-selected.
    pub placements: Vec<NativeStaticPlacement>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeStaticPlacementScope {
    Stage,
    Room,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeStaticPlacementKind {
    Actor,
    ScaledActor,
    Treasure,
    PlayerSpawn,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeStaticPlacement {
    pub stable_id: String,
    pub source_sha256: Digest,
    pub scope: NativeStaticPlacementScope,
    pub room: Option<i8>,
    pub chunk_tag: String,
    pub record_index: usize,
    pub layer: Option<u8>,
    pub kind: NativeStaticPlacementKind,
    /// Exact authored eight-byte actor token with trailing zeroes removed by
    /// the world parser. This remains a categorical identity, not an ordinal.
    pub name: String,
    pub parameters: u32,
    pub absolute_position: [f32; 3],
    pub angle: [i16; 3],
    pub set_id: u16,
    /// Runtime scale produced by the ordinary stage loader (authored bytes ×
    /// 0.1). Unscaled placement records keep this explicitly absent.
    pub scale: Option<[f32; 3]>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeGeometryObservation {
    pub episode_id: String,
    pub step_index: u32,
    pub phase: GeometryObservationPhase,
    pub boundary_index: u64,
    pub state_identity_xxh3_128: String,
    pub stage: String,
    pub room: i8,
    pub layer: i8,
    pub player_present: bool,
    pub player_position: [f32; 3],
    pub player_yaw: i16,
    pub status: GeometryObservationStatus,
    pub probes: Vec<LocalGeometryProbe>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeEpisodeGeometryView {
    pub schema: String,
    pub native_shard_sha256: Digest,
    pub observation_schema: String,
    pub world_context_sha256: Option<Digest>,
    pub configuration: NativeGeometryViewConfiguration,
    pub worlds: Vec<GeometryWorldReference>,
    pub observations: Vec<NativeGeometryObservation>,
    pub view_sha256: Digest,
}

impl NativeEpisodeGeometryView {
    pub fn build(
        shard: &NativeEpisodeShard,
        inventories: &[WorldInventory],
        configuration: NativeGeometryViewConfiguration,
    ) -> Result<Self, NativeGeometryViewError> {
        configuration.validate()?;
        if shard.content_sha256 == Digest::ZERO
            || shard.episodes.is_empty()
            || inventories.is_empty()
        {
            return Err(NativeGeometryViewError::new(
                "native geometry view requires a nonempty authenticated shard and world set",
            ));
        }

        let mut ordered = inventories.iter().collect::<Vec<_>>();
        ordered.sort_by(|left, right| left.stage.cmp(&right.stage));
        if ordered
            .windows(2)
            .any(|pair| pair[0].stage == pair[1].stage)
        {
            return Err(NativeGeometryViewError::new(
                "native geometry view world stages must be unique",
            ));
        }
        for inventory in &ordered {
            inventory
                .validate()
                .map_err(|error| NativeGeometryViewError::new(error.to_string()))?;
        }
        let world_context_sha256 = match (
            shard.metadata.game_data_sha256,
            shard.metadata.world_context_sha256,
        ) {
            (Some(game_data_sha256), Some(expected)) => {
                let context = WorldContext::build(game_data_sha256, inventories)
                    .map_err(|error| NativeGeometryViewError::new(error.to_string()))?;
                let actual = context
                    .digest()
                    .map_err(|error| NativeGeometryViewError::new(error.to_string()))?;
                if actual != expected {
                    return Err(NativeGeometryViewError::new(
                        "native shard world-context identity does not match supplied inventories",
                    ));
                }
                Some(actual)
            }
            (None, None) => None,
            _ => {
                return Err(NativeGeometryViewError::new(
                    "native shard has an incomplete world-context identity",
                ));
            }
        };
        let indexes = ordered
            .iter()
            .map(|inventory| {
                WorldSpatialIndex::build(inventory)
                    .map_err(|error| NativeGeometryViewError::new(error.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let worlds = ordered
            .iter()
            .zip(&indexes)
            .map(|(inventory, index)| {
                Ok(GeometryWorldReference {
                    stage: inventory.stage.clone(),
                    inventory_sha256: inventory
                        .digest()
                        .map_err(|error| NativeGeometryViewError::new(error.to_string()))?,
                    spatial_index_sha256: index
                        .artifact_digest()
                        .map_err(|error| NativeGeometryViewError::new(error.to_string()))?,
                    placements: materialize_static_placements(inventory)?,
                })
            })
            .collect::<Result<Vec<_>, NativeGeometryViewError>>()?;

        let mut observations = Vec::new();
        for episode in &shard.episodes {
            for (step_index, step) in episode.steps.iter().enumerate() {
                let step_index = u32::try_from(step_index)
                    .map_err(|_| NativeGeometryViewError::new("episode step index overflowed"))?;
                observations.push(materialize_observation(
                    &episode.id,
                    step_index,
                    &step.pre_input,
                    &ordered,
                    &indexes,
                    configuration,
                )?);
                observations.push(materialize_observation(
                    &episode.id,
                    step_index,
                    &step.post_simulation,
                    &ordered,
                    &indexes,
                    configuration,
                )?);
            }
        }
        let mut view = Self {
            schema: NATIVE_GEOMETRY_VIEW_SCHEMA_V3.into(),
            native_shard_sha256: shard.content_sha256,
            observation_schema: shard.metadata.observation_schema.clone(),
            world_context_sha256,
            configuration,
            worlds,
            observations,
            view_sha256: Digest::ZERO,
        };
        view.view_sha256 = view.compute_identity()?;
        view.validate()?;
        Ok(view)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, NativeGeometryViewError> {
        let view: Self = serde_json::from_slice(bytes)
            .map_err(|error| NativeGeometryViewError::new(error.to_string()))?;
        view.validate()?;
        if serde_json::to_vec(&view)
            .map_err(|error| NativeGeometryViewError::new(error.to_string()))?
            != bytes
        {
            return Err(NativeGeometryViewError::new(
                "native geometry view bytes are not canonical",
            ));
        }
        Ok(view)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, NativeGeometryViewError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| NativeGeometryViewError::new(error.to_string()))
    }

    pub fn validate(&self) -> Result<(), NativeGeometryViewError> {
        self.configuration.validate()?;
        if self.schema != NATIVE_GEOMETRY_VIEW_SCHEMA_V3
            || self.native_shard_sha256 == Digest::ZERO
            || self.observation_schema.is_empty()
            || self.world_context_sha256 == Some(Digest::ZERO)
            || self.worlds.is_empty()
            || self.observations.is_empty()
            || self.view_sha256 != self.compute_identity()?
        {
            return Err(NativeGeometryViewError::new(
                "native geometry view envelope or seal is invalid",
            ));
        }
        let mut stages = BTreeSet::new();
        let mut previous_stage = None;
        for world in &self.worlds {
            if world.stage.is_empty()
                || world.inventory_sha256 == Digest::ZERO
                || world.spatial_index_sha256 == Digest::ZERO
                || previous_stage.is_some_and(|previous| previous >= world.stage.as_str())
                || !stages.insert(world.stage.as_str())
            {
                return Err(NativeGeometryViewError::new(
                    "native geometry view contains an invalid world reference",
                ));
            }
            if world.placements.len() > MAX_STATIC_PLACEMENTS_PER_WORLD {
                return Err(NativeGeometryViewError::new(
                    "native geometry view world placement population is oversized",
                ));
            }
            let mut previous_placement = None;
            for placement in &world.placements {
                validate_static_placement(placement)?;
                if previous_placement
                    .is_some_and(|previous: &str| previous >= placement.stable_id.as_str())
                {
                    return Err(NativeGeometryViewError::new(
                        "native geometry view placements are duplicate or noncanonical",
                    ));
                }
                previous_placement = Some(placement.stable_id.as_str());
            }
            previous_stage = Some(world.stage.as_str());
        }
        let mut keys = BTreeSet::new();
        for observation in &self.observations {
            if observation.episode_id.is_empty()
                || !stages.contains(observation.stage.as_str())
                || !is_lower_hex(&observation.state_identity_xxh3_128, 32)
                || observation
                    .player_position
                    .iter()
                    .any(|value| !value.is_finite())
                || observation.probes.len() > self.configuration.surface_limit
                || !keys.insert((
                    observation.episode_id.as_str(),
                    observation.step_index,
                    observation.phase,
                ))
            {
                return Err(NativeGeometryViewError::new(
                    "native geometry view contains an invalid observation identity",
                ));
            }
            let status_is_consistent = match observation.status {
                GeometryObservationStatus::Present => observation.player_present,
                GeometryObservationStatus::PlayerAbsent => {
                    !observation.player_present && observation.probes.is_empty()
                }
                GeometryObservationStatus::RoomUnavailable => {
                    observation.player_present && observation.probes.is_empty()
                }
            };
            if !status_is_consistent {
                return Err(NativeGeometryViewError::new(
                    "native geometry observation status is inconsistent",
                ));
            }
            for probe in &observation.probes {
                probe
                    .validate()
                    .map_err(|error| NativeGeometryViewError::new(error.to_string()))?;
                if probe.distance > self.configuration.maximum_distance {
                    return Err(NativeGeometryViewError::new(
                        "native geometry probe exceeds its declared radius",
                    ));
                }
            }
        }
        Ok(())
    }

    fn compute_identity(&self) -> Result<Digest, NativeGeometryViewError> {
        let mut canonical = self.clone();
        canonical.view_sha256 = Digest::ZERO;
        let bytes = serde_json::to_vec(&canonical)
            .map_err(|error| NativeGeometryViewError::new(error.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.native-geometry-view/v3\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }
}

fn materialize_static_placements(
    inventory: &WorldInventory,
) -> Result<Vec<NativeStaticPlacement>, NativeGeometryViewError> {
    let mut placements = inventory
        .placements
        .iter()
        .chain(&inventory.player_spawns)
        .map(|placement| NativeStaticPlacement {
            stable_id: placement.stable_id.clone(),
            source_sha256: placement.source_sha256,
            scope: match placement.scope.kind {
                SourceKind::Stage => NativeStaticPlacementScope::Stage,
                SourceKind::Room => NativeStaticPlacementScope::Room,
            },
            room: placement.scope.room,
            chunk_tag: placement.chunk_tag.clone(),
            record_index: placement.record_index,
            layer: placement.layer,
            kind: match placement.kind {
                PlacementKind::Actor => NativeStaticPlacementKind::Actor,
                PlacementKind::ScaledActor => NativeStaticPlacementKind::ScaledActor,
                PlacementKind::Treasure => NativeStaticPlacementKind::Treasure,
                PlacementKind::PlayerSpawn => NativeStaticPlacementKind::PlayerSpawn,
            },
            name: placement.name.clone(),
            parameters: placement.parameters,
            absolute_position: [
                placement.position.x,
                placement.position.y,
                placement.position.z,
            ],
            angle: placement.angle,
            set_id: placement.set_id,
            scale: placement.scale_raw.map(|scale| {
                [
                    f32::from(scale[0]) * 0.1,
                    f32::from(scale[1]) * 0.1,
                    f32::from(scale[2]) * 0.1,
                ]
            }),
        })
        .collect::<Vec<_>>();
    if placements.len() > MAX_STATIC_PLACEMENTS_PER_WORLD {
        return Err(NativeGeometryViewError::new(
            "native geometry view world placement population is oversized",
        ));
    }
    placements.sort_by(|left, right| left.stable_id.cmp(&right.stable_id));
    for placement in &placements {
        validate_static_placement(placement)?;
    }
    if placements
        .windows(2)
        .any(|pair| pair[0].stable_id == pair[1].stable_id)
    {
        return Err(NativeGeometryViewError::new(
            "native geometry view placements have duplicate stable identities",
        ));
    }
    Ok(placements)
}

fn validate_static_placement(
    placement: &NativeStaticPlacement,
) -> Result<(), NativeGeometryViewError> {
    let scope_consistent = match placement.scope {
        NativeStaticPlacementScope::Stage => placement.room.is_none(),
        NativeStaticPlacementScope::Room => placement.room.is_some(),
    };
    let scale_consistent = placement.scale.is_some()
        == (placement.kind == NativeStaticPlacementKind::ScaledActor)
        && placement
            .scale
            .is_none_or(|scale| scale.iter().all(|value| value.is_finite()));
    if placement.stable_id.is_empty()
        || placement.source_sha256 == Digest::ZERO
        || !scope_consistent
        || placement.chunk_tag.len() != 4
        || placement.record_index >= MAX_STATIC_PLACEMENTS_PER_WORLD
        || placement.name.is_empty()
        || placement.name.len() > 8
        || !placement.name.bytes().all(|byte| byte.is_ascii_graphic())
        || placement.layer.is_some_and(|layer| layer > 15)
        || !scale_consistent
        || placement
            .absolute_position
            .iter()
            .any(|value| !value.is_finite())
    {
        return Err(NativeGeometryViewError::new(
            "native geometry view contains an invalid static placement",
        ));
    }
    Ok(())
}

fn materialize_observation(
    episode_id: &str,
    step_index: u32,
    observation: &NativeLearningObservation,
    inventories: &[&WorldInventory],
    indexes: &[WorldSpatialIndex<'_>],
    configuration: NativeGeometryViewConfiguration,
) -> Result<NativeGeometryObservation, NativeGeometryViewError> {
    let world_index = inventories
        .binary_search_by(|inventory| inventory.stage.as_str().cmp(&observation.stage))
        .map_err(|_| {
            NativeGeometryViewError::new(format!(
                "native observation stage {:?} has no static world artifact",
                observation.stage
            ))
        })?;
    let phase = match observation.phase {
        NativeObservationPhase::PreInput => GeometryObservationPhase::PreInput,
        NativeObservationPhase::PostSimulation => GeometryObservationPhase::PostSimulation,
    };
    let state_identity_xxh3_128 = hex(&observation.state_identity);
    let mut result = NativeGeometryObservation {
        episode_id: episode_id.into(),
        step_index,
        phase,
        boundary_index: observation.boundary_index,
        state_identity_xxh3_128,
        stage: observation.stage.clone(),
        room: observation.room,
        layer: observation.layer,
        player_present: observation.player_present,
        player_position: observation.player_position,
        player_yaw: observation.player_shape_angle[1],
        status: GeometryObservationStatus::PlayerAbsent,
        probes: Vec::new(),
    };
    if !observation.player_present {
        return Ok(result);
    }
    let room_available = inventories[world_index]
        .sources
        .iter()
        .any(|source| source.scope.room == Some(observation.room) && source.addressable_prisms > 0);
    if !room_available {
        result.status = GeometryObservationStatus::RoomUnavailable;
        return Ok(result);
    }
    let report = indexes[world_index]
        .point_query(WorldPointQueryRequest {
            point: Vec3 {
                x: observation.player_position[0],
                y: observation.player_position[1],
                z: observation.player_position[2],
            },
            max_distance: Some(configuration.maximum_distance),
            limit: configuration.surface_limit,
            filter: WorldSurfaceFilter {
                room: observation.room,
                load_triggers_only: false,
                trigger_stable_id: None,
                destination_stage: None,
                destination_room: None,
                destination_point: None,
            },
        })
        .map_err(|error| NativeGeometryViewError::new(error.to_string()))?;
    result.status = GeometryObservationStatus::Present;
    result.probes = LocalGeometryProbe::from_point_report(&report)
        .map_err(|error| NativeGeometryViewError::new(error.to_string()))?;
    Ok(result)
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(DIGITS[usize::from(byte >> 4)] as char);
        encoded.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    encoded
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeGeometryViewError(String);

impl NativeGeometryViewError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for NativeGeometryViewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeGeometryViewError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_surface_graph_view::{
        NativeEpisodeSurfaceGraphView, NativeSurfaceGraphViewConfiguration,
        SurfaceGraphObservationStatus,
    };
    use dusklight_evidence::native_episode_shard::NativeEpisodeShard;
    use dusklight_world::world_geometry::{
        CollisionCode, CollisionPlane, KclAuthoredPrism, KclInventoryPrism, KclReconstruction,
        KclSourceIndices,
    };
    use dusklight_world::world_inventory::{
        CollisionInventoryRecord, PlacementKind, PlacementRecord, SourceKind, SourceScope,
        WORLD_INVENTORY_SCHEMA, WorldSource,
    };

    fn digest(label: &[u8]) -> Digest {
        Digest(Sha256::digest(label).into())
    }

    fn shard() -> NativeEpisodeShard {
        let mut shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v4.dseps"
        ))
        .unwrap();
        shard.episodes.truncate(1);
        shard.episodes[0].steps.truncate(1);
        shard
    }

    fn inventory_for(observation: &NativeLearningObservation) -> WorldInventory {
        let kcl = digest(b"geometry-view-kcl");
        let plc = digest(b"geometry-view-plc");
        let [x, y, z] = observation.player_position;
        let surface_y = y - 10.0;
        WorldInventory {
            schema: WORLD_INVENTORY_SCHEMA.into(),
            stage: observation.stage.clone(),
            sources: vec![
                WorldSource {
                    scope: SourceScope {
                        kind: SourceKind::Stage,
                        room: None,
                    },
                    archive_sha256: digest(b"geometry-view-stage-archive"),
                    stage_data_path: "stage.dzs".into(),
                    stage_data_sha256: digest(b"geometry-view-stage-data"),
                    kcl_path: None,
                    kcl_sha256: None,
                    plc_path: None,
                    plc_sha256: None,
                    addressable_prisms: 0,
                },
                WorldSource {
                    scope: SourceScope {
                        kind: SourceKind::Room,
                        room: Some(observation.room),
                    },
                    archive_sha256: digest(b"geometry-view-room-archive"),
                    stage_data_path: "room.dzr".into(),
                    stage_data_sha256: digest(b"geometry-view-room-data"),
                    kcl_path: Some("room.kcl".into()),
                    kcl_sha256: Some(kcl),
                    plc_path: Some("room.plc".into()),
                    plc_sha256: Some(plc),
                    addressable_prisms: 1,
                },
            ],
            chunks: Vec::new(),
            placements: vec![PlacementRecord {
                stable_id: "room-placement/0".into(),
                source_sha256: digest(b"geometry-view-room-data"),
                scope: SourceScope {
                    kind: SourceKind::Room,
                    room: Some(observation.room),
                },
                chunk_tag: "ACTR".into(),
                record_index: 0,
                layer: None,
                kind: PlacementKind::Actor,
                name: "Scex".into(),
                parameters: 0x1234_5678,
                position: Vec3 {
                    x: x + 10.0,
                    y,
                    z: z - 20.0,
                },
                angle: [1, 2, 3],
                set_id: 7,
                scale_raw: None,
                raw_hex: "00".repeat(32),
            }],
            player_spawns: vec![PlacementRecord {
                stable_id: "stage-spawn/0".into(),
                source_sha256: digest(b"geometry-view-stage-data"),
                scope: SourceScope {
                    kind: SourceKind::Stage,
                    room: None,
                },
                chunk_tag: "PLYR".into(),
                record_index: 0,
                layer: None,
                kind: PlacementKind::PlayerSpawn,
                name: "Link".into(),
                parameters: 0,
                position: Vec3 { x, y, z },
                angle: [0, 0, 0],
                set_id: 0xffff,
                scale_raw: None,
                raw_hex: "00".repeat(32),
            }],
            exits: Vec::new(),
            collisions: vec![CollisionInventoryRecord {
                room: observation.room,
                prism: KclInventoryPrism {
                    authored: KclAuthoredPrism {
                        stable_id: format!("kcl-sha256:{kcl}/plc-sha256:{plc}/prism/1"),
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
                            raw: [0x3f, 0, 0, 0, 0],
                            exit_id: 0x3f,
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
                    reconstruction: KclReconstruction::Reconstructed {
                        plane: CollisionPlane {
                            anchor: Vec3 { x, y: surface_y, z },
                            normal: Vec3 {
                                x: 0.0,
                                y: 1.0,
                                z: 0.0,
                            },
                            d: -surface_y,
                        },
                        triangle: [
                            Vec3 {
                                x: x - 200.0,
                                y: surface_y,
                                z: z - 200.0,
                            },
                            Vec3 {
                                x: x + 200.0,
                                y: surface_y,
                                z: z - 200.0,
                            },
                            Vec3 {
                                x,
                                y: surface_y,
                                z: z + 200.0,
                            },
                        ],
                    },
                },
            }],
            load_triggers: Vec::new(),
        }
    }

    fn connected_inventory_for(observation: &NativeLearningObservation) -> WorldInventory {
        let mut inventory = inventory_for(observation);
        let first = inventory.collisions[0].clone();
        let (plane, triangle) = match first.prism.reconstruction {
            KclReconstruction::Reconstructed { plane, triangle } => (plane, triangle),
            KclReconstruction::Degenerate { .. } => unreachable!(),
        };
        let stable_prefix = first
            .prism
            .authored
            .stable_id
            .rsplit_once("/prism/")
            .unwrap()
            .0
            .to_owned();
        let mut authored = first.prism.authored;
        authored.stable_id = format!("{stable_prefix}/prism/2");
        authored.prism_index = 2;
        let third = Vec3 {
            x: (triangle[0].x + triangle[1].x) * 0.5,
            y: triangle[0].y,
            z: triangle[0].z - 200.0,
        };
        inventory.collisions.push(CollisionInventoryRecord {
            room: observation.room,
            prism: KclInventoryPrism {
                authored,
                reconstruction: KclReconstruction::Reconstructed {
                    plane,
                    triangle: [triangle[0], triangle[1], third],
                },
            },
        });
        inventory.sources[1].addressable_prisms = 2;
        inventory
    }

    #[test]
    fn native_episode_joins_to_static_world_and_round_trips_canonically() {
        let shard = shard();
        let inventory = inventory_for(&shard.episodes[0].steps[0].pre_input);
        let view = NativeEpisodeGeometryView::build(
            &shard,
            &[inventory],
            NativeGeometryViewConfiguration {
                maximum_distance: 64.0,
                surface_limit: 4,
            },
        )
        .unwrap();
        assert_eq!(view.worlds.len(), 1);
        assert_eq!(view.worlds[0].placements.len(), 2);
        let placement = &view.worlds[0].placements[0];
        assert_eq!(placement.stable_id, "room-placement/0");
        assert_eq!(placement.scope, NativeStaticPlacementScope::Room);
        assert_eq!(placement.room, Some(view.observations[0].room));
        assert_eq!(placement.kind, NativeStaticPlacementKind::Actor);
        assert_eq!(placement.name, "Scex");
        assert_eq!(placement.parameters, 0x1234_5678);
        assert_eq!(placement.set_id, 7);
        assert_eq!(
            view.worlds[0].placements[1].kind,
            NativeStaticPlacementKind::PlayerSpawn
        );
        assert_eq!(view.observations.len(), 2);
        assert!(view.observations.iter().all(|observation| {
            observation.status == GeometryObservationStatus::Present
                && observation.probes.len() == 1
                && (observation.probes[0].distance - 10.0).abs() < 0.001
                && observation.layer == shard.episodes[0].steps[0].pre_input.layer
        }));
        let bytes = view.canonical_bytes().unwrap();
        assert_eq!(
            NativeEpisodeGeometryView::decode_canonical(&bytes).unwrap(),
            view
        );

        let mut tampered = view;
        tampered.observations[0].probes[0].distance = 65.0;
        assert!(tampered.validate().is_err());
    }

    #[test]
    fn static_placement_contract_rejects_invalid_semantics_and_order() {
        let shard = shard();
        let inventory = inventory_for(&shard.episodes[0].steps[0].pre_input);
        let view = NativeEpisodeGeometryView::build(
            &shard,
            &[inventory],
            NativeGeometryViewConfiguration::default(),
        )
        .unwrap();

        let mut invalid_scope = view.clone();
        invalid_scope.worlds[0].placements[0].scope = NativeStaticPlacementScope::Stage;
        invalid_scope.view_sha256 = invalid_scope.compute_identity().unwrap();
        assert!(invalid_scope.validate().is_err());

        let mut invalid_scale = view.clone();
        invalid_scale.worlds[0].placements[0].scale = Some([1.0; 3]);
        invalid_scale.view_sha256 = invalid_scale.compute_identity().unwrap();
        assert!(invalid_scale.validate().is_err());

        let mut noncanonical_order = view;
        noncanonical_order.worlds[0].placements.swap(0, 1);
        noncanonical_order.view_sha256 = noncanonical_order.compute_identity().unwrap();
        assert!(noncanonical_order.validate().is_err());
    }

    #[test]
    fn absent_stage_fails_and_absent_room_is_explicitly_masked() {
        let mut shard = shard();
        let inventory = inventory_for(&shard.episodes[0].steps[0].pre_input);
        let mut wrong_stage = inventory.clone();
        wrong_stage.stage = "OTHER".into();
        assert!(
            NativeEpisodeGeometryView::build(
                &shard,
                &[wrong_stage],
                NativeGeometryViewConfiguration::default(),
            )
            .is_err()
        );

        shard.episodes[0].steps[0].pre_input.room = 100;
        shard.episodes[0].steps[0].post_simulation.room = 100;
        let view = NativeEpisodeGeometryView::build(
            &shard,
            &[inventory],
            NativeGeometryViewConfiguration::default(),
        )
        .unwrap();
        assert!(view.observations.iter().all(|observation| {
            observation.status == GeometryObservationStatus::RoomUnavailable
                && observation.probes.is_empty()
        }));
    }

    #[test]
    fn geometry_seeds_expand_into_content_bound_surface_neighborhoods() {
        let shard = shard();
        let inventory = connected_inventory_for(&shard.episodes[0].steps[0].pre_input);
        inventory.validate().unwrap();
        let geometry = NativeEpisodeGeometryView::build(
            &shard,
            &[inventory.clone()],
            NativeGeometryViewConfiguration {
                maximum_distance: 64.0,
                surface_limit: 4,
            },
        )
        .unwrap();
        assert!(
            geometry
                .observations
                .iter()
                .all(|observation| observation.probes.len() == 1)
        );

        let topology = NativeEpisodeSurfaceGraphView::build(
            &geometry,
            &[inventory],
            NativeSurfaceGraphViewConfiguration {
                maximum_hops: 1,
                maximum_nodes: 8,
            },
        )
        .unwrap();
        assert_eq!(topology.worlds.len(), 1);
        assert_eq!(topology.worlds[0].node_count, 2);
        assert_eq!(topology.worlds[0].edge_count, 1);
        assert!(topology.observations.iter().all(|observation| {
            observation.status == SurfaceGraphObservationStatus::Present
                && observation.seed_collision_ids.len() == 1
                && observation.neighborhood.as_ref().is_some_and(|report| {
                    report.reachable_within_hops == 2
                        && report.returned_nodes == 2
                        && report.induced_edge_indices.len() == 1
                        && !report.truncated
                })
        }));
        let bytes = topology.canonical_bytes().unwrap();
        assert_eq!(
            NativeEpisodeSurfaceGraphView::decode_canonical(&bytes).unwrap(),
            topology
        );

        let mut tampered = topology;
        tampered.observations[0]
            .neighborhood
            .as_mut()
            .unwrap()
            .surface_graph_sha256 = Digest([0x55; 32]);
        assert!(tampered.validate().is_err());
    }
}
