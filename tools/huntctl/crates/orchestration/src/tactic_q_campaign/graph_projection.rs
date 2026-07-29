use super::*;

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
