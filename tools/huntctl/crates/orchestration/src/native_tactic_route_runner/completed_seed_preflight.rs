use super::*;

pub(super) struct CompletedSeedPreflight {
    pub(super) indexed_results: Vec<(usize, NativeTacticSeedResult)>,
    pub(super) initial_facts: FactSnapshot,
    pub(super) root_checkpoint_sha256: Digest,
    pub(super) tactic_macro_discovery: NativeTacticMacroDiscoveryReport,
}

pub(super) fn load_completed_seed_preflight(
    config: &NativeTacticRouteRunConfig<'_>,
    execution_plan_sha256: Digest,
) -> Result<Option<CompletedSeedPreflight>, NativeTacticRouteRunError> {
    if !config.resume
        || config.execution_plan.demonstration_chunk_ticks.is_some()
        || !config
            .output_root
            .join(NATIVE_TACTIC_MACRO_DISCOVERY_FILE)
            .is_file()
        || config.execution_plan.lanes.is_empty()
    {
        return Ok(None);
    }
    let result_paths = config
        .execution_plan
        .lanes
        .iter()
        .enumerate()
        .map(|(seed_index, lane)| {
            config
                .output_root
                .join(format!("seed-{seed_index:03}-{}", lane.seed))
                .join("seed-result.json")
        })
        .collect::<Vec<_>>();
    if result_paths.iter().any(|path| !path.is_file()) {
        return Ok(None);
    }

    let mut indexed_results = Vec::with_capacity(result_paths.len());
    let mut initial_facts = None;
    let mut root_checkpoint_sha256 = None;
    let mut feature_schema_sha256 = None;
    let mut objective_sha256 = None;
    for (seed_index, (lane, result_path)) in config
        .execution_plan
        .lanes
        .iter()
        .zip(result_paths)
        .enumerate()
    {
        let completed = read_completed_seed(
            &result_path,
            lane.seed,
            config.execution_plan.budgets.decisions_per_lane,
            execution_plan_sha256,
            lane,
            config.execution_plan.demonstration_chunk_ticks.is_some(),
        )?;
        let root = completed
            .checkpoint
            .state_graph
            .node(completed.checkpoint.state_graph.root())
            .ok_or_else(|| route_message("completed seed graph has no root state"))?
            .state
            .as_ref()
            .clone();
        if initial_facts
            .as_ref()
            .is_some_and(|expected| expected != &root)
            || root_checkpoint_sha256
                .is_some_and(|expected| expected != completed.checkpoint.root_checkpoint_sha256)
            || feature_schema_sha256
                .is_some_and(|expected| expected != completed.checkpoint.feature_schema_sha256)
            || objective_sha256
                .is_some_and(|expected| expected != completed.checkpoint.objective_sha256)
        {
            return Err(route_message(
                "completed tactic seeds disagree on campaign root authority",
            ));
        }
        initial_facts.get_or_insert(root);
        root_checkpoint_sha256.get_or_insert(completed.checkpoint.root_checkpoint_sha256);
        feature_schema_sha256.get_or_insert(completed.checkpoint.feature_schema_sha256);
        objective_sha256.get_or_insert(completed.checkpoint.objective_sha256);
        indexed_results.push((seed_index, completed.result));
    }
    let initial_facts = initial_facts
        .ok_or_else(|| route_message("completed tactic preflight has no root facts"))?;
    let root_checkpoint_sha256 = root_checkpoint_sha256
        .ok_or_else(|| route_message("completed tactic preflight has no root checkpoint"))?;
    let tactic_macro_discovery = read_macro_discovery_report(
        config.output_root,
        execution_plan_sha256,
        objective_sha256
            .ok_or_else(|| route_message("completed tactic preflight has no objective"))?,
        feature_schema_sha256
            .ok_or_else(|| route_message("completed tactic preflight has no feature schema"))?,
        root_checkpoint_sha256,
    )?;
    Ok(Some(CompletedSeedPreflight {
        indexed_results,
        initial_facts,
        root_checkpoint_sha256,
        tactic_macro_discovery,
    }))
}
