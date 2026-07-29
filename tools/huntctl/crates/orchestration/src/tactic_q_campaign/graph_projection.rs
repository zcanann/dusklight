use super::*;
use crate::state_graph::{ActionExpansionStatus, ExpansionEvidenceAuthority};

pub(super) struct GraphTrainingProjection {
    pub transitions: Vec<OptionTransitionSample>,
    pub routes: Vec<InputTape>,
    pub episode_groups: Vec<u64>,
}

pub(super) fn graph_training_projection(
    graph: &StateGraph,
) -> Result<GraphTrainingProjection, TacticQCampaignError> {
    graph.validate()?;
    let mut transitions = Vec::with_capacity(graph.expansion_count());
    let mut routes = Vec::with_capacity(graph.expansion_count());
    let mut episode_groups = Vec::with_capacity(graph.expansion_count());
    let mut identities = BTreeSet::new();
    for (transition, route, episode_group) in graph.completed_evidence() {
        let identity = transition.replay_identity_sha256()?;
        if !identities.insert(identity) {
            return Err(TacticQCampaignError::InvalidState(
                "state graph contains duplicate completed evidence",
            ));
        }
        transitions.push(transition.clone());
        routes.push(route.clone());
        episode_groups.push(episode_group);
    }
    Ok(GraphTrainingProjection {
        transitions,
        routes,
        episode_groups,
    })
}

pub(super) fn validate_training_projection(
    graph: &StateGraph,
    transitions: &[OptionTransitionSample],
    routes: &[InputTape],
    episode_groups: &[u64],
) -> Result<(), TacticQCampaignError> {
    let projection = graph_training_projection(graph)?;
    if projection.transitions != transitions
        || projection.routes != routes
        || projection.episode_groups != episode_groups
    {
        return Err(TacticQCampaignError::InvalidState(
            "training replay is not a read-only state graph projection",
        ));
    }
    Ok(())
}

pub(super) fn graph_root_branch(
    graph: &StateGraph,
) -> Result<TacticCampaignBranch, TacticQCampaignError> {
    let root = graph
        .node(graph.root())
        .ok_or(TacticQCampaignError::InvalidState(
            "state graph root is absent",
        ))?;
    let route =
        graph
            .route(root.id.route_checkpoint_sha256)
            .ok_or(TacticQCampaignError::InvalidState(
                "state graph root route is absent",
            ))?;
    Ok(TacticCampaignBranch {
        kind: TacticBranchKind::Root,
        logical_frontier: LogicalTacticFrontierRecord {
            identity_sha256: root.id.route_checkpoint_sha256,
            state_sha256: root.id.state_sha256,
            route_frames: route.frames.len() as u64,
            replayed_prefix_ticks: 0,
        },
        restorable_native_checkpoint: None,
        acquisition: None,
        state: root.state.clone(),
        route_tape: route.clone(),
        descriptor: None,
    })
}

pub(super) fn graph_frontier_entries(
    graph: &StateGraph,
    maximum_route_frames: usize,
) -> Result<Vec<TacticFrontierEntry>, TacticQCampaignError> {
    graph.validate()?;
    let mut entries = Vec::new();
    for node in graph.nodes().filter(|node| {
        node.id != graph.root()
            && node.restoration.executable
            && !node.terminal
            && node.restoration.route.tape_frames <= maximum_route_frames as u64
    }) {
        let executable = |expansion: &&crate::state_graph::ActionExpansion| {
            matches!(
                expansion.status,
                ActionExpansionStatus::Completed {
                    authority: ExpansionEvidenceAuthority::Executable,
                    ..
                }
            )
        };
        let expansion = node
            .incoming_segments
            .iter()
            .find_map(|identity| graph.segment(*identity))
            .and_then(|segment| graph.expansion(segment.parent_expansion_sha256))
            .filter(executable)
            .or_else(|| {
                node.outgoing_expansions
                    .iter()
                    .find_map(|identity| graph.expansion(*identity))
                    .filter(executable)
            })
            .ok_or(TacticQCampaignError::InvalidState(
                "branchable graph node has no realized expansion evidence",
            ))?;
        let ActionExpansionStatus::Completed {
            authority: ExpansionEvidenceAuthority::Executable,
            evidence,
            ..
        } = &expansion.status
        else {
            return Err(TacticQCampaignError::InvalidState(
                "branchable graph node has no executable completed expansion",
            ));
        };
        let transition = evidence
            .values()
            .find(|row| row.authority == ExpansionEvidenceAuthority::Executable)
            .or_else(|| evidence.values().next())
            .ok_or(TacticQCampaignError::InvalidState(
                "branchable graph expansion has no evidence",
            ))?
            .transition
            .as_ref();
        let route = graph.route(node.id.route_checkpoint_sha256).ok_or(
            TacticQCampaignError::InvalidState("branchable graph node route is absent"),
        )?;
        entries.push(TacticFrontierEntry {
            descriptor: tactic_endpoint_descriptor_for_state(&node.state, false, &expansion.action)
                .map_err(|error| TacticQCampaignError::Frontier(error.to_string()))?,
            root_checkpoint_sha256: graph.identity.root_checkpoint_sha256,
            route_checkpoint_sha256: node.id.route_checkpoint_sha256,
            frontier_state_sha256: node.id.state_sha256,
            frontier_state: node.state.clone(),
            transition: transition.clone(),
            route_tape: route.clone(),
            first_seen_generation: node.root_ticks,
        });
    }
    Ok(entries)
}
