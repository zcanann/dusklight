use super::*;

pub(super) fn edit_route_book(args: &[String]) -> Result<(), Box<dyn Error>> {
    let route_book_path = required_path(args, "--route-book")?;
    let edits_path = required_path(args, "--edits")?;
    let output = required_path(args, "--output")?;
    let book = RouteBook::decode_canonical(&fs::read(route_book_path)?)?;
    let batch = RouteBookEditBatch::decode_canonical(&fs::read(edits_path)?)?;
    let previous_sha256 = book.digest()?;
    let edited = match (
        option(args, "--catalog"),
        option(args, "--facts"),
        option(args, "--mechanics"),
    ) {
        (Some(path), None, None) => {
            let catalog = ComposedPlannerCatalog::decode_canonical(&fs::read(path)?)?;
            batch.apply_composed(&book, &catalog)?
        }
        (None, Some(facts), Some(mechanics)) => {
            let facts = FactCatalog::decode_canonical(&fs::read(facts)?)?;
            let mechanics = MechanicsCatalog::decode_canonical(&fs::read(mechanics)?)?;
            batch.apply(&book, &facts, &mechanics)?
        }
        _ => {
            return Err(
                "edit-route-book requires either --catalog CATALOG.json or both --facts FACTS.json and --mechanics MECHANICS.json"
                    .into(),
            );
        }
    };
    let bytes = edited.canonical_bytes()?;
    write_file(&output, &bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": edited.schema,
            "route_book_id": edited.manifest.id,
            "previous_sha256": previous_sha256,
            "sha256": edited.digest()?,
            "output": output,
            "bytes": bytes.len(),
        }))?
    );
    Ok(())
}

pub(super) fn validate_route_book(args: &[String]) -> Result<(), Box<dyn Error>> {
    let route_book_path = required_path(args, "--route-book")?;
    let book = RouteBook::decode_canonical(&fs::read(route_book_path)?)?;
    match (
        option(args, "--catalog"),
        option(args, "--facts"),
        option(args, "--mechanics"),
    ) {
        (Some(path), None, None) => {
            let catalog = ComposedPlannerCatalog::decode_canonical(&fs::read(path)?)?;
            book.validate_against_composed(&catalog)?;
        }
        (None, Some(facts), Some(mechanics)) => {
            let facts = FactCatalog::decode_canonical(&fs::read(facts)?)?;
            let mechanics = MechanicsCatalog::decode_canonical(&fs::read(mechanics)?)?;
            book.validate_against(&facts, &mechanics)?;
        }
        _ => {
            return Err(
                "validate-route-book requires either --catalog CATALOG.json or both --facts FACTS.json and --mechanics MECHANICS.json"
                    .into(),
            );
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": book.schema,
            "route_book_id": book.manifest.id,
            "sha256": book.digest()?,
            "goals": book.goal_ids.len(),
            "steps": book.steps.len(),
            "methods": book.methods.len(),
            "regions": book.regions.len(),
            "directives": book.directives.len(),
        }))?
    );
    Ok(())
}

pub(super) fn inspect_state_command(args: &[String]) -> Result<(), Box<dyn Error>> {
    let state_path = required_path(args, "--state")?;
    let output = required_path(args, "--output")?;
    let state =
        PlannerExecutionStateDocument::decode_canonical(&fs::read(state_path)?)?.into_state()?;
    let facts = match (option(args, "--catalog"), option(args, "--facts")) {
        (Some(path), None) => ComposedPlannerCatalog::decode_canonical(&fs::read(path)?)?.facts,
        (None, Some(path)) => FactCatalog::decode_canonical(&fs::read(path)?)?,
        _ => {
            return Err(
                "inspect-state requires exactly one of --catalog CATALOG.json or --facts FACTS.json"
                    .into(),
            );
        }
    };
    let inspection = inspect_state(
        &state,
        &facts,
        &[],
        if flag(args, "--research") {
            RuntimeEvidenceMode::Research
        } else {
            RuntimeEvidenceMode::EstablishedOnly
        },
    )?;
    let bytes = serde_json::to_vec_pretty(&inspection)?;
    write_file(&output, &bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": inspection.schema,
            "output": output,
            "execution_state_sha256": inspection.execution_state_sha256,
            "semantic_state_sha256": inspection.semantic_state_sha256,
            "components": inspection.state.snapshot.environment.components.len(),
            "serialized_component_stores": inspection.state.serialized_component_stores.len(),
            "facts": inspection.facts.len(),
        }))?
    );
    Ok(())
}

