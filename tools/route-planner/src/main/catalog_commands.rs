use super::*;

pub(super) fn extract_gcm(args: &[String]) -> Result<(), Box<dyn Error>> {
    let iso = required_path(args, "--iso")?;
    let output = required_path(args, "--output")?;
    let report = extract_gamecube_disc(&iso, &output)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

pub(super) fn diagnose_refinement_packs_command(args: &[String]) -> Result<(), Box<dyn Error>> {
    let paths = repeated_option(args, "--pack");
    if paths.is_empty() {
        return Err("diagnose-refinement-packs requires at least one --pack PACK.json".into());
    }
    let mut packs = Vec::new();
    let mut parse_diagnostics = Vec::new();
    for path in &paths {
        match serde_json::from_slice::<RefinementPack>(&fs::read(path)?) {
            Ok(pack) => packs.push(pack),
            Err(error) => parse_diagnostics.push(RefinementDiagnostic {
                pack_id: None,
                field: format!("pack[{path}]"),
                detail: error.to_string(),
                suggestion:
                    "Correct the reported JSON shape or unknown field, then diagnose again.".into(),
            }),
        }
    }
    let mut report = diagnose_refinement_packs(&packs);
    report.diagnostics.extend(parse_diagnostics);
    report.diagnostics.sort();
    report.diagnostics.dedup();
    report.valid = report.diagnostics.is_empty();
    let bytes = serde_json::to_vec_pretty(&report)?;
    if let Some(output) = option(args, "--output") {
        write_file(Path::new(&output), &bytes)?;
    }
    println!("{}", String::from_utf8(bytes)?);
    if !report.valid {
        return Err(format!(
            "refinement packs contain {} diagnostic(s)",
            report.diagnostics.len()
        )
        .into());
    }
    Ok(())
}

pub(super) fn export_evidence_citations(args: &[String]) -> Result<(), Box<dyn Error>> {
    let catalog =
        ComposedPlannerCatalog::decode_canonical(&fs::read(required_path(args, "--catalog")?)?)?;
    let input = required_path(args, "--input")?;
    let output = required_path(args, "--output")?;
    let index: EvidenceCitationIndex = serde_json::from_slice(&fs::read(&input)?)?;
    let bytes = index.canonical_bytes(&catalog)?;
    write_file(&output, &bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": index.schema,
            "input": input,
            "output": output,
            "sha256": index.digest(&catalog)?,
            "citations": index.citations.len(),
            "fact_catalog_sha256": index.fact_catalog_sha256,
            "mechanics_catalog_sha256": index.mechanics_catalog_sha256,
        }))?
    );
    Ok(())
}

pub(super) fn report_extraction_coverage(args: &[String]) -> Result<(), Box<dyn Error>> {
    let manifest_paths = repeated_option(args, "--manifest");
    if manifest_paths.is_empty() {
        return Err("report-extraction-coverage requires at least one --manifest FILE.json".into());
    }
    let manifests = manifest_paths
        .iter()
        .map(|path| Ok(FactPackManifest::decode_canonical(&fs::read(path)?)?))
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    let report = ExtractionCoverageReport::build(&manifests)?;
    let output = required_path(args, "--output")?;
    write_file(&output, &report.canonical_bytes()?)?;
    let unreported_domains = report
        .contexts
        .iter()
        .flat_map(|context| &context.domains)
        .filter(|domain| !domain.reported)
        .count();
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": report.schema,
            "output": output,
            "sha256": report.digest()?,
            "contexts": report.contexts.len(),
            "manifests": manifests.len(),
            "unreported_domains": unreported_domains,
        }))?
    );
    Ok(())
}

