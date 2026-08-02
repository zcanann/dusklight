use super::*;
use crate::state_graph::{ActionExpansionStatus, ExpansionEvidenceAuthority};

pub(crate) struct GraphTrainingProjection {
    pub keys: Vec<(Digest, Digest)>,
    pub transitions: Vec<OptionTransitionSample>,
    pub routes: Vec<InputTape>,
    pub episode_groups: Vec<u64>,
}

pub(crate) struct GraphTrainingProjectionRow {
    key: (Digest, Digest),
    transition: OptionTransitionSample,
    route: InputTape,
    episode_group: u64,
}

pub(crate) fn graph_training_projection(
    graph: &StateGraph,
) -> Result<GraphTrainingProjection, TacticQCampaignError> {
    graph_training_projection_validated(graph.validated()?)
}

pub(crate) fn graph_training_projection_validated(
    validated: ValidatedStateGraph<'_>,
) -> Result<GraphTrainingProjection, TacticQCampaignError> {
    let graph = validated.graph();
    let mut transitions = Vec::with_capacity(graph.expansion_count());
    let mut routes = Vec::with_capacity(graph.expansion_count());
    let mut episode_groups = Vec::with_capacity(graph.expansion_count());
    let mut keys = Vec::with_capacity(graph.expansion_count());
    let mut identities = BTreeSet::new();
    for expansion in graph.expansions() {
        let ActionExpansionStatus::Completed {
            route_checkpoint_sha256,
            evidence,
            ..
        } = &expansion.status
        else {
            continue;
        };
        let route =
            graph
                .route(*route_checkpoint_sha256)
                .ok_or(TacticQCampaignError::InvalidState(
                    "completed graph evidence route is absent",
                ))?;
        for (evidence_sha256, row) in evidence {
            if !identities.insert(*evidence_sha256) {
                return Err(TacticQCampaignError::InvalidState(
                    "state graph contains duplicate completed evidence",
                ));
            }
            keys.push((expansion.identity_sha256, *evidence_sha256));
            transitions.push(row.transition.as_ref().clone());
            routes.push(route.clone());
            episode_groups.push(row.episode_group);
        }
    }
    Ok(GraphTrainingProjection {
        keys,
        transitions,
        routes,
        episode_groups,
    })
}

pub(crate) fn graph_training_projection_rows(
    graph: &StateGraph,
    admitted_keys: impl IntoIterator<Item = (Digest, Digest)>,
) -> Result<Vec<GraphTrainingProjectionRow>, TacticQCampaignError> {
    let requested = admitted_keys.into_iter().collect::<BTreeSet<_>>();
    let mut rows = Vec::with_capacity(requested.len());
    for (expansion_sha256, evidence_sha256) in requested {
        let expansion =
            graph
                .expansion(expansion_sha256)
                .ok_or(TacticQCampaignError::InvalidState(
                    "admitted graph expansion is absent",
                ))?;
        let ActionExpansionStatus::Completed {
            route_checkpoint_sha256,
            evidence,
            ..
        } = &expansion.status
        else {
            return Err(TacticQCampaignError::InvalidState(
                "admitted graph expansion is not completed",
            ));
        };
        let evidence = evidence
            .get(&evidence_sha256)
            .ok_or(TacticQCampaignError::InvalidState(
                "admitted graph evidence is absent",
            ))?;
        let route =
            graph
                .route(*route_checkpoint_sha256)
                .ok_or(TacticQCampaignError::InvalidState(
                    "admitted graph evidence route is absent",
                ))?;
        rows.push(GraphTrainingProjectionRow {
            key: (expansion_sha256, evidence_sha256),
            transition: evidence.transition.as_ref().clone(),
            route: route.clone(),
            episode_group: evidence.episode_group,
        });
    }
    Ok(rows)
}

pub(crate) fn validate_graph_training_projection_merge(
    keys: &[(Digest, Digest)],
    transitions: &[OptionTransitionSample],
    routes: &[InputTape],
    episode_groups: &[u64],
    rows: &[GraphTrainingProjectionRow],
) -> Result<(), TacticQCampaignError> {
    if keys.len() != transitions.len()
        || keys.len() != routes.len()
        || keys.len() != episode_groups.len()
        || !keys.windows(2).all(|pair| pair[0] < pair[1])
    {
        return Err(TacticQCampaignError::InvalidState(
            "cached graph learner projection has invalid shape or order",
        ));
    }
    for row in rows {
        if let Ok(index) = keys.binary_search(&row.key)
            && (transitions[index] != row.transition
                || routes[index] != row.route
                || episode_groups[index] != row.episode_group)
        {
            return Err(TacticQCampaignError::InvalidState(
                "cached graph learner projection conflicts with admitted evidence",
            ));
        }
    }
    Ok(())
}