pub(super) fn diff_state_command(args: &[String]) -> Result<(), Box<dyn Error>> {
    let before_path = required_path(args, "--before")?;
    let after_path = required_path(args, "--after")?;
    let output = required_path(args, "--output")?;
    let boundary_name = option(args, "--boundary")
        .ok_or_else(|| "missing required --boundary <kind>".to_owned())?;
    let boundary: BoundaryKind = if let Some(id) = boundary_name.strip_prefix("custom:") {
        serde_json::from_value(json!({"kind": "custom", "id": id}))?
    } else {
        serde_json::from_value(json!({"kind": boundary_name}))?
    };
    let before =
        PlannerExecutionStateDocument::decode_canonical(&fs::read(before_path)?)?.into_state()?;
    let after =
        PlannerExecutionStateDocument::decode_canonical(&fs::read(after_path)?)?.into_state()?;
    let facts = match (option(args, "--catalog"), option(args, "--facts")) {
        (Some(path), None) => ComposedPlannerCatalog::decode_canonical(&fs::read(path)?)?.facts,
        (None, Some(path)) => FactCatalog::decode_canonical(&fs::read(path)?)?,
        _ => {
            return Err(
                "diff-state requires exactly one of --catalog CATALOG.json or --facts FACTS.json"
                    .into(),
            );
        }
    };
    let inspection = inspect_state_diff(
        &before,
        &after,
        boundary,
        &facts,
        &[],
        if flag(args, "--research") {
            RuntimeEvidenceMode::Research
        } else {
            RuntimeEvidenceMode::EstablishedOnly
        },
    )?;
    let bytes = serde_json::to_vec_pretty(&inspection)?;
    write_file(&output, &bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": inspection.schema,
            "output": output,
            "from_snapshot_sha256": inspection.state_diff.from_snapshot_sha256,
            "to_snapshot_sha256": inspection.state_diff.to_snapshot_sha256,
            "component_deltas": inspection.state_diff.component_deltas.len(),
            "fact_deltas": inspection.fact_deltas.len(),
        }))?
    );
    Ok(())
}

pub(super) fn compare_semantic_contexts_command(args: &[String]) -> Result<(), Box<dyn Error>> {
    let left_state = PlannerExecutionStateDocument::decode_canonical(&fs::read(required_path(
        args,
        "--left-state",
    )?)?)?
    .into_state()?;
    let right_state = PlannerExecutionStateDocument::decode_canonical(&fs::read(required_path(
        args,
        "--right-state",
    )?)?)?
    .into_state()?;
    let left_catalog = ComposedPlannerCatalog::decode_canonical(&fs::read(required_path(
        args,
        "--left-catalog",
    )?)?)?;
    let right_catalog = ComposedPlannerCatalog::decode_canonical(&fs::read(required_path(
        args,
        "--right-catalog",
    )?)?)?;
    let load_equivalence_sets = |name: &str| {
        repeated_option(args, name)
            .into_iter()
            .map(|path| Ok(EquivalenceSet::decode_canonical(&fs::read(path)?)?))
            .collect::<Result<Vec<_>, Box<dyn Error>>>()
    };
    let report = compare_semantic_contexts(
        &left_state,
        &left_catalog,
        &load_equivalence_sets("--left-equivalence-set")?,
        &right_state,
        &right_catalog,
        &load_equivalence_sets("--right-equivalence-set")?,
        if flag(args, "--research") {
            RuntimeEvidenceMode::Research
        } else {
            RuntimeEvidenceMode::EstablishedOnly
        },
    )?;
    let output = required_path(args, "--output")?;
    let bytes = serde_json::to_vec_pretty(&report)?;
    write_file(&output, &bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": report.schema,
            "output": output,
            "relation": report.relation,
            "fallback_used": report.fallback_used,
            "facts": report.facts.len(),
            "mechanics": report.mechanics.len(),
            "left_inapplicable_facts": report.summary.left_inapplicable_fact_ids.len(),
            "right_inapplicable_facts": report.summary.right_inapplicable_fact_ids.len(),
        }))?
    );
    Ok(())
}

pub(super) fn serve_stdio(args: &[String]) -> Result<(), Box<dyn Error>> {
    if !args.is_empty() {
        return Err("serve-stdio does not accept arguments".into());
    }
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<PlannerServiceEnvelope>(&line) {
            Ok(envelope) => handle_envelope(envelope),
            Err(error) => error_response(None, "json", error.to_string()),
        };
        serde_json::to_writer(&mut stdout, &response)?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    }
    Ok(())
}