pub(super) fn report_obligation_coverage(args: &[String]) -> Result<(), Box<dyn Error>> {
    let mechanics = match (option(args, "--catalog"), option(args, "--mechanics")) {
        (Some(path), None) => ComposedPlannerCatalog::decode_canonical(&fs::read(path)?)?.mechanics,
        (None, Some(path)) => MechanicsCatalog::decode_canonical(&fs::read(path)?)?,
        _ => {
            return Err(
                "report-obligation-coverage requires exactly one of --catalog CATALOG.json or --mechanics MECHANICS.json"
                    .into(),
            );
        }
    };
    let report = ObligationCoverageReport::build(&mechanics)?;
    let output = required_path(args, "--output")?;
    write_file(&output, &report.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": report.schema,
            "output": output,
            "sha256": report.digest()?,
            "transitions": report.transitions.len(),
            "reach_obligations": report.transitions.iter().map(|row| row.reach_obligation_ids.len()).sum::<usize>(),
            "activation_obligations": report.transitions.iter().map(|row| row.activation_obligation_ids.len()).sum::<usize>(),
            "effect_obligations": report.transitions.iter().map(|row| row.effect_obligation_ids.len()).sum::<usize>(),
            "interruption_obligations": report.transitions.iter().map(|row| row.interruption_obligation_ids.len()).sum::<usize>(),
            "state_producers": report.transitions.iter().filter(|row| row.effect_operation_count > 0).count(),
        }))?
    );
    Ok(())
}

pub(super) fn report_route_evidence_coverage(args: &[String]) -> Result<(), Box<dyn Error>> {
    let catalog =
        ComposedPlannerCatalog::decode_canonical(&fs::read(required_path(args, "--catalog")?)?)?;
    let route_paths = repeated_option(args, "--route-book");
    if route_paths.is_empty() {
        return Err(
            "report-route-evidence-coverage requires at least one --route-book BOOK.json".into(),
        );
    }
    let route_books = route_paths
        .iter()
        .map(|path| Ok(RouteBook::decode_canonical(&fs::read(path)?)?))
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    let report = RouteEvidenceCoverageReport::build(
        &catalog,
        &route_books,
        usize_option(args, "--minimum-route-count", 2)?,
    )?;
    let output = required_path(args, "--output")?;
    write_file(&output, &report.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": report.schema,
            "output": output,
            "sha256": report.digest()?,
            "routes": report.routes.len(),
            "used_facts": report.facts.len(),
            "weak_high_usage_facts": report.weak_high_usage_fact_ids.len(),
            "minimum_route_count": report.minimum_route_count,
        }))?
    );
    Ok(())
}

pub(super) fn report_route_suite_coverage(args: &[String]) -> Result<(), Box<dyn Error>> {
    let catalog =
        ComposedPlannerCatalog::decode_canonical(&fs::read(required_path(args, "--catalog")?)?)?;
    let mut categorized = Vec::new();
    for (option_name, suite) in [
        ("--glitchless", RouteSuiteKind::GlitchlessStory),
        ("--hundred-percent", RouteSuiteKind::HundredPercent),
        ("--any-percent", RouteSuiteKind::AnyPercent),
        ("--hypothetical", RouteSuiteKind::Hypothetical),
    ] {
        for path in repeated_option(args, option_name) {
            categorized.push((suite, RouteBook::decode_canonical(&fs::read(path)?)?));
        }
    }
    let report = RouteSuiteCoverageReport::build(&catalog, &categorized)?;
    let output = required_path(args, "--output")?;
    write_file(&output, &report.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": report.schema,
            "output": output,
            "sha256": report.digest()?,
            "reported_suites": report.suites.iter().filter(|suite| suite.reported).count(),
            "routes": categorized.len(),
            "suite_fact_counts": report.suites.iter().map(|suite| (suite.suite, suite.exercised_fact_ids.len())).collect::<Vec<_>>(),
            "suite_obligation_counts": report.suites.iter().map(|suite| (suite.suite, suite.exercised_obligation_ids.len())).collect::<Vec<_>>(),
        }))?
    );
    Ok(())
}

