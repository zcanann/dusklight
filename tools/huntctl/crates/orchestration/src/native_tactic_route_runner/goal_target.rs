use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct GoalTransitionTarget {
    source_stage: String,
    source_room: i8,
    destination_stage: String,
    destination_room: i8,
    destination_point: i16,
    coordinate: [f32; 3],
    supporting_load_triggers: usize,
    source_inventory_sha256: Digest,
}

pub(crate) struct GoalConditionedTacticRuntime {
    pub catalog: dusklight_learning::tactic_asset::TacticAssetCatalog,
    pub encoder: GoalConditionedTacticFeatureEncoder,
    pub report: NativeTacticGoalTargetReport,
}

pub(crate) struct GoalConditionedTacticContext {
    pub encoder: GoalConditionedTacticFeatureEncoder,
    pub report: NativeTacticGoalTargetReport,
}

pub(super) fn parameterized_policy_action_schema_sha256() -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight-parameterized-policy-action-schema/v2");
    hasher.update(parameterized_tactic_family_schema_sha256().0);
    Digest(hasher.finalize().into())
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticGoalTargetReport {
    pub source_stage: String,
    pub source_room: i8,
    pub destination_stage: String,
    pub destination_room: i8,
    pub destination_point: i16,
    pub coordinate: [f32; 3],
    pub source_coordinate: [f32; 3],
    pub tactic_targets: Vec<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub route_sequences: Vec<Vec<[f32; 3]>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authored_route_ids: Vec<String>,
    pub supporting_load_triggers: usize,
    pub source_inventory_sha256: Digest,
    pub authored_route_coordinates_used: bool,
}

impl GoalTransitionTarget {
    fn report(
        &self,
        source_coordinate: [f32; 3],
        tactic_targets: Vec<[f32; 3]>,
        route_sequences: Vec<Vec<[f32; 3]>>,
        authored_route_ids: Vec<String>,
    ) -> NativeTacticGoalTargetReport {
        let authored_route_coordinates_used = !authored_route_ids.is_empty();
        NativeTacticGoalTargetReport {
            source_stage: self.source_stage.clone(),
            source_room: self.source_room,
            destination_stage: self.destination_stage.clone(),
            destination_room: self.destination_room,
            destination_point: self.destination_point,
            coordinate: self.coordinate,
            source_coordinate,
            tactic_targets,
            route_sequences,
            authored_route_ids,
            supporting_load_triggers: self.supporting_load_triggers,
            source_inventory_sha256: self.source_inventory_sha256,
            authored_route_coordinates_used,
        }
    }
}

pub(crate) fn goal_conditioned_tactic_runtime(
    root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    initial_facts: &FactSnapshot,
) -> Result<GoalConditionedTacticRuntime, NativeTacticRouteRunError> {
    let context = goal_conditioned_tactic_context(root, optimization, execution, initial_facts)?;
    let maximum_ticks = goal_tactic_maximum_ticks(optimization.budgets.exploration_horizon_ticks)?;
    let route_sequence_maximum_ticks =
        goal_route_sequence_maximum_ticks(optimization.budgets.exploration_horizon_ticks)?;
    let catalog =
        dusklight_learning::default_tactic_catalog::goal_conditioned_route_tactic_catalog(
            &context.report.tactic_targets,
            &context.report.route_sequences,
            maximum_ticks,
            route_sequence_maximum_ticks,
        )
        .map_err(route_error)?;
    Ok(GoalConditionedTacticRuntime {
        catalog,
        encoder: context.encoder,
        report: context.report,
    })
}

pub(super) fn goal_conditioned_tactic_context(
    root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    initial_facts: &FactSnapshot,
) -> Result<GoalConditionedTacticContext, NativeTacticRouteRunError> {
    let source_coordinate = initial_facts.player.position_f32_bits.map(f32::from_bits);
    let (target, inventory) =
        resolve_goal_transition_target(root, optimization, execution, source_coordinate)?;
    if initial_facts.world.stage != target.source_stage
        || initial_facts.world.room != target.source_room
    {
        return Err(route_message(
            "native source observation differs from the objective's source world",
        ));
    }
    let (tactic_targets, route_sequences, authored_route_ids) = goal_route_targets(
        source_coordinate,
        target.coordinate,
        target.source_room,
        &inventory,
    )?;
    let encoder =
        GoalConditionedTacticFeatureEncoder::new(target.coordinate).map_err(route_error)?;
    Ok(GoalConditionedTacticContext {
        encoder,
        report: target.report(
            source_coordinate,
            tactic_targets,
            route_sequences,
            authored_route_ids,
        ),
    })
}