pub(super) fn serve_web_command(args: &[String]) -> Result<(), Box<dyn Error>> {
    let mut listen = "127.0.0.1:32170".parse::<SocketAddr>()?;
    let mut project_root = PathBuf::from("tools/route-planner/projects");
    let mut index = 0;
    while index < args.len() {
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("{} requires a value", args[index]))?;
        match args[index].as_str() {
            "--listen" => listen = value.parse::<SocketAddr>()?,
            "--projects" => project_root = PathBuf::from(value),
            flag => return Err(format!("serve-web does not recognize {flag}").into()),
        }
        index += 2;
    }
    if !listen.ip().is_loopback() {
        return Err("serve-web currently accepts only a loopback listen address".into());
    }
    println!(
        "route planner: http://{listen} (projects: {})",
        project_root.display()
    );
    serve_web(PlannerWebConfig {
        listen,
        project_root,
    })?;
    Ok(())
}

pub(super) fn project_graph(args: &[String]) -> Result<(), Box<dyn Error>> {
    let output = required_path(args, "--output")?;
    let route_book = if let Some(path) = option(args, "--route-book") {
        Some(RouteBook::decode_canonical(&fs::read(path)?)?)
    } else {
        None
    };
    let catalog_path = option(args, "--catalog");
    let facts_path = option(args, "--facts");
    let mechanics_path = option(args, "--mechanics");
    let graph = match (catalog_path, facts_path, mechanics_path) {
        (Some(path), None, None) => {
            let catalog = ComposedPlannerCatalog::decode_canonical(&fs::read(path)?)?;
            if let Some(book) = &route_book {
                PlannerGraph::project_composed_with_route_book(&catalog, book)?
            } else {
                PlannerGraph::project_composed(&catalog)?
            }
        }
        (None, Some(facts), Some(mechanics)) => {
            let facts = FactCatalog::decode_canonical(&fs::read(facts)?)?;
            let mechanics = MechanicsCatalog::decode_canonical(&fs::read(mechanics)?)?;
            if let Some(book) = &route_book {
                PlannerGraph::project_with_route_book(&facts, &mechanics, book)?
            } else {
                PlannerGraph::project(&facts, &mechanics)?
            }
        }
        _ => {
            return Err(
                "project-graph requires either --catalog CATALOG.json or both --facts FACTS.json and --mechanics MECHANICS.json"
                    .into(),
            );
        }
    };
    let bytes = graph.canonical_bytes()?;
    write_file(&output, &bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": graph.schema,
            "output": output,
            "sha256": graph.digest()?,
            "bytes": bytes.len(),
            "nodes": graph.nodes.len(),
            "edges": graph.edges.len(),
            "regions": graph.regions.len(),
            "refinement_stack_sha256": graph.refinement_stack_sha256,
            "route_book_sha256": graph.route_book_sha256,
        }))?
    );
    Ok(())
}

pub(super) fn project_feasibility_diff(args: &[String]) -> Result<(), Box<dyn Error>> {
    let state_path = required_path(args, "--state")?;
    let output = required_path(args, "--output")?;
    let state =
        PlannerExecutionStateDocument::decode_canonical(&fs::read(state_path)?)?.into_state()?;
    let equivalence_sets = repeated_option(args, "--equivalence-set")
        .into_iter()
        .map(|path| Ok(EquivalenceSet::decode_canonical(&fs::read(path)?)?))
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    let policy = if flag(args, "--research") {
        EvidencePolicy::RESEARCH
    } else {
        EvidencePolicy::ESTABLISHED_ONLY
    };
    let diff = match (
        option(args, "--catalog"),
        option(args, "--facts"),
        option(args, "--mechanics"),
    ) {
        (Some(path), None, None) => {
            let catalog = ComposedPlannerCatalog::decode_canonical(&fs::read(path)?)?;
            PlannerFeasibilityGraphDiff::project_composed(
                &state,
                &catalog,
                &equivalence_sets,
                policy,
            )?
        }
        (None, Some(facts), Some(mechanics)) => {
            let facts = FactCatalog::decode_canonical(&fs::read(facts)?)?;
            let mechanics = MechanicsCatalog::decode_canonical(&fs::read(mechanics)?)?;
            PlannerFeasibilityGraphDiff::project(
                &state,
                &facts,
                &mechanics,
                &equivalence_sets,
                policy,
            )?
        }
        _ => {
            return Err(
                "project-feasibility-diff requires either --catalog CATALOG.json or both --facts FACTS.json and --mechanics MECHANICS.json"
                    .into(),
            );
        }
    };
    let bytes = diff.canonical_bytes()?;
    write_file(&output, &bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": diff.schema,
            "output": output,
            "sha256": diff.digest()?,
            "execution_state_sha256": diff.execution_state_sha256,
            "transitions": diff.transitions.len(),
        }))?
    );
    Ok(())
}