pub(super) fn match_route_observations(args: &[String]) -> Result<(), Box<dyn Error>> {
    let catalog =
        ComposedPlannerCatalog::decode_canonical(&fs::read(required_path(args, "--catalog")?)?)?;
    let route_book = RouteBook::decode_canonical(&fs::read(required_path(args, "--route-book")?)?)?;
    let manifest = PlannedEdgeObservationManifest::decode_canonical(&fs::read(required_path(
        args,
        "--manifest",
    )?)?)?;
    let mut snapshots = Vec::new();
    for path in repeated_option(args, "--snapshot") {
        snapshots.push(StateSnapshot::decode_canonical(&fs::read(path)?)?);
    }
    let report = RouteObservationMatchReport::build(&catalog, &route_book, &manifest, &snapshots)?;
    let output = required_path(args, "--output")?;
    write_file(&output, &report.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": report.schema,
            "output": output,
            "sha256": report.digest()?,
            "observed_steps": report.steps.iter().filter(|step| step.observed).count(),
            "planned_steps": report.steps.len(),
            "observation_windows": manifest.observations.len(),
        }))?
    );
    Ok(())
}

pub(super) fn validate_route_observations(args: &[String]) -> Result<(), Box<dyn Error>> {
    let catalog =
        ComposedPlannerCatalog::decode_canonical(&fs::read(required_path(args, "--catalog")?)?)?;
    let route_book = RouteBook::decode_canonical(&fs::read(required_path(args, "--route-book")?)?)?;
    let matches = RouteObservationMatchReport::decode_canonical(&fs::read(required_path(
        args,
        "--matches",
    )?)?)?;
    let mut snapshots = Vec::new();
    for path in repeated_option(args, "--snapshot") {
        snapshots.push(StateSnapshot::decode_canonical(&fs::read(path)?)?);
    }
    let equivalence_sets = repeated_option(args, "--equivalence-set")
        .into_iter()
        .map(|path| Ok(EquivalenceSet::decode_canonical(&fs::read(path)?)?))
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    let report = RouteObservationValidationReport::build(
        &catalog,
        &route_book,
        &matches,
        &snapshots,
        &equivalence_sets,
        if flag(args, "--research") {
            EvidencePolicy::RESEARCH
        } else {
            EvidencePolicy::ESTABLISHED_ONLY
        },
    )?;
    let output = required_path(args, "--output")?;
    write_file(&output, &report.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": report.schema,
            "output": output,
            "sha256": report.digest()?,
            "validated_windows": report.validations.len(),
            "verified_postconditions": report.validations.iter().filter(|row| row.postcondition_status == dusklight_route_planner::route_observation_validation::VerificationStatus::Verified).count(),
            "verified_preservation": report.validations.iter().filter(|row| row.component_preservation_status == dusklight_route_planner::route_observation_validation::VerificationStatus::Verified).count(),
        }))?
    );
    Ok(())
}

pub(super) fn promote_witnessed_actions_command(args: &[String]) -> Result<(), Box<dyn Error>> {
    let catalog =
        ComposedPlannerCatalog::decode_canonical(&fs::read(required_path(args, "--catalog")?)?)?;
    let validation = RouteObservationValidationReport::decode_canonical(&fs::read(
        required_path(args, "--validation")?,
    )?)?;
    let request =
        WitnessPromotionRequest::decode_canonical(&fs::read(required_path(args, "--request")?)?)?;
    let (pack, receipt) = promote_witnessed_actions(&catalog, &validation, &request)?;
    let output = required_path(args, "--output")?;
    let receipt_output = required_path(args, "--receipt")?;
    write_file(&output, &pack.canonical_bytes()?)?;
    write_file(&receipt_output, &receipt.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": receipt.schema,
            "pack_id": pack.manifest.id,
            "output": output,
            "pack_sha256": pack.digest()?,
            "receipt": receipt_output,
            "receipt_sha256": receipt.digest()?,
            "promoted_actions": receipt.promotions.len(),
            "action_census_unchanged": receipt.action_ids_before == receipt.action_ids_after,
        }))?
    );
    Ok(())
}

