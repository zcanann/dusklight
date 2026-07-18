//! Deterministic offline spatial queries over a [`WorldInventory`].
//!
//! This module indexes already-reconstructed immutable triangles. It never
//! calls the game collision system, reads a live process, or mutates gameplay.

use crate::artifact::Digest;
use crate::world_geometry::{
    CollisionPlane, KclAuthoredPrism, KclReconstruction, PointTriangleQuery, Vec3,
    WorldGeometryError, query_triangle_point,
};
use crate::world_inventory::{
    CollisionLoadTrigger, WORLD_INVENTORY_SCHEMA, WorldInventory, WorldInventoryError,
};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BinaryHeap};
use std::error::Error;
use std::fmt;

pub const WORLD_POINT_QUERY_SCHEMA: &str = "dusklight-world-point-query/v1";
pub const WORLD_AABB_QUERY_SCHEMA: &str = "dusklight-world-aabb-query/v1";
pub const WORLD_RAY_QUERY_SCHEMA: &str = "dusklight-world-ray-query/v1";
pub const WORLD_SPATIAL_INDEX_SCHEMA: &str = "dusklight-world-spatial-index/v1";
pub const MAX_WORLD_QUERY_RESULTS: usize = 256;
const BVH_LEAF_SIZE: usize = 8;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorldSpatialError(String);

impl WorldSpatialError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for WorldSpatialError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for WorldSpatialError {}

impl From<WorldGeometryError> for WorldSpatialError {
    fn from(value: WorldGeometryError) -> Self {
        Self::new(value.to_string())
    }
}

