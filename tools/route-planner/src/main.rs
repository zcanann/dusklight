use dusklight_route_planner::artifact::Digest;
use dusklight_route_planner::cutscene::CutsceneProgram;
use dusklight_route_planner::evaluation::EvidencePolicy;
use dusklight_route_planner::execution::{PlannerExecutionState, PlannerExecutionStateDocument};
use dusklight_route_planner::fact_pack::{
    CoverageDomain, CoverageStatus, ExtractorIdentity, FactPackCoverage, FactPackManifest,
    FactPackSource, SourceArtifactKind,
};
use dusklight_route_planner::fact_pack_cache::{load_fact_pack, store_fact_pack};
use dusklight_route_planner::graph::{PlannerFeasibilityGraphDiff, PlannerGraph};
use dusklight_route_planner::identity::{ContentIdentity, EquivalenceSet, RuntimeConfiguration};
use dusklight_route_planner::logic::FactCatalog;
use dusklight_route_planner::orig_discovery::{
    EXTRACTED_ORIG_BUNDLE_SCHEMA, OrigSupportStatus, SupportedBuildRegistry,
    bundled_supported_build_registry, extract_orig_bundle, scan_orig_tree,
};
use dusklight_route_planner::orig_extraction::{
    EXTRACTED_STAGE_DATA_SCHEMA, extract_unique_rarc_resource, parse_message_flow, parse_stage_data,
};
use dusklight_route_planner::refinement::{
    ComposedPlannerCatalog, RefinementLayers, RefinementPack,
};
use dusklight_route_planner::route_book::{RouteBook, RouteBookEditBatch};
use dusklight_route_planner::snapshot::StateSnapshot;
use dusklight_route_planner::state::BoundaryKind;
use dusklight_route_planner::transition::MechanicsCatalog;
use dusklight_route_planner::world_data::{WorldContext, WorldInventory};
use dusklight_route_planner::world_import::{EXTRACTED_WORLD_FACTS_SCHEMA, ExtractedWorldFacts};
use dusklight_route_planner_runtime::inspection::{inspect_state, inspect_state_diff};
use dusklight_route_planner_runtime::service::{
    PlannerServiceEnvelope, error_response, handle_envelope,
};
use dusklight_route_planner_runtime::{
    RuntimeEvidenceMode, RuntimeFeasibilityMode, RuntimeSolveOptions, solve_catalog_goal,
    solve_catalog_portable_route_book_goal, solve_catalog_route_book_goal,
    solve_composed_catalog_goal, solve_composed_portable_route_book_goal,
    solve_composed_route_book_goal,
};
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
        Some("cache-fact-pack") => cache_fact_pack(&args[1..]),
        Some("compile-cutscene") => compile_cutscene(&args[1..]),
        Some("compose") => compose(&args[1..]),
        Some("extract-message-flow") => extract_message_flow(&args[1..]),
        Some("extract-orig") => extract_orig(&args[1..]),
        Some("extract-resource") => extract_resource(&args[1..]),
        Some("extract-stage-data") => extract_stage_data(&args[1..]),
        Some("extract-world") => extract_world(&args[1..]),
        Some("edit-route-book") => edit_route_book(&args[1..]),
        Some("inspect-state") => inspect_state_command(&args[1..]),
        Some("identify-orig") => identify_orig(&args[1..]),
        Some("materialize-fact-pack") => materialize_fact_pack(&args[1..]),
        Some("diff-state") => diff_state_command(&args[1..]),
        Some("project-graph") => project_graph(&args[1..]),
        Some("project-feasibility-diff") => project_feasibility_diff(&args[1..]),
        Some("serve-stdio") => serve_stdio(&args[1..]),
        Some("state-from-snapshot") => state_from_snapshot(&args[1..]),
        Some("validate-route-book") => validate_route_book(&args[1..]),
        Some("solve") => solve(&args[1..]),
        Some("solve-portable") => solve_portable(&args[1..]),
        Some("scan-orig") => scan_orig(&args[1..]),
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

fn compile_cutscene(args: &[String]) -> Result<(), Box<dyn Error>> {
    let program_path = required_path(args, "--program")?;
    let output = required_path(args, "--output")?;
    let program = CutsceneProgram::decode_canonical(&fs::read(program_path)?)?;
    let artifact = program.compile_artifact()?;
    let bytes = artifact.canonical_bytes()?;
    write_file(&output, &bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": artifact.schema,
            "program_sha256": artifact.program_sha256,
            "transitions": artifact.transitions.len(),
            "output": output,
        }))?
    );
    Ok(())
}