pub(super) fn project_authorization_graph(args: &[String]) -> Result<(), Box<dyn Error>> {
    let state_path = required_path(args, "--state")?;
    let output = required_path(args, "--output")?;
    let state =
        PlannerExecutionStateDocument::decode_canonical(&fs::read(state_path)?)?.into_state()?;
    let equivalence_sets = repeated_option(args, "--equivalence-set")
        .into_iter()
        .map(|path| -> Result<EquivalenceSet, Box<dyn Error>> {
            Ok(EquivalenceSet::decode_canonical(&fs::read(path)?)?)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let defaults = SolverOptions::default();
    let options = SolverOptions {
        max_depth: usize_option(args, "--max-depth", defaults.max_depth)?,
        max_states: usize_option(args, "--max-states", defaults.max_states)?,
        max_resolution_combinations: usize_option(
            args,
            "--max-resolution-combinations",
            defaults.max_resolution_combinations,
        )?,
        feasibility_mode: FeasibilityMode::UpperBound,
        evidence_policy: if flag(args, "--research") {
            EvidencePolicy::RESEARCH
        } else {
            EvidencePolicy::ESTABLISHED_ONLY
        },
    };
    let graph = match (
        option(args, "--catalog"),
        option(args, "--facts"),
        option(args, "--mechanics"),
    ) {
        (Some(path), None, None) => {
            let catalog = ComposedPlannerCatalog::decode_canonical(&fs::read(path)?)?;
            let graph = ForwardSolver::new(
                &catalog.facts,
                &catalog.mechanics,
                &equivalence_sets,
                options,
            )?
            .authorization_graph(state)?;
            graph.with_refinement_stack_sha256(catalog.refinement_stack.digest()?)?
        }
        (None, Some(facts), Some(mechanics)) => {
            let facts = FactCatalog::decode_canonical(&fs::read(facts)?)?;
            let mechanics = MechanicsCatalog::decode_canonical(&fs::read(mechanics)?)?;
            ForwardSolver::new(&facts, &mechanics, &equivalence_sets, options)?
                .authorization_graph(state)?
        }
        _ => {
            return Err(
                "project-authorization-graph requires either --catalog CATALOG.json or both --facts FACTS.json and --mechanics MECHANICS.json"
                    .into(),
            );
        }
    };
    let bytes = graph.canonical_bytes()?;
    write_file(&output, &bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": graph.schema,
            "output": output,
            "sha256": graph.digest()?,
            "bytes": bytes.len(),
            "initial_state_sha256": graph.initial_state_sha256,
            "nodes": graph.nodes.len(),
            "evaluated_states": graph.evaluated_states,
            "edges": graph.edges.len(),
            "traversal_complete": graph.traversal_complete,
            "unknown_activation_candidates": graph.unknown_activation_candidates.len(),
            "unknown_transitions": graph.unknown_transition_ids.len(),
            "unknown_writers": graph.unknown_writer_ids.len(),
            "execution_errors": graph.execution_error_ids.len(),
            "refinement_stack_sha256": graph.refinement_stack_sha256,
        }))?
    );
    Ok(())
}