pub(super) fn atomic_goal_conditioned_tactic_context(
    root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    initial_facts: &FactSnapshot,
) -> Result<GoalConditionedTacticContext, NativeTacticRouteRunError> {
    let source_coordinate = initial_facts.player.position_f32_bits.map(f32::from_bits);
    let (target, _) =
        resolve_goal_transition_target(root, optimization, execution, source_coordinate)?;
    if initial_facts.world.stage != target.source_stage
        || initial_facts.world.room != target.source_room
    {
        return Err(route_message(
            "native source observation differs from the objective's source world",
        ));
    }
    let encoder =
        GoalConditionedTacticFeatureEncoder::new(target.coordinate).map_err(route_error)?;
    Ok(GoalConditionedTacticContext {
        encoder,
        report: target.report(
            source_coordinate,
            vec![target.coordinate],
            Vec::new(),
            Vec::new(),
        ),
    })
}

pub(super) fn goal_corridor_targets(
    source: [f32; 3],
    goal: [f32; 3],
) -> Result<(Vec<[f32; 3]>, Vec<Vec<[f32; 3]>>), NativeTacticRouteRunError> {
    if source
        .iter()
        .chain(goal.iter())
        .any(|value| !value.is_finite())
    {
        return Err(route_message(
            "goal corridor requires finite source and target coordinates",
        ));
    }
    let dx = goal[0] - source[0];
    let dz = goal[2] - source[2];
    let distance = dx.hypot(dz);
    if distance <= 0.0 || !distance.is_finite() {
        return Err(route_message(
            "goal corridor requires distinct source and target coordinates",
        ));
    }
    let perpendicular = [-dz / distance, dx / distance];
    let mut targets = vec![goal];
    let mut identities = BTreeSet::from([goal.map(f32::to_bits)]);
    let offsets = [-768.0_f32, -384.0, 0.0, 384.0, 768.0];
    let mut route_sequences = vec![Vec::new(); offsets.len()];
    for fraction in [0.25_f32, 0.5, 0.75, 1.0] {
        let center = [
            source[0] + dx * fraction,
            source[1] + (goal[1] - source[1]) * fraction,
            source[2] + dz * fraction,
        ];
        for (lane, offset) in offsets.iter().copied().enumerate() {
            let target = [
                center[0] + perpendicular[0] * offset,
                center[1],
                center[2] + perpendicular[1] * offset,
            ];
            if identities.insert(target.map(f32::to_bits)) {
                targets.push(target);
            }
            if fraction < 1.0 {
                route_sequences[lane].push(target);
            }
        }
    }
    for route in &mut route_sequences {
        route.push(goal);
    }
    Ok((targets, route_sequences))
}

#[derive(Clone, Debug)]
pub(super) struct NavigableSurfaceNode {
    pub(super) collision_id: String,
    pub(super) coordinate: [f32; 3],
}

#[derive(Clone, Debug)]
pub(super) struct NavigableSurfaceEdge {
    pub(super) left_collision_id: String,
    pub(super) right_collision_id: String,
    pub(super) shared_edge: [[f32; 3]; 2],
}

#[derive(Clone, Copy, Debug)]
pub(super) struct NavigableSurfaceFrontier {
    distance: f32,
    node: usize,
}

impl PartialEq for NavigableSurfaceFrontier {
    fn eq(&self, other: &Self) -> bool {
        self.distance.to_bits() == other.distance.to_bits() && self.node == other.node
    }
}

impl Eq for NavigableSurfaceFrontier {}

impl PartialOrd for NavigableSurfaceFrontier {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for NavigableSurfaceFrontier {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // BinaryHeap is a max-heap; reverse distance ordering for a min frontier.
        other
            .distance
            .total_cmp(&self.distance)
            .then_with(|| other.node.cmp(&self.node))
    }
}

pub(super) fn goal_surface_routes(
    source: [f32; 3],
    goal: [f32; 3],
    room: i8,
    inventory: &WorldInventory,
) -> Result<Option<Vec<(String, Vec<[f32; 3]>)>>, NativeTacticRouteRunError> {
    if inventory.collisions.is_empty() {
        return Ok(None);
    }
    let graph = WorldSurfaceGraph::build(inventory).map_err(route_error)?;
    let inventory_sha256 = graph.artifact().inventory_sha256;
    let nodes = graph
        .artifact()
        .nodes
        .iter()
        .filter(|node| {
            node.room == room && node.plane_normal.y >= NAVIGABLE_SURFACE_MINIMUM_UP_NORMAL
        })
        .map(|node| NavigableSurfaceNode {
            collision_id: node.collision_id.clone(),
            coordinate: [node.centroid.x, node.centroid.y, node.centroid.z],
        })
        .collect::<Vec<_>>();
    let edges = graph
        .artifact()
        .edges
        .iter()
        .filter(|edge| edge.room == room)
        .map(|edge| NavigableSurfaceEdge {
            left_collision_id: edge.left_collision_id.clone(),
            right_collision_id: edge.right_collision_id.clone(),
            shared_edge: edge.shared_edge.map(|point| [point.x, point.y, point.z]),
        })
        .collect::<Vec<_>>();
    let Some(routes) = shortest_navigable_surface_routes(&nodes, &edges, source, goal)? else {
        return Ok(None);
    };
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight-goal-surface-route/v1");
    hasher.update(inventory_sha256.0);
    hasher.update(room.to_le_bytes());
    for route in &routes {
        hasher.update((route.len() as u64).to_le_bytes());
        for coordinate in route {
            for component in coordinate {
                hasher.update(component.to_bits().to_le_bytes());
            }
        }
    }
    let digest = hasher.finalize();
    let identity = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(Some(
        routes
            .into_iter()
            .map(|route| {
                (
                    format!("surface-graph:{identity}:resolution.{:02}", route.len()),
                    route,
                )
            })
            .collect(),
    ))
}

