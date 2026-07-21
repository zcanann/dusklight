use dusklight_route_planner::artifact::Digest;
use dusklight_route_planner::execution::{PlannerExecutionState, PlannerExecutionStateDocument};
use dusklight_route_planner::fact_pack::{
    CoverageDomain, CoverageStatus, ExtractorIdentity, FactPackCoverage, FactPackManifest,
    FactPackSource, SourceArtifactKind,
};
use dusklight_route_planner::graph::PlannerGraph;
use dusklight_route_planner::identity::{ContentIdentity, RuntimeConfiguration};
use dusklight_route_planner::logic::FactCatalog;
use dusklight_route_planner::refinement::{ComposedPlannerCatalog, RefinementPack};
use dusklight_route_planner::route_book::{RouteBook, RouteBookEditBatch};
use dusklight_route_planner::snapshot::StateSnapshot;
use dusklight_route_planner::transition::MechanicsCatalog;
use dusklight_route_planner::world_import::{EXTRACTED_WORLD_FACTS_SCHEMA, ExtractedWorldFacts};
use dusklight_route_planner_runtime::inspection::inspect_state;
use dusklight_route_planner_runtime::service::{
    PlannerServiceEnvelope, error_response, handle_envelope,
};
use dusklight_route_planner_runtime::{
    RuntimeEvidenceMode, RuntimeFeasibilityMode, RuntimeSolveOptions, solve_catalog_goal,
    solve_composed_catalog_goal,
};
use dusklight_world::world_context::WorldContext;
use dusklight_world::world_inventory::WorldInventory;
use serde_json::json;
use sha2::{Digest as _, Sha256};
use std::env;
use std::error::Error;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