fn identify_orig(args: &[String]) -> Result<(), Box<dyn Error>> {
    let orig = required_path(args, "--orig")?;
    let output = required_path(args, "--output")?;
    let registry = load_supported_build_registry(args)?;
    let requested_content_id = option(args, "--content-id");
    let requested_identity = requested_content_id
        .as_deref()
        .map(|id| {
            registry
                .identities
                .iter()
                .find(|identity| identity.id == id)
                .ok_or_else(|| format!("content ID {id} is absent from the registry"))
        })
        .transpose()?;
    let product_id = requested_identity.map(|identity| identity.fingerprint.product_id.as_str());
    let scan = scan_orig_tree(&orig, product_id)?;
    let identification = registry.identify(&scan, requested_content_id.as_deref())?;
    let bytes = identification.canonical_bytes()?;
    write_file(&output, &bytes)?;
    let (status, content_id) = match &identification.support {
        OrigSupportStatus::Supported { content } => ("supported", Some(content.id.as_str())),
        OrigSupportStatus::Unsupported { .. } => ("unsupported", None),
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": identification.schema,
            "output": output,
            "status": status,
            "content_id": content_id,
            "scan_sha256": identification.scan_sha256,
            "product_id": scan.fingerprint.product_id,
        }))?
    );
    Ok(())
}

fn cache_fact_pack(args: &[String]) -> Result<(), Box<dyn Error>> {
    let cache = required_path(args, "--cache")?;
    let payload_path = required_path(args, "--payload")?;
    let manifest_path = required_path(args, "--manifest")?;
    let receipt_path = required_path(args, "--receipt")?;
    let manifest = FactPackManifest::decode_canonical(&fs::read(manifest_path)?)?;
    let payload = fs::read(payload_path)?;
    let receipt = store_fact_pack(&cache, &manifest, &payload)?;
    write_file(&receipt_path, &receipt.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": receipt.schema,
            "receipt": receipt_path,
            "manifest_sha256": receipt.manifest_sha256,
            "payload_sha256": receipt.payload_sha256,
            "reused": receipt.reused,
        }))?
    );
    Ok(())
}

fn materialize_fact_pack(args: &[String]) -> Result<(), Box<dyn Error>> {
    let cache = required_path(args, "--cache")?;
    let digest = option(args, "--manifest-sha256")
        .ok_or("missing required --manifest-sha256 <digest>")?
        .parse::<Digest>()?;
    let payload_output = required_path(args, "--payload")?;
    let manifest_output = required_path(args, "--manifest")?;
    let cached = load_fact_pack(&cache, digest)?;
    write_file(&payload_output, &cached.payload_bytes)?;
    write_file(&manifest_output, &cached.manifest_bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "manifest_sha256": digest,
            "payload_sha256": cached.manifest.payload_sha256,
            "payload": payload_output,
            "manifest": manifest_output,
        }))?
    );
    Ok(())
}

fn scan_orig(args: &[String]) -> Result<(), Box<dyn Error>> {
    let orig = required_path(args, "--orig")?;
    let output = required_path(args, "--output")?;
    let scan = scan_orig_tree(&orig, option(args, "--product-id").as_deref())?;
    let bytes = scan.canonical_bytes()?;
    write_file(&output, &bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": scan.schema,
            "output": output,
            "sha256": scan.digest()?,
            "product_id": scan.fingerprint.product_id,
            "platform": scan.fingerprint.platform,
            "region": scan.fingerprint.region,
            "revision": scan.fingerprint.revision,
            "files": scan.files.len(),
            "extractable_archives": scan.extractable_archive_paths.len(),
        }))?
    );
    Ok(())
}