pub(super) fn compose(args: &[String]) -> Result<(), Box<dyn Error>> {
    let facts_path = required_path(args, "--facts")?;
    let mechanics_path = required_path(args, "--mechanics")?;
    let output = required_path(args, "--output")?;
    let pack_paths = repeated_option(args, "--pack");
    let route_overlay_paths = repeated_option(args, "--route-overlay");
    let what_if_overlay_paths = repeated_option(args, "--what-if-overlay");
    let mut facts = FactCatalog::decode_canonical(&fs::read(facts_path)?)?;
    let mut mechanics = MechanicsCatalog::decode_canonical(&fs::read(mechanics_path)?)?;
    let message_flow_set_paths = repeated_option(args, "--message-flow-set");
    let mut message_flow_dependencies = Vec::with_capacity(message_flow_set_paths.len());
    for path in &message_flow_set_paths {
        let set = CompiledMessageFlowSet::decode_canonical(&fs::read(path)?)?;
        message_flow_dependencies.push((set.digest()?, set.exact_context.clone()));
        set.merge_into(&mut facts, &mut mechanics)?;
    }
    let message_entry_set_paths = repeated_option(args, "--message-entry-set");
    for path in &message_entry_set_paths {
        let set = CompiledMessageFlowEntrySet::decode_canonical(&fs::read(path)?)?;
        let dependency = message_flow_dependencies
            .iter()
            .find(|(digest, _)| *digest == set.source_contracts.compiled_message_flow_set_sha256);
        let Some((_, exact_context)) = dependency else {
            return Err(format!(
                "message entry set {} requires its exact --message-flow-set dependency",
                set.source_contracts.id
            )
            .into());
        };
        if exact_context != &set.exact_context {
            return Err(format!(
                "message entry set {} does not share its message-flow set's exact context",
                set.source_contracts.id
            )
            .into());
        }
        set.merge_into(&mut mechanics)?;
    }
    let load_packs = |paths: Vec<String>| {
        paths
            .into_iter()
            .map(|path| Ok(RefinementPack::decode_canonical(&fs::read(path)?)?))
            .collect::<Result<Vec<_>, Box<dyn Error>>>()
    };
    let layers = RefinementLayers {
        enabled_packs: load_packs(pack_paths)?,
        route_local_overlays: load_packs(route_overlay_paths)?,
        ephemeral_what_if_overlays: load_packs(what_if_overlay_paths)?,
    };
    let catalog = ComposedPlannerCatalog::compose_layered(&facts, &mechanics, &layers)?;
    let bytes = catalog.canonical_bytes()?;
    write_file(&output, &bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": catalog.schema,
            "output": output,
            "sha256": catalog.digest()?,
            "base_fact_catalog_sha256": catalog.base_fact_catalog_sha256,
            "base_mechanics_catalog_sha256": catalog.base_mechanics_catalog_sha256,
            "bytes": bytes.len(),
            "packs": catalog.refinement_stack.entries.len(),
            "enabled_packs": layers.enabled_packs.len(),
            "route_local_overlays": layers.route_local_overlays.len(),
            "ephemeral_what_if_overlays": layers.ephemeral_what_if_overlays.len(),
            "message_flow_sets": message_flow_set_paths.len(),
            "message_entry_sets": message_entry_set_paths.len(),
            "aliases": catalog.facts.aliases.len(),
            "derived_facts": catalog.facts.derived_facts.len(),
            "transitions": catalog.mechanics.transitions.len(),
            "obligations": catalog.mechanics.obligations.len(),
            "obstructions": catalog.mechanics.obstructions.len(),
            "resolvers": catalog.mechanics.resolvers.len(),
            "techniques": catalog.mechanics.techniques.len(),
        }))?
    );
    Ok(())
}