pub(super) fn shortest_navigable_surface_routes(
    nodes: &[NavigableSurfaceNode],
    edges: &[NavigableSurfaceEdge],
    source: [f32; 3],
    goal: [f32; 3],
) -> Result<Option<Vec<Vec<[f32; 3]>>>, NativeTacticRouteRunError> {
    if nodes.is_empty()
        || source
            .iter()
            .chain(goal.iter())
            .any(|value| !value.is_finite())
    {
        return Ok(None);
    }
    let nearest = |target: [f32; 3]| {
        nodes
            .iter()
            .enumerate()
            .map(|(index, node)| {
                let dx = node.coordinate[0] - target[0];
                let dy = node.coordinate[1] - target[1];
                let dz = node.coordinate[2] - target[2];
                (index, dx.mul_add(dx, dy.mul_add(dy, dz * dz)).sqrt())
            })
            .min_by(|left, right| {
                left.1
                    .total_cmp(&right.1)
                    .then_with(|| nodes[left.0].collision_id.cmp(&nodes[right.0].collision_id))
            })
    };
    let Some((source_index, source_distance)) = nearest(source) else {
        return Ok(None);
    };
    let Some((goal_index, goal_distance)) = nearest(goal) else {
        return Ok(None);
    };
    if source_distance > NAVIGABLE_SURFACE_MAXIMUM_ATTACHMENT_DISTANCE
        || goal_distance > NAVIGABLE_SURFACE_MAXIMUM_ATTACHMENT_DISTANCE
    {
        return Ok(None);
    }

    let indices = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.collision_id.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    let mut adjacency = vec![Vec::new(); nodes.len()];
    for (edge_index, edge) in edges.iter().enumerate() {
        let (Some(&left), Some(&right)) = (
            indices.get(edge.left_collision_id.as_str()),
            indices.get(edge.right_collision_id.as_str()),
        ) else {
            continue;
        };
        adjacency[left].push((right, edge_index));
        adjacency[right].push((left, edge_index));
    }
    for neighbors in &mut adjacency {
        neighbors.sort_unstable_by(|left, right| {
            nodes[left.0]
                .collision_id
                .cmp(&nodes[right.0].collision_id)
                .then_with(|| left.1.cmp(&right.1))
        });
        neighbors.dedup();
    }

    let mut distances = vec![f32::INFINITY; nodes.len()];
    let mut previous = vec![None; nodes.len()];
    distances[source_index] = 0.0;
    previous[source_index] = Some((source_index, usize::MAX));
    let mut pending = BinaryHeap::from([NavigableSurfaceFrontier {
        distance: 0.0,
        node: source_index,
    }]);
    while let Some(frontier) = pending.pop() {
        if frontier.distance > distances[frontier.node] {
            continue;
        }
        if frontier.node == goal_index {
            break;
        }
        for &(neighbor, edge_index) in &adjacency[frontier.node] {
            let edge_distance =
                planar_distance(nodes[frontier.node].coordinate, nodes[neighbor].coordinate);
            let candidate_distance = frontier.distance + edge_distance;
            let improves_distance = candidate_distance < distances[neighbor];
            let improves_tie_break = candidate_distance.to_bits() == distances[neighbor].to_bits()
                && previous[neighbor].is_some_and(|(predecessor, prior_edge)| {
                    (nodes[frontier.node].collision_id.as_str(), edge_index)
                        < (nodes[predecessor].collision_id.as_str(), prior_edge)
                });
            if improves_distance || improves_tie_break {
                distances[neighbor] = candidate_distance;
                previous[neighbor] = Some((frontier.node, edge_index));
                pending.push(NavigableSurfaceFrontier {
                    distance: candidate_distance,
                    node: neighbor,
                });
            }
        }
    }
    if previous[goal_index].is_none() {
        return Ok(None);
    }
    let mut path = vec![goal_index];
    let mut path_edges = Vec::new();
    while *path
        .last()
        .ok_or_else(|| route_message("surface route reconstruction is empty"))?
        != source_index
    {
        let current = *path.last().expect("surface route is nonempty");
        let (predecessor, edge_index) = previous[current]
            .ok_or_else(|| route_message("surface route predecessor is absent"))?;
        path.push(predecessor);
        path_edges.push(edge_index);
    }
    path.reverse();
    path_edges.reverse();
    let portal_centers = path_edges
        .iter()
        .map(|edge_index| {
            let edge = edges[*edge_index].shared_edge;
            [
                (edge[0][0] + edge[1][0]) * 0.5,
                (edge[0][1] + edge[1][1]) * 0.5,
                (edge[0][2] + edge[1][2]) * 0.5,
            ]
        })
        .collect::<Vec<_>>();
    let centerline = std::iter::once(source)
        .chain(portal_centers)
        .chain(std::iter::once(goal))
        .collect::<Vec<_>>();
    let mut routes = vec![funnel_surface_route(
        nodes,
        &path,
        edges,
        &path_edges,
        source,
        goal,
        NAVIGABLE_SURFACE_PORTAL_CLEARANCE,
    )?];
    routes.extend(
        NAVIGABLE_SURFACE_ROUTE_RESOLUTIONS
            .iter()
            .map(|target_count| {
                simplify_planar_surface_route(&centerline, target_count.saturating_add(1))
                    .into_iter()
                    .skip(1)
                    .collect::<Vec<_>>()
            })
            .filter(|route| !route.is_empty() && route.len() <= NAVIGABLE_SURFACE_ROUTE_TARGETS),
    );
    routes.retain(|route| !route.is_empty() && route.len() <= NAVIGABLE_SURFACE_ROUTE_TARGETS);
    routes.dedup();
    if routes.is_empty() {
        Err(route_message(
            "surface route simplification exceeded its bounded target count",
        ))
    } else {
        Ok(Some(routes))
    }
}