fn extract_orig(args: &[String]) -> Result<(), Box<dyn Error>> {
    let orig = required_path(args, "--orig")?;
    let output = required_path(args, "--output")?;
    let manifest_output = required_path(args, "--manifest")?;
    let content = if let Some(content_path) = option(args, "--content-identity") {
        if option(args, "--registry").is_some() || option(args, "--content-id").is_some() {
            return Err(
                "--content-identity cannot be combined with --registry or --content-id".into(),
            );
        }
        ContentIdentity::decode_canonical(&fs::read(content_path)?)?
    } else {
        let registry = load_supported_build_registry(args)?;
        let requested_content_id = option(args, "--content-id");
        let requested_identity = requested_content_id
            .as_deref()
            .map(|id| {
                registry
                    .identities
                    .iter()
                    .find(|identity| identity.id == id)
                    .ok_or_else(|| format!("content ID {id} is absent from the registry"))
            })
            .transpose()?;
        let product_id =
            requested_identity.map(|identity| identity.fingerprint.product_id.as_str());
        let scan = scan_orig_tree(&orig, product_id)?;
        match registry
            .identify(&scan, requested_content_id.as_deref())?
            .support
        {
            OrigSupportStatus::Supported { content } => content,
            OrigSupportStatus::Unsupported { fingerprint } => {
                return Err(format!(
                    "unsupported orig fingerprint for {} revision {} (executable {}, game data {}, resources {})",
                    fingerprint.product_id,
                    fingerprint.revision,
                    fingerprint.executable_sha256,
                    fingerprint.game_data_sha256,
                    fingerprint.resource_manifest_sha256,
                )
                .into());
            }
        }
    };
    let bundle = extract_orig_bundle(&orig, &content)?;
    let bytes = bundle.canonical_bytes()?;
    let mut sources = vec![FactPackSource {
        kind: SourceArtifactKind::Executable,
        id: "orig/sys/main.dol".into(),
        sha256: bundle.input_scan.fingerprint.executable_sha256,
    }];
    sources.extend(bundle.stages.iter().map(|record| FactPackSource {
        kind: SourceArtifactKind::StageArchive,
        id: format!(
            "orig/stage/{}",
            Digest(Sha256::digest(record.relative_path.as_bytes()).into())
        ),
        sha256: record.archive_sha256,
    }));
    sources.extend(bundle.message_flows.iter().map(|record| FactPackSource {
        kind: SourceArtifactKind::MessageArchive,
        id: format!(
            "orig/message/{}",
            Digest(Sha256::digest(record.relative_path.as_bytes()).into())
        ),
        sha256: record.archive_sha256,
    }));
    let manifest = FactPackManifest::build(
        format!("{}.orig-extraction", content.id),
        content,
        ExtractorIdentity {
            name: "route-planner-orig-extraction".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            executable_sha256: Digest(Sha256::digest(fs::read(env::current_exe()?)?).into()),
            schema_sha256: Digest(Sha256::digest(EXTRACTED_ORIG_BUNDLE_SCHEMA).into()),
        },
        sources,
        vec![
            FactPackCoverage {
                domain: CoverageDomain::Topology,
                scope: "orig".into(),
                status: CoverageStatus::Partial,
                detail: "DZS/DZR chunks, STAG message groups, and indexed SCLS destinations are decoded; unresolved chunk semantics and physical reachability remain explicit.".into(),
            },
            FactPackCoverage {
                domain: CoverageDomain::ActorPlacements,
                scope: "orig".into(),
                status: CoverageStatus::Partial,
                detail: "Recognized placement chunks retain parameters, transforms, layer, and raw bytes.".into(),
            },
            FactPackCoverage {
                domain: CoverageDomain::MessageFlows,
                scope: "orig".into(),
                status: CoverageStatus::Partial,
                detail: "FLW1/FLI1 graphs and known temporary, persistent, and switch accesses are decoded for discovered language bundles.".into(),
            },
            FactPackCoverage {
                domain: CoverageDomain::PhysicalFeasibility,
                scope: "orig".into(),
                status: CoverageStatus::Unavailable,
                detail: "Resource extraction does not infer collision reachability, interaction geometry, or timing witnesses.".into(),
            },
        ],
        EXTRACTED_ORIG_BUNDLE_SCHEMA,
        bundle.digest()?,
    )?;
    let manifest_bytes = manifest.canonical_bytes()?;
    write_file(&output, &bytes)?;
    write_file(&manifest_output, &manifest_bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": bundle.schema,
            "output": output,
            "manifest": manifest_output,
            "manifest_sha256": manifest.digest()?,
            "sha256": bundle.digest()?,
            "product_id": bundle.content.fingerprint.product_id,
            "files": bundle.input_scan.files.len(),
            "stage_archives": bundle.stages.len(),
            "message_archives": bundle.message_flows.len(),
        }))?
    );
    Ok(())
}