pub(super) fn extract_world(args: &[String]) -> Result<(), Box<dyn Error>> {
    let content_path = required_path(args, "--content-identity")?;
    let runtime_path = required_path(args, "--runtime-configuration")?;
    let context_path = required_path(args, "--world-context")?;
    let output = required_path(args, "--output")?;
    let manifest_output = required_path(args, "--manifest")?;
    let inventory_paths = repeated_option(args, "--inventory");
    if inventory_paths.is_empty() {
        return Err("extract-world requires at least one --inventory FILE".into());
    }
    let content = ContentIdentity::decode_canonical(&fs::read(content_path)?)?;
    let runtime = RuntimeConfiguration::decode_canonical(&fs::read(runtime_path)?)?;
    let context = WorldContext::decode_canonical(&fs::read(context_path)?)?;
    let inventories = inventory_paths
        .iter()
        .map(|path| WorldInventory::read_canonical(Path::new(path)))
        .collect::<Result<Vec<_>, _>>()?;
    let facts = ExtractedWorldFacts::build(&content, &runtime, &context, &inventories)?;
    let bytes = facts.canonical_bytes()?;
    let mut sources = vec![FactPackSource {
        kind: SourceArtifactKind::WorldContext,
        id: "world-context".into(),
        sha256: facts
            .world_context_sha256
            .ok_or("compatible world import did not retain its world-context digest")?,
    }];
    sources.extend(facts.inventories.iter().map(|inventory| FactPackSource {
        kind: SourceArtifactKind::WorldInventory,
        id: format!("world-inventory/{}", inventory.stage.to_ascii_lowercase()),
        sha256: inventory.inventory_sha256,
    }));
    let executable_sha256 = Digest(Sha256::digest(fs::read(env::current_exe()?)?).into());
    let manifest = FactPackManifest::build(
        format!("{}.world", content.id),
        content,
        ExtractorIdentity {
            name: "route-planner-world-facts".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            executable_sha256,
            schema_sha256: Digest(Sha256::digest(EXTRACTED_WORLD_FACTS_SCHEMA).into()),
        },
        sources,
        vec![
            FactPackCoverage {
                domain: CoverageDomain::Topology,
                scope: "world".into(),
                status: CoverageStatus::Partial,
                detail: "SCLS records and collision/SCLS joins are imported; exact actor/event/player consumers are source-censused, while unaudited activation contracts remain explicit.".into(),
            },
            FactPackCoverage {
                domain: CoverageDomain::ActorPlacements,
                scope: "world".into(),
                status: CoverageStatus::Partial,
                detail: "Recognized DZS/DZR placement chunks are imported with raw records; actor reconstruction remains unaudited.".into(),
            },
            FactPackCoverage {
                domain: CoverageDomain::Collision,
                scope: "world".into(),
                status: CoverageStatus::Partial,
                detail: "Addressable room collision and exit-code joins are indexed. Reconstructed trigger triangles retain exact plane/bounds and same-room spawn candidates; connectivity and reachability are not inferred.".into(),
            },
            FactPackCoverage {
                domain: CoverageDomain::PhysicalFeasibility,
                scope: "world".into(),
                status: CoverageStatus::Partial,
                detail: "Collision exits retain explicit approach obligations. Exact GZ2E01 L1/L5 boss doors additionally import source-and-placement-bound yaw-oriented checkArea shapes, form-specific compound tests, L5 positive-local-Z planes, and circular facing; actor-phase execution remains explicit.".into(),
            },
        ],
        EXTRACTED_WORLD_FACTS_SCHEMA,
        facts.digest()?,
    )?;
    let manifest_bytes = manifest.canonical_bytes()?;
    write_file(&output, &bytes)?;
    write_file(&manifest_output, &manifest_bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": facts.schema,
            "exact_context": facts.exact_context,
            "world_context_sha256": facts.world_context_sha256,
            "output": output,
            "manifest": manifest_output,
            "manifest_sha256": manifest.digest()?,
            "sha256": facts.digest()?,
            "bytes": bytes.len(),
            "stages": facts.inventories.len(),
            "static_world_objects": facts.static_world_objects.len(),
            "spatial_volumes": facts.spatial_volumes.len(),
            "spatial_planes": facts.spatial_planes.len(),
            "spawns": facts.spawns.len(),
            "encoded_exits": facts.encoded_exits.len(),
            "approach_geometries": facts.approach_geometries.len(),
            "candidate_transitions": facts.mechanics.transitions.len(),
            "physical_obligations": facts.mechanics.obligations.len(),
        }))?
    );
    Ok(())
}