pub(super) fn funnel_surface_route(
    nodes: &[NavigableSurfaceNode],
    path: &[usize],
    edges: &[NavigableSurfaceEdge],
    path_edges: &[usize],
    source: [f32; 3],
    goal: [f32; 3],
    clearance: f32,
) -> Result<Vec<[f32; 3]>, NativeTacticRouteRunError> {
    if path.len() != path_edges.len().saturating_add(1)
        || path.iter().any(|index| *index >= nodes.len())
        || path_edges.iter().any(|index| *index >= edges.len())
        || !clearance.is_finite()
        || clearance < 0.0
    {
        return Err(route_message("surface funnel inputs are invalid"));
    }
    let mut portals = Vec::with_capacity(path_edges.len().saturating_add(2));
    portals.push((source, source));
    for (step, edge_index) in path_edges.iter().copied().enumerate() {
        let edge = edges[edge_index].shared_edge;
        let from = nodes[path[step]].coordinate;
        let to = nodes[path[step + 1]].coordinate;
        let first_area = planar_signed_area(from, to, edge[0]);
        let second_area = planar_signed_area(from, to, edge[1]);
        // The funnel predicate below follows the controller's X/Z winding,
        // where the smaller signed area is the left portal bound.
        let (left, right) = if first_area <= second_area {
            (edge[0], edge[1])
        } else {
            (edge[1], edge[0])
        };
        portals.push(inset_portal(left, right, clearance));
    }
    portals.push((goal, goal));

    let mut route = Vec::new();
    let mut apex = portals[0].0;
    let mut left = portals[0].0;
    let mut right = portals[0].1;
    let mut left_index = 0;
    let mut right_index = 0;
    let mut index = 1;
    while index < portals.len() {
        let (next_left, next_right) = portals[index];
        if planar_signed_area(apex, right, next_right) <= 0.0 {
            if same_planar_point(apex, right) || planar_signed_area(apex, left, next_right) > 0.0 {
                right = next_right;
                right_index = index;
            } else {
                push_distinct_planar(&mut route, left);
                apex = left;
                let restart_index = left_index;
                left = apex;
                right = apex;
                left_index = restart_index;
                right_index = restart_index;
                index = restart_index.saturating_add(1);
                continue;
            }
        }
        if planar_signed_area(apex, left, next_left) >= 0.0 {
            if same_planar_point(apex, left) || planar_signed_area(apex, right, next_left) < 0.0 {
                left = next_left;
                left_index = index;
            } else {
                push_distinct_planar(&mut route, right);
                apex = right;
                let restart_index = right_index;
                left = apex;
                right = apex;
                left_index = restart_index;
                right_index = restart_index;
                index = restart_index.saturating_add(1);
                continue;
            }
        }
        index += 1;
    }
    push_distinct_planar(&mut route, goal);
    Ok(route)
}

pub(super) fn inset_portal(
    left: [f32; 3],
    right: [f32; 3],
    clearance: f32,
) -> ([f32; 3], [f32; 3]) {
    let dx = right[0] - left[0];
    let dy = right[1] - left[1];
    let dz = right[2] - left[2];
    let planar_length = dx.hypot(dz);
    if planar_length <= f32::EPSILON {
        return (left, right);
    }
    let fraction = (clearance / planar_length).clamp(0.0, 0.25);
    (
        [
            left[0] + dx * fraction,
            left[1] + dy * fraction,
            left[2] + dz * fraction,
        ],
        [
            right[0] - dx * fraction,
            right[1] - dy * fraction,
            right[2] - dz * fraction,
        ],
    )
}

