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
use dusklight_world::world_geometry::Vec3;
use dusklight_world::world_inventory::WorldInventory;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const NATIVE_GEOMETRY_VIEW_SCHEMA_V1: &str = "dusklight-native-geometry-view/v1";
pub const MAX_GEOMETRY_QUERY_DISTANCE: f32 = 65_536.0;

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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeometryWorldReference {
    pub stage: String,
    pub inventory_sha256: Digest,
    pub spatial_index_sha256: Digest,
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
            schema: NATIVE_GEOMETRY_VIEW_SCHEMA_V1.into(),
            native_shard_sha256: shard.content_sha256,
            observation_schema: shard.metadata.observation_schema.clone(),
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
        if self.schema != NATIVE_GEOMETRY_VIEW_SCHEMA_V1
            || self.native_shard_sha256 == Digest::ZERO
            || self.observation_schema.is_empty()
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
        hasher.update(b"dusklight.native-geometry-view/v1\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }
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
    use dusklight_evidence::native_episode_shard::NativeEpisodeShard;
    use dusklight_world::world_geometry::{
        CollisionCode, CollisionPlane, KclAuthoredPrism, KclInventoryPrism, KclReconstruction,
        KclSourceIndices,
    };
    use dusklight_world::world_inventory::{
        CollisionInventoryRecord, SourceKind, SourceScope, WORLD_INVENTORY_SCHEMA, WorldSource,
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
            placements: Vec::new(),
            player_spawns: Vec::new(),
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
        assert_eq!(view.observations.len(), 2);
        assert!(view.observations.iter().all(|observation| {
            observation.status == GeometryObservationStatus::Present
                && observation.probes.len() == 1
                && (observation.probes[0].distance - 10.0).abs() < 0.001
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
}