pub(super) fn extract_native_world(args: &[String]) -> Result<(), Box<dyn Error>> {
    let content_path = required_path(args, "--content-identity")?;
    let runtime_path = required_path(args, "--runtime-configuration")?;
    let inventories_path = required_path(args, "--inventories")?;
    let output = required_path(args, "--output")?;
    let manifest_output = required_path(args, "--manifest")?;
    let content = ContentIdentity::decode_canonical(&fs::read(content_path)?)?;
    let runtime = RuntimeConfiguration::decode_canonical(&fs::read(runtime_path)?)?;
    let inventories =
        ExtractedOrigWorldInventories::decode_canonical(&fs::read(inventories_path)?)?;
    let facts =
        ExtractedWorldFacts::build_from_orig_world_inventories(&content, &runtime, &inventories)?;
    let bytes = facts.canonical_bytes()?;
    let native_sha256 = facts
        .native_inventory_set_sha256
        .ok_or("native world import did not retain its inventory-set digest")?;
    let manifest = FactPackManifest::build(
        format!("{}.native-world", content.id),
        content,
        ExtractorIdentity {
            name: "route-planner-native-world-facts".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            executable_sha256: Digest(Sha256::digest(fs::read(env::current_exe()?)?).into()),
            schema_sha256: Digest(Sha256::digest(EXTRACTED_WORLD_FACTS_SCHEMA).into()),
        },
        vec![FactPackSource {
            kind: SourceArtifactKind::WorldInventory,
            id: "native-world-inventory-set".into(),
            sha256: native_sha256,
        }],
        vec![
            FactPackCoverage {
                domain: CoverageDomain::Topology,
                scope: "world".into(),
                status: CoverageStatus::Partial,
                detail: "All planner-native SCLS, MULT, FILI, RTBL, RCAM, RARO, RPAT, and RPPN records are imported; SCLS stays inert unless a source-audited actor transition imports separately.".into(),
            },
            FactPackCoverage {
                domain: CoverageDomain::ActorPlacements,
                scope: "world".into(),
                status: CoverageStatus::Partial,
                detail: "Every recognized planner-native actor, treasure, and player-spawn record is imported; generic actor behavior remains unaudited.".into(),
            },
            FactPackCoverage {
                domain: CoverageDomain::Collision,
                scope: "world".into(),
                status: CoverageStatus::Unavailable,
                detail: "The planner-native inventory-set v5 does not decode KCL/PLC, spatial indexes, or collision/SCLS joins.".into(),
            },
            FactPackCoverage {
                domain: CoverageDomain::PhysicalFeasibility,
                scope: "world".into(),
                status: CoverageStatus::Partial,
                detail: "No spatial-index identity or generic collision reachability is manufactured; exact source-audited actor interaction shapes and staged obligations remain represented where available.".into(),
            },
        ],
        EXTRACTED_WORLD_FACTS_SCHEMA,
        facts.digest()?,
    )?;
    write_file(&output, &bytes)?;
    write_file(&manifest_output, &manifest.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": facts.schema,
            "exact_context": facts.exact_context,
            "native_inventory_set_sha256": native_sha256,
            "world_context_sha256": facts.world_context_sha256,
            "output": output,
            "manifest": manifest_output,
            "manifest_sha256": manifest.digest()?,
            "sha256": facts.digest()?,
            "bytes": bytes.len(),
            "stages": facts.inventories.len(),
            "room_transforms": facts.native_stage_metadata.iter().map(|metadata| metadata.room_transforms.len()).sum::<usize>(),
            "file_lists": facts.native_stage_metadata.iter().map(|metadata| metadata.file_lists.len()).sum::<usize>(),
            "room_reads": facts.native_stage_metadata.iter().map(|metadata| metadata.room_reads.len()).sum::<usize>(),
            "cameras": facts.native_stage_metadata.iter().map(|metadata| metadata.cameras.len()).sum::<usize>(),
            "camera_arrows": facts.native_stage_metadata.iter().map(|metadata| metadata.camera_arrows.len()).sum::<usize>(),
            "paths": facts.native_stage_metadata.iter().map(|metadata| metadata.paths.len()).sum::<usize>(),
            "path_points": facts.native_stage_metadata.iter().map(|metadata| metadata.path_points.len()).sum::<usize>(),
            "static_world_objects": facts.static_world_objects.len(),
            "spawns": facts.spawns.len(),
            "encoded_exits": facts.encoded_exits.len(),
            "candidate_transitions": facts.mechanics.transitions.len(),
            "physical_obligations": facts.mechanics.obligations.len(),
        }))?
    );
    Ok(())
}

pub(super) fn state_from_snapshot(args: &[String]) -> Result<(), Box<dyn Error>> {
    let snapshot_path = required_path(args, "--snapshot")?;
    let output = required_path(args, "--output")?;
    let snapshot = StateSnapshot::decode_canonical(&fs::read(&snapshot_path)?)?;
    let document = PlannerExecutionState::new(snapshot)?.to_document()?;
    let bytes = document.canonical_bytes()?;
    write_file(&output, &bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": document.schema,
            "output": output,
            "sha256": document.digest()?,
            "bytes": bytes.len(),
        }))?
    );
    Ok(())
}