pub(super) fn planar_signed_area(left: [f32; 3], right: [f32; 3], point: [f32; 3]) -> f32 {
    (right[0] - left[0]) * (point[2] - left[2]) - (right[2] - left[2]) * (point[0] - left[0])
}

pub(super) fn same_planar_point(left: [f32; 3], right: [f32; 3]) -> bool {
    (left[0] - right[0]).abs() <= f32::EPSILON && (left[2] - right[2]).abs() <= f32::EPSILON
}

pub(super) fn push_distinct_planar(route: &mut Vec<[f32; 3]>, point: [f32; 3]) {
    if route
        .last()
        .is_none_or(|previous| !same_planar_point(*previous, point))
    {
        route.push(point);
    }
}

pub(super) fn simplify_planar_surface_route(
    path: &[[f32; 3]],
    maximum_points: usize,
) -> Vec<[f32; 3]> {
    if path.len() <= 2 || maximum_points < 2 {
        return path.to_vec();
    }
    let mut retained = BTreeSet::from([0_usize, path.len() - 1]);
    while retained.len() < maximum_points {
        let indices = retained.iter().copied().collect::<Vec<_>>();
        let mut best = None::<(f32, usize)>;
        for pair in indices.windows(2) {
            for index in pair[0] + 1..pair[1] {
                let error = planar_point_segment_distance_squared(
                    path[index],
                    path[pair[0]],
                    path[pair[1]],
                );
                if best.is_none_or(|(best_error, best_index)| {
                    error
                        .total_cmp(&best_error)
                        .then_with(|| best_index.cmp(&index))
                        .is_gt()
                }) {
                    best = Some((error, index));
                }
            }
        }
        let Some((error, index)) = best else {
            break;
        };
        if error <= f32::EPSILON {
            break;
        }
        retained.insert(index);
    }
    retained.into_iter().map(|index| path[index]).collect()
}

pub(super) fn planar_point_segment_distance_squared(
    point: [f32; 3],
    start: [f32; 3],
    end: [f32; 3],
) -> f32 {
    let dx = end[0] - start[0];
    let dz = end[2] - start[2];
    let length_squared = dx.mul_add(dx, dz * dz);
    if length_squared <= f32::EPSILON {
        let px = point[0] - start[0];
        let pz = point[2] - start[2];
        return px.mul_add(px, pz * pz);
    }
    let projection = (((point[0] - start[0]) * dx + (point[2] - start[2]) * dz) / length_squared)
        .clamp(0.0, 1.0);
    let px = point[0] - start[0] - projection * dx;
    let pz = point[2] - start[2] - projection * dz;
    px.mul_add(px, pz * pz)
}

#[derive(Clone)]
pub(super) struct AuthoredRouteCandidate {
    identity: String,
    coordinates: Vec<[f32; 3]>,
    endpoint_cost: f32,
}