impl From<WorldInventoryError> for WorldSpatialError {
    fn from(value: WorldInventoryError) -> Self {
        Self::new(value.to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Aabb3 {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb3 {
    pub fn new(min: Vec3, max: Vec3) -> Result<Self, WorldSpatialError> {
        if !finite_vec3(min) || !finite_vec3(max) {
            return Err(WorldSpatialError::new("AABB components must be finite"));
        }
        if min.x > max.x || min.y > max.y || min.z > max.z {
            return Err(WorldSpatialError::new(
                "AABB minimum must not exceed maximum",
            ));
        }
        Ok(Self { min, max })
    }

    fn from_triangle(triangle: [Vec3; 3]) -> Result<Self, WorldSpatialError> {
        if !triangle.into_iter().all(finite_vec3) {
            return Err(WorldSpatialError::new(
                "indexed triangle contains a non-finite component",
            ));
        }
        let mut min = triangle[0];
        let mut max = triangle[0];
        for vertex in &triangle[1..] {
            min.x = min.x.min(vertex.x);
            min.y = min.y.min(vertex.y);
            min.z = min.z.min(vertex.z);
            max.x = max.x.max(vertex.x);
            max.y = max.y.max(vertex.y);
            max.z = max.z.max(vertex.z);
        }
        Ok(Self { min, max })
    }

    fn union(self, rhs: Self) -> Self {
        Self {
            min: Vec3 {
                x: self.min.x.min(rhs.min.x),
                y: self.min.y.min(rhs.min.y),
                z: self.min.z.min(rhs.min.z),
            },
            max: Vec3 {
                x: self.max.x.max(rhs.max.x),
                y: self.max.y.max(rhs.max.y),
                z: self.max.z.max(rhs.max.z),
            },
        }
    }

    fn centroid(self) -> Vec3 {
        Vec3 {
            x: (self.min.x + self.max.x) * 0.5,
            y: (self.min.y + self.max.y) * 0.5,
            z: (self.min.z + self.max.z) * 0.5,
        }
    }

    fn extent(self) -> Vec3 {
        Vec3 {
            x: self.max.x - self.min.x,
            y: self.max.y - self.min.y,
            z: self.max.z - self.min.z,
        }
    }

    fn distance_squared(self, point: Vec3) -> f64 {
        let axis = |value: f32, min: f32, max: f32| -> f64 {
            let value = f64::from(value);
            let min = f64::from(min);
            let max = f64::from(max);
            if value < min {
                min - value
            } else if value > max {
                value - max
            } else {
                0.0
            }
        };
        let x = axis(point.x, self.min.x, self.max.x);
        let y = axis(point.y, self.min.y, self.max.y);
        let z = axis(point.z, self.min.z, self.max.z);
        x * x + y * y + z * z
    }

    fn overlaps(self, rhs: Self) -> bool {
        self.min.x <= rhs.max.x
            && self.max.x >= rhs.min.x
            && self.min.y <= rhs.max.y
            && self.max.y >= rhs.min.y
            && self.min.z <= rhs.max.z
            && self.max.z >= rhs.min.z
    }

    fn ray_intersects(self, origin: Vec3, direction: Vec3, max_distance: f64) -> bool {
        let mut enter = 0.0_f64;
        let mut exit = max_distance;
        for (origin, direction, min, max) in [
            (origin.x, direction.x, self.min.x, self.max.x),
            (origin.y, direction.y, self.min.y, self.max.y),
            (origin.z, direction.z, self.min.z, self.max.z),
        ] {
            let origin = f64::from(origin);
            let direction = f64::from(direction);
            let min = f64::from(min);
            let max = f64::from(max);
            if direction.abs() <= f64::EPSILON {
                if origin < min || origin > max {
                    return false;
                }
                continue;
            }
            let inverse = 1.0 / direction;
            let mut near = (min - origin) * inverse;
            let mut far = (max - origin) * inverse;
            if near > far {
                std::mem::swap(&mut near, &mut far);
            }
            enter = enter.max(near);
            exit = exit.min(far);
            if enter > exit {
                return false;
            }
        }
        exit >= 0.0 && enter <= max_distance
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorldSurfaceFilter {
    /// Required authored room-KCL coordinate scope. Room transforms have not
    /// yet been decoded, so cross-room coordinate queries are forbidden.
    pub room: i8,
    pub load_triggers_only: bool,
    pub trigger_stable_id: Option<String>,
    pub destination_stage: Option<String>,
    pub destination_room: Option<i8>,
    pub destination_point: Option<i16>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorldPointQueryRequest {
    pub point: Vec3,
    pub max_distance: Option<f32>,
    pub limit: usize,
    pub filter: WorldSurfaceFilter,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorldAabbQueryRequest {
    pub bounds: Aabb3,
    pub limit: usize,
    pub filter: WorldSurfaceFilter,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorldRayQueryRequest {
    pub origin: Vec3,
    pub direction: Vec3,
    pub max_distance: f32,
    pub limit: usize,
    pub filter: WorldSurfaceFilter,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceGeometry {
    pub room: i8,
    pub authored: KclAuthoredPrism,
    pub plane: CollisionPlane,
    pub triangle: [Vec3; 3],
    pub load_trigger: Option<CollisionLoadTrigger>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SurfacePointHit {
    pub surface: SurfaceGeometry,
    pub point_query: PointTriangleQuery,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceRayHit {
    pub surface: SurfaceGeometry,
    pub distance: f64,
    pub position: Vec3,
    pub barycentric: [f32; 3],
    pub front_facing: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorldPointQueryReport {
    pub schema: &'static str,
    pub stage: String,
    pub coordinate_space: &'static str,
    pub inventory_sha256: Digest,
    pub spatial_index_sha256: Digest,
    pub request: WorldPointQueryRequest,
    pub indexed_surface_count: usize,
    pub excluded_degenerate_count: usize,
    pub excluded_matching_filter_count: usize,
    pub eligible_surface_count: usize,
    pub within_distance_count: usize,
    pub returned_count: usize,
    pub truncated: bool,
    pub bvh_node_visits: usize,
    pub triangle_tests: usize,
    pub results: Vec<SurfacePointHit>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorldAabbQueryReport {
    pub schema: &'static str,
    pub stage: String,
    pub coordinate_space: &'static str,
    pub semantics: &'static str,
    pub inventory_sha256: Digest,
    pub spatial_index_sha256: Digest,
    pub request: WorldAabbQueryRequest,
    pub indexed_surface_count: usize,
    pub excluded_degenerate_count: usize,
    pub excluded_matching_filter_count: usize,
    pub eligible_surface_count: usize,
    pub overlapping_aabb_count: usize,
    pub returned_count: usize,
    pub truncated: bool,
    pub bvh_node_visits: usize,
    pub results: Vec<SurfaceGeometry>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorldRayQueryReport {
    pub schema: &'static str,
    pub stage: String,
    pub coordinate_space: &'static str,
    pub sidedness: &'static str,
    pub inventory_sha256: Digest,
    pub spatial_index_sha256: Digest,
    pub request: WorldRayQueryRequest,
    pub normalized_direction: Vec3,
    pub indexed_surface_count: usize,
    pub excluded_degenerate_count: usize,
    pub excluded_matching_filter_count: usize,
    pub eligible_surface_count: usize,
    pub hit_count: usize,
    pub returned_count: usize,
    pub truncated: bool,
    pub bvh_node_visits: usize,
    pub triangle_tests: usize,
    pub results: Vec<SurfaceRayHit>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SpatialIndexAlgorithm {
    pub id: &'static str,
    pub leaf_capacity: usize,
    pub scalar_contract: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExcludedSpatialSurface {
    pub room: i8,
    pub collision_id: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RoomSpatialArtifact {
    pub room: i8,
    pub coordinate_space: &'static str,
    /// Stable-ID-sorted primitive identity table.
    pub primitive_ids: Vec<String>,
    /// Bounds parallel to `primitive_ids`.
    pub primitive_bounds: Vec<Aabb3>,
    /// BVH leaf order as indices into `primitive_ids`.
    pub leaf_order: Vec<usize>,
    pub nodes: Vec<BvhNode>,
    pub root: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorldSpatialArtifact {
    pub schema: &'static str,
    pub inventory_sha256: Digest,
    pub algorithm: SpatialIndexAlgorithm,
    pub rooms: Vec<RoomSpatialArtifact>,
    pub excluded: Vec<ExcludedSpatialSurface>,
}

impl WorldSpatialArtifact {
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, WorldSpatialError> {
        serde_json::to_vec(self).map_err(|error| WorldSpatialError::new(error.to_string()))
    }

    pub fn digest(&self) -> Result<Digest, WorldSpatialError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

#[derive(Clone, Copy)]
struct IndexedSurface {
    collision_index: usize,
    primitive_index: usize,
    bounds: Aabb3,
    centroid: Vec3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BvhNodeKind {
    Leaf { start: usize, end: usize },
    Branch { left: usize, right: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BvhNode {
    pub bounds: Aabb3,
    pub kind: BvhNodeKind,
}

#[derive(Clone, Copy, Debug)]
struct PendingNode {
    lower_bound: f64,
    node_index: usize,
}

#[derive(Clone, Copy)]
struct RayTraversal {
    origin: Vec3,
    direction: Vec3,
    max_distance: f64,
}

impl PartialEq for PendingNode {
    fn eq(&self, rhs: &Self) -> bool {
        self.lower_bound.to_bits() == rhs.lower_bound.to_bits() && self.node_index == rhs.node_index
    }
}

impl Eq for PendingNode {}

impl PartialOrd for PendingNode {
    fn partial_cmp(&self, rhs: &Self) -> Option<Ordering> {
        Some(self.cmp(rhs))
    }
}

impl Ord for PendingNode {
    fn cmp(&self, rhs: &Self) -> Ordering {
        // Reverse distance ordering so BinaryHeap pops the smallest bound.
        rhs.lower_bound
            .total_cmp(&self.lower_bound)
            .then_with(|| rhs.node_index.cmp(&self.node_index))
    }
}

struct RoomSpatialRuntime {
    surfaces: Vec<IndexedSurface>,
    nodes: Vec<BvhNode>,
    root: Option<usize>,
    excluded_degenerate_count: usize,
}

pub struct WorldSpatialIndex<'a> {
    inventory: &'a WorldInventory,
    inventory_sha256: Digest,
    artifact: WorldSpatialArtifact,
    artifact_sha256: Digest,
    rooms: BTreeMap<i8, RoomSpatialRuntime>,
    load_triggers: BTreeMap<(i8, &'a str), &'a CollisionLoadTrigger>,
}

impl<'a> WorldSpatialIndex<'a> {
    pub fn build(inventory: &'a WorldInventory) -> Result<Self, WorldSpatialError> {
        if inventory.schema != WORLD_INVENTORY_SCHEMA {
            return Err(WorldSpatialError::new(format!(
                "unsupported world inventory schema {:?}",
                inventory.schema
            )));
        }
        let inventory_sha256 = inventory.digest()?;
        let mut grouped = BTreeMap::<i8, Vec<IndexedSurface>>::new();
        let mut excluded = Vec::new();
        for (collision_index, collision) in inventory.collisions.iter().enumerate() {
            let triangle = match &collision.prism.reconstruction {
                KclReconstruction::Reconstructed { triangle, .. } => *triangle,
                KclReconstruction::Degenerate { reason } => {
                    excluded.push(ExcludedSpatialSurface {
                        room: collision.room,
                        collision_id: collision.prism.authored.stable_id.clone(),
                        reason: reason.clone(),
                    });
                    continue;
                }
            };
            let bounds = Aabb3::from_triangle(triangle)?;
            grouped
                .entry(collision.room)
                .or_default()
                .push(IndexedSurface {
                    collision_index,
                    primitive_index: 0,
                    bounds,
                    centroid: bounds.centroid(),
                });
        }
        excluded.sort_by(|left, right| {
            left.room
                .cmp(&right.room)
                .then_with(|| left.collision_id.cmp(&right.collision_id))
        });

        let mut rooms = BTreeMap::new();
        let mut room_artifacts = Vec::new();
        for (room, mut surfaces) in grouped {
            surfaces.sort_by(|left, right| {
                inventory.collisions[left.collision_index]
                    .prism
                    .authored
                    .stable_id
                    .cmp(
                        &inventory.collisions[right.collision_index]
                            .prism
                            .authored
                            .stable_id,
                    )
            });
            for (primitive_index, surface) in surfaces.iter_mut().enumerate() {
                surface.primitive_index = primitive_index;
            }
            let primitive_ids = surfaces
                .iter()
                .map(|surface| {
                    inventory.collisions[surface.collision_index]
                        .prism
                        .authored
                        .stable_id
                        .clone()
                })
                .collect::<Vec<_>>();
            let primitive_bounds = surfaces
                .iter()
                .map(|surface| surface.bounds)
                .collect::<Vec<_>>();
            let mut nodes = Vec::with_capacity(surfaces.len().saturating_mul(2));
            let root = if surfaces.is_empty() {
                None
            } else {
                Some(build_bvh(&mut surfaces, 0, &mut nodes))
            };
            let leaf_order = surfaces
                .iter()
                .map(|surface| surface.primitive_index)
                .collect();
            let excluded_degenerate_count = excluded
                .iter()
                .filter(|surface| surface.room == room)
                .count();
            room_artifacts.push(RoomSpatialArtifact {
                room,
                coordinate_space: "room-kcl-authored/v1",
                primitive_ids,
                primitive_bounds,
                leaf_order,
                nodes: nodes.clone(),
                root,
            });
            rooms.insert(
                room,
                RoomSpatialRuntime {
                    surfaces,
                    nodes,
                    root,
                    excluded_degenerate_count,
                },
            );
        }

        let indexed_count = rooms
            .values()
            .map(|room| room.surfaces.len())
            .sum::<usize>();
        if indexed_count + excluded.len() != inventory.collisions.len() {
            return Err(WorldSpatialError::new(
                "spatial inventory accounting does not cover every collision",
            ));
        }

        let mut load_triggers = BTreeMap::new();
        for trigger in &inventory.load_triggers {
            if !inventory.collisions.iter().any(|collision| {
                collision.room == trigger.room
                    && collision.prism.authored.stable_id == trigger.collision_id
            }) {
                return Err(WorldSpatialError::new(format!(
                    "load trigger {:?} references an unknown room/collision pair",
                    trigger.stable_id
                )));
            }
            if load_triggers
                .insert((trigger.room, trigger.collision_id.as_str()), trigger)
                .is_some()
            {
                return Err(WorldSpatialError::new(format!(
                    "duplicate load trigger room/collision pair {}/{:?}",
                    trigger.room, trigger.collision_id
                )));
            }
        }

        let artifact = WorldSpatialArtifact {
            schema: WORLD_SPATIAL_INDEX_SCHEMA,
            inventory_sha256,
            algorithm: SpatialIndexAlgorithm {
                id: "stable-median-aabb-bvh/v1",
                leaf_capacity: BVH_LEAF_SIZE,
                scalar_contract: "source-f32/query-f64",
            },
            rooms: room_artifacts,
            excluded,
        };
        let artifact_sha256 = artifact.digest()?;

        Ok(Self {
            inventory,
            inventory_sha256,
            artifact,
            artifact_sha256,
            rooms,
            load_triggers,
        })
    }

    pub fn artifact(&self) -> &WorldSpatialArtifact {
        &self.artifact
    }

    pub fn artifact_digest(&self) -> Result<Digest, WorldSpatialError> {
        Ok(self.artifact_sha256)
    }

    pub fn point_query(
        &self,
        request: WorldPointQueryRequest,
    ) -> Result<WorldPointQueryReport, WorldSpatialError> {
        validate_limit(request.limit)?;
        if !finite_vec3(request.point) {
            return Err(WorldSpatialError::new("query point must be finite"));
        }
        if let Some(max_distance) = request.max_distance
            && (!max_distance.is_finite() || max_distance < 0.0)
        {
            return Err(WorldSpatialError::new(
                "maximum point distance must be finite and nonnegative",
            ));
        }
        let radius_squared = request
            .max_distance
            .map(|distance| f64::from(distance) * f64::from(distance))
            .unwrap_or(f64::INFINITY);
        validate_filter(&request.filter)?;
        let room = self.room(request.filter.room)?;
        let eligible_surface_count = self.eligible_count(room, &request.filter);
        let excluded_matching_filter_count = self.excluded_matching_count(&request.filter);
        let mut bvh_node_visits = 0;
        let mut triangle_tests = 0;
        let mut best = Vec::<(IndexedSurface, PointTriangleQuery)>::new();
        let mut within_distance_count = 0;
        if let Some(max_distance) = request.max_distance {
            let mut candidate_indices = Vec::new();
            if let Some(root) = room.root {
                self.collect_point_candidates(
                    room,
                    root,
                    request.point,
                    radius_squared,
                    &mut bvh_node_visits,
                    &mut candidate_indices,
                );
            }
            for surface_index in candidate_indices {
                let surface = room.surfaces[surface_index];
                if !self.matches_filter(surface, &request.filter) {
                    continue;
                }
                triangle_tests += 1;
                let (plane, triangle) = self.reconstructed(surface);
                let point_query = query_triangle_point(plane, triangle, request.point)?;
                if point_query.distance <= f64::from(max_distance) {
                    within_distance_count += 1;
                    best.push((surface, point_query));
                    self.sort_point_candidates(&mut best);
                    best.truncate(request.limit);
                }
            }
        } else if let Some(root) = room.root {
            // Best-first traversal tightens the remaining lower bound after K
            // exact hits. Equal-bound nodes remain eligible so stable-ID tie
            // ordering cannot be changed by BVH topology.
            let mut pending = BinaryHeap::new();
            pending.push(PendingNode {
                lower_bound: room.nodes[root]
                    .bounds
                    .distance_squared(request.point)
                    .sqrt(),
                node_index: root,
            });
            while let Some(next) = pending.pop() {
                let worst = best
                    .last()
                    .map(|(_, query)| query.distance)
                    .unwrap_or(f64::INFINITY);
                if best.len() == request.limit && next.lower_bound > worst {
                    break;
                }
                bvh_node_visits += 1;
                match room.nodes[next.node_index].kind {
                    BvhNodeKind::Leaf { start, end } => {
                        for surface in &room.surfaces[start..end] {
                            if !self.matches_filter(*surface, &request.filter) {
                                continue;
                            }
                            triangle_tests += 1;
                            let (plane, triangle) = self.reconstructed(*surface);
                            let point_query = query_triangle_point(plane, triangle, request.point)?;
                            best.push((*surface, point_query));
                            self.sort_point_candidates(&mut best);
                            best.truncate(request.limit);
                        }
                    }
                    BvhNodeKind::Branch { left, right } => {
                        for node_index in [left, right] {
                            pending.push(PendingNode {
                                lower_bound: room.nodes[node_index]
                                    .bounds
                                    .distance_squared(request.point)
                                    .sqrt(),
                                node_index,
                            });
                        }
                    }
                }
            }
            within_distance_count = eligible_surface_count;
        }
        let results = best
            .into_iter()
            .map(|(surface, point_query)| SurfacePointHit {
                surface: self.surface_geometry(surface),
                point_query,
            })
            .collect::<Vec<_>>();
        let returned_count = results.len();
        Ok(WorldPointQueryReport {
            schema: WORLD_POINT_QUERY_SCHEMA,
            stage: self.inventory.stage.clone(),
            coordinate_space: "room-kcl-authored/v1",
            inventory_sha256: self.inventory_sha256,
            spatial_index_sha256: self.artifact_sha256,
            request,
            indexed_surface_count: room.surfaces.len(),
            excluded_degenerate_count: room.excluded_degenerate_count,
            excluded_matching_filter_count,
            eligible_surface_count,
            within_distance_count,
            returned_count,
            truncated: within_distance_count > returned_count,
            bvh_node_visits,
            triangle_tests,
            results,
        })
    }

    pub fn aabb_query(
        &self,
        request: WorldAabbQueryRequest,
    ) -> Result<WorldAabbQueryReport, WorldSpatialError> {
        validate_limit(request.limit)?;
        Aabb3::new(request.bounds.min, request.bounds.max)?;
        validate_filter(&request.filter)?;
        let room = self.room(request.filter.room)?;
        let eligible_surface_count = self.eligible_count(room, &request.filter);
        let excluded_matching_filter_count = self.excluded_matching_count(&request.filter);
        let mut candidate_indices = Vec::new();
        let mut bvh_node_visits = 0;
        if let Some(root) = room.root {
            self.collect_aabb_candidates(
                room,
                root,
                request.bounds,
                &mut bvh_node_visits,
                &mut candidate_indices,
            );
        }
        let mut results = candidate_indices
            .into_iter()
            .map(|index| room.surfaces[index])
            .filter(|surface| self.matches_filter(*surface, &request.filter))
            .map(|surface| self.surface_geometry(surface))
            .collect::<Vec<_>>();
        results.sort_by(|left, right| left.authored.stable_id.cmp(&right.authored.stable_id));
        let overlapping_aabb_count = results.len();
        results.truncate(request.limit);
        let returned_count = results.len();
        Ok(WorldAabbQueryReport {
            schema: WORLD_AABB_QUERY_SCHEMA,
            stage: self.inventory.stage.clone(),
            coordinate_space: "room-kcl-authored/v1",
            semantics: "triangle-aabb-overlap-broad-phase",
            inventory_sha256: self.inventory_sha256,
            spatial_index_sha256: self.artifact_sha256,
            request,
            indexed_surface_count: room.surfaces.len(),
            excluded_degenerate_count: room.excluded_degenerate_count,
            excluded_matching_filter_count,
            eligible_surface_count,
            overlapping_aabb_count,
            returned_count,
            truncated: overlapping_aabb_count > returned_count,
            bvh_node_visits,
            results,
        })
    }

    pub fn ray_query(
        &self,
        request: WorldRayQueryRequest,
    ) -> Result<WorldRayQueryReport, WorldSpatialError> {
        validate_limit(request.limit)?;
        if !finite_vec3(request.origin) || !finite_vec3(request.direction) {
            return Err(WorldSpatialError::new(
                "ray origin and direction must be finite",
            ));
        }
        if !request.max_distance.is_finite() || request.max_distance <= 0.0 {
            return Err(WorldSpatialError::new(
                "ray maximum distance must be finite and positive",
            ));
        }
        let length = (f64::from(request.direction.x).powi(2)
            + f64::from(request.direction.y).powi(2)
            + f64::from(request.direction.z).powi(2))
        .sqrt();
        if length <= f64::EPSILON {
            return Err(WorldSpatialError::new("ray direction must be nonzero"));
        }
        let normalized_direction = Vec3 {
            x: (f64::from(request.direction.x) / length) as f32,
            y: (f64::from(request.direction.y) / length) as f32,
            z: (f64::from(request.direction.z) / length) as f32,
        };
        validate_filter(&request.filter)?;
        let room = self.room(request.filter.room)?;
        let eligible_surface_count = self.eligible_count(room, &request.filter);
        let excluded_matching_filter_count = self.excluded_matching_count(&request.filter);
        let mut candidate_indices = Vec::new();
        let mut bvh_node_visits = 0;
        if let Some(root) = room.root {
            self.collect_ray_candidates(
                room,
                root,
                RayTraversal {
                    origin: request.origin,
                    direction: normalized_direction,
                    max_distance: f64::from(request.max_distance),
                },
                &mut bvh_node_visits,
                &mut candidate_indices,
            );
        }
        let mut triangle_tests = 0;
        let mut results = Vec::new();
        for surface_index in candidate_indices {
            let surface = room.surfaces[surface_index];
            if !self.matches_filter(surface, &request.filter) {
                continue;
            }
            triangle_tests += 1;
            let (plane, triangle) = self.reconstructed(surface);
            if let Some((distance, position, barycentric)) = ray_triangle_intersection(
                request.origin,
                normalized_direction,
                triangle,
                f64::from(request.max_distance),
            ) {
                let facing = f64::from(plane.normal.x) * f64::from(normalized_direction.x)
                    + f64::from(plane.normal.y) * f64::from(normalized_direction.y)
                    + f64::from(plane.normal.z) * f64::from(normalized_direction.z);
                results.push(SurfaceRayHit {
                    surface: self.surface_geometry(surface),
                    distance,
                    position,
                    barycentric,
                    front_facing: facing < 0.0,
                });
            }
        }
        results.sort_by(|left, right| {
            left.distance.total_cmp(&right.distance).then_with(|| {
                left.surface
                    .authored
                    .stable_id
                    .cmp(&right.surface.authored.stable_id)
            })
        });
        let hit_count = results.len();
        results.truncate(request.limit);
        let returned_count = results.len();
        Ok(WorldRayQueryReport {
            schema: WORLD_RAY_QUERY_SCHEMA,
            stage: self.inventory.stage.clone(),
            coordinate_space: "room-kcl-authored/v1",
            sidedness: "double-sided",
            inventory_sha256: self.inventory_sha256,
            spatial_index_sha256: self.artifact_sha256,
            request,
            normalized_direction,
            indexed_surface_count: room.surfaces.len(),
            excluded_degenerate_count: room.excluded_degenerate_count,
            excluded_matching_filter_count,
            eligible_surface_count,
            hit_count,
            returned_count,
            truncated: hit_count > returned_count,
            bvh_node_visits,
            triangle_tests,
            results,
        })
    }

    fn room(&self, room: i8) -> Result<&RoomSpatialRuntime, WorldSpatialError> {
        self.rooms.get(&room).ok_or_else(|| {
            WorldSpatialError::new(format!(
                "room {room} has no reconstructed KCL coordinate scope in this inventory"
            ))
        })
    }

    fn eligible_count(&self, room: &RoomSpatialRuntime, filter: &WorldSurfaceFilter) -> usize {
        room.surfaces
            .iter()
            .filter(|surface| self.matches_filter(**surface, filter))
            .count()
    }

    fn matches_filter(&self, surface: IndexedSurface, filter: &WorldSurfaceFilter) -> bool {
        let collision = &self.inventory.collisions[surface.collision_index];
        self.matches_collision_filter(collision, filter)
    }

    fn excluded_matching_count(&self, filter: &WorldSurfaceFilter) -> usize {
        self.inventory
            .collisions
            .iter()
            .filter(|collision| {
                matches!(
                    collision.prism.reconstruction,
                    KclReconstruction::Degenerate { .. }
                ) && self.matches_collision_filter(collision, filter)
            })
            .count()
    }

    fn matches_collision_filter(
        &self,
        collision: &crate::world_inventory::CollisionInventoryRecord,
        filter: &WorldSurfaceFilter,
    ) -> bool {
        if collision.room != filter.room {
            return false;
        }
        let trigger = self
            .load_triggers
            .get(&(collision.room, collision.prism.authored.stable_id.as_str()))
            .copied();
        let requires_trigger = filter.load_triggers_only
            || filter.trigger_stable_id.is_some()
            || filter.destination_stage.is_some()
            || filter.destination_room.is_some()
            || filter.destination_point.is_some();
        if requires_trigger && trigger.is_none() {
            return false;
        }
        trigger.is_none_or(|trigger| {
            filter
                .trigger_stable_id
                .as_ref()
                .is_none_or(|id| trigger.stable_id == *id)
                && filter
                    .destination_stage
                    .as_ref()
                    .is_none_or(|stage| trigger.destination_stage == *stage)
                && filter
                    .destination_room
                    .is_none_or(|room| trigger.destination_room == room)
                && filter
                    .destination_point
                    .is_none_or(|point| trigger.destination_point == point)
        })
    }

    fn reconstructed(&self, surface: IndexedSurface) -> (CollisionPlane, [Vec3; 3]) {
        match self.inventory.collisions[surface.collision_index]
            .prism
            .reconstruction
        {
            KclReconstruction::Reconstructed { plane, triangle } => (plane, triangle),
            KclReconstruction::Degenerate { .. } => {
                unreachable!("degenerate prisms are excluded while building the index")
            }
        }
    }

    fn surface_geometry(&self, surface: IndexedSurface) -> SurfaceGeometry {
        let collision = &self.inventory.collisions[surface.collision_index];
        let (plane, triangle) = self.reconstructed(surface);
        SurfaceGeometry {
            room: collision.room,
            authored: collision.prism.authored.clone(),
            plane,
            triangle,
            load_trigger: self
                .load_triggers
                .get(&(collision.room, collision.prism.authored.stable_id.as_str()))
                .map(|trigger| (*trigger).clone()),
        }
    }

    fn sort_point_candidates(&self, candidates: &mut [(IndexedSurface, PointTriangleQuery)]) {
        candidates.sort_by(|(left_surface, left_query), (right_surface, right_query)| {
            left_query
                .distance
                .total_cmp(&right_query.distance)
                .then_with(|| {
                    self.inventory.collisions[left_surface.collision_index]
                        .prism
                        .authored
                        .stable_id
                        .cmp(
                            &self.inventory.collisions[right_surface.collision_index]
                                .prism
                                .authored
                                .stable_id,
                        )
                })
        });
    }

    fn collect_point_candidates(
        &self,
        room: &RoomSpatialRuntime,
        node_index: usize,
        point: Vec3,
        radius_squared: f64,
        node_visits: &mut usize,
        output: &mut Vec<usize>,
    ) {
        *node_visits += 1;
        let node = room.nodes[node_index];
        if node.bounds.distance_squared(point) > radius_squared {
            return;
        }
        match node.kind {
            BvhNodeKind::Leaf { start, end } => output.extend(start..end),
            BvhNodeKind::Branch { left, right } => {
                self.collect_point_candidates(
                    room,
                    left,
                    point,
                    radius_squared,
                    node_visits,
                    output,
                );
                self.collect_point_candidates(
                    room,
                    right,
                    point,
                    radius_squared,
                    node_visits,
                    output,
                );
            }
        }
    }

    fn collect_aabb_candidates(
        &self,
        room: &RoomSpatialRuntime,
        node_index: usize,
        bounds: Aabb3,
        node_visits: &mut usize,
        output: &mut Vec<usize>,
    ) {
        *node_visits += 1;
        let node = room.nodes[node_index];
        if !node.bounds.overlaps(bounds) {
            return;
        }
        match node.kind {
            BvhNodeKind::Leaf { start, end } => {
                output.extend(
                    (start..end).filter(|index| room.surfaces[*index].bounds.overlaps(bounds)),
                );
            }
            BvhNodeKind::Branch { left, right } => {
                self.collect_aabb_candidates(room, left, bounds, node_visits, output);
                self.collect_aabb_candidates(room, right, bounds, node_visits, output);
            }
        }
    }

    fn collect_ray_candidates(
        &self,
        room: &RoomSpatialRuntime,
        node_index: usize,
        ray: RayTraversal,
        node_visits: &mut usize,
        output: &mut Vec<usize>,
    ) {
        *node_visits += 1;
        let node = room.nodes[node_index];
        if !node
            .bounds
            .ray_intersects(ray.origin, ray.direction, ray.max_distance)
        {
            return;
        }
        match node.kind {
            BvhNodeKind::Leaf { start, end } => {
                output.extend((start..end).filter(|index| {
                    room.surfaces[*index].bounds.ray_intersects(
                        ray.origin,
                        ray.direction,
                        ray.max_distance,
                    )
                }));
            }
            BvhNodeKind::Branch { left, right } => {
                self.collect_ray_candidates(room, left, ray, node_visits, output);
                self.collect_ray_candidates(room, right, ray, node_visits, output);
            }
        }
    }
}

fn build_bvh(surfaces: &mut [IndexedSurface], base: usize, nodes: &mut Vec<BvhNode>) -> usize {
    let bounds = surfaces
        .iter()
        .map(|surface| surface.bounds)
        .reduce(Aabb3::union)
        .expect("BVH nodes are never built from empty slices");
    let index = nodes.len();
    nodes.push(BvhNode {
        bounds,
        kind: BvhNodeKind::Leaf {
            start: base,
            end: base + surfaces.len(),
        },
    });
    if surfaces.len() <= BVH_LEAF_SIZE {
        return index;
    }

    let centroid_bounds = surfaces
        .iter()
        .map(|surface| Aabb3 {
            min: surface.centroid,
            max: surface.centroid,
        })
        .reduce(Aabb3::union)
        .expect("nonempty surface slice");
    let extent = centroid_bounds.extent();
    let axis = if extent.x >= extent.y && extent.x >= extent.z {
        0
    } else if extent.y >= extent.z {
        1
    } else {
        2
    };
    surfaces.sort_by(|left, right| {
        component(left.centroid, axis)
            .total_cmp(&component(right.centroid, axis))
            .then_with(|| left.primitive_index.cmp(&right.primitive_index))
    });
    let midpoint = surfaces.len() / 2;
    let (left_surfaces, right_surfaces) = surfaces.split_at_mut(midpoint);
    let left = build_bvh(left_surfaces, base, nodes);
    let right = build_bvh(right_surfaces, base + midpoint, nodes);
    nodes[index] = BvhNode {
        bounds,
        kind: BvhNodeKind::Branch { left, right },
    };
    index
}

fn ray_triangle_intersection(
    origin: Vec3,
    direction: Vec3,
    triangle: [Vec3; 3],
    max_distance: f64,
) -> Option<(f64, Vec3, [f32; 3])> {
    let [a, b, c] = triangle.map(Vec3d::from);
    let origin = Vec3d::from(origin);
    let direction = Vec3d::from(direction);
    let edge_1 = b - a;
    let edge_2 = c - a;
    let cross = direction.cross(edge_2);
    let determinant = edge_1.dot(cross);
    let scale = edge_1.length().max(edge_2.length()).max(1.0);
    if determinant.abs() <= 64.0 * f64::EPSILON * scale * scale {
        return None;
    }
    let inverse = 1.0 / determinant;
    let from_a = origin - a;
    let u = from_a.dot(cross) * inverse;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = from_a.cross(edge_1);
    let v = direction.dot(q) * inverse;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let distance = edge_2.dot(q) * inverse;
    if distance < 0.0 || distance > max_distance {
        return None;
    }
    let position = origin + direction * distance;
    let barycentric = [(1.0 - u - v) as f32, u as f32, v as f32];
    Some((distance, position.to_vec3(), barycentric))
}

#[derive(Clone, Copy)]
struct Vec3d {
    x: f64,
    y: f64,
    z: f64,
}

impl Vec3d {
    fn dot(self, rhs: Self) -> f64 {
        self.x * rhs.x + self.y * rhs.y + self.z * rhs.z
    }

    fn cross(self, rhs: Self) -> Self {
        Self {
            x: self.y * rhs.z - self.z * rhs.y,
            y: self.z * rhs.x - self.x * rhs.z,
            z: self.x * rhs.y - self.y * rhs.x,
        }
    }

    fn length(self) -> f64 {
        self.dot(self).sqrt()
    }

    fn to_vec3(self) -> Vec3 {
        Vec3 {
            x: self.x as f32,
            y: self.y as f32,
            z: self.z as f32,
        }
    }
}

impl From<Vec3> for Vec3d {
    fn from(value: Vec3) -> Self {
        Self {
            x: f64::from(value.x),
            y: f64::from(value.y),
            z: f64::from(value.z),
        }
    }
}

impl std::ops::Add for Vec3d {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
            z: self.z + rhs.z,
        }
    }
}

impl std::ops::Sub for Vec3d {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
            z: self.z - rhs.z,
        }
    }
}

impl std::ops::Mul<f64> for Vec3d {
    type Output = Self;

    fn mul(self, rhs: f64) -> Self::Output {
        Self {
            x: self.x * rhs,
            y: self.y * rhs,
            z: self.z * rhs,
        }
    }
}

fn validate_limit(limit: usize) -> Result<(), WorldSpatialError> {
    if !(1..=MAX_WORLD_QUERY_RESULTS).contains(&limit) {
        return Err(WorldSpatialError::new(format!(
            "world query limit must be within 1..={MAX_WORLD_QUERY_RESULTS}"
        )));
    }
    Ok(())
}

fn validate_filter(filter: &WorldSurfaceFilter) -> Result<(), WorldSpatialError> {
    if filter
        .trigger_stable_id
        .as_ref()
        .is_some_and(|value| value.is_empty() || value.len() > 512)
    {
        return Err(WorldSpatialError::new(
            "trigger stable-ID filter must be nonempty and at most 512 bytes",
        ));
    }
    if filter.destination_stage.as_ref().is_some_and(|stage| {
        stage.is_empty()
            || stage.len() > 8
            || !stage.as_bytes().iter().all(|byte| byte.is_ascii_graphic())
    }) {
        return Err(WorldSpatialError::new(
            "destination-stage filter must be 1..=8 printable ASCII bytes",
        ));
    }
    Ok(())
}

fn finite_vec3(value: Vec3) -> bool {
    value.x.is_finite() && value.y.is_finite() && value.z.is_finite()
}

fn component(value: Vec3, axis: usize) -> f32 {
    match axis {
        0 => value.x,
        1 => value.y,
        2 => value.z,
        _ => unreachable!("BVH axis is always 0..=2"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world_geometry::{CollisionCode, KclInventoryPrism, KclSourceIndices};
    use crate::world_inventory::{CollisionInventoryRecord, SourceKind, SourceScope, WorldSource};

    fn authored(id: &str, index: u16) -> KclAuthoredPrism {
        KclAuthoredPrism {
            stable_id: id.into(),
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
            code: CollisionCode {
                raw: [0; 5],
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
        }
    }

    fn triangle_at(x: f32) -> (CollisionPlane, [Vec3; 3]) {
        let anchor = Vec3 { x, y: 0.0, z: 0.0 };
        (
            CollisionPlane {
                anchor,
                normal: Vec3 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                },
                d: 0.0,
            },
            [
                anchor,
                Vec3 { x, y: 0.0, z: 1.0 },
                Vec3 {
                    x: x + 1.0,
                    y: 0.0,
                    z: 0.0,
                },
            ],
        )
    }

    fn inventory() -> WorldInventory {
        let mut collisions = Vec::new();
        for (index, x) in [0.0_f32, 10.0, 20.0].into_iter().enumerate() {
            let (plane, triangle) = triangle_at(x);
            collisions.push(CollisionInventoryRecord {
                room: if index == 2 { 1 } else { 0 },
                prism: KclInventoryPrism {
                    authored: authored(&format!("surface/{index}"), index as u16 + 1),
                    reconstruction: KclReconstruction::Reconstructed { plane, triangle },
                },
            });
        }
        collisions.push(CollisionInventoryRecord {
            room: 0,
            prism: KclInventoryPrism {
                authored: authored("surface/degenerate", 4),
                reconstruction: KclReconstruction::Degenerate {
                    reason: "synthetic".into(),
                },
            },
        });
        let trigger = CollisionLoadTrigger {
            stable_id: "trigger/0".into(),
            room: 0,
            collision_id: "surface/1".into(),
            collision_exit_id: 0,
            scls_id: "exit/0".into(),
            destination_stage: "NEXT".into(),
            destination_room: 0,
            destination_layer: -1,
            destination_point: 0,
            inferred_semantics: true,
        };
        WorldInventory {
            schema: WORLD_INVENTORY_SCHEMA.into(),
            stage: "TEST".into(),
            sources: vec![WorldSource {
                scope: SourceScope {
                    kind: SourceKind::Room,
                    room: Some(0),
                },
                archive_sha256: Digest([0; 32]),
                stage_data_path: "room.dzr".into(),
                stage_data_sha256: Digest([1; 32]),
                kcl_path: Some("room.kcl".into()),
                kcl_sha256: Some(Digest([2; 32])),
                plc_path: Some("room.plc".into()),
                plc_sha256: Some(Digest([3; 32])),
                addressable_prisms: 4,
            }],
            chunks: Vec::new(),
            placements: Vec::new(),
            player_spawns: Vec::new(),
            exits: Vec::new(),
            collisions,
            load_triggers: vec![trigger],
        }
    }

    fn filter(room: i8) -> WorldSurfaceFilter {
        WorldSurfaceFilter {
            room,
            load_triggers_only: false,
            trigger_stable_id: None,
            destination_stage: None,
            destination_room: None,
            destination_point: None,
        }
    }

    #[test]
    fn point_queries_are_ranked_bounded_filtered_and_explicit_about_degeneracy() {
        let inventory = inventory();
        let index = WorldSpatialIndex::build(&inventory).unwrap();
        let second = WorldSpatialIndex::build(&inventory).unwrap();
        assert_eq!(
            index.artifact().canonical_bytes().unwrap(),
            second.artifact().canonical_bytes().unwrap()
        );
        assert_eq!(
            index.artifact_digest().unwrap(),
            second.artifact_digest().unwrap()
        );
        assert_eq!(index.artifact().rooms.len(), 2);
        assert_eq!(index.artifact().excluded.len(), 1);
        assert_eq!(index.artifact().rooms[0].root, Some(0));
        assert!(index.artifact().rooms[0].primitive_ids.is_sorted());
        assert_eq!(
            index
                .artifact()
                .rooms
                .iter()
                .map(|room| room.primitive_ids.len())
                .sum::<usize>()
                + index.artifact().excluded.len(),
            inventory.collisions.len()
        );
        let report = index
            .point_query(WorldPointQueryRequest {
                point: Vec3 {
                    x: 0.25,
                    y: 2.0,
                    z: 0.25,
                },
                max_distance: None,
                limit: 1,
                filter: filter(0),
            })
            .unwrap();
        assert_eq!(report.indexed_surface_count, 2);
        assert_eq!(report.excluded_degenerate_count, 1);
        assert_eq!(report.within_distance_count, 2);
        assert_eq!(report.returned_count, 1);
        assert!(report.truncated);
        assert_eq!(report.results[0].surface.authored.stable_id, "surface/0");
        assert!((report.results[0].point_query.distance - 2.0).abs() < 1e-6);

        let load_only = index
            .point_query(WorldPointQueryRequest {
                point: Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                max_distance: Some(11.0),
                limit: 4,
                filter: WorldSurfaceFilter {
                    room: 0,
                    load_triggers_only: true,
                    trigger_stable_id: None,
                    destination_stage: None,
                    destination_room: None,
                    destination_point: None,
                },
            })
            .unwrap();
        assert_eq!(load_only.eligible_surface_count, 1);
        assert_eq!(load_only.results[0].surface.authored.stable_id, "surface/1");
        assert!(load_only.results[0].surface.load_trigger.is_some());

        let exact_destination = index
            .point_query(WorldPointQueryRequest {
                point: Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                max_distance: None,
                limit: 1,
                filter: WorldSurfaceFilter {
                    destination_stage: Some("NEXT".into()),
                    ..filter(0)
                },
            })
            .unwrap();
        assert_eq!(exact_destination.eligible_surface_count, 1);
        assert_eq!(
            exact_destination.results[0].surface.authored.stable_id,
            "surface/1"
        );
    }

    #[test]
    fn aabb_and_double_sided_ray_queries_use_exact_bounded_results() {
        let inventory = inventory();
        let index = WorldSpatialIndex::build(&inventory).unwrap();
        let area = index
            .aabb_query(WorldAabbQueryRequest {
                bounds: Aabb3::new(
                    Vec3 {
                        x: -1.0,
                        y: -1.0,
                        z: -1.0,
                    },
                    Vec3 {
                        x: 11.0,
                        y: 1.0,
                        z: 2.0,
                    },
                )
                .unwrap(),
                limit: 8,
                filter: filter(0),
            })
            .unwrap();
        assert_eq!(area.overlapping_aabb_count, 2);

        let ray = index
            .ray_query(WorldRayQueryRequest {
                origin: Vec3 {
                    x: 0.25,
                    y: 3.0,
                    z: 0.25,
                },
                direction: Vec3 {
                    x: 0.0,
                    y: -2.0,
                    z: 0.0,
                },
                max_distance: 10.0,
                limit: 4,
                filter: filter(0),
            })
            .unwrap();
        assert_eq!(ray.hit_count, 1);
        assert_eq!(ray.results[0].surface.authored.stable_id, "surface/0");
        assert!((ray.results[0].distance - 3.0).abs() < 1e-6);
        assert!(ray.results[0].front_facing);
        assert!((ray.results[0].barycentric.iter().sum::<f32>() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn query_validation_rejects_ambiguous_or_unbounded_requests() {
        assert!(
            Aabb3::new(
                Vec3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0
                },
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0
                }
            )
            .is_err()
        );
        let inventory = inventory();
        let index = WorldSpatialIndex::build(&inventory).unwrap();
        assert!(
            index
                .point_query(WorldPointQueryRequest {
                    point: Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0
                    },
                    max_distance: None,
                    limit: 0,
                    filter: filter(0),
                })
                .is_err()
        );
        assert!(
            index
                .ray_query(WorldRayQueryRequest {
                    origin: Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0
                    },
                    direction: Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0
                    },
                    max_distance: 1.0,
                    limit: 1,
                    filter: filter(0),
                })
                .is_err()
        );
    }

    #[test]
    fn bvh_nearest_matches_brute_force_and_ignores_source_enumeration_order() {
        let mut fixture = inventory();
        fixture.collisions.clear();
        fixture.load_triggers.clear();
        for index in 0..96_u16 {
            let x = f32::from(index % 12) * 37.0 - 180.0;
            let z = f32::from(index / 12) * 41.0 - 140.0;
            let y = f32::from(index % 7) * 3.0;
            let anchor = Vec3 { x, y, z };
            fixture.collisions.push(CollisionInventoryRecord {
                room: 0,
                prism: KclInventoryPrism {
                    authored: authored(&format!("surface/{index:03}"), index + 1),
                    reconstruction: KclReconstruction::Reconstructed {
                        plane: CollisionPlane {
                            anchor,
                            normal: Vec3 {
                                x: 0.0,
                                y: 1.0,
                                z: 0.0,
                            },
                            d: -y,
                        },
                        triangle: [
                            anchor,
                            Vec3 { x: x + 21.0, y, z },
                            Vec3 { x, y, z: z + 19.0 },
                        ],
                    },
                },
            });
        }
        fixture.sources[0].addressable_prisms = fixture.collisions.len();
        let index = WorldSpatialIndex::build(&fixture).unwrap();

        let mut seed = 0x1234_5678_u32;
        for sample in 0..200 {
            let mut next = || {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 8) as f32 / (u32::MAX >> 8) as f32
            };
            let point = Vec3 {
                x: next() * 500.0 - 250.0,
                y: next() * 120.0 - 30.0,
                z: next() * 450.0 - 200.0,
            };
            let limit = [1, 3, 8][sample % 3];
            let report = index
                .point_query(WorldPointQueryRequest {
                    point,
                    max_distance: None,
                    limit,
                    filter: filter(0),
                })
                .unwrap();

            let mut brute = fixture
                .collisions
                .iter()
                .map(|collision| {
                    let KclReconstruction::Reconstructed { plane, triangle } =
                        collision.prism.reconstruction
                    else {
                        unreachable!()
                    };
                    (
                        collision.prism.authored.stable_id.as_str(),
                        query_triangle_point(plane, triangle, point)
                            .unwrap()
                            .distance,
                    )
                })
                .collect::<Vec<_>>();
            brute.sort_by(|left, right| {
                left.1.total_cmp(&right.1).then_with(|| left.0.cmp(right.0))
            });
            assert_eq!(report.results.len(), limit);
            for (result, expected) in report.results.iter().zip(&brute[..limit]) {
                assert_eq!(result.surface.authored.stable_id, expected.0);
                assert_eq!(result.point_query.distance.to_bits(), expected.1.to_bits());
            }
        }

        let mut reversed = fixture.clone();
        reversed.collisions.reverse();
        let reversed_index = WorldSpatialIndex::build(&reversed).unwrap();
        assert_eq!(index.artifact().rooms, reversed_index.artifact().rooms);
        assert_eq!(
            index.artifact().excluded,
            reversed_index.artifact().excluded
        );
    }

    #[test]
    fn real_f_sp103_spatial_goldens_match_when_disc_is_present() {
        let stage_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("orig/GZ2E01/files/res/Stage/F_SP103");
        if !stage_dir.is_dir() {
            eprintln!("skipping F_SP103 spatial golden: original disc data is absent");
            return;
        }
        let inventory = WorldInventory::build(&stage_dir, "F_SP103").unwrap();
        let index = WorldSpatialIndex::build(&inventory).unwrap();
        assert_eq!(
            index.artifact_digest().unwrap().to_string(),
            "2ad975eee45193b4325bb420a7ba5a78d533bed80cbcfeace29dcc5418e73834"
        );
        let room0 = index
            .artifact()
            .rooms
            .iter()
            .find(|room| room.room == 0)
            .unwrap();
        let room1 = index
            .artifact()
            .rooms
            .iter()
            .find(|room| room.room == 1)
            .unwrap();
        assert_eq!(room0.primitive_ids.len(), 8_566);
        assert_eq!(room1.primitive_ids.len(), 2_224);
        assert_eq!(index.artifact().excluded.len(), 4);

        let load_trigger_accounting = index
            .point_query(WorldPointQueryRequest {
                point: Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                max_distance: None,
                limit: 1,
                filter: WorldSurfaceFilter {
                    load_triggers_only: true,
                    ..filter(1)
                },
            })
            .unwrap();
        assert_eq!(load_trigger_accounting.excluded_matching_filter_count, 1);

        // This point is deliberately closer to an ordinary surface than to
        // the desired exit. Destination filtering must happen before ranking.
        let point = Vec3 {
            x: -1_965.393_3,
            y: 821.341_8,
            z: -4364.942,
        };
        let nearest_any = index
            .point_query(WorldPointQueryRequest {
                point,
                max_distance: None,
                limit: 1,
                filter: filter(1),
            })
            .unwrap();
        assert_eq!(nearest_any.results[0].surface.authored.prism_index, 2187);
        assert!(nearest_any.triangle_tests < nearest_any.eligible_surface_count);

        let nearest_destination = index
            .point_query(WorldPointQueryRequest {
                point,
                max_distance: None,
                limit: 1,
                filter: WorldSurfaceFilter {
                    destination_stage: Some("F_SP104".into()),
                    ..filter(1)
                },
            })
            .unwrap();
        assert_eq!(
            nearest_destination.results[0].surface.authored.prism_index,
            2217
        );
        assert!((nearest_destination.results[0].point_query.distance - 100.0).abs() < 1.0e-3);

        let live_point = index
            .point_query(WorldPointQueryRequest {
                point: Vec3 {
                    x: -2_037.332_4,
                    y: 729.72,
                    z: -4264.551,
                },
                max_distance: Some(0.001),
                limit: 4,
                filter: WorldSurfaceFilter {
                    destination_stage: Some("F_SP104".into()),
                    ..filter(1)
                },
            })
            .unwrap();
        assert_eq!(live_point.results[0].surface.authored.prism_index, 2217);
        assert!(live_point.results[0].point_query.distance < 1.0e-3);

        let ray = index
            .ray_query(WorldRayQueryRequest {
                origin: Vec3 {
                    x: -1_970.221_8,
                    y: 771.584_2,
                    z: -4364.008,
                },
                direction: Vec3 {
                    x: -0.096_570_1,
                    y: -0.995_150_86,
                    z: 0.018_679_99,
                },
                max_distance: 60.0,
                limit: 4,
                filter: WorldSurfaceFilter {
                    destination_stage: Some("F_SP104".into()),
                    ..filter(1)
                },
            })
            .unwrap();
        assert_eq!(ray.results[0].surface.authored.prism_index, 2217);
        assert!((ray.results[0].distance - 50.0).abs() < 1.0e-3);
        assert!(ray.results[0].front_facing);
    }
}