pub(super) fn solve(args: &[String]) -> Result<(), Box<dyn Error>> {
    enum CatalogInput {
        Composed(ComposedPlannerCatalog),
        Base(FactCatalog, MechanicsCatalog),
    }

    let state_path = required_path(args, "--state")?;
    let output = required_path(args, "--output")?;
    let goal_id = option(args, "--goal").ok_or("missing required --goal ID")?;
    let state =
        PlannerExecutionStateDocument::decode_canonical(&fs::read(state_path)?)?.into_state()?;
    let route_book = option(args, "--route-book")
        .map(|path| fs::read(path).map_err(Box::<dyn Error>::from))
        .transpose()?
        .map(|bytes| RouteBook::decode_canonical(&bytes))
        .transpose()?;
    let catalog_path = option(args, "--catalog");
    let facts_path = option(args, "--facts");
    let mechanics_path = option(args, "--mechanics");
    let input = match (catalog_path, facts_path, mechanics_path) {
        (Some(path), None, None) => {
            let catalog = ComposedPlannerCatalog::decode_canonical(&fs::read(path)?)?;
            CatalogInput::Composed(catalog)
        }
        (None, Some(facts), Some(mechanics)) => CatalogInput::Base(
            FactCatalog::decode_canonical(&fs::read(facts)?)?,
            MechanicsCatalog::decode_canonical(&fs::read(mechanics)?)?,
        ),
        _ => {
            return Err(
                "solve requires either --catalog CATALOG.json or both --facts FACTS.json and --mechanics MECHANICS.json"
                    .into(),
            );
        }
    };
    let options = solve_options(args)?;
    let report = match &input {
        CatalogInput::Composed(catalog) => match &route_book {
            Some(book) => {
                solve_composed_route_book_goal(state, catalog, &[], book, &goal_id, options)?
            }
            None => solve_composed_catalog_goal(state, catalog, &[], &goal_id, options)?,
        },
        CatalogInput::Base(facts, mechanics) => match &route_book {
            Some(book) => solve_catalog_route_book_goal(
                state,
                facts,
                mechanics,
                &[],
                book,
                &goal_id,
                options,
            )?,
            None => solve_catalog_goal(state, facts, mechanics, &[], &goal_id, options)?,
        },
    };
    let bytes = serde_json::to_vec_pretty(&report)?;
    write_file(&output, &bytes)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

pub(super) fn solve_portable(args: &[String]) -> Result<(), Box<dyn Error>> {
    enum CatalogInput {
        Composed(ComposedPlannerCatalog),
        Base(FactCatalog, MechanicsCatalog),
    }

    let state_paths = repeated_option(args, "--state");
    if state_paths.is_empty() {
        return Err("solve-portable requires at least one --state STATE.json".into());
    }
    let states = state_paths
        .into_iter()
        .map(|path| -> Result<PlannerExecutionState, Box<dyn Error>> {
            Ok(PlannerExecutionStateDocument::decode_canonical(&fs::read(path)?)?.into_state()?)
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    let equivalence_sets = repeated_option(args, "--equivalence-set")
        .into_iter()
        .map(|path| -> Result<EquivalenceSet, Box<dyn Error>> {
            Ok(EquivalenceSet::decode_canonical(&fs::read(path)?)?)
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    let route_book = RouteBook::decode_canonical(&fs::read(required_path(args, "--route-book")?)?)?;
    let output = required_path(args, "--output")?;
    let goal_id = option(args, "--goal").ok_or("missing required --goal ID")?;
    let input = match (
        option(args, "--catalog"),
        option(args, "--facts"),
        option(args, "--mechanics"),
    ) {
        (Some(path), None, None) => {
            CatalogInput::Composed(ComposedPlannerCatalog::decode_canonical(&fs::read(path)?)?)
        }
        (None, Some(facts), Some(mechanics)) => CatalogInput::Base(
            FactCatalog::decode_canonical(&fs::read(facts)?)?,
            MechanicsCatalog::decode_canonical(&fs::read(mechanics)?)?,
        ),
        _ => {
            return Err(
                "solve-portable requires either --catalog CATALOG.json or both --facts FACTS.json and --mechanics MECHANICS.json"
                    .into(),
            );
        }
    };
    let options = solve_options(args)?;
    let report = match &input {
        CatalogInput::Composed(catalog) => solve_composed_portable_route_book_goal(
            states,
            catalog,
            &equivalence_sets,
            &route_book,
            &goal_id,
            options,
        )?,
        CatalogInput::Base(facts, mechanics) => solve_catalog_portable_route_book_goal(
            states,
            facts,
            mechanics,
            &equivalence_sets,
            &route_book,
            &goal_id,
            options,
        )?,
    };
    let bytes = serde_json::to_vec_pretty(&report)?;
    write_file(&output, &bytes)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