pub(super) fn goal_route_targets(
    source: [f32; 3],
    goal: [f32; 3],
    room: i8,
    inventory: &WorldInventory,
) -> Result<(Vec<[f32; 3]>, Vec<Vec<[f32; 3]>>, Vec<String>), NativeTacticRouteRunError> {
    if source
        .iter()
        .chain(goal.iter())
        .any(|value| !value.is_finite())
    {
        return Err(route_message(
            "goal routes require finite source and target coordinates",
        ));
    }
    let surface_routes = goal_surface_routes(source, goal, room, inventory)?;
    let paths = inventory
        .paths
        .iter()
        .filter(|path| path.scope.room == Some(room))
        .map(|path| ((path.source_sha256, path.record_index), path))
        .collect::<BTreeMap<_, _>>();
    if paths.is_empty() {
        return fallback_goal_routes(source, goal, surface_routes);
    }
    let points = inventory
        .path_points
        .iter()
        .filter(|point| point.scope.room == Some(room))
        .fold(BTreeMap::<_, Vec<_>>::new(), |mut by_source, point| {
            by_source
                .entry(point.source_sha256)
                .or_default()
                .push(point);
            by_source
        });
    let incoming = paths
        .values()
        .filter_map(|path| {
            path.next_path_index
                .map(|next| (path.source_sha256, usize::from(next)))
        })
        .collect::<BTreeSet<_>>();
    let roots = paths
        .keys()
        .filter(|key| !incoming.contains(key))
        .copied()
        .collect::<Vec<_>>();
    let direct_distance = planar_distance(source, goal);
    let attachment_limit = (direct_distance * 0.25).max(512.0);
    let mut candidates = Vec::new();
    for root in roots {
        let mut coordinates = Vec::new();
        let mut identities = Vec::new();
        let mut visited = BTreeSet::new();
        let mut current = Some(root);
        while let Some(key) = current {
            if !visited.insert(key) {
                return Err(route_message("authored path chain contains a cycle"));
            }
            let path = paths
                .get(&key)
                .ok_or_else(|| route_message("authored path chain references an absent path"))?;
            let source_points = points
                .get(&path.source_sha256)
                .ok_or_else(|| route_message("authored path has no point table"))?;
            let end = path
                .first_point_index
                .checked_add(usize::from(path.point_count))
                .ok_or_else(|| route_message("authored path point range overflowed"))?;
            let path_points = source_points
                .get(path.first_point_index..end)
                .ok_or_else(|| route_message("authored path point range is unavailable"))?;
            for point in path_points {
                let coordinate = [point.position.x, point.position.y, point.position.z];
                if coordinates.last() != Some(&coordinate) {
                    coordinates.push(coordinate);
                }
            }
            identities.push(path.stable_id.as_str());
            current = path
                .next_path_index
                .map(|next| (path.source_sha256, usize::from(next)));
        }
        if coordinates.is_empty() || coordinates.len() >= MAX_GOAL_SEEK_TARGETS {
            continue;
        }
        for (orientation, mut oriented) in [
            ("forward", coordinates.clone()),
            ("reverse", {
                let mut reverse = coordinates.clone();
                reverse.reverse();
                reverse
            }),
        ] {
            let first = *oriented.first().expect("nonempty authored path");
            let last = *oriented.last().expect("nonempty authored path");
            let source_cost = planar_distance(source, first);
            let goal_cost = planar_distance(last, goal);
            if source_cost > attachment_limit || goal_cost > attachment_limit {
                continue;
            }
            if last.map(f32::to_bits) != goal.map(f32::to_bits) {
                oriented.push(goal);
            }
            candidates.push(AuthoredRouteCandidate {
                identity: format!("{}:{orientation}", identities.join("+")),
                coordinates: oriented,
                endpoint_cost: source_cost + goal_cost,
            });
        }
    }
    candidates.sort_by(|left, right| {
        left.endpoint_cost
            .total_cmp(&right.endpoint_cost)
            .then_with(|| left.identity.cmp(&right.identity))
    });
    candidates.dedup_by(|left, right| {
        left.coordinates
            .iter()
            .map(|coordinate| coordinate.map(f32::to_bits))
            .eq(right
                .coordinates
                .iter()
                .map(|coordinate| coordinate.map(f32::to_bits)))
    });
    candidates.truncate(5);
    if candidates.is_empty() {
        return fallback_goal_routes(source, goal, surface_routes);
    }

    let mut route_sequences = candidates
        .iter()
        .map(|candidate| candidate.coordinates.clone())
        .collect::<Vec<_>>();
    let mut authored_route_ids = candidates
        .iter()
        .map(|candidate| candidate.identity.clone())
        .collect::<Vec<_>>();
    if let Some(surface_routes) = surface_routes {
        for (identity, route) in surface_routes.into_iter().rev() {
            if !route_sequences.iter().any(|candidate| candidate == &route) {
                route_sequences.insert(0, route);
                authored_route_ids.insert(0, identity);
            }
        }
        route_sequences.truncate(5);
        authored_route_ids.truncate(5);
    }
    let mut targets = vec![goal];
    let mut target_identities = BTreeSet::from([goal.map(f32::to_bits)]);
    for coordinate in route_sequences.iter().flatten().copied() {
        if targets.len() == MAX_GOAL_SEEK_TARGETS {
            break;
        }
        if target_identities.insert(coordinate.map(f32::to_bits)) {
            targets.push(coordinate);
        }
    }
    Ok((targets, route_sequences, authored_route_ids))
}

pub(super) fn fallback_goal_routes(
    source: [f32; 3],
    goal: [f32; 3],
    surface_routes: Option<Vec<(String, Vec<[f32; 3]>)>>,
) -> Result<(Vec<[f32; 3]>, Vec<Vec<[f32; 3]>>, Vec<String>), NativeTacticRouteRunError> {
    let (mut targets, mut routes) = goal_corridor_targets(source, goal)?;
    let mut route_ids = Vec::new();
    if let Some(surface_routes) = surface_routes {
        for (_, route) in &surface_routes {
            for coordinate in route {
                if targets.len() == MAX_GOAL_SEEK_TARGETS {
                    break;
                }
                if !targets
                    .iter()
                    .any(|target| target.map(f32::to_bits) == coordinate.map(f32::to_bits))
                {
                    targets.push(*coordinate);
                }
            }
        }
        for (identity, route) in surface_routes.into_iter().rev() {
            routes.insert(0, route);
            route_ids.insert(0, identity);
        }
        routes.truncate(5);
        route_ids.truncate(5);
    }
    Ok((targets, routes, route_ids))
}

pub(super) fn planar_distance(left: [f32; 3], right: [f32; 3]) -> f32 {
    (right[0] - left[0]).hypot(right[2] - left[2])
}

