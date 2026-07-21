//! Bounded static-topology views derived from authenticated geometry probes.
//!
//! This layer performs no gameplay collection. It uses the stable collision IDs
//! already retained by [`NativeEpisodeGeometryView`] as graph seeds, expands a
//! bounded neighborhood in an immutable [`WorldSurfaceGraph`], and binds every
//! result to all source artifact identities. A larger neighborhood can therefore
//! be regenerated without rerunning the game.

use crate::artifact::Digest;
use crate::native_geometry_view::{
    GeometryObservationPhase, GeometryObservationStatus, NativeEpisodeGeometryView,
};
use dusklight_world::world_inventory::WorldInventory;
use dusklight_world::world_surface_graph::{
    MAX_SURFACE_GRAPH_HOPS, MAX_SURFACE_GRAPH_RESULTS, RoomSurfaceGraphCoverage, WorldSurfaceGraph,
    WorldSurfaceNeighborhoodReport, WorldSurfaceNeighborhoodRequest,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const NATIVE_SURFACE_GRAPH_VIEW_SCHEMA_V1: &str = "dusklight-native-surface-graph-view/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSurfaceGraphViewConfiguration {
    pub maximum_hops: u8,
    pub maximum_nodes: usize,
}

impl Default for NativeSurfaceGraphViewConfiguration {
    fn default() -> Self {
        Self {
            maximum_hops: 8,
            maximum_nodes: 256,
        }
    }
}