fn extract_stage_data(args: &[String]) -> Result<(), Box<dyn Error>> {
    let archive_path = required_path(args, "--archive")?;
    let resource_name = option(args, "--resource")
        .ok_or_else(|| "missing required --resource <stage.dzs|room.dzr>".to_owned())?;
    let output = required_path(args, "--output")?;
    let archive = fs::read(&archive_path)?;
    let resource = extract_unique_rarc_resource(&archive, &resource_name)?;
    let stage = parse_stage_data(&resource)?;
    let archive_sha256 = Digest(Sha256::digest(&archive).into());
    let resource_sha256 = Digest(Sha256::digest(&resource).into());
    let bytes = serde_json::to_vec_pretty(&json!({
        "schema": EXTRACTED_STAGE_DATA_SCHEMA,
        "archive": archive_path,
        "archive_sha256": archive_sha256,
        "resource": resource_name,
        "resource_sha256": resource_sha256,
        "stage": stage,
    }))?;
    write_file(&output, &bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": EXTRACTED_STAGE_DATA_SCHEMA,
            "output": output,
            "archive_sha256": archive_sha256,
            "resource_sha256": resource_sha256,
            "chunks": stage.chunks.len(),
            "scene_transitions": stage.scene_transitions.len(),
            "actor_placements": stage.actor_placements.len(),
        }))?
    );
    Ok(())
}

fn extract_resource(args: &[String]) -> Result<(), Box<dyn Error>> {
    let archive_path = required_path(args, "--archive")?;
    let resource_name = option(args, "--resource")
        .ok_or_else(|| "missing required --resource <basename>".to_owned())?;
    let output = required_path(args, "--output")?;
    let archive = fs::read(&archive_path)?;
    let resource = extract_unique_rarc_resource(&archive, &resource_name)?;
    let archive_sha256 = Digest(Sha256::digest(&archive).into());
    let resource_sha256 = Digest(Sha256::digest(&resource).into());
    write_file(&output, &resource)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": "dusklight.route-planner.extracted-resource/v1",
            "output": output,
            "archive": archive_path,
            "archive_sha256": archive_sha256,
            "resource": resource_name,
            "resource_sha256": resource_sha256,
            "bytes": resource.len(),
        }))?
    );
    Ok(())
}

fn extract_message_flow(args: &[String]) -> Result<(), Box<dyn Error>> {
    let archive_path = required_path(args, "--archive")?;
    let resource_name = option(args, "--resource")
        .ok_or_else(|| "missing required --resource <basename>".to_owned())?;
    let output = required_path(args, "--output")?;
    let archive = fs::read(&archive_path)?;
    let resource = extract_unique_rarc_resource(&archive, &resource_name)?;
    let flow = parse_message_flow(&resource)?;
    let archive_sha256 = Digest(Sha256::digest(&archive).into());
    let resource_sha256 = Digest(Sha256::digest(&resource).into());
    let bytes = serde_json::to_vec_pretty(&json!({
        "schema": "dusklight.route-planner.extracted-message-flow/v1",
        "archive": archive_path,
        "archive_sha256": archive_sha256,
        "resource": resource_name,
        "resource_sha256": resource_sha256,
        "flow": flow,
    }))?;
    write_file(&output, &bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": "dusklight.route-planner.extracted-message-flow/v1",
            "output": output,
            "archive_sha256": archive_sha256,
            "resource_sha256": resource_sha256,
            "nodes": flow.node_count,
            "labels": flow.labels.len(),
        }))?
    );
    Ok(())
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