pub(super) fn resolve_goal_transition_target(
    root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    source_coordinate: [f32; 3],
) -> Result<(GoalTransitionTarget, WorldInventory), NativeTacticRouteRunError> {
    let program_bytes =
        fs::read(root.join(&execution.milestone_program.path)).map_err(route_error)?;
    let decoded =
        dusklight_objectives::milestone_dsl::decode(&program_bytes).map_err(route_error)?;
    let definition = decoded
        .program
        .definitions
        .iter()
        .find(|definition| definition.name == optimization.terminal_predicate.goal)
        .ok_or_else(|| route_message("goal definition is absent from milestone program"))?;
    let source_stage = exact_symbol_literal(&definition.when, Field::StageName)?;
    let source_room = exact_i8_literal(&definition.when, Field::StageRoom)?;
    let destination_stage = exact_symbol_literal(&definition.when, Field::NextStageName)?;
    let destination_room = exact_i8_literal(&definition.when, Field::NextStageRoom)?;
    let destination_point = exact_i16_literal(&definition.when, Field::NextStageSpawn)?;

    let context_path = root.join(&execution.world_context.path);
    let context_bytes = fs::read(&context_path).map_err(route_error)?;
    let context = WorldContext::decode_canonical(&context_bytes).map_err(route_error)?;
    if context.digest().map_err(route_error)? != execution.world_context.sha256 {
        return Err(route_message(
            "goal target world context differs from its execution binding",
        ));
    }
    let stage_binding = context
        .stages
        .iter()
        .find(|stage| stage.stage == source_stage)
        .ok_or_else(|| route_message("goal source stage is absent from world context"))?;
    let inventory_path = context_path
        .parent()
        .ok_or_else(|| route_message("world context has no artifact directory"))?
        .join(format!("{source_stage}.inventory.json"));
    let inventory =
        WorldInventory::decode_canonical(&fs::read(&inventory_path).map_err(route_error)?)
            .map_err(route_error)?;
    if inventory.stage != source_stage
        || inventory.digest().map_err(route_error)? != stage_binding.inventory_sha256
    {
        return Err(route_message(
            "goal source inventory differs from the pinned world context",
        ));
    }

    let collision_ids = inventory
        .load_triggers
        .iter()
        .filter(|trigger| {
            trigger.room == source_room
                && trigger.destination_stage == destination_stage
                && trigger.destination_room == destination_room
                && trigger.destination_point == destination_point
        })
        .map(|trigger| trigger.collision_id.as_str())
        .collect::<BTreeSet<_>>();
    if collision_ids.is_empty() {
        return Err(route_message(
            "goal transition has no matching load trigger in the pinned world",
        ));
    }

    let mut triangles = Vec::new();
    for collision in &inventory.collisions {
        if !collision_ids.contains(collision.prism.authored.stable_id.as_str()) {
            continue;
        }
        let KclReconstruction::Reconstructed { triangle, .. } = &collision.prism.reconstruction
        else {
            continue;
        };
        triangles.push(triangle.map(|point| [point.x, point.y, point.z]));
    }
    if triangles.is_empty() {
        return Err(route_message(
            "goal load triggers have no reconstructed target surface",
        ));
    }
    let coordinate = nearest_interior_load_trigger_target(
        source_coordinate,
        &triangles,
        LOAD_TRIGGER_TARGET_INTERIOR_CLEARANCE,
    )
    .ok_or_else(|| route_message("goal load trigger target selection failed"))?;
    if coordinate.iter().any(|value| !value.is_finite()) {
        return Err(route_message("goal target coordinate is non-finite"));
    }
    Ok((
        GoalTransitionTarget {
            source_stage,
            source_room,
            destination_stage,
            destination_room,
            destination_point,
            coordinate,
            supporting_load_triggers: collision_ids.len(),
            source_inventory_sha256: stage_binding.inventory_sha256,
        },
        inventory,
    ))
}

pub(super) fn nearest_interior_load_trigger_target(
    source: [f32; 3],
    triangles: &[[[f32; 3]; 3]],
    clearance: f32,
) -> Option<[f32; 3]> {
    if source.iter().any(|value| !value.is_finite()) || !clearance.is_finite() || clearance < 0.0 {
        return None;
    }
    triangles
        .iter()
        .filter(|triangle| triangle.iter().flatten().all(|value| value.is_finite()))
        .filter_map(|triangle| {
            let boundary = closest_planar_point_on_triangle(source, *triangle)?;
            let centroid = [
                (triangle[0][0] + triangle[1][0] + triangle[2][0]) / 3.0,
                (triangle[0][1] + triangle[1][1] + triangle[2][1]) / 3.0,
                (triangle[0][2] + triangle[1][2] + triangle[2][2]) / 3.0,
            ];
            let interior_distance = planar_distance(boundary, centroid);
            let fraction = if interior_distance <= f32::EPSILON {
                0.0
            } else {
                (clearance / interior_distance).min(1.0)
            };
            let interior = [
                boundary[0] + (centroid[0] - boundary[0]) * fraction,
                boundary[1] + (centroid[1] - boundary[1]) * fraction,
                boundary[2] + (centroid[2] - boundary[2]) * fraction,
            ];
            Some((interior, planar_distance(source, interior)))
        })
        .min_by(|left, right| {
            left.1
                .total_cmp(&right.1)
                .then_with(|| left.0.map(f32::to_bits).cmp(&right.0.map(f32::to_bits)))
        })
        .map(|(coordinate, _)| coordinate)
}