pub(super) fn export_refinement_pack(args: &[String]) -> Result<(), Box<dyn Error>> {
    let input = required_path(args, "--input")?;
    let output = required_path(args, "--output")?;
    let pack: RefinementPack = serde_json::from_slice(&fs::read(&input)?)?;
    let report = pack.diagnose();
    if !report.valid {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Err(format!(
            "refinement pack contains {} diagnostic(s)",
            report.diagnostics.len()
        )
        .into());
    }
    let bytes = pack.canonical_bytes()?;
    write_file(&output, &bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "id": pack.manifest.id,
            "version": pack.manifest.version,
            "sha256": pack.digest()?,
            "rules": pack.rules.len(),
            "input": input,
            "output": output,
        }))?
    );
    Ok(())
}

pub(super) fn list_builtin_refinement_packs() -> Result<(), Box<dyn Error>> {
    let packs = bundled_refinement_pack_ids()
        .into_iter()
        .map(|id| {
            let pack = bundled_refinement_pack(id)?;
            Ok(json!({
                "id": id,
                "version": pack.manifest.version,
                "sha256": pack.digest()?,
                "rules": pack.rules.len(),
                "scope": pack.manifest.scope,
            }))
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({ "packs": packs }))?
    );
    Ok(())
}

pub(super) fn export_builtin_refinement_pack(args: &[String]) -> Result<(), Box<dyn Error>> {
    let id = option(args, "--id").ok_or("missing required --id ID")?;
    let output = required_path(args, "--output")?;
    let pack = bundled_refinement_pack(&id)?;
    let bytes = pack.canonical_bytes()?;
    write_file(&output, &bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "id": pack.manifest.id,
            "version": pack.manifest.version,
            "sha256": pack.digest()?,
            "rules": pack.rules.len(),
            "output": output,
        }))?
    );
    Ok(())
}

pub(super) fn catalog_state_boundaries(args: &[String]) -> Result<(), Box<dyn Error>> {
    let state_path = required_path(args, "--state")?;
    let state =
        PlannerExecutionStateDocument::decode_canonical(&fs::read(state_path)?)?.into_state()?;
    let policy_paths = repeated_option(args, "--policy");
    let policies = policy_paths
        .iter()
        .map(|path| {
            let policy: BoundaryPolicy = serde_json::from_slice(&fs::read(path)?)?;
            policy.validate()?;
            Ok(policy)
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    let catalog = ComponentBoundaryCatalog::derive(&state, policies)?;
    let output = required_path(args, "--output")?;
    write_file(&output, &catalog.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": catalog.schema,
            "content_sha256": catalog.content_sha256,
            "source_execution_state_sha256": catalog.source_execution_state_sha256,
            "inventory_entries": catalog.inventory.len(),
            "live_components": catalog
                .inventory
                .iter()
                .filter(|entry| matches!(&entry.storage, dusklight_route_planner::component_catalog::ComponentStorageLocation::Live))
                .count(),
            "boundary_policies": catalog.boundary_policies.len(),
            "effective_live_boundaries": catalog.effective_live_boundaries.len(),
        }))?
    );
    Ok(())
}

pub(super) fn audit_scene_change_consumers(args: &[String]) -> Result<(), Box<dyn Error>> {
    let source_root = required_path(args, "--source-root")?;
    let content: ContentIdentity =
        serde_json::from_slice(&fs::read(required_path(args, "--content-identity")?)?)?;
    let output = required_path(args, "--output")?;
    let audit = SceneChangeConsumerAudit::extract(&source_root, content)?;
    write_file(&output, &audit.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": audit.schema,
            "content_sha256": audit.content_sha256,
            "source_files": audit.source_files.len(),
            "call_sites": audit.counts.iter().map(|row| row.call_sites).sum::<u64>(),
            "counts": audit.counts,
            "output": output,
        }))?
    );
    Ok(())
}

pub(super) fn validate_scene_change_consumer_audit(args: &[String]) -> Result<(), Box<dyn Error>> {
    let input = required_path(args, "--input")?;
    let audit = SceneChangeConsumerAudit::decode_canonical(&fs::read(&input)?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": audit.schema,
            "content_sha256": audit.content_sha256,
            "source_files": audit.source_files.len(),
            "call_sites": audit.counts.iter().map(|row| row.call_sites).sum::<u64>(),
            "counts": audit.counts,
            "input": input,
        }))?
    );
    Ok(())
}