fn main() {
    if let Err(error) = run() {
        eprintln!("route-planner: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("compose") => compose(&args[1..]),
        Some("extract-world") => extract_world(&args[1..]),
        Some("edit-route-book") => edit_route_book(&args[1..]),
        Some("inspect-state") => inspect_state_command(&args[1..]),
        Some("project-graph") => project_graph(&args[1..]),
        Some("serve-stdio") => serve_stdio(&args[1..]),
        Some("state-from-snapshot") => state_from_snapshot(&args[1..]),
        Some("validate-route-book") => validate_route_book(&args[1..]),
        Some("solve") => solve(&args[1..]),
        Some("help" | "--help" | "-h") | None => {
            print_usage();
            Ok(())
        }
        _ => {
            print_usage();
            Err("unknown route-planner command".into())
        }
    }
}

fn edit_route_book(args: &[String]) -> Result<(), Box<dyn Error>> {
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

fn validate_route_book(args: &[String]) -> Result<(), Box<dyn Error>> {
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

fn inspect_state_command(args: &[String]) -> Result<(), Box<dyn Error>> {
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

fn serve_stdio(args: &[String]) -> Result<(), Box<dyn Error>> {
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

fn project_graph(args: &[String]) -> Result<(), Box<dyn Error>> {
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

fn compose(args: &[String]) -> Result<(), Box<dyn Error>> {
    let facts_path = required_path(args, "--facts")?;
    let mechanics_path = required_path(args, "--mechanics")?;
    let output = required_path(args, "--output")?;
    let pack_paths = repeated_option(args, "--pack");
    let facts = FactCatalog::decode_canonical(&fs::read(facts_path)?)?;
    let mechanics = MechanicsCatalog::decode_canonical(&fs::read(mechanics_path)?)?;
    let mut packs = Vec::with_capacity(pack_paths.len());
    for path in pack_paths {
        packs.push(RefinementPack::decode_canonical(&fs::read(path)?)?);
    }
    let catalog = ComposedPlannerCatalog::compose(&facts, &mechanics, &packs)?;
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

fn extract_world(args: &[String]) -> Result<(), Box<dyn Error>> {
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
        sha256: facts.world_context_sha256,
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
                detail: "SCLS records and collision/SCLS joins are imported; actor-driven transitions remain unaudited.".into(),
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
                detail: "Addressable room collision and exit-code joins are indexed; reachability is not inferred.".into(),
            },
            FactPackCoverage {
                domain: CoverageDomain::PhysicalFeasibility,
                scope: "world".into(),
                status: CoverageStatus::Unavailable,
                detail: "Every imported collision exit retains an unresolved physical approach obligation.".into(),
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
            "spawns": facts.spawns.len(),
            "encoded_exits": facts.encoded_exits.len(),
            "candidate_transitions": facts.mechanics.transitions.len(),
            "physical_obligations": facts.mechanics.obligations.len(),
        }))?
    );
    Ok(())
}

fn state_from_snapshot(args: &[String]) -> Result<(), Box<dyn Error>> {
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

fn solve(args: &[String]) -> Result<(), Box<dyn Error>> {
    enum CatalogInput {
        Composed(ComposedPlannerCatalog),
        Base(FactCatalog, MechanicsCatalog),
    }

    let state_path = required_path(args, "--state")?;
    let output = required_path(args, "--output")?;
    let goal_id = option(args, "--goal").ok_or("missing required --goal ID")?;
    let state =
        PlannerExecutionStateDocument::decode_canonical(&fs::read(state_path)?)?.into_state()?;
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
    let defaults = RuntimeSolveOptions::default();
    let options = RuntimeSolveOptions {
        max_depth: usize_option(args, "--max-depth", defaults.max_depth)?,
        max_states: usize_option(args, "--max-states", defaults.max_states)?,
        max_resolution_combinations: usize_option(
            args,
            "--max-resolution-combinations",
            defaults.max_resolution_combinations,
        )?,
        feasibility_mode: if flag(args, "--upper-bound") {
            RuntimeFeasibilityMode::UpperBound
        } else {
            RuntimeFeasibilityMode::Modeled
        },
        evidence_mode: if flag(args, "--research") {
            RuntimeEvidenceMode::Research
        } else {
            RuntimeEvidenceMode::EstablishedOnly
        },
    };
    let report = match &input {
        CatalogInput::Composed(catalog) => {
            solve_composed_catalog_goal(state, catalog, &[], &goal_id, options)?
        }
        CatalogInput::Base(facts, mechanics) => {
            solve_catalog_goal(state, facts, mechanics, &[], &goal_id, options)?
        }
    };
    let bytes = serde_json::to_vec_pretty(&report)?;
    write_file(&output, &bytes)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn option(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
}

fn repeated_option(args: &[String], name: &str) -> Vec<String> {
    args.windows(2)
        .filter(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
        .collect()
}

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|argument| argument == name)
}

fn required_path(args: &[String], name: &str) -> Result<PathBuf, Box<dyn Error>> {
    option(args, name)
        .map(PathBuf::from)
        .ok_or_else(|| format!("missing required {name} <path>").into())
}

fn usize_option(args: &[String], name: &str, default: usize) -> Result<usize, Box<dyn Error>> {
    Ok(option(args, name)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(default))
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes)?;
    Ok(())
}

fn print_usage() {
    eprintln!(
        "Independent TP route planner:\n  route-planner compose --facts FACTS.json --mechanics MECHANICS.json [--pack REFINEMENT.json]... --output CATALOG.json\n  route-planner edit-route-book --route-book BOOK.json --edits EDITS.json (--catalog CATALOG.json | --facts FACTS.json --mechanics MECHANICS.json) --output EDITED.json\n  route-planner extract-world --content-identity CONTENT.json --runtime-configuration RUNTIME.json --world-context CONTEXT.json --inventory INVENTORY.json [--inventory MORE.json] --output FACTS.json --manifest MANIFEST.json\n  route-planner inspect-state --state STATE.json (--catalog CATALOG.json | --facts FACTS.json) --output INSPECTION.json [--research]\n  route-planner project-graph (--catalog CATALOG.json | --facts FACTS.json --mechanics MECHANICS.json) [--route-book BOOK.json] --output GRAPH.json\n  route-planner state-from-snapshot --snapshot SNAPSHOT.json --output STATE.json\n  route-planner validate-route-book --route-book BOOK.json (--catalog CATALOG.json | --facts FACTS.json --mechanics MECHANICS.json)\n  route-planner solve --state STATE.json (--catalog CATALOG.json | --facts FACTS.json --mechanics MECHANICS.json) --goal ID --output REPORT.json [--max-depth N] [--max-states N] [--max-resolution-combinations N] [--upper-bound] [--research]\n  route-planner serve-stdio"
    );
}