fn diff_state_command(args: &[String]) -> Result<(), Box<dyn Error>> {
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

fn project_feasibility_diff(args: &[String]) -> Result<(), Box<dyn Error>> {
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

fn compose(args: &[String]) -> Result<(), Box<dyn Error>> {
    let facts_path = required_path(args, "--facts")?;
    let mechanics_path = required_path(args, "--mechanics")?;
    let output = required_path(args, "--output")?;
    let pack_paths = repeated_option(args, "--pack");
    let route_overlay_paths = repeated_option(args, "--route-overlay");
    let what_if_overlay_paths = repeated_option(args, "--what-if-overlay");
    let facts = FactCatalog::decode_canonical(&fs::read(facts_path)?)?;
    let mechanics = MechanicsCatalog::decode_canonical(&fs::read(mechanics_path)?)?;
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

fn solve_portable(args: &[String]) -> Result<(), Box<dyn Error>> {
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

fn solve_options(args: &[String]) -> Result<RuntimeSolveOptions, Box<dyn Error>> {
    let defaults = RuntimeSolveOptions::default();
    Ok(RuntimeSolveOptions {
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
    })
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

fn load_supported_build_registry(
    args: &[String],
) -> Result<SupportedBuildRegistry, Box<dyn Error>> {
    match option(args, "--registry") {
        Some(path) => Ok(SupportedBuildRegistry::decode_canonical(&fs::read(path)?)?),
        None => Ok(bundled_supported_build_registry()?),
    }
}

fn print_usage() {
    eprintln!(
        "Independent TP route planner:\n  route-planner cache-fact-pack --cache CACHE --payload PAYLOAD.json --manifest MANIFEST.json --receipt RECEIPT.json\n  route-planner compile-cutscene --program PROGRAM.json --output TRANSITIONS.json\n  route-planner compose --facts FACTS.json --mechanics MECHANICS.json [--pack REFINEMENT.json]... [--route-overlay ROUTE.json]... [--what-if-overlay WHAT_IF.json]... --output CATALOG.json\n  route-planner diff-state --before STATE.json --after STATE.json --boundary KIND (--catalog CATALOG.json | --facts FACTS.json) --output DIFF.json [--research]\n  route-planner edit-route-book --route-book BOOK.json --edits EDITS.json (--catalog CATALOG.json | --facts FACTS.json --mechanics MECHANICS.json) --output EDITED.json\n  route-planner extract-message-flow --archive ARCHIVE.arc --resource FILE.bmg --output FLOW.json\n  route-planner extract-orig --orig ORIG_ROOT [--content-identity CONTENT.json | [--registry REGISTRY.json] [--content-id ID]] --output BUNDLE.json --manifest MANIFEST.json\n  route-planner extract-resource --archive ARCHIVE.arc --resource FILE --output FILE\n  route-planner extract-stage-data --archive ARCHIVE.arc --resource stage.dzs|room.dzr --output STAGE.json\n  route-planner extract-world --content-identity CONTENT.json --runtime-configuration RUNTIME.json --world-context CONTEXT.json --inventory INVENTORY.json [--inventory MORE.json] --output FACTS.json --manifest MANIFEST.json\n  route-planner identify-orig --orig ORIG_ROOT [--registry REGISTRY.json] [--content-id ID] --output IDENTIFICATION.json\n  route-planner inspect-state --state STATE.json (--catalog CATALOG.json | --facts FACTS.json) --output INSPECTION.json [--research]\n  route-planner materialize-fact-pack --cache CACHE --manifest-sha256 SHA256 --payload PAYLOAD.json --manifest MANIFEST.json\n  route-planner project-feasibility-diff --state STATE.json (--catalog CATALOG.json | --facts FACTS.json --mechanics MECHANICS.json) [--equivalence-set SET.json]... --output DIFF.json [--research]\n  route-planner project-graph (--catalog CATALOG.json | --facts FACTS.json --mechanics MECHANICS.json) [--route-book BOOK.json] --output GRAPH.json\n  route-planner scan-orig --orig ORIG_ROOT [--product-id ID] --output SCAN.json\n  route-planner state-from-snapshot --snapshot SNAPSHOT.json --output STATE.json\n  route-planner validate-route-book --route-book BOOK.json (--catalog CATALOG.json | --facts FACTS.json --mechanics MECHANICS.json)\n  route-planner solve --state STATE.json (--catalog CATALOG.json | --facts FACTS.json --mechanics MECHANICS.json) --goal ID --output REPORT.json [--route-book BOOK.json] [--max-depth N] [--max-states N] [--max-resolution-combinations N] [--upper-bound] [--research]\n  route-planner solve-portable --state STATE.json [--state STATE.json]... [--equivalence-set SET.json]... --route-book BOOK.json (--catalog CATALOG.json | --facts FACTS.json --mechanics MECHANICS.json) --goal ID --output REPORT.json [--max-depth N] [--max-states N] [--max-resolution-combinations N] [--upper-bound] [--research]\n  route-planner serve-stdio"
    );
}
