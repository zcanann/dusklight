use super::*;

pub(super) fn recover_completed_campaign(
    config: &NativeTacticRouteRunConfig<'_>,
    imported_promoted_tactics: Option<&ImportedPromotedTactics>,
) -> Result<Option<NativeTacticRouteReport>, NativeTacticRouteRunError> {
    if !config.resume {
        return Ok(None);
    }
    let report_path = config.output_root.join("report.json");
    let summary_path = config.output_root.join(NATIVE_TACTIC_CAMPAIGN_SUMMARY_FILE);
    let completion_path = config
        .output_root
        .join(NATIVE_TACTIC_CAMPAIGN_COMPLETION_FILE);
    if completion_path.exists() {
        let completion = NativeTacticCampaignCompletion::read(&completion_path)?;
        completion.validate_files(&report_path, &summary_path)?;
        let report = read_native_tactic_route_report(&report_path)?;
        validate_report_binding(config, imported_promoted_tactics, &report)?;
        return Ok(Some(report));
    }
    if !report_path.is_file() || !summary_path.is_file() {
        return Ok(None);
    }

    // Both JSON files are an uncommitted derived tail until every durable
    // authority is reattached. A torn or stale tail falls back to the ordinary
    // resume path, which removes and rebuilds it from seed evidence.
    let recovery_started = Instant::now();
    let recovered = (|| {
        let report_bytes = fs::read(&report_path).map_err(route_error)?;
        let summary_bytes = fs::read(&summary_path).map_err(route_error)?;
        let report = read_native_tactic_route_report(&report_path)?;
        let summary: NativeTacticCampaignSummary =
            serde_json::from_slice(&summary_bytes).map_err(route_error)?;
        validate_report_binding(config, imported_promoted_tactics, &report)?;
        summary.validate()?;
        if summary != NativeTacticCampaignSummary::build(&report, config.execution_plan)? {
            return Err(route_message(
                "orphan tactic campaign summary is detached from its report",
            ));
        }
        let first_checkpoint = validate_report_seed_authority(config, &report)?;
        validate_report_control_plane(config, &report, &first_checkpoint)?;
        validate_report_macro_authority(config, &report, &first_checkpoint)?;

        let report_build_micros = report.timing.reporting_micros;
        let observed_wall_micros = report
            .timing
            .wall_micros
            .saturating_add(report_build_micros)
            .saturating_add(elapsed_micros(recovery_started.elapsed()));
        let completion = NativeTacticCampaignCompletion::build(
            report.execution_plan_sha256,
            &report_bytes,
            &summary_bytes,
            report.timing.wall_micros,
            report_build_micros,
            0,
            0,
            observed_wall_micros,
        )?;
        publish_completion(&completion_path, &completion)?;
        Ok(report)
    })();
    match recovered {
        Ok(report) => Ok(Some(report)),
        Err(_) => Ok(None),
    }
}

fn validate_report_binding(
    config: &NativeTacticRouteRunConfig<'_>,
    imported_promoted_tactics: Option<&ImportedPromotedTactics>,
    report: &NativeTacticRouteReport,
) -> Result<(), NativeTacticRouteRunError> {
    let execution_plan_sha256 = config.execution_plan.identity()?;
    let expected_import = imported_promoted_tactics.map(|imported| &imported.report);
    if !matches!(
        report.schema.as_str(),
        NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V44 | NATIVE_TACTIC_ROUTE_REPORT_SCHEMA_V45
    ) || report.optimization_request_sha256 != config.optimization.content_sha256
        || report.execution_binding_sha256 != config.execution.content_sha256
        || report.execution_plan_sha256 != execution_plan_sha256
        || report.objective_sha256 != config.optimization.terminal_predicate.definition_sha256
        || report.execution_plan_path
            != path_text(&config.output_root.join(NATIVE_TACTIC_EXECUTION_PLAN_FILE))
        || report.replay_control_plane_path
            != path_text(
                &config
                    .output_root
                    .join(NATIVE_TACTIC_REPLAY_CONTROL_PLANE_FILE),
            )
        || report.exploration_seeds != config.execution_plan.seeds
        || report.proposal_policy != config.execution_plan.proposal_policy
        || report.value_treatment != config.execution_plan.value_treatment
        || report.execution_strategy != config.execution_plan.execution_strategy
        || report.decisions_per_seed != config.execution_plan.budgets.decisions_per_lane
        || report.resource_budgets != config.execution_plan.budgets
        || report.refit_every_decisions != config.execution_plan.refit_every_decisions
        || report.imported_promoted_tactics.as_ref() != expected_import
    {
        return Err(route_message(
            "completed tactic campaign report belongs to another run",
        ));
    }
    Ok(())
}

struct ValidatedReportSeedAuthority {
    feature_schema_sha256: Digest,
    objective_sha256: Digest,
    root_checkpoint_sha256: Digest,
}

