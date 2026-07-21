//! Deterministic surface connectivity derived from immutable world inventory.
//!
//! Retail KCL triangle reconstruction introduces small floating-point
//! disagreements at vertices that originate at the same mesh corner. The graph
//! reports exact-edge coverage separately, then clusters vertices within one
//! explicit, small world-space tolerance before deriving adjacency. It does not
//! infer walkability or join coordinate spaces across rooms. A learner can
//! combine this static topology with dynamic state; the graph itself is built
//! and stored once per world identity.

use crate::artifact::Digest;
use crate::world_geometry::{CollisionCode, KclReconstruction, Vec3};
use crate::world_inventory::{CollisionLoadTrigger, WORLD_INVENTORY_SCHEMA, WorldInventory};
use crate::world_spatial::WorldSpatialIndex;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::error::Error;
use std::fmt;

pub const WORLD_SURFACE_GRAPH_SCHEMA: &str = "dusklight-world-surface-graph/v1";
pub const WORLD_SURFACE_NEIGHBORHOOD_SCHEMA: &str = "dusklight-world-surface-neighborhood/v1";
pub const MAX_SURFACE_GRAPH_SEEDS: usize = 64;
pub const MAX_SURFACE_GRAPH_HOPS: u8 = 32;
pub const MAX_SURFACE_GRAPH_RESULTS: usize = 4096;
pub const SURFACE_GRAPH_VERTEX_TOLERANCE: f32 = 0.05;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceGraphAlgorithm {
    pub id: String,
    pub vertex_equivalence: String,
    pub coordinate_scope: String,
    pub reachability_semantics: String,
    pub maximum_vertex_distance: f32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceGraphNode {
    pub room: i8,
    pub collision_id: String,
    pub attribute: u16,
    pub collision_code: CollisionCode,
    pub centroid: Vec3,
    pub plane_normal: Vec3,
    pub load_trigger: Option<CollisionLoadTrigger>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceGraphEdge {
    pub room: i8,
    pub left_collision_id: String,
    pub right_collision_id: String,
    pub shared_edge: [Vec3; 2],
    pub incidence_count: u16,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoomSurfaceGraphCoverage {
    pub room: i8,
    pub reconstructed_surfaces: usize,
    pub excluded_surfaces: usize,
    pub exact_shared_edge_groups: usize,
    pub clustered_shared_edge_groups: usize,
    pub boundary_edge_groups: usize,
    pub nonmanifold_edge_groups: usize,
    pub collapsed_triangle_edges: usize,
    pub vertex_clusters: usize,
    pub maximum_vertex_cluster_diameter: f32,
    pub adjacency_edges: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExcludedSurfaceGraphNode {
    pub room: i8,
    pub collision_id: String,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorldSurfaceGraphArtifact {
    pub schema: String,
    pub stage: String,
    pub inventory_sha256: Digest,
    pub spatial_index_sha256: Digest,
    pub algorithm: SurfaceGraphAlgorithm,
    pub source_collision_count: usize,
    pub nodes: Vec<SurfaceGraphNode>,
    pub edges: Vec<SurfaceGraphEdge>,
    pub excluded: Vec<ExcludedSurfaceGraphNode>,
    pub rooms: Vec<RoomSurfaceGraphCoverage>,
}

impl WorldSurfaceGraphArtifact {
    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, WorldSurfaceGraphError> {
        let artifact: Self = serde_json::from_slice(bytes)
            .map_err(|error| WorldSurfaceGraphError::new(error.to_string()))?;
        artifact.validate()?;
        if serde_json::to_vec(&artifact)
            .map_err(|error| WorldSurfaceGraphError::new(error.to_string()))?
            != bytes
        {
            return Err(WorldSurfaceGraphError::new(
                "world surface graph bytes are not canonical",
            ));
        }
        Ok(artifact)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, WorldSurfaceGraphError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| WorldSurfaceGraphError::new(error.to_string()))
    }

    pub fn digest(&self) -> Result<Digest, WorldSurfaceGraphError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }

    pub fn validate(&self) -> Result<(), WorldSurfaceGraphError> {
        if self.schema != WORLD_SURFACE_GRAPH_SCHEMA
            || self.stage.is_empty()
            || self.inventory_sha256 == Digest::ZERO
            || self.spatial_index_sha256 == Digest::ZERO
            || self.algorithm.id != "bounded-clustered-room-edge/v1"
            || self.algorithm.vertex_equivalence
                != "euclidean-connected-components-signed-zero-canonical/v1"
            || self.algorithm.coordinate_scope != "room-kcl-authored/v1"
            || self.algorithm.reachability_semantics != "topological-adjacency-not-walkability"
            || self.algorithm.maximum_vertex_distance.to_bits()
                != SURFACE_GRAPH_VERTEX_TOLERANCE.to_bits()
        {
            return Err(WorldSurfaceGraphError::new(
                "world surface graph envelope is invalid",
            ));
        }
        if self.nodes.len() + self.excluded.len() != self.source_collision_count {
            return Err(WorldSurfaceGraphError::new(
                "world surface graph does not account for every source collision",
            ));
        }

        let mut node_keys = BTreeSet::new();
        let mut previous_node = None;
        for node in &self.nodes {
            let key = (node.room, node.collision_id.as_str());
            if node.collision_id.is_empty()
                || !finite_vec3(node.centroid)
                || !finite_vec3(node.plane_normal)
                || previous_node.is_some_and(|previous| previous >= key)
                || !node_keys.insert(key)
                || node.load_trigger.as_ref().is_some_and(|trigger| {
                    trigger.room != node.room || trigger.collision_id != node.collision_id
                })
            {
                return Err(WorldSurfaceGraphError::new(
                    "world surface graph contains an invalid node",
                ));
            }
            previous_node = Some(key);
        }

        let mut edge_keys = BTreeSet::new();
        let mut previous_edge = None;
        for edge in &self.edges {
            let key = (
                edge.room,
                edge.left_collision_id.as_str(),
                edge.right_collision_id.as_str(),
                edge_key(edge.shared_edge)?,
            );
            if edge.left_collision_id >= edge.right_collision_id
                || edge.incidence_count < 2
                || !node_keys.contains(&(edge.room, edge.left_collision_id.as_str()))
                || !node_keys.contains(&(edge.room, edge.right_collision_id.as_str()))
                || previous_edge.is_some_and(|previous| previous >= key)
                || !edge_keys.insert(key)
            {
                return Err(WorldSurfaceGraphError::new(
                    "world surface graph contains an invalid edge",
                ));
            }
            previous_edge = Some(key);
        }

        let mut excluded_keys = BTreeSet::new();
        let mut previous_excluded = None;
        for excluded in &self.excluded {
            let key = (excluded.room, excluded.collision_id.as_str());
            if excluded.collision_id.is_empty()
                || excluded.reason.is_empty()
                || node_keys.contains(&key)
                || previous_excluded.is_some_and(|previous| previous >= key)
                || !excluded_keys.insert(key)
            {
                return Err(WorldSurfaceGraphError::new(
                    "world surface graph contains invalid excluded-surface accounting",
                ));
            }
            previous_excluded = Some(key);
        }

        let mut previous_room = None;
        for room in &self.rooms {
            if previous_room.is_some_and(|previous| previous >= room.room)
                || room.reconstructed_surfaces
                    != self
                        .nodes
                        .iter()
                        .filter(|node| node.room == room.room)
                        .count()
                || room.excluded_surfaces
                    != self
                        .excluded
                        .iter()
                        .filter(|surface| surface.room == room.room)
                        .count()
                || room.adjacency_edges
                    != self
                        .edges
                        .iter()
                        .filter(|edge| edge.room == room.room)
                        .count()
                || room.nonmanifold_edge_groups > room.clustered_shared_edge_groups
                || room.vertex_clusters > room.reconstructed_surfaces.saturating_mul(3)
                || !room.maximum_vertex_cluster_diameter.is_finite()
                || room.maximum_vertex_cluster_diameter < 0.0
            {
                return Err(WorldSurfaceGraphError::new(
                    "world surface graph room coverage is invalid",
                ));
            }
            previous_room = Some(room.room);
        }
        let covered_rooms = self
            .nodes
            .iter()
            .map(|node| node.room)
            .chain(self.excluded.iter().map(|surface| surface.room))
            .collect::<BTreeSet<_>>();
        if self
            .rooms
            .iter()
            .map(|room| room.room)
            .collect::<BTreeSet<_>>()
            != covered_rooms
        {
            return Err(WorldSurfaceGraphError::new(
                "world surface graph room coverage is incomplete",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorldSurfaceNeighborhoodRequest {
    pub room: i8,
    pub seed_collision_ids: Vec<String>,
    pub maximum_hops: u8,
    pub maximum_nodes: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceGraphVisit {
    pub collision_id: String,
    pub minimum_hops: u8,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorldSurfaceNeighborhoodReport {
    pub schema: String,
    pub stage: String,
    pub inventory_sha256: Digest,
    pub spatial_index_sha256: Digest,
    pub surface_graph_sha256: Digest,
    pub request: WorldSurfaceNeighborhoodRequest,
    pub surface_graph_edge_count: usize,
    pub eligible_room_nodes: usize,
    pub reachable_within_hops: usize,
    pub returned_nodes: usize,
    pub truncated: bool,
    pub visits: Vec<SurfaceGraphVisit>,
    pub induced_edge_indices: Vec<usize>,
    pub report_sha256: Digest,
}

impl WorldSurfaceNeighborhoodReport {
    pub fn validate(&self) -> Result<(), WorldSurfaceGraphError> {
        validate_request(&self.request)?;
        if self.schema != WORLD_SURFACE_NEIGHBORHOOD_SCHEMA
            || self.stage.is_empty()
            || self.inventory_sha256 == Digest::ZERO
            || self.spatial_index_sha256 == Digest::ZERO
            || self.surface_graph_sha256 == Digest::ZERO
            || self.returned_nodes != self.visits.len()
            || self.eligible_room_nodes < self.reachable_within_hops
            || self.returned_nodes > self.request.maximum_nodes
            || self.reachable_within_hops < self.returned_nodes
            || self.truncated != (self.reachable_within_hops > self.returned_nodes)
            || self.report_sha256 != self.compute_digest()?
        {
            return Err(WorldSurfaceGraphError::new(
                "world surface neighborhood report is invalid",
            ));
        }
        let mut previous = None;
        let mut ids = BTreeSet::new();
        for visit in &self.visits {
            let key = (visit.minimum_hops, visit.collision_id.as_str());
            if visit.collision_id.is_empty()
                || visit.minimum_hops > self.request.maximum_hops
                || previous.is_some_and(|previous| previous >= key)
                || !ids.insert(visit.collision_id.as_str())
            {
                return Err(WorldSurfaceGraphError::new(
                    "world surface neighborhood visits are invalid",
                ));
            }
            previous = Some(key);
        }
        if !self.induced_edge_indices.is_sorted()
            || self
                .induced_edge_indices
                .windows(2)
                .any(|pair| pair[0] == pair[1])
            || self
                .induced_edge_indices
                .iter()
                .any(|index| *index >= self.surface_graph_edge_count)
        {
            return Err(WorldSurfaceGraphError::new(
                "world surface neighborhood edge references are invalid",
            ));
        }
        Ok(())
    }

    fn compute_digest(&self) -> Result<Digest, WorldSurfaceGraphError> {
        let mut canonical = self.clone();
        canonical.report_sha256 = Digest::ZERO;
        let bytes = serde_json::to_vec(&canonical)
            .map_err(|error| WorldSurfaceGraphError::new(error.to_string()))?;
        let mut hasher = Sha256::new();
        hasher.update(b"dusklight.world-surface-neighborhood/v1\0");
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
        Ok(Digest(hasher.finalize().into()))
    }
}

pub struct WorldSurfaceGraph {
    artifact: WorldSurfaceGraphArtifact,
    artifact_sha256: Digest,
    node_indices: BTreeMap<(i8, String), usize>,
    adjacency: Vec<Vec<usize>>,
}

impl WorldSurfaceGraph {
    pub fn build(inventory: &WorldInventory) -> Result<Self, WorldSurfaceGraphError> {
        if inventory.schema != WORLD_INVENTORY_SCHEMA {
            return Err(WorldSurfaceGraphError::new(format!(
                "unsupported world inventory schema {:?}",
                inventory.schema
            )));
        }
        inventory
            .validate()
            .map_err(|error| WorldSurfaceGraphError::new(error.to_string()))?;
        let inventory_sha256 = inventory
            .digest()
            .map_err(|error| WorldSurfaceGraphError::new(error.to_string()))?;
        let spatial = WorldSpatialIndex::build(inventory)
            .map_err(|error| WorldSurfaceGraphError::new(error.to_string()))?;
        let spatial_index_sha256 = spatial
            .artifact_digest()
            .map_err(|error| WorldSurfaceGraphError::new(error.to_string()))?;
        let excluded = spatial
            .artifact()
            .excluded
            .iter()
            .map(|surface| ExcludedSurfaceGraphNode {
                room: surface.room,
                collision_id: surface.collision_id.clone(),
                reason: surface.reason.clone(),
            })
            .collect::<Vec<_>>();

        let triggers = inventory
            .load_triggers
            .iter()
            .map(|trigger| ((trigger.room, trigger.collision_id.as_str()), trigger))
            .collect::<BTreeMap<_, _>>();
        let mut node_inputs = Vec::new();
        for collision in &inventory.collisions {
            let (plane, triangle) = match &collision.prism.reconstruction {
                KclReconstruction::Reconstructed { plane, triangle } => (*plane, *triangle),
                KclReconstruction::Degenerate { .. } => continue,
            };
            let centroid = Vec3 {
                x: (triangle[0].x + triangle[1].x + triangle[2].x) / 3.0,
                y: (triangle[0].y + triangle[1].y + triangle[2].y) / 3.0,
                z: (triangle[0].z + triangle[1].z + triangle[2].z) / 3.0,
            };
            if !finite_vec3(centroid) || !finite_vec3(plane.normal) {
                return Err(WorldSurfaceGraphError::new(
                    "reconstructed collision contains nonfinite graph geometry",
                ));
            }
            node_inputs.push((
                SurfaceGraphNode {
                    room: collision.room,
                    collision_id: collision.prism.authored.stable_id.clone(),
                    attribute: collision.prism.authored.attribute,
                    collision_code: collision.prism.authored.code,
                    centroid,
                    plane_normal: plane.normal,
                    load_trigger: triggers
                        .get(&(collision.room, collision.prism.authored.stable_id.as_str()))
                        .map(|trigger| (*trigger).clone()),
                },
                triangle,
            ));
        }
        node_inputs.sort_by(|left, right| {
            left.0
                .room
                .cmp(&right.0.room)
                .then_with(|| left.0.collision_id.cmp(&right.0.collision_id))
        });
        if node_inputs.windows(2).any(|pair| {
            pair[0].0.room == pair[1].0.room && pair[0].0.collision_id == pair[1].0.collision_id
        }) {
            return Err(WorldSurfaceGraphError::new(
                "world inventory contains duplicate collision identities",
            ));
        }

        let mut exact_edge_groups = BTreeMap::<(i8, EdgeKey), Vec<usize>>::new();
        for (node_index, (node, triangle)) in node_inputs.iter().enumerate() {
            let keys = triangle_edge_keys(*triangle)?;
            if keys[0] == keys[1] || keys[0] == keys[2] || keys[1] == keys[2] {
                return Err(WorldSurfaceGraphError::new(
                    "reconstructed collision contains duplicate graph edges",
                ));
            }
            for key in keys {
                exact_edge_groups
                    .entry((node.room, key))
                    .or_default()
                    .push(node_index);
            }
        }
        let mut exact_shared_by_room = BTreeMap::<i8, usize>::new();
        for ((room, _), members) in &exact_edge_groups {
            if members.iter().copied().collect::<BTreeSet<_>>().len() >= 2 {
                *exact_shared_by_room.entry(*room).or_default() += 1;
            }
        }

        let clustering = cluster_vertices(&node_inputs)?;
        let mut edge_groups = BTreeMap::<(i8, ClusterEdgeKey), Vec<usize>>::new();
        let mut collapsed_by_room = BTreeMap::<i8, usize>::new();
        for (node_index, (node, triangle)) in node_inputs.iter().enumerate() {
            for edge in triangle_edge_keys(*triangle)? {
                let left = clustering.cluster_for(node.room, edge.0[0])?;
                let right = clustering.cluster_for(node.room, edge.0[1])?;
                if left == right {
                    *collapsed_by_room.entry(node.room).or_default() += 1;
                    continue;
                }
                edge_groups
                    .entry((node.room, ClusterEdgeKey::new(left, right)))
                    .or_default()
                    .push(node_index);
            }
        }

        let room_ids = node_inputs
            .iter()
            .map(|(node, _)| node.room)
            .chain(excluded.iter().map(|surface| surface.room))
            .collect::<BTreeSet<_>>();
        let mut room_counts = room_ids
            .iter()
            .map(|room| {
                (
                    *room,
                    RoomSurfaceGraphCoverage {
                        room: *room,
                        reconstructed_surfaces: node_inputs
                            .iter()
                            .filter(|(node, _)| node.room == *room)
                            .count(),
                        excluded_surfaces: excluded
                            .iter()
                            .filter(|surface| surface.room == *room)
                            .count(),
                        exact_shared_edge_groups: exact_shared_by_room
                            .get(room)
                            .copied()
                            .unwrap_or_default(),
                        clustered_shared_edge_groups: 0,
                        boundary_edge_groups: 0,
                        nonmanifold_edge_groups: 0,
                        collapsed_triangle_edges: collapsed_by_room
                            .get(room)
                            .copied()
                            .unwrap_or_default(),
                        vertex_clusters: clustering
                            .room_metrics
                            .get(room)
                            .map(|metrics| metrics.clusters)
                            .unwrap_or_default(),
                        maximum_vertex_cluster_diameter: clustering
                            .room_metrics
                            .get(room)
                            .map(|metrics| metrics.maximum_diameter)
                            .unwrap_or_default(),
                        adjacency_edges: 0,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut edges = Vec::new();
        for ((room, key), mut members) in edge_groups {
            members.sort_unstable();
            members.dedup();
            let coverage = room_counts
                .get_mut(&room)
                .expect("node room has coverage accounting");
            if members.len() == 1 {
                coverage.boundary_edge_groups += 1;
                continue;
            }
            coverage.clustered_shared_edge_groups += 1;
            if members.len() > 2 {
                coverage.nonmanifold_edge_groups += 1;
            }
            let incidence_count = u16::try_from(members.len()).map_err(|_| {
                WorldSurfaceGraphError::new("surface edge incidence exceeds the graph limit")
            })?;
            for left_offset in 0..members.len() {
                for right_offset in left_offset + 1..members.len() {
                    let left = &node_inputs[members[left_offset]].0;
                    let right = &node_inputs[members[right_offset]].0;
                    let (left_collision_id, right_collision_id) =
                        if left.collision_id < right.collision_id {
                            (left.collision_id.clone(), right.collision_id.clone())
                        } else {
                            (right.collision_id.clone(), left.collision_id.clone())
                        };
                    edges.push(SurfaceGraphEdge {
                        room,
                        left_collision_id,
                        right_collision_id,
                        shared_edge: clustering.edge_points(key),
                        incidence_count,
                    });
                }
            }
        }
        edges.sort_by(|left, right| {
            left.room
                .cmp(&right.room)
                .then_with(|| left.left_collision_id.cmp(&right.left_collision_id))
                .then_with(|| left.right_collision_id.cmp(&right.right_collision_id))
                .then_with(|| {
                    edge_key(left.shared_edge)
                        .expect("built edge is finite")
                        .cmp(&edge_key(right.shared_edge).expect("built edge is finite"))
                })
        });
        for edge in &edges {
            room_counts
                .get_mut(&edge.room)
                .expect("edge room has coverage accounting")
                .adjacency_edges += 1;
        }

        let artifact = WorldSurfaceGraphArtifact {
            schema: WORLD_SURFACE_GRAPH_SCHEMA.into(),
            stage: inventory.stage.clone(),
            inventory_sha256,
            spatial_index_sha256,
            algorithm: SurfaceGraphAlgorithm {
                id: "bounded-clustered-room-edge/v1".into(),
                vertex_equivalence: "euclidean-connected-components-signed-zero-canonical/v1"
                    .into(),
                coordinate_scope: "room-kcl-authored/v1".into(),
                reachability_semantics: "topological-adjacency-not-walkability".into(),
                maximum_vertex_distance: SURFACE_GRAPH_VERTEX_TOLERANCE,
            },
            source_collision_count: inventory.collisions.len(),
            nodes: node_inputs.into_iter().map(|(node, _)| node).collect(),
            edges,
            excluded,
            rooms: room_counts.into_values().collect(),
        };
        artifact.validate()?;
        let artifact_sha256 = artifact.digest()?;
        Self::from_artifact(artifact, artifact_sha256)
    }

    fn from_artifact(
        artifact: WorldSurfaceGraphArtifact,
        artifact_sha256: Digest,
    ) -> Result<Self, WorldSurfaceGraphError> {
        let node_indices = artifact
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| ((node.room, node.collision_id.clone()), index))
            .collect::<BTreeMap<_, _>>();
        let mut adjacency = vec![Vec::new(); artifact.nodes.len()];
        for edge in &artifact.edges {
            let left = *node_indices
                .get(&(edge.room, edge.left_collision_id.clone()))
                .ok_or_else(|| WorldSurfaceGraphError::new("graph edge has no left node"))?;
            let right = *node_indices
                .get(&(edge.room, edge.right_collision_id.clone()))
                .ok_or_else(|| WorldSurfaceGraphError::new("graph edge has no right node"))?;
            adjacency[left].push(right);
            adjacency[right].push(left);
        }
        for neighbors in &mut adjacency {
            neighbors.sort_unstable();
            neighbors.dedup();
        }
        Ok(Self {
            artifact,
            artifact_sha256,
            node_indices,
            adjacency,
        })
    }

    pub fn artifact(&self) -> &WorldSurfaceGraphArtifact {
        &self.artifact
    }

    pub fn artifact_digest(&self) -> Digest {
        self.artifact_sha256
    }

    pub fn neighborhood(
        &self,
        request: WorldSurfaceNeighborhoodRequest,
    ) -> Result<WorldSurfaceNeighborhoodReport, WorldSurfaceGraphError> {
        validate_request(&request)?;
        let mut queue = VecDeque::new();
        let mut distances = BTreeMap::<usize, u8>::new();
        for seed in &request.seed_collision_ids {
            let index = *self
                .node_indices
                .get(&(request.room, seed.clone()))
                .ok_or_else(|| {
                    WorldSurfaceGraphError::new(format!(
                        "surface graph seed is unavailable in room {}: {seed:?}",
                        request.room
                    ))
                })?;
            distances.insert(index, 0);
            queue.push_back(index);
        }
        while let Some(current) = queue.pop_front() {
            let hops = distances[&current];
            if hops == request.maximum_hops {
                continue;
            }
            let next_hops = hops + 1;
            for neighbor in &self.adjacency[current] {
                if !distances.contains_key(neighbor) {
                    distances.insert(*neighbor, next_hops);
                    queue.push_back(*neighbor);
                }
            }
        }
        let reachable_within_hops = distances.len();
        let mut ordered = distances.into_iter().collect::<Vec<_>>();
        ordered.sort_by(|(left_index, left_hops), (right_index, right_hops)| {
            left_hops.cmp(right_hops).then_with(|| {
                self.artifact.nodes[*left_index]
                    .collision_id
                    .cmp(&self.artifact.nodes[*right_index].collision_id)
            })
        });
        ordered.truncate(request.maximum_nodes);
        let returned_indices = ordered
            .iter()
            .map(|(index, _)| *index)
            .collect::<BTreeSet<_>>();
        let visits = ordered
            .iter()
            .map(|(index, hops)| SurfaceGraphVisit {
                collision_id: self.artifact.nodes[*index].collision_id.clone(),
                minimum_hops: *hops,
            })
            .collect::<Vec<_>>();
        let induced_edge_indices = self
            .artifact
            .edges
            .iter()
            .enumerate()
            .filter_map(|(edge_index, edge)| {
                if edge.room != request.room {
                    return None;
                }
                let left = self
                    .node_indices
                    .get(&(edge.room, edge.left_collision_id.clone()))?;
                let right = self
                    .node_indices
                    .get(&(edge.room, edge.right_collision_id.clone()))?;
                (returned_indices.contains(left) && returned_indices.contains(right))
                    .then_some(edge_index)
            })
            .collect::<Vec<_>>();
        let request_room = request.room;
        let mut report = WorldSurfaceNeighborhoodReport {
            schema: WORLD_SURFACE_NEIGHBORHOOD_SCHEMA.into(),
            stage: self.artifact.stage.clone(),
            inventory_sha256: self.artifact.inventory_sha256,
            spatial_index_sha256: self.artifact.spatial_index_sha256,
            surface_graph_sha256: self.artifact_sha256,
            request,
            surface_graph_edge_count: self.artifact.edges.len(),
            eligible_room_nodes: self
                .artifact
                .nodes
                .iter()
                .filter(|node| node.room == request_room)
                .count(),
            reachable_within_hops,
            returned_nodes: visits.len(),
            truncated: reachable_within_hops > visits.len(),
            visits,
            induced_edge_indices,
            report_sha256: Digest::ZERO,
        };
        report.report_sha256 = report.compute_digest()?;
        report.validate()?;
        Ok(report)
    }
}

fn validate_request(
    request: &WorldSurfaceNeighborhoodRequest,
) -> Result<(), WorldSurfaceGraphError> {
    if request.seed_collision_ids.is_empty()
        || request.seed_collision_ids.len() > MAX_SURFACE_GRAPH_SEEDS
        || request.maximum_hops > MAX_SURFACE_GRAPH_HOPS
        || !(1..=MAX_SURFACE_GRAPH_RESULTS).contains(&request.maximum_nodes)
        || !request.seed_collision_ids.is_sorted()
        || request
            .seed_collision_ids
            .windows(2)
            .any(|pair| pair[0] == pair[1])
        || request
            .seed_collision_ids
            .iter()
            .any(|id| id.is_empty() || id.len() > 512)
    {
        return Err(WorldSurfaceGraphError::new(
            "world surface neighborhood request is noncanonical or unbounded",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct VertexKey([u32; 3]);

impl VertexKey {
    fn from_vec3(value: Vec3) -> Result<Self, WorldSurfaceGraphError> {
        if !finite_vec3(value) {
            return Err(WorldSurfaceGraphError::new(
                "surface graph vertex must be finite",
            ));
        }
        Ok(Self([
            canonical_float_bits(value.x),
            canonical_float_bits(value.y),
            canonical_float_bits(value.z),
        ]))
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct EdgeKey([VertexKey; 2]);

impl EdgeKey {
    fn new(left: Vec3, right: Vec3) -> Result<Self, WorldSurfaceGraphError> {
        let left = VertexKey::from_vec3(left)?;
        let right = VertexKey::from_vec3(right)?;
        if left == right {
            return Err(WorldSurfaceGraphError::new(
                "surface graph edge has identical vertices",
            ));
        }
        Ok(if left < right {
            Self([left, right])
        } else {
            Self([right, left])
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ClusterEdgeKey([usize; 2]);

impl ClusterEdgeKey {
    fn new(left: usize, right: usize) -> Self {
        if left < right {
            Self([left, right])
        } else {
            Self([right, left])
        }
    }
}

#[derive(Clone, Copy)]
struct VertexSample {
    room: i8,
    key: VertexKey,
    point: Vec3,
}

#[derive(Clone, Copy, Default)]
struct RoomClusterMetrics {
    clusters: usize,
    maximum_diameter: f32,
}

struct VertexClustering {
    samples: Vec<VertexSample>,
    cluster_by_vertex: BTreeMap<(i8, VertexKey), usize>,
    room_metrics: BTreeMap<i8, RoomClusterMetrics>,
}

impl VertexClustering {
    fn cluster_for(&self, room: i8, vertex: VertexKey) -> Result<usize, WorldSurfaceGraphError> {
        self.cluster_by_vertex
            .get(&(room, vertex))
            .copied()
            .ok_or_else(|| WorldSurfaceGraphError::new("surface vertex has no cluster"))
    }

    fn edge_points(&self, edge: ClusterEdgeKey) -> [Vec3; 2] {
        [self.samples[edge.0[0]].point, self.samples[edge.0[1]].point]
    }
}

struct DisjointSet {
    parent: Vec<usize>,
}

impl DisjointSet {
    fn new(size: usize) -> Self {
        Self {
            parent: (0..size).collect(),
        }
    }

    fn find(&mut self, mut node: usize) -> usize {
        while self.parent[node] != node {
            self.parent[node] = self.parent[self.parent[node]];
            node = self.parent[node];
        }
        node
    }

    fn union(&mut self, left: usize, right: usize) {
        let left = self.find(left);
        let right = self.find(right);
        if left == right {
            return;
        }
        let (root, child) = if left < right {
            (left, right)
        } else {
            (right, left)
        };
        self.parent[child] = root;
    }
}

fn cluster_vertices(
    node_inputs: &[(SurfaceGraphNode, [Vec3; 3])],
) -> Result<VertexClustering, WorldSurfaceGraphError> {
    let mut unique = BTreeMap::<(i8, VertexKey), Vec3>::new();
    for (node, triangle) in node_inputs {
        for point in triangle {
            let key = VertexKey::from_vec3(*point)?;
            unique.entry((node.room, key)).or_insert(*point);
        }
    }
    let samples = unique
        .into_iter()
        .map(|((room, key), point)| VertexSample { room, key, point })
        .collect::<Vec<_>>();
    let mut disjoint = DisjointSet::new(samples.len());
    let mut cells = BTreeMap::<(i8, i64, i64, i64), Vec<usize>>::new();
    let tolerance = f64::from(SURFACE_GRAPH_VERTEX_TOLERANCE);
    let tolerance_squared = tolerance * tolerance;
    for (index, sample) in samples.iter().enumerate() {
        let cell = (
            sample.room,
            vertex_cell(sample.point.x)?,
            vertex_cell(sample.point.y)?,
            vertex_cell(sample.point.z)?,
        );
        for x_offset in -1_i64..=1 {
            for y_offset in -1_i64..=1 {
                for z_offset in -1_i64..=1 {
                    let neighbor_cell = (
                        sample.room,
                        cell.1.checked_add(x_offset).ok_or_else(|| {
                            WorldSurfaceGraphError::new("surface vertex cell overflowed")
                        })?,
                        cell.2.checked_add(y_offset).ok_or_else(|| {
                            WorldSurfaceGraphError::new("surface vertex cell overflowed")
                        })?,
                        cell.3.checked_add(z_offset).ok_or_else(|| {
                            WorldSurfaceGraphError::new("surface vertex cell overflowed")
                        })?,
                    );
                    for candidate in cells.get(&neighbor_cell).into_iter().flatten() {
                        if squared_distance(sample.point, samples[*candidate].point)
                            <= tolerance_squared
                        {
                            disjoint.union(index, *candidate);
                        }
                    }
                }
            }
        }
        cells.entry(cell).or_default().push(index);
    }

    let roots = (0..samples.len())
        .map(|index| disjoint.find(index))
        .collect::<Vec<_>>();
    let cluster_by_vertex = samples
        .iter()
        .zip(&roots)
        .map(|(sample, root)| ((sample.room, sample.key), *root))
        .collect::<BTreeMap<_, _>>();
    let mut members = BTreeMap::<usize, Vec<usize>>::new();
    for (index, root) in roots.iter().enumerate() {
        members.entry(*root).or_default().push(index);
    }
    let mut room_metrics = BTreeMap::<i8, RoomClusterMetrics>::new();
    for indices in members.values() {
        let room = samples[indices[0]].room;
        let metrics = room_metrics.entry(room).or_default();
        metrics.clusters += 1;
        let mut maximum_squared = 0.0_f64;
        for left_offset in 0..indices.len() {
            for right_offset in 0..left_offset {
                maximum_squared = maximum_squared.max(squared_distance(
                    samples[indices[left_offset]].point,
                    samples[indices[right_offset]].point,
                ));
            }
        }
        metrics.maximum_diameter = metrics.maximum_diameter.max(maximum_squared.sqrt() as f32);
    }
    Ok(VertexClustering {
        samples,
        cluster_by_vertex,
        room_metrics,
    })
}

fn vertex_cell(value: f32) -> Result<i64, WorldSurfaceGraphError> {
    let cell = (f64::from(value) / f64::from(SURFACE_GRAPH_VERTEX_TOLERANCE)).floor();
    if cell < i64::MIN as f64 || cell > i64::MAX as f64 {
        return Err(WorldSurfaceGraphError::new(
            "surface vertex exceeds the clustering coordinate bound",
        ));
    }
    Ok(cell as i64)
}

fn squared_distance(left: Vec3, right: Vec3) -> f64 {
    let x = f64::from(left.x) - f64::from(right.x);
    let y = f64::from(left.y) - f64::from(right.y);
    let z = f64::from(left.z) - f64::from(right.z);
    x.mul_add(x, y.mul_add(y, z * z))
}

fn triangle_edge_keys(triangle: [Vec3; 3]) -> Result<[EdgeKey; 3], WorldSurfaceGraphError> {
    Ok([
        EdgeKey::new(triangle[0], triangle[1])?,
        EdgeKey::new(triangle[1], triangle[2])?,
        EdgeKey::new(triangle[2], triangle[0])?,
    ])
}

fn edge_key(edge: [Vec3; 2]) -> Result<EdgeKey, WorldSurfaceGraphError> {
    EdgeKey::new(edge[0], edge[1])
}

fn canonical_float_bits(value: f32) -> u32 {
    if value == 0.0 { 0 } else { value.to_bits() }
}

fn finite_vec3(value: Vec3) -> bool {
    value.x.is_finite() && value.y.is_finite() && value.z.is_finite()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorldSurfaceGraphError(String);

impl WorldSurfaceGraphError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for WorldSurfaceGraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for WorldSurfaceGraphError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world_geometry::{
        CollisionPlane, KclAuthoredPrism, KclInventoryPrism, KclSourceIndices,
    };
    use crate::world_inventory::{
        CollisionInventoryRecord, SourceKind, SourceScope, StageExitRecord, WorldSource,
    };

    fn collision_code(exit_id: u8) -> CollisionCode {
        let raw = [u32::from(exit_id), 0, 0, 0, 0];
        CollisionCode {
            raw,
            exit_id,
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
        }
    }

    fn inventory() -> WorldInventory {
        let kcl = Digest([3; 32]);
        let plc = Digest([4; 32]);
        let collision_id = |index: u16| format!("kcl-sha256:{kcl}/plc-sha256:{plc}/prism/{index}");
        let point = |x: f32, z: f32| Vec3 { x, y: 0.0, z };
        let triangles = [
            [point(0.0, 0.0), point(1.0, 0.0), point(0.0, 1.0)],
            [
                Vec3 {
                    x: 1.0,
                    y: -0.0,
                    z: 0.0,
                },
                point(1.0, 1.0),
                Vec3 {
                    x: 0.0,
                    y: -0.0,
                    z: 1.0,
                },
            ],
            [point(1.0, 0.0), point(2.0, 0.0), point(1.0, 1.0)],
            [point(10.0, 0.0), point(11.0, 0.0), point(10.0, 1.0)],
        ];
        let mut collisions = triangles
            .into_iter()
            .enumerate()
            .map(|(offset, triangle)| {
                let index = u16::try_from(offset + 1).unwrap();
                CollisionInventoryRecord {
                    room: 0,
                    prism: KclInventoryPrism {
                        authored: KclAuthoredPrism {
                            stable_id: collision_id(index),
                            prism_index: index,
                            height: 1.0,
                            source_indices: KclSourceIndices {
                                position: 0,
                                face_normal: 0,
                                edge_normal_1: 0,
                                edge_normal_2: 0,
                                edge_normal_3: 0,
                            },
                            attribute: index,
                            code: collision_code(if index == 3 { 0 } else { 0x3f }),
                        },
                        reconstruction: KclReconstruction::Reconstructed {
                            plane: CollisionPlane {
                                anchor: triangle[0],
                                normal: Vec3 {
                                    x: 0.0,
                                    y: 1.0,
                                    z: 0.0,
                                },
                                d: 0.0,
                            },
                            triangle,
                        },
                    },
                }
            })
            .collect::<Vec<_>>();
        collisions.push(CollisionInventoryRecord {
            room: 0,
            prism: KclInventoryPrism {
                authored: KclAuthoredPrism {
                    stable_id: collision_id(5),
                    prism_index: 5,
                    height: 0.0,
                    source_indices: KclSourceIndices {
                        position: 0,
                        face_normal: 0,
                        edge_normal_1: 0,
                        edge_normal_2: 0,
                        edge_normal_3: 0,
                    },
                    attribute: 5,
                    code: collision_code(0x3f),
                },
                reconstruction: KclReconstruction::Degenerate {
                    reason: "synthetic degenerate prism".into(),
                },
            },
        });
        let room_scope = SourceScope {
            kind: SourceKind::Room,
            room: Some(0),
        };
        let exit = StageExitRecord {
            stable_id: "synthetic-scls/0".into(),
            source_sha256: Digest([6; 32]),
            scope: room_scope,
            chunk_tag: "SCLS".into(),
            record_index: 0,
            destination_stage: "NEXT".into(),
            destination_point: 2,
            destination_room: 1,
            destination_layer: -1,
            wipe: 0,
            wipe_time: 0,
            time_hour: 0,
            raw_start: 0,
            raw_field_a: 0,
            raw_field_b: 0,
            raw_wipe: 0,
            raw_hex: "00".repeat(13),
        };
        let mut trigger_hasher = Sha256::new();
        trigger_hasher.update(b"dusklight.collision-load-trigger/v1\0");
        trigger_hasher.update(collision_id(3).as_bytes());
        trigger_hasher.update([0]);
        trigger_hasher.update(exit.stable_id.as_bytes());
        let trigger_digest = Digest(trigger_hasher.finalize().into());
        WorldInventory {
            schema: WORLD_INVENTORY_SCHEMA.into(),
            stage: "TEST".into(),
            sources: vec![
                WorldSource {
                    scope: SourceScope {
                        kind: SourceKind::Stage,
                        room: None,
                    },
                    archive_sha256: Digest([1; 32]),
                    stage_data_path: "stage.dzs".into(),
                    stage_data_sha256: Digest([2; 32]),
                    kcl_path: None,
                    kcl_sha256: None,
                    plc_path: None,
                    plc_sha256: None,
                    addressable_prisms: 0,
                },
                WorldSource {
                    scope: room_scope,
                    archive_sha256: Digest([5; 32]),
                    stage_data_path: "room.dzr".into(),
                    stage_data_sha256: Digest([6; 32]),
                    kcl_path: Some("room.kcl".into()),
                    kcl_sha256: Some(kcl),
                    plc_path: Some("room.plc".into()),
                    plc_sha256: Some(plc),
                    addressable_prisms: 5,
                },
            ],
            chunks: Vec::new(),
            placements: Vec::new(),
            player_spawns: Vec::new(),
            exits: vec![exit],
            collisions,
            load_triggers: vec![CollisionLoadTrigger {
                stable_id: format!("load-trigger-sha256:{trigger_digest}"),
                room: 0,
                collision_id: collision_id(3),
                collision_exit_id: 0,
                scls_id: "synthetic-scls/0".into(),
                destination_stage: "NEXT".into(),
                destination_room: 1,
                destination_layer: -1,
                destination_point: 2,
                inferred_semantics: true,
            }],
        }
    }

    #[test]
    fn exact_shared_edges_form_a_bounded_content_bound_neighborhood() {
        let inventory = inventory();
        inventory.validate().unwrap();
        let graph = WorldSurfaceGraph::build(&inventory).unwrap();
        let second = WorldSurfaceGraph::build(&inventory).unwrap();
        assert_eq!(graph.artifact(), second.artifact());
        assert_eq!(graph.artifact_digest(), second.artifact_digest());
        assert_eq!(graph.artifact().nodes.len(), 4);
        assert_eq!(graph.artifact().edges.len(), 2);
        assert_eq!(graph.artifact().excluded.len(), 1);
        assert_eq!(graph.artifact().rooms[0].exact_shared_edge_groups, 2);
        assert_eq!(graph.artifact().rooms[0].clustered_shared_edge_groups, 2);
        assert_eq!(graph.artifact().rooms[0].boundary_edge_groups, 8);
        assert_eq!(graph.artifact().rooms[0].nonmanifold_edge_groups, 0);
        assert_eq!(graph.artifact().rooms[0].adjacency_edges, 2);
        assert!(graph.artifact().nodes[2].load_trigger.is_some());

        let seed = graph.artifact().nodes[0].collision_id.clone();
        let report = graph
            .neighborhood(WorldSurfaceNeighborhoodRequest {
                room: 0,
                seed_collision_ids: vec![seed.clone()],
                maximum_hops: 2,
                maximum_nodes: 4,
            })
            .unwrap();
        assert_eq!(report.reachable_within_hops, 3);
        assert_eq!(report.returned_nodes, 3);
        assert!(!report.truncated);
        assert_eq!(
            report
                .visits
                .iter()
                .map(|visit| visit.minimum_hops)
                .collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert_eq!(report.induced_edge_indices.len(), 2);
        assert_ne!(report.report_sha256, Digest::ZERO);

        let truncated = graph
            .neighborhood(WorldSurfaceNeighborhoodRequest {
                room: 0,
                seed_collision_ids: vec![seed],
                maximum_hops: 2,
                maximum_nodes: 2,
            })
            .unwrap();
        assert_eq!(truncated.reachable_within_hops, 3);
        assert_eq!(truncated.returned_nodes, 2);
        assert!(truncated.truncated);
        assert_eq!(truncated.induced_edge_indices.len(), 1);
    }

    #[test]
    fn canonical_artifacts_reject_tampering_and_queries_fail_closed() {
        let inventory = inventory();
        let graph = WorldSurfaceGraph::build(&inventory).unwrap();
        let bytes = graph.artifact().canonical_bytes().unwrap();
        assert_eq!(
            WorldSurfaceGraphArtifact::decode_canonical(&bytes).unwrap(),
            *graph.artifact()
        );
        let mut tampered = graph.artifact().clone();
        tampered.rooms[0].adjacency_edges += 1;
        let tampered = serde_json::to_vec(&tampered).unwrap();
        assert!(WorldSurfaceGraphArtifact::decode_canonical(&tampered).is_err());

        let unknown = graph.neighborhood(WorldSurfaceNeighborhoodRequest {
            room: 0,
            seed_collision_ids: vec!["unknown".into()],
            maximum_hops: 1,
            maximum_nodes: 4,
        });
        assert!(unknown.is_err());
        let unsorted = graph.neighborhood(WorldSurfaceNeighborhoodRequest {
            room: 0,
            seed_collision_ids: vec![
                graph.artifact().nodes[1].collision_id.clone(),
                graph.artifact().nodes[0].collision_id.clone(),
            ],
            maximum_hops: 1,
            maximum_nodes: 4,
        });
        assert!(unsorted.is_err());
    }
}