pub(super) fn audit_return_restart_writers(args: &[String]) -> Result<(), Box<dyn Error>> {
    let repository_root = required_path(args, "--repository-root")?;
    let bundle = dusklight_route_planner::orig_discovery::ExtractedOrigBundle::decode_canonical(
        &fs::read(required_path(args, "--bundle")?)?,
    )?;
    let output = required_path(args, "--output")?;
    let audit = ReturnRestartAudit::extract(&repository_root, &bundle)?;
    write_file(&output, &audit.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": audit.schema,
            "content_sha256": audit.content_sha256,
            "source_bundle_sha256": audit.source_bundle_sha256,
            "source_files": audit.source_files.len(),
            "call_sites": audit.writer_counts.iter().map(|row| row.call_sites).sum::<u64>(),
            "writer_counts": audit.writer_counts,
            "savmem_placements": audit.savmem_placements.len(),
            "output": output,
        }))?
    );
    Ok(())
}

pub(super) fn validate_return_restart_audit(args: &[String]) -> Result<(), Box<dyn Error>> {
    let input = required_path(args, "--input")?;
    let audit = ReturnRestartAudit::decode_canonical(&fs::read(&input)?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": audit.schema,
            "content_sha256": audit.content_sha256,
            "source_bundle_sha256": audit.source_bundle_sha256,
            "source_files": audit.source_files.len(),
            "call_sites": audit.writer_counts.iter().map(|row| row.call_sites).sum::<u64>(),
            "writer_counts": audit.writer_counts,
            "savmem_placements": audit.savmem_placements.len(),
            "input": input,
        }))?
    );
    Ok(())
}

pub(super) fn refresh_return_restart_audit_sources(args: &[String]) -> Result<(), Box<dyn Error>> {
    let repository_root = required_path(args, "--repository-root")?;
    let input = required_path(args, "--input")?;
    let output = required_path(args, "--output")?;
    let audit = ReturnRestartAudit::refresh_source_census(&repository_root, &fs::read(&input)?)?;
    write_file(&output, &audit.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": audit.schema,
            "content_sha256": audit.content_sha256,
            "source_bundle_sha256": audit.source_bundle_sha256,
            "source_files": audit.source_files.len(),
            "call_sites": audit.writer_counts.iter().map(|row| row.call_sites).sum::<u64>(),
            "writer_counts": audit.writer_counts,
            "savmem_placements": audit.savmem_placements.len(),
            "input": input,
            "output": output,
        }))?
    );
    Ok(())
}

pub(super) fn diff_orig(args: &[String]) -> Result<(), Box<dyn Error>> {
    let left_path = required_path(args, "--left")?;
    let right_path = required_path(args, "--right")?;
    let output = required_path(args, "--output")?;
    let left_locale = option(args, "--left-locale");
    let right_locale = option(args, "--right-locale");
    let locale_pair = match (left_locale.as_deref(), right_locale.as_deref()) {
        (Some(left), Some(right)) => Some((left, right)),
        (None, None) => None,
        _ => return Err("--left-locale and --right-locale must be supplied together".into()),
    };
    let left = dusklight_route_planner::orig_discovery::ExtractedOrigBundle::decode_canonical(
        &fs::read(left_path)?,
    )?;
    let right = dusklight_route_planner::orig_discovery::ExtractedOrigBundle::decode_canonical(
        &fs::read(right_path)?,
    )?;
    let diff = compare_orig_bundles(&left, &right, locale_pair)?;
    write_file(&output, &diff.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": diff.schema,
            "output": output,
            "left_bundle_sha256": diff.left_bundle_sha256,
            "right_bundle_sha256": diff.right_bundle_sha256,
            "left_content_sha256": diff.left_content_sha256,
            "right_content_sha256": diff.right_content_sha256,
            "locale_comparison": diff.locale_comparison,
            "domain_coverage": diff.domain_coverage,
            "summary": diff.summary,
        }))?
    );
    Ok(())
}