fn validate_report_seed_authority(
    config: &NativeTacticRouteRunConfig<'_>,
    report: &NativeTacticRouteReport,
) -> Result<ValidatedReportSeedAuthority, NativeTacticRouteRunError> {
    if report.seeds.len() != config.execution_plan.lanes.len() {
        return Err(route_message(
            "completed tactic campaign report has detached seed cardinality",
        ));
    }
    let mut first_authority = None;
    for (seed_index, lane) in config.execution_plan.lanes.iter().enumerate() {
        let seed = lane.seed;
        let result_path = config
            .output_root
            .join(format!("seed-{seed_index:03}-{seed}"))
            .join("seed-result.json");
        let completed = read_completed_seed_preflight(
            &result_path,
            seed,
            config.execution_plan.budgets.decisions_per_lane,
            report.execution_plan_sha256,
            lane,
            config.execution_plan.demonstration_chunk_ticks.is_some(),
        )?;
        let reported = report
            .seeds
            .get(seed_index)
            .ok_or_else(|| route_message("completed tactic campaign report is missing a seed"))?;
        if serde_cbor::to_vec(&completed.result).map_err(route_error)?
            != serde_cbor::to_vec(reported).map_err(route_error)?
        {
            return Err(route_message(
                "completed tactic campaign report seed differs from durable seed evidence",
            ));
        }
        let authority = ValidatedReportSeedAuthority {
            feature_schema_sha256: completed.feature_schema_sha256,
            objective_sha256: completed.objective_sha256,
            root_checkpoint_sha256: completed.root_checkpoint_sha256,
        };
        if first_authority
            .as_ref()
            .is_some_and(|first: &ValidatedReportSeedAuthority| {
                first.feature_schema_sha256 != authority.feature_schema_sha256
                    || first.objective_sha256 != authority.objective_sha256
                    || first.root_checkpoint_sha256 != authority.root_checkpoint_sha256
            })
        {
            return Err(route_message(
                "completed tactic campaign seeds disagree on authority",
            ));
        }
        first_authority.get_or_insert(authority);
    }
    first_authority.ok_or_else(|| route_message("completed tactic campaign has no seed authority"))
}

fn validate_report_control_plane(
    config: &NativeTacticRouteRunConfig<'_>,
    report: &NativeTacticRouteReport,
    authority: &ValidatedReportSeedAuthority,
) -> Result<(), NativeTacticRouteRunError> {
    let identity = TacticReplayControlPlaneIdentity::new(
        report.execution_plan_sha256,
        authority.feature_schema_sha256,
        authority.objective_sha256,
        authority.root_checkpoint_sha256,
    )
    .map_err(route_error)?;
    let store = TacticQContentStore::open(
        config
            .output_root
            .join(NATIVE_TACTIC_CONTENT_STORE_DIRECTORY),
    )
    .map_err(route_error)?;
    let replay = TacticReplayControlPlane::open_with_content_store(
        &config
            .output_root
            .join(NATIVE_TACTIC_REPLAY_CONTROL_PLANE_FILE),
        store,
        &identity,
    )
    .map_err(route_error)?;
    let replay_snapshot = replay.replay_snapshot();
    let learner_heads = CampaignLearnerHeadJournal::open_or_create(&replay)?;
    let learner_head = learner_heads
        .latest()
        .ok_or_else(|| route_message("completed tactic campaign has no durable learner head"))?;
    let published_snapshots = learner_heads.snapshot_sha256s().collect::<BTreeSet<_>>();
    if replay_snapshot.revision != report.replay_revision
        || replay_snapshot.sha256 != report.replay_snapshot_sha256
        || replay.len() as u64 != report.shared_training_replay_rows
        || learner_head.learner_snapshot_sha256
            != report.learner_authority.latest_model_snapshot_sha256
        || learner_head.replay_revision != report.learner_authority.latest_training_replay_rows
        || learner_head.model_revision != report.learner_authority.latest_model_revision
        || learner_head.model_revision != report.learner_updates
        || published_snapshots.len() as u64 != report.learner_authority.model_snapshots_published
    {
        return Err(route_message(
            "completed tactic campaign report is detached from replay or learner authority",
        ));
    }
    Ok(())
}

fn validate_report_macro_authority(
    config: &NativeTacticRouteRunConfig<'_>,
    report: &NativeTacticRouteReport,
    authority: &ValidatedReportSeedAuthority,
) -> Result<(), NativeTacticRouteRunError> {
    let macro_report = &report.tactic_macro_discovery;
    let registry_path = Path::new(&macro_report.registry_path);
    let confined = registry_path
        .canonicalize()
        .ok()
        .zip(config.output_root.canonicalize().ok())
        .is_some_and(|(registry, output)| registry.starts_with(output));
    if !registry_path.is_file() || !confined {
        return Err(route_message(
            "completed tactic campaign macro registry is outside its output root",
        ));
    }
    let registry = read_tactic_macro_registry(registry_path).map_err(route_error)?;
    if registry.content_sha256 != macro_report.registry_sha256 {
        return Err(route_message(
            "completed tactic campaign macro registry identity is detached",
        ));
    }
    let durable_report_path = config.output_root.join(NATIVE_TACTIC_MACRO_DISCOVERY_FILE);
    if durable_report_path.is_file()
        && read_macro_discovery_report(
            config.output_root,
            report.execution_plan_sha256,
            authority.objective_sha256,
            authority.feature_schema_sha256,
            authority.root_checkpoint_sha256,
        )? != *macro_report
    {
        return Err(route_message(
            "completed tactic campaign macro report differs from durable authority",
        ));
    }
    Ok(())
}