impl NativeSurfaceGraphViewConfiguration {
    fn validate(self) -> Result<(), NativeSurfaceGraphViewError> {
        if self.maximum_hops > MAX_SURFACE_GRAPH_HOPS
            || !(1..=MAX_SURFACE_GRAPH_RESULTS).contains(&self.maximum_nodes)
        {
            return Err(NativeSurfaceGraphViewError::new(
                "native surface graph view configuration is unbounded",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceGraphWorldReference {
    pub stage: String,
    pub inventory_sha256: Digest,
    pub spatial_index_sha256: Digest,
    pub surface_graph_sha256: Digest,
    pub source_collision_count: usize,
    pub node_count: usize,
    pub edge_count: usize,
    pub excluded_node_count: usize,
    pub rooms: Vec<RoomSurfaceGraphCoverage>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceGraphObservationStatus {
    Present,
    NoSurfaceSeed,
    PlayerAbsent,
    RoomUnavailable,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSurfaceGraphObservation {
    pub episode_id: String,
    pub step_index: u32,
    pub phase: GeometryObservationPhase,
    pub boundary_index: u64,
    pub state_identity_xxh3_128: String,
    pub stage: String,
    pub room: i8,
    pub status: SurfaceGraphObservationStatus,
    pub seed_collision_ids: Vec<String>,
    pub neighborhood: Option<WorldSurfaceNeighborhoodReport>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeEpisodeSurfaceGraphView {
    pub schema: String,
    pub native_geometry_view_sha256: Digest,
    pub native_shard_sha256: Digest,
    pub observation_schema: String,
    pub world_context_sha256: Option<Digest>,
    pub configuration: NativeSurfaceGraphViewConfiguration,
    pub worlds: Vec<SurfaceGraphWorldReference>,
    pub observations: Vec<NativeSurfaceGraphObservation>,
    pub view_sha256: Digest,
}

impl NativeEpisodeSurfaceGraphView {
    pub fn build(
        geometry: &NativeEpisodeGeometryView,
        inventories: &[WorldInventory],
        configuration: NativeSurfaceGraphViewConfiguration,
    ) -> Result<Self, NativeSurfaceGraphViewError> {
        configuration.validate()?;
        geometry
            .validate()
            .map_err(|error| NativeSurfaceGraphViewError::new(error.to_string()))?;
        if inventories.is_empty() {
            return Err(NativeSurfaceGraphViewError::new(
                "native surface graph view requires static world inventories",
            ));
        }

        let mut ordered = inventories.iter().collect::<Vec<_>>();
        ordered.sort_by(|left, right| left.stage.cmp(&right.stage));
        if ordered
            .windows(2)
            .any(|pair| pair[0].stage == pair[1].stage)
        {
            return Err(NativeSurfaceGraphViewError::new(
                "native surface graph view world stages must be unique",
            ));
        }
        let graphs = ordered
            .iter()
            .map(|inventory| {
                WorldSurfaceGraph::build(inventory)
                    .map_err(|error| NativeSurfaceGraphViewError::new(error.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if geometry.worlds.len() != graphs.len() {
            return Err(NativeSurfaceGraphViewError::new(
                "native surface graph world set differs from the geometry view",
            ));
        }
        let worlds = geometry
            .worlds
            .iter()
            .zip(&graphs)
            .map(|(source, graph)| {
                let artifact = graph.artifact();
                if source.stage != artifact.stage
                    || source.inventory_sha256 != artifact.inventory_sha256
                    || source.spatial_index_sha256 != artifact.spatial_index_sha256
                {
                    return Err(NativeSurfaceGraphViewError::new(
                        "native surface graph identity differs from the geometry view",
                    ));
                }
                Ok(SurfaceGraphWorldReference {
                    stage: artifact.stage.clone(),
                    inventory_sha256: artifact.inventory_sha256,
                    spatial_index_sha256: artifact.spatial_index_sha256,
                    surface_graph_sha256: graph.artifact_digest(),
                    source_collision_count: artifact.source_collision_count,
                    node_count: artifact.nodes.len(),
                    edge_count: artifact.edges.len(),
                    excluded_node_count: artifact.excluded.len(),
                    rooms: artifact.rooms.clone(),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let mut observations = Vec::with_capacity(geometry.observations.len());
        for source in &geometry.observations {
            let world_index = worlds
                .binary_search_by(|world| world.stage.as_str().cmp(&source.stage))
                .map_err(|_| {
                    NativeSurfaceGraphViewError::new(format!(
                        "geometry observation stage {:?} has no surface graph",
                        source.stage
                    ))
                })?;
            let mut observation = NativeSurfaceGraphObservation {
                episode_id: source.episode_id.clone(),
                step_index: source.step_index,
                phase: source.phase,
                boundary_index: source.boundary_index,
                state_identity_xxh3_128: source.state_identity_xxh3_128.clone(),
                stage: source.stage.clone(),
                room: source.room,
                status: SurfaceGraphObservationStatus::PlayerAbsent,
                seed_collision_ids: Vec::new(),
                neighborhood: None,
            };
            match source.status {
                GeometryObservationStatus::PlayerAbsent => {}
                GeometryObservationStatus::RoomUnavailable => {
                    observation.status = SurfaceGraphObservationStatus::RoomUnavailable;
                }
                GeometryObservationStatus::Present => {
                    observation.seed_collision_ids = source
                        .probes
                        .iter()
                        .map(|probe| probe.stable_id.clone())
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect();
                    if observation.seed_collision_ids.is_empty() {
                        observation.status = SurfaceGraphObservationStatus::NoSurfaceSeed;
                    } else {
                        observation.status = SurfaceGraphObservationStatus::Present;
                        observation.neighborhood = Some(
                            graphs[world_index]
                                .neighborhood(WorldSurfaceNeighborhoodRequest {
                                    room: source.room,
                                    seed_collision_ids: observation.seed_collision_ids.clone(),
                                    maximum_hops: configuration.maximum_hops,
                                    maximum_nodes: configuration.maximum_nodes,
                                })
                                .map_err(|error| {
                                    NativeSurfaceGraphViewError::new(error.to_string())
                                })?,
                        );
                    }
                }
            }
            observations.push(observation);
        }

        let mut view = Self {
            schema: NATIVE_SURFACE_GRAPH_VIEW_SCHEMA_V1.into(),
            native_geometry_view_sha256: geometry.view_sha256,
            native_shard_sha256: geometry.native_shard_sha256,
            observation_schema: geometry.observation_schema.clone(),
            world_context_sha256: geometry.world_context_sha256,
            configuration,
            worlds,
            observations,
            view_sha256: Digest::ZERO,
        };
        view.view_sha256 = view.compute_digest()?;
        view.validate()?;
        Ok(view)
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, NativeSurfaceGraphViewError> {
        let view: Self = serde_json::from_slice(bytes)
            .map_err(|error| NativeSurfaceGraphViewError::new(error.to_string()))?;
        view.validate()?;
        if serde_json::to_vec(&view)
            .map_err(|error| NativeSurfaceGraphViewError::new(error.to_string()))?
            != bytes
        {
            return Err(NativeSurfaceGraphViewError::new(
                "native surface graph view bytes are not canonical",
            ));
        }
        Ok(view)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, NativeSurfaceGraphViewError> {
        self.validate()?;
        serde_json::to_vec(self)
            .map_err(|error| NativeSurfaceGraphViewError::new(error.to_string()))
    }

    pub fn validate(&self) -> Result<(), NativeSurfaceGraphViewError> {
        self.configuration.validate()?;
        if self.schema != NATIVE_SURFACE_GRAPH_VIEW_SCHEMA_V1
            || self.native_geometry_view_sha256 == Digest::ZERO
            || self.native_shard_sha256 == Digest::ZERO
            || self.observation_schema.is_empty()
            || self.world_context_sha256 == Some(Digest::ZERO)
            || self.worlds.is_empty()
            || self.observations.is_empty()
            || self.view_sha256 != self.compute_digest()?
        {
            return Err(NativeSurfaceGraphViewError::new(
                "native surface graph view envelope or seal is invalid",
            ));
        }
        let mut stages = BTreeSet::new();
        let mut previous_stage = None;
        for world in &self.worlds {
            if world.stage.is_empty()
                || world.inventory_sha256 == Digest::ZERO
                || world.spatial_index_sha256 == Digest::ZERO
                || world.surface_graph_sha256 == Digest::ZERO
                || world.node_count + world.excluded_node_count != world.source_collision_count
                || world.rooms.is_empty()
                || world
                    .rooms
                    .iter()
                    .map(|room| room.adjacency_edges)
                    .sum::<usize>()
                    != world.edge_count
                || previous_stage.is_some_and(|previous| previous >= world.stage.as_str())
                || !stages.insert(world.stage.as_str())
            {
                return Err(NativeSurfaceGraphViewError::new(
                    "native surface graph view contains an invalid world reference",
                ));
            }
            previous_stage = Some(world.stage.as_str());
        }

        let mut keys = BTreeSet::new();
        for observation in &self.observations {
            if observation.episode_id.is_empty()
                || !stages.contains(observation.stage.as_str())
                || !is_lower_hex(&observation.state_identity_xxh3_128, 32)
                || !observation.seed_collision_ids.is_sorted()
                || observation
                    .seed_collision_ids
                    .windows(2)
                    .any(|pair| pair[0] == pair[1])
                || !keys.insert((
                    observation.episode_id.as_str(),
                    observation.step_index,
                    observation.phase,
                ))
            {
                return Err(NativeSurfaceGraphViewError::new(
                    "native surface graph observation identity is invalid",
                ));
            }
            let world = self
                .worlds
                .binary_search_by(|world| world.stage.as_str().cmp(&observation.stage))
                .ok()
                .map(|index| &self.worlds[index])
                .ok_or_else(|| NativeSurfaceGraphViewError::new("observation world is missing"))?;
            match (observation.status, &observation.neighborhood) {
                (SurfaceGraphObservationStatus::Present, Some(report)) => {
                    report
                        .validate()
                        .map_err(|error| NativeSurfaceGraphViewError::new(error.to_string()))?;
                    if observation.seed_collision_ids.is_empty()
                        || report.stage != observation.stage
                        || report.inventory_sha256 != world.inventory_sha256
                        || report.spatial_index_sha256 != world.spatial_index_sha256
                        || report.surface_graph_sha256 != world.surface_graph_sha256
                        || report.surface_graph_edge_count != world.edge_count
                        || report.request.room != observation.room
                        || report.request.seed_collision_ids != observation.seed_collision_ids
                        || report.request.maximum_hops != self.configuration.maximum_hops
                        || report.request.maximum_nodes != self.configuration.maximum_nodes
                    {
                        return Err(NativeSurfaceGraphViewError::new(
                            "native surface graph neighborhood identity is invalid",
                        ));
                    }
                }
                (SurfaceGraphObservationStatus::NoSurfaceSeed, None) => {
                    if !observation.seed_collision_ids.is_empty() {
                        return Err(NativeSurfaceGraphViewError::new(
                            "no-seed observation retains surface seeds",
                        ));
                    }
                }
                (
                    SurfaceGraphObservationStatus::PlayerAbsent
                    | SurfaceGraphObservationStatus::RoomUnavailable,
                    None,
                ) => {
                    if !observation.seed_collision_ids.is_empty() {
                        return Err(NativeSurfaceGraphViewError::new(
                            "masked surface graph observation retains seeds",
                        ));
                    }
                }
                _ => {
                    return Err(NativeSurfaceGraphViewError::new(
                        "native surface graph observation status is inconsistent",
                    ));
                }
            }
        }
        Ok(())
    }

    fn compute_digest(&self) -> Result<Digest, NativeSurfaceGraphViewError> {
        let mut canonical = self.clone();
        canonical.view_sha256 = Digest::ZERO;
        let bytes = serde_json::to_vec(&canonical)
            .map_err(|error| NativeSurfaceGraphViewError::new(error.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.native-surface-graph-view/v1\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeSurfaceGraphViewError(String);

impl NativeSurfaceGraphViewError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for NativeSurfaceGraphViewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeSurfaceGraphViewError {}
