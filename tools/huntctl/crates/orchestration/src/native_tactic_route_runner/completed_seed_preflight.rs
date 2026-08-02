use super::worker_pool::load_existing_demonstration;
use super::*;

pub(super) struct CompletedSeedPreflight {
    pub(super) indexed_results: Vec<(usize, NativeTacticSeedResult)>,
    pub(super) initial_facts: FactSnapshot,
    pub(super) root_checkpoint_sha256: Digest,
    pub(super) tactic_macro_discovery: Option<NativeTacticMacroDiscoveryReport>,
    pub(super) demonstration: Option<NativeTacticDemonstrationReport>,
}

pub(super) fn load_completed_seed_preflight(
    config: &NativeTacticRouteRunConfig<'_>,
    execution_plan_sha256: Digest,
    process_tape: &InputTape,
) -> Result<Option<CompletedSeedPreflight>, NativeTacticRouteRunError> {
    if !config.resume || config.execution_plan.lanes.is_empty() {
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
    let feature_schema_sha256 = feature_schema_sha256
        .ok_or_else(|| route_message("completed tactic preflight has no feature schema"))?;
    let objective_sha256 = objective_sha256
        .ok_or_else(|| route_message("completed tactic preflight has no objective"))?;
    let root_source_frame = usize::try_from(initial_facts.tape_frame)
        .map_err(|_| route_message("completed tactic root frame exceeds platform limits"))?;
    let demonstration = load_existing_demonstration(
        config,
        feature_schema_sha256,
        &route_tactic_reward_spec(),
        process_tape,
        root_source_frame,
        root_checkpoint_sha256,
    )?;
    if !demonstration_evidence_matches_treatment(
        demonstration.is_some(),
        config.execution_plan.demonstration_chunk_ticks,
    ) {
        return Ok(None);
    }
    let tactic_macro_discovery = config
        .output_root
        .join(NATIVE_TACTIC_MACRO_DISCOVERY_FILE)
        .is_file()
        .then(|| {
            read_macro_discovery_report(
                config.output_root,
                execution_plan_sha256,
                objective_sha256,
                feature_schema_sha256,
                root_checkpoint_sha256,
            )
        })
        .transpose()?;
    Ok(Some(CompletedSeedPreflight {
        indexed_results,
        initial_facts,
        root_checkpoint_sha256,
        tactic_macro_discovery,
        demonstration: demonstration.map(|demonstration| demonstration.report),
    }))
}

pub(super) fn completed_seed_preflight_requires_native_fleet(
    preflight: Option<&CompletedSeedPreflight>,
) -> bool {
    recovery_requires_native_fleet(
        preflight.is_some(),
        preflight
            .and_then(|preflight| preflight.tactic_macro_discovery.as_ref())
            .is_some(),
    )
}

fn recovery_requires_native_fleet(
    completed_seed_evidence: bool,
    macro_discovery_evidence: bool,
) -> bool {
    !completed_seed_evidence || !macro_discovery_evidence
}

fn demonstration_evidence_matches_treatment(
    evidence_present: bool,
    demonstration_chunk_ticks: Option<u32>,
) -> bool {
    evidence_present == demonstration_chunk_ticks.is_some()
}

#[cfg(test)]
mod tests {
    use super::{demonstration_evidence_matches_treatment, recovery_requires_native_fleet};

    #[test]
    fn completed_seed_preflight_requires_exact_demonstration_treatment_evidence() {
        assert!(demonstration_evidence_matches_treatment(false, None));
        assert!(demonstration_evidence_matches_treatment(true, Some(4)));
        assert!(!demonstration_evidence_matches_treatment(true, None));
        assert!(!demonstration_evidence_matches_treatment(false, Some(4)));
    }

    #[test]
    fn completed_seeds_skip_native_work_except_for_unfinished_macro_validation() {
        assert!(recovery_requires_native_fleet(false, false));
        assert!(recovery_requires_native_fleet(false, true));
        assert!(recovery_requires_native_fleet(true, false));
        assert!(!recovery_requires_native_fleet(true, true));
    }
}