pub(crate) fn merge_graph_training_projection(
    keys: &mut Vec<(Digest, Digest)>,
    transitions: &mut Vec<OptionTransitionSample>,
    routes: &mut Vec<InputTape>,
    episode_groups: &mut Vec<u64>,
    rows: Vec<GraphTrainingProjectionRow>,
) {
    for row in rows {
        let Err(index) = keys.binary_search(&row.key) else {
            continue;
        };
        keys.insert(index, row.key);
        transitions.insert(index, row.transition);
        routes.insert(index, row.route);
        episode_groups.insert(index, row.episode_group);
    }
}

pub(super) fn validate_training_projection_and_keys(
    validated: ValidatedStateGraph<'_>,
    transitions: &[OptionTransitionSample],
    routes: &[InputTape],
    episode_groups: &[u64],
) -> Result<Vec<(Digest, Digest)>, TacticQCampaignError> {
    if transitions.len() != routes.len() || transitions.len() != episode_groups.len() {
        return Err(TacticQCampaignError::InvalidState(
            "training replay projection shape is invalid",
        ));
    }
    let graph = validated.graph();
    let mut keys = Vec::with_capacity(transitions.len());
    let mut index = 0_usize;
    for expansion in graph.expansions() {
        let ActionExpansionStatus::Completed {
            route_checkpoint_sha256,
            evidence,
            ..
        } = &expansion.status
        else {
            continue;
        };
        let route =
            graph
                .route(*route_checkpoint_sha256)
                .ok_or(TacticQCampaignError::InvalidState(
                    "completed graph evidence route is absent",
                ))?;
        for (evidence_sha256, row) in evidence {
            if transitions.get(index) != Some(row.transition.as_ref())
                || routes.get(index) != Some(route)
                || episode_groups.get(index) != Some(&row.episode_group)
            {
                return Err(TacticQCampaignError::InvalidState(
                    "training replay is not a read-only state graph projection",
                ));
            }
            keys.push((expansion.identity_sha256, *evidence_sha256));
            index = index.saturating_add(1);
        }
    }
    if index != transitions.len() {
        return Err(TacticQCampaignError::InvalidState(
            "training replay is not a read-only state graph projection",
        ));
    }
    Ok(keys)
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
        state: root.state.as_ref().clone(),
        route_tape: route.clone(),
        descriptor: None,
    })
}

pub(super) fn graph_frontier_entries_validated(
    validated: ValidatedStateGraph<'_>,
    maximum_route_frames: usize,
) -> Result<Vec<TacticFrontierEntry>, TacticQCampaignError> {
    let graph = validated.graph();
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
            frontier_state: node.state.as_ref().clone(),
            transition: transition.clone(),
            route_tape: route.clone(),
            first_seen_generation: node.root_ticks,
        });
    }
    Ok(entries)
}

impl TacticQCampaign {
    pub fn graph_learning_batch(
        &self,
    ) -> Result<crate::learner::GraphLearningBatch, TacticQCampaignError> {
        let graph = self
            .state_graph
            .as_ref()
            .ok_or(TacticQCampaignError::InvalidState(
                "learner target projection requires a bound state graph",
            ))?;
        Ok(crate::learner::GraphLearningBatch::from_graph(graph)?)
    }

    pub fn best_graph_terminal_path(
        &self,
    ) -> Result<Option<&crate::state_graph::TerminalPath>, TacticQCampaignError> {
        let graph = self
            .state_graph
            .as_ref()
            .ok_or(TacticQCampaignError::InvalidState(
                "terminal query requires a bound state graph",
            ))?;
        graph.validate()?;
        Ok(graph.best_terminal_path())
    }

    pub fn graph_terminal_path_available(&self) -> Result<bool, TacticQCampaignError> {
        let graph = self
            .state_graph
            .as_ref()
            .ok_or(TacticQCampaignError::InvalidState(
                "terminal query requires a bound state graph",
            ))?;
        Ok(graph.best_terminal_path().is_some())
    }

    pub fn final_result_matches_graph_terminal(
        &self,
        result: &TacticQFinalResult,
    ) -> Result<bool, TacticQCampaignError> {
        validate_final_result(result)?;
        let graph = self
            .state_graph
            .as_ref()
            .ok_or(TacticQCampaignError::InvalidState(
                "terminal result validation requires a bound state graph",
            ))?;
        graph.validate()?;
        let Some(best) = graph.best_terminal_path() else {
            return Ok(false);
        };
        Ok(
            result.execution_authority_sha256 == self.execution_authority_sha256
                && result.objective_sha256 == self.objective_sha256
                && result.root_checkpoint_sha256 == self.root_checkpoint_sha256
                && result.terminal_state_sha256 == best.terminal.state_sha256
                && route_checkpoint(self.root_checkpoint_sha256, &result.route_tape)?
                    == best.route_checkpoint_sha256
                && graph.route(best.route_checkpoint_sha256) == Some(&result.route_tape),
        )
    }
}