pub(super) fn closest_planar_point_on_triangle(
    point: [f32; 3],
    triangle: [[f32; 3]; 3],
) -> Option<[f32; 3]> {
    let ax = triangle[0][0];
    let az = triangle[0][2];
    let bx = triangle[1][0];
    let bz = triangle[1][2];
    let cx = triangle[2][0];
    let cz = triangle[2][2];
    let denominator = (bz - cz).mul_add(ax - cx, (cx - bx) * (az - cz));
    if denominator.abs() > f32::EPSILON {
        let first = ((bz - cz).mul_add(point[0] - cx, (cx - bx) * (point[2] - cz))) / denominator;
        let second = ((cz - az).mul_add(point[0] - cx, (ax - cx) * (point[2] - cz))) / denominator;
        let third = 1.0 - first - second;
        if first >= 0.0 && second >= 0.0 && third >= 0.0 {
            return Some([
                point[0],
                first.mul_add(
                    triangle[0][1],
                    second.mul_add(triangle[1][1], third * triangle[2][1]),
                ),
                point[2],
            ]);
        }
    }
    [(0, 1), (1, 2), (2, 0)]
        .into_iter()
        .map(|(start, end)| closest_planar_point_on_segment(point, triangle[start], triangle[end]))
        .min_by(|left, right| {
            planar_distance(point, *left)
                .total_cmp(&planar_distance(point, *right))
                .then_with(|| left.map(f32::to_bits).cmp(&right.map(f32::to_bits)))
        })
}

pub(super) fn closest_planar_point_on_segment(
    point: [f32; 3],
    start: [f32; 3],
    end: [f32; 3],
) -> [f32; 3] {
    let dx = end[0] - start[0];
    let dz = end[2] - start[2];
    let length_squared = dx.mul_add(dx, dz * dz);
    let fraction = if length_squared <= f32::EPSILON {
        0.0
    } else {
        ((point[0] - start[0]).mul_add(dx, (point[2] - start[2]) * dz) / length_squared)
            .clamp(0.0, 1.0)
    };
    [
        start[0] + dx * fraction,
        start[1] + (end[1] - start[1]) * fraction,
        start[2] + dz * fraction,
    ]
}

pub(super) fn exact_symbol_literal(
    expression: &Expression,
    field: Field,
) -> Result<String, NativeTacticRouteRunError> {
    let values = exact_literals(expression, field);
    let mut symbols = values
        .into_iter()
        .map(|value| match value {
            Value::Symbol(symbol) => Ok(symbol),
            _ => Err(route_message("goal transition literal has the wrong type")),
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if symbols.len() != 1 {
        return Err(route_message(
            "goal transition requires one exact symbolic field literal",
        ));
    }
    Ok(symbols.pop_first().expect("one checked symbol"))
}

pub(super) fn exact_i8_literal(
    expression: &Expression,
    field: Field,
) -> Result<i8, NativeTacticRouteRunError> {
    i8::try_from(exact_integer_literal(expression, field)?).map_err(route_error)
}

pub(super) fn exact_i16_literal(
    expression: &Expression,
    field: Field,
) -> Result<i16, NativeTacticRouteRunError> {
    i16::try_from(exact_integer_literal(expression, field)?).map_err(route_error)
}

pub(super) fn exact_integer_literal(
    expression: &Expression,
    field: Field,
) -> Result<i64, NativeTacticRouteRunError> {
    let values = exact_literals(expression, field);
    let integers = values
        .into_iter()
        .map(|value| match value {
            Value::I32(value) => Ok(i64::from(value)),
            Value::U32(value) => Ok(i64::from(value)),
            Value::U64(value) => i64::try_from(value).map_err(route_error),
            _ => Err(route_message("goal transition literal has the wrong type")),
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if integers.len() != 1 {
        return Err(route_message(
            "goal transition requires one exact integer field literal",
        ));
    }
    Ok(*integers.first().expect("one checked integer"))
}

pub(super) fn exact_literals(expression: &Expression, field: Field) -> Vec<Value> {
    match expression {
        Expression::Compare {
            field: candidate,
            operator: Comparison::Equal,
            value,
        } if *candidate == field => vec![value.clone()],
        Expression::And(left, right) => {
            let mut values = exact_literals(left, field);
            values.extend(exact_literals(right, field));
            values
        }
        _ => Vec::new(),
    }
}
