use dusklight_route_planner::artifact::Digest;
use dusklight_route_planner::binary_evidence::{
    extract_dol_function_evidence, extract_dol_range_evidence,
};
use dusklight_route_planner::cutscene::CutsceneProgram;
use dusklight_route_planner::cutscene_corruption::compile_actor_corruption_hypothesis;
use dusklight_route_planner::cutscene_import::{
    CutsceneWrapperSourceIdentity, CutsceneWrapperTopology,
};
use dusklight_route_planner::cutscene_outer::{
    CutsceneOuterRuntimeProfile, bundled_gz2e01_cutscene_outer_runtime_profile,
    resolve_cutscene_outer_event,
};
use dusklight_route_planner::cutscene_runtime::{
    CutscenePackageRuntimeProfile, bundled_gz2e01_cutscene_runtime_profile,
    resolve_cutscene_package,
};
use dusklight_route_planner::demo_actor::extract_gz2e01_demo_actor_program;
use dusklight_route_planner::evaluation::{EvidencePolicy, FeasibilityMode};
use dusklight_route_planner::execution::{PlannerExecutionState, PlannerExecutionStateDocument};
use dusklight_route_planner::fact_pack::{
    CoverageDomain, CoverageStatus, ExtractorIdentity, FactPackCoverage, FactPackManifest,
    FactPackSource, SourceArtifactKind,
};
use dusklight_route_planner::fact_pack_cache::{load_fact_pack, store_fact_pack};
use dusklight_route_planner::graph::{PlannerFeasibilityGraphDiff, PlannerGraph};
use dusklight_route_planner::identity::{ContentIdentity, EquivalenceSet, RuntimeConfiguration};
use dusklight_route_planner::jstudio_import::parse_jstudio_stb;
use dusklight_route_planner::jstudio_semantics::{
    JstudioAdaptorProfile, JstudioSemanticProgram, bundled_gz2e01_adaptor_profile,
    resolve_jstudio_stb_semantics,
};
use dusklight_route_planner::logic::FactCatalog;
use dusklight_route_planner::message_flow::{MessageFlowImportProfile, MessageFlowProgramSet};
use dusklight_route_planner::message_import::{
    COMPILED_MESSAGE_FLOW_ENTRY_SET_SCHEMA, COMPILED_MESSAGE_FLOW_SET_SCHEMA,
    CompiledMessageFlowEntrySet, CompiledMessageFlowSet, MessageFlowEntryContractSet,
    MessageFlowResourceOverlaySet,
};
use dusklight_route_planner::orig_diff::compare_orig_bundles;
use dusklight_route_planner::orig_discovery::{
    EXTRACTED_ORIG_BUNDLE_SCHEMA, OrigSupportStatus, SupportedBuildRegistry,
    bundled_supported_build_registry, extract_orig_bundle, scan_orig_tree,
};
use dusklight_route_planner::orig_extraction::{
    EXTRACTED_EVENT_LIST_SCHEMA, EXTRACTED_STAGE_DATA_SCHEMA, extract_unique_rarc_resource,
    list_rarc_resource_names, parse_event_list, parse_message_flow, parse_stage_data,
};
use dusklight_route_planner::refinement::{
    ComposedPlannerCatalog, RefinementLayers, RefinementPack,
};
use dusklight_route_planner::return_place::gz2e01_tower_return_place_mechanics;
use dusklight_route_planner::route_book::{RouteBook, RouteBookEditBatch};
use dusklight_route_planner::snapshot::StateSnapshot;
use dusklight_route_planner::solver::{ForwardSolver, SolverOptions};
use dusklight_route_planner::state::BoundaryKind;
use dusklight_route_planner::title_boundary::gz2e01_reset_to_opening_mechanics;
use dusklight_route_planner::transition::MechanicsCatalog;
use dusklight_route_planner::world_data::{WorldContext, WorldInventory};
use dusklight_route_planner::world_import::{EXTRACTED_WORLD_FACTS_SCHEMA, ExtractedWorldFacts};
use dusklight_route_planner_runtime::inspection::{inspect_state, inspect_state_diff};
use dusklight_route_planner_runtime::service::{
    PlannerServiceEnvelope, error_response, handle_envelope,
};
use dusklight_route_planner_runtime::web::{PlannerWebConfig, serve_web};
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
use std::net::SocketAddr;
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
        Some("compile-cutscene-corruption-hypothesis") => {
            compile_cutscene_corruption_hypothesis_command(&args[1..])
        }
        Some("compile-message-entries") => compile_message_entries(&args[1..]),
        Some("compile-message-flows") => compile_message_flows(&args[1..]),
        Some("compile-return-place-mechanics") => compile_return_place_mechanics(&args[1..]),
        Some("compile-title-boundary-mechanics") => compile_title_boundary_mechanics(&args[1..]),
        Some("construct-message-flows") => construct_message_flows(&args[1..]),
        Some("compose") => compose(&args[1..]),
        Some("diff-orig") => diff_orig(&args[1..]),
        Some("extract-binary-range-evidence") => extract_binary_range_evidence(&args[1..]),
        Some("extract-event-list") => extract_event_list(&args[1..]),
        Some("extract-demo-actor-program") => extract_demo_actor_program(&args[1..]),
        Some("extract-function-evidence") => extract_function_evidence(&args[1..]),
        Some("extract-jstudio-stb") => extract_jstudio_stb(&args[1..]),
        Some("resolve-jstudio-stb") => resolve_jstudio_stb(&args[1..]),
        Some("resolve-cutscene-package") => resolve_cutscene_package_command(&args[1..]),
        Some("resolve-cutscene-outer") => resolve_cutscene_outer_command(&args[1..]),
        Some("extract-cutscene-wrapper") => extract_cutscene_wrapper(&args[1..]),
        Some("extract-message-flow") => extract_message_flow(&args[1..]),
        Some("extract-orig") => extract_orig(&args[1..]),
        Some("extract-resource") => extract_resource(&args[1..]),
        Some("extract-stage-data") => extract_stage_data(&args[1..]),
        Some("extract-world") => extract_world(&args[1..]),
        Some("edit-route-book") => edit_route_book(&args[1..]),
        Some("inspect-state") => inspect_state_command(&args[1..]),
        Some("identify-orig") => identify_orig(&args[1..]),
        Some("list-archive-resources") => list_archive_resources(&args[1..]),
        Some("materialize-fact-pack") => materialize_fact_pack(&args[1..]),
        Some("diff-state") => diff_state_command(&args[1..]),
        Some("project-graph") => project_graph(&args[1..]),
        Some("project-authorization-graph") => project_authorization_graph(&args[1..]),
        Some("project-feasibility-diff") => project_feasibility_diff(&args[1..]),
        Some("serve-stdio") => serve_stdio(&args[1..]),
        Some("serve-web") => serve_web_command(&args[1..]),
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

fn diff_orig(args: &[String]) -> Result<(), Box<dyn Error>> {
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
            "summary": diff.summary,
        }))?
    );
    Ok(())
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

fn compile_return_place_mechanics(args: &[String]) -> Result<(), Box<dyn Error>> {
    let content_path = required_path(args, "--content-identity")?;
    let runtime_path = required_path(args, "--runtime-configuration")?;
    let output = required_path(args, "--output")?;
    let content = ContentIdentity::decode_canonical(&fs::read(content_path)?)?;
    let runtime = RuntimeConfiguration::decode_canonical(&fs::read(runtime_path)?)?;
    let mechanics = gz2e01_tower_return_place_mechanics(&content, &runtime)?;
    write_file(&output, &mechanics.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": mechanics.schema,
            "output": output,
            "sha256": mechanics.digest()?,
            "writers": mechanics.writers.len(),
            "gates": mechanics.gates.len(),
            "readers": mechanics.readers.len(),
            "transitions": mechanics.transitions.len(),
        }))?
    );
    Ok(())
}

fn compile_title_boundary_mechanics(args: &[String]) -> Result<(), Box<dyn Error>> {
    let content_path = required_path(args, "--content-identity")?;
    let runtime_path = required_path(args, "--runtime-configuration")?;
    let output = required_path(args, "--output")?;
    let content = ContentIdentity::decode_canonical(&fs::read(content_path)?)?;
    let runtime = RuntimeConfiguration::decode_canonical(&fs::read(runtime_path)?)?;
    let mechanics = gz2e01_reset_to_opening_mechanics(&content, &runtime)?;
    write_file(&output, &mechanics.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": mechanics.schema,
            "output": output,
            "sha256": mechanics.digest()?,
            "transitions": mechanics.transitions.len(),
        }))?
    );
    Ok(())
}

fn construct_message_flows(args: &[String]) -> Result<(), Box<dyn Error>> {
    let bundle_path = required_path(args, "--bundle")?;
    let runtime_path = required_path(args, "--runtime-configuration")?;
    let profile_path = required_path(args, "--profile")?;
    let output = required_path(args, "--output")?;
    let bundle = dusklight_route_planner::orig_discovery::ExtractedOrigBundle::decode_canonical(
        &fs::read(bundle_path)?,
    )?;
    let runtime = RuntimeConfiguration::decode_canonical(&fs::read(runtime_path)?)?;
    let profile = MessageFlowImportProfile::decode_canonical(&fs::read(profile_path)?)?;
    let set = MessageFlowProgramSet::build(&bundle, &runtime, &profile)?;
    write_file(&output, &set.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": set.schema,
            "output": output,
            "sha256": set.digest()?,
            "profile_sha256": set.profile_sha256,
            "bundle_sha256": set.bundle_sha256,
            "locale_bundle": set.locale_bundle,
            "programs": set.programs.len(),
        }))?
    );
    Ok(())
}

fn compile_message_flows(args: &[String]) -> Result<(), Box<dyn Error>> {
    let bundle_path = required_path(args, "--bundle")?;
    let runtime_path = required_path(args, "--runtime-configuration")?;
    let profile_path = required_path(args, "--profile")?;
    let output = required_path(args, "--output")?;
    let manifest_output = required_path(args, "--manifest")?;
    let bundle = dusklight_route_planner::orig_discovery::ExtractedOrigBundle::decode_canonical(
        &fs::read(bundle_path)?,
    )?;
    let runtime = RuntimeConfiguration::decode_canonical(&fs::read(runtime_path)?)?;
    let profile = MessageFlowImportProfile::decode_canonical(&fs::read(profile_path)?)?;
    let overlays = match option(args, "--overlays") {
        Some(path) => Some(MessageFlowResourceOverlaySet::decode_canonical(&fs::read(
            path,
        )?)?),
        None => None,
    };
    let set = CompiledMessageFlowSet::build(&bundle, &runtime, &profile, overlays.as_ref())?;
    let bytes = set.canonical_bytes()?;
    let mut sources = vec![FactPackSource {
        kind: SourceArtifactKind::SourceAudit,
        id: "message-flow/import-profile".into(),
        sha256: profile.digest()?,
    }];
    if let Some(overlays) = &overlays {
        sources.push(FactPackSource {
            kind: SourceArtifactKind::SourceAudit,
            id: "message-flow/resource-overlays".into(),
            sha256: overlays.digest()?,
        });
    }
    sources.extend(set.resources.iter().map(|resource| FactPackSource {
        kind: SourceArtifactKind::MessageArchive,
        id: format!(
            "message-flow/{}/group-{:03}",
            set.locale_bundle.to_ascii_lowercase(),
            resource.message_group
        ),
        sha256: resource.archive_sha256,
    }));
    let manifest = FactPackManifest::build(
        format!(
            "message-flow.{}.{}",
            set.locale_bundle.to_ascii_lowercase(),
            &set.digest()?.to_string()[..24]
        ),
        bundle.content.clone(),
        ExtractorIdentity {
            name: "route-planner-message-flow".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            executable_sha256: Digest(Sha256::digest(fs::read(env::current_exe()?)?).into()),
            schema_sha256: Digest(Sha256::digest(COMPILED_MESSAGE_FLOW_SET_SCHEMA).into()),
        },
        sources,
        vec![
            FactPackCoverage {
                domain: CoverageDomain::MessageFlows,
                scope: "message-flow".into(),
                status: CoverageStatus::Partial,
                detail: "Every selected FLW1/FLI1 node is retained; known generic handlers compile and unsupported handlers remain explicit unknown requirements.".into(),
            },
            FactPackCoverage {
                domain: CoverageDomain::StorageBindings,
                scope: "message-flow".into(),
                status: CoverageStatus::Partial,
                detail: "The exact import profile supplies known temporary, persistent, and switch layouts; additional handler-owned stores remain open.".into(),
            },
            FactPackCoverage {
                domain: CoverageDomain::HardGuards,
                scope: "message-flow".into(),
                status: CoverageStatus::Partial,
                detail: "Known branch predicates are executable; actor entry, interaction, and unsupported event guards remain separate.".into(),
            },
            FactPackCoverage {
                domain: CoverageDomain::PhysicalFeasibility,
                scope: "message-flow".into(),
                status: CoverageStatus::Unavailable,
                detail: "Message resources do not establish actor reachability, trigger geometry, interruption timing, or player control.".into(),
            },
        ],
        COMPILED_MESSAGE_FLOW_SET_SCHEMA,
        set.digest()?,
    )?;
    manifest.verify_payload(&bytes)?;
    write_file(&output, &bytes)?;
    write_file(&manifest_output, &manifest.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": set.schema,
            "output": output,
            "manifest": manifest_output,
            "manifest_sha256": manifest.digest()?,
            "sha256": set.digest()?,
            "program_set_sha256": set.program_set_sha256,
            "overlay_set_sha256": set.overlay_set_sha256,
            "locale_bundle": set.locale_bundle,
            "resources": set.resources.len(),
            "aliases": set.facts.aliases.len(),
            "transitions": set.mechanics.transitions.len(),
            "readers": set.mechanics.readers.len(),
        }))?
    );
    Ok(())
}

fn compile_message_entries(args: &[String]) -> Result<(), Box<dyn Error>> {
    let bundle_path = required_path(args, "--bundle")?;
    let message_flow_set_path = required_path(args, "--message-flow-set")?;
    let contracts_path = required_path(args, "--contracts")?;
    let output = required_path(args, "--output")?;
    let manifest_output = required_path(args, "--manifest")?;
    let bundle = dusklight_route_planner::orig_discovery::ExtractedOrigBundle::decode_canonical(
        &fs::read(bundle_path)?,
    )?;
    let message_flow_set =
        CompiledMessageFlowSet::decode_canonical(&fs::read(message_flow_set_path)?)?;
    let contracts = MessageFlowEntryContractSet::decode_canonical(&fs::read(contracts_path)?)?;
    let artifact = contracts.compile(&bundle, &message_flow_set)?;
    let bytes = artifact.canonical_bytes()?;

    let mut sources = vec![
        FactPackSource {
            kind: SourceArtifactKind::SourceAudit,
            id: "message-entry/contracts".into(),
            sha256: contracts.digest()?,
        },
        FactPackSource {
            kind: SourceArtifactKind::SourceAudit,
            id: "message-entry/compiled-message-flow-set".into(),
            sha256: message_flow_set.digest()?,
        },
    ];
    let mut stage_paths = Vec::new();
    for entry in &contracts.entries {
        stage_paths.push(entry.stage_archive_path.as_str());
        if let Some(placement) = &entry.speaker.placement {
            stage_paths.push(placement.archive_path.as_str());
        }
    }
    stage_paths.sort_unstable();
    stage_paths.dedup();
    for (index, path) in stage_paths.into_iter().enumerate() {
        let stage = bundle
            .stages
            .iter()
            .find(|stage| stage.relative_path == path)
            .ok_or("validated entry stage disappeared from the extracted bundle")?;
        sources.push(FactPackSource {
            kind: SourceArtifactKind::StageArchive,
            id: format!("message-entry/stage-{index:05}"),
            sha256: stage.archive_sha256,
        });
    }
    let unresolved_entries = contracts
        .entries
        .iter()
        .filter(|entry| !entry.unknown_requirements.is_empty())
        .count();
    let manifest = FactPackManifest::build(
        format!(
            "message-entry.{}",
            &artifact.digest()?.to_string()[..24]
        ),
        bundle.content.clone(),
        ExtractorIdentity {
            name: "route-planner-message-entry".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            executable_sha256: Digest(Sha256::digest(fs::read(env::current_exe()?)?).into()),
            schema_sha256: Digest(
                Sha256::digest(COMPILED_MESSAGE_FLOW_ENTRY_SET_SCHEMA).into(),
            ),
        },
        sources,
        vec![
            FactPackCoverage {
                domain: CoverageDomain::MessageFlows,
                scope: "message-entry".into(),
                status: CoverageStatus::Partial,
                detail: "Every authored entry is pinned to an exact compiled flow label; only audited actor and non-actor callers are present.".into(),
            },
            FactPackCoverage {
                domain: CoverageDomain::ActorPlacements,
                scope: "message-entry".into(),
                status: CoverageStatus::Partial,
                detail: "Actor-backed entries reproduce one raw placement record from the exact stage resource; caller reconstruction remains separately authored.".into(),
            },
            FactPackCoverage {
                domain: CoverageDomain::HardGuards,
                scope: "message-entry".into(),
                status: CoverageStatus::Partial,
                detail: "Stage, room, layer, and authored guards compile into entry transitions; unaudited conditions remain explicit unknown requirements.".into(),
            },
            FactPackCoverage {
                domain: CoverageDomain::PhysicalFeasibility,
                scope: "message-entry".into(),
                status: CoverageStatus::Partial,
                detail: "Authored interaction obligations are retained without inferring reachability from placement alone.".into(),
            },
        ],
        COMPILED_MESSAGE_FLOW_ENTRY_SET_SCHEMA,
        artifact.digest()?,
    )?;
    manifest.verify_payload(&bytes)?;
    write_file(&output, &bytes)?;
    write_file(&manifest_output, &manifest.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": artifact.schema,
            "output": output,
            "manifest": manifest_output,
            "manifest_sha256": manifest.digest()?,
            "sha256": artifact.digest()?,
            "contract_set_sha256": contracts.digest()?,
            "compiled_message_flow_set_sha256": contracts.compiled_message_flow_set_sha256,
            "entries": artifact.resolved_entries.len(),
            "transitions": artifact.mechanics.transitions.len(),
            "obligations": artifact.mechanics.obligations.len(),
            "entries_with_unknown_requirements": unresolved_entries,
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

fn list_archive_resources(args: &[String]) -> Result<(), Box<dyn Error>> {
    let archive_path = required_path(args, "--archive")?;
    let output = required_path(args, "--output")?;
    let archive = fs::read(&archive_path)?;
    let archive_sha256 = Digest(Sha256::digest(&archive).into());
    let resource_names = list_rarc_resource_names(&archive)?;
    let bytes = serde_json::to_vec_pretty(&json!({
        "schema": "dusklight.route-planner.rarc-resource-list/v1",
        "archive_sha256": archive_sha256,
        "resource_names": resource_names,
    }))?;
    write_file(&output, &bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": "dusklight.route-planner.rarc-resource-list/v1",
            "output": output,
            "archive_sha256": archive_sha256,
            "resources": resource_names.len(),
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
    sources.extend(bundle.ignored_archives.iter().map(|record| FactPackSource {
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
            "ignored_archives": bundle.ignored_archives.len(),
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
            "map_events": stage.map_events.len(),
            "demo_archive_banks": stage.demo_archive_banks.len(),
            "actor_placements": stage.actor_placements.len(),
        }))?
    );
    Ok(())
}

fn extract_function_evidence(args: &[String]) -> Result<(), Box<dyn Error>> {
    let dol_path = required_path(args, "--dol")?;
    let symbols_path = required_path(args, "--symbols")?;
    let symbol = option(args, "--symbol")
        .ok_or_else(|| "missing required --symbol <exact-name>".to_owned())?;
    let output = required_path(args, "--output")?;
    let evidence =
        extract_dol_function_evidence(&fs::read(&dol_path)?, &fs::read(&symbols_path)?, &symbol)?;
    write_file(&output, &evidence.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": evidence.schema,
            "output": output,
            "sha256": evidence.digest()?,
            "symbol": evidence.symbol,
            "virtual_address": evidence.virtual_address,
            "function_size": evidence.function_size,
            "file_offset": evidence.file_offset,
            "code_sha256": evidence.code_sha256,
            "shape": evidence.shape,
        }))?
    );
    Ok(())
}

fn extract_binary_range_evidence(args: &[String]) -> Result<(), Box<dyn Error>> {
    let dol_path = required_path(args, "--dol")?;
    let virtual_address = required_u32(args, "--virtual-address")?;
    let byte_size = required_u32(args, "--size")?;
    let output = required_path(args, "--output")?;
    let evidence = extract_dol_range_evidence(&fs::read(&dol_path)?, virtual_address, byte_size)?;
    write_file(&output, &evidence.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": evidence.schema,
            "output": output,
            "sha256": evidence.digest()?,
            "virtual_address": evidence.virtual_address,
            "byte_size": evidence.byte_size,
            "section_kind": evidence.section_kind,
            "section_index": evidence.section_index,
            "file_offset": evidence.file_offset,
            "bytes_sha256": evidence.bytes_sha256,
        }))?
    );
    Ok(())
}

fn extract_jstudio_stb(args: &[String]) -> Result<(), Box<dyn Error>> {
    let archive_path = required_path(args, "--archive")?;
    let resource_name = option(args, "--resource")
        .ok_or_else(|| "missing required --resource <file.stb>".to_owned())?;
    let output = required_path(args, "--output")?;
    let archive = fs::read(&archive_path)?;
    let resource = extract_unique_rarc_resource(&archive, &resource_name)?;
    let program = parse_jstudio_stb(
        Digest(Sha256::digest(&archive).into()),
        &resource_name,
        &resource,
    )?;
    write_file(&output, &program.canonical_bytes()?)?;
    let object_count = program
        .blocks
        .iter()
        .filter(|block| {
            matches!(
                block.body,
                dusklight_route_planner::jstudio_import::JstudioStbBlockBody::Object { .. }
            )
        })
        .count();
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": program.schema,
            "output": output,
            "sha256": program.digest()?,
            "archive_sha256": program.source.archive_sha256,
            "resource_sha256": program.source.resource_sha256,
            "blocks": program.blocks.len(),
            "objects": object_count,
            "coverage": program.coverage,
        }))?
    );
    Ok(())
}

fn extract_demo_actor_program(args: &[String]) -> Result<(), Box<dyn Error>> {
    let archive_path = required_path(args, "--archive")?;
    let resource_name = option(args, "--resource")
        .ok_or_else(|| "missing required --resource <file.stb>".to_owned())?;
    let content_path = required_path(args, "--content-identity")?;
    let output = required_path(args, "--output")?;
    let archive = fs::read(&archive_path)?;
    let resource = extract_unique_rarc_resource(&archive, &resource_name)?;
    let content = ContentIdentity::decode_canonical(&fs::read(content_path)?)?;
    let program = extract_gz2e01_demo_actor_program(
        &content,
        Digest(Sha256::digest(&archive).into()),
        &resource_name,
        &resource,
    )?;
    write_file(&output, &program.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": program.schema,
            "output": output,
            "sha256": program.digest()?,
            "source_program_sha256": program.source_program_sha256,
            "source_resource_sha256": program.source_resource_sha256,
            "coverage": program.coverage,
        }))?
    );
    Ok(())
}

fn resolve_jstudio_stb(args: &[String]) -> Result<(), Box<dyn Error>> {
    let archive_path = required_path(args, "--archive")?;
    let resource_name = option(args, "--resource")
        .ok_or_else(|| "missing required --resource <file.stb>".to_owned())?;
    let content_path = required_path(args, "--content-identity")?;
    let output = required_path(args, "--output")?;
    let archive = fs::read(&archive_path)?;
    let resource = extract_unique_rarc_resource(&archive, &resource_name)?;
    let content = ContentIdentity::decode_canonical(&fs::read(content_path)?)?;
    let profile = match option(args, "--profile") {
        Some(profile_path) => JstudioAdaptorProfile::decode_canonical(&fs::read(profile_path)?)?,
        None => bundled_gz2e01_adaptor_profile()?,
    };
    let program = resolve_jstudio_stb_semantics(
        &content,
        &profile,
        Digest(Sha256::digest(&archive).into()),
        &resource_name,
        &resource,
    )?;
    write_file(&output, &program.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": program.schema,
            "output": output,
            "sha256": program.digest()?,
            "source_program_sha256": program.source_program_sha256,
            "profile_sha256": program.profile_sha256,
            "coverage": program.coverage,
        }))?
    );
    Ok(())
}

fn resolve_cutscene_package_command(args: &[String]) -> Result<(), Box<dyn Error>> {
    let content_path = required_path(args, "--content-identity")?;
    let topology_path = required_path(args, "--topology")?;
    let semantics_path = required_path(args, "--semantics")?;
    let output = required_path(args, "--output")?;
    let content = ContentIdentity::decode_canonical(&fs::read(content_path)?)?;
    let topology = CutsceneWrapperTopology::decode_canonical(&fs::read(topology_path)?)?;
    let semantics = JstudioSemanticProgram::decode_canonical(&fs::read(semantics_path)?)?;
    let profile = match option(args, "--profile") {
        Some(profile_path) => {
            CutscenePackageRuntimeProfile::decode_canonical(&fs::read(profile_path)?)?
        }
        None => bundled_gz2e01_cutscene_runtime_profile()?,
    };
    let package = resolve_cutscene_package(&content, &topology, &semantics, &profile)?;
    write_file(&output, &package.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": package.schema,
            "output": output,
            "sha256": package.digest()?,
            "event_name": package.event_name,
            "demo_archive_name": package.demo_archive_name,
            "stb_file": package.stb_file,
            "coverage": package.coverage,
        }))?
    );
    Ok(())
}

fn resolve_cutscene_outer_command(args: &[String]) -> Result<(), Box<dyn Error>> {
    let content_path = required_path(args, "--content-identity")?;
    let runtime_path = required_path(args, "--runtime-configuration")?;
    let topology_path = required_path(args, "--topology")?;
    let package_path = required_path(args, "--package")?;
    let stage_resource_path = required_path(args, "--stage-resource-file")?;
    let event_list_resource_path = required_path(args, "--event-list-resource-file")?;
    let output = required_path(args, "--output")?;
    let content = ContentIdentity::decode_canonical(&fs::read(content_path)?)?;
    let runtime = RuntimeConfiguration::decode_canonical(&fs::read(runtime_path)?)?;
    let topology = CutsceneWrapperTopology::decode_canonical(&fs::read(topology_path)?)?;
    let package =
        dusklight_route_planner::cutscene_runtime::ResolvedCutscenePackage::decode_canonical(
            &fs::read(package_path)?,
        )?;
    let stage_resource = fs::read(stage_resource_path)?;
    let event_list_resource = fs::read(event_list_resource_path)?;
    let stage = parse_stage_data(&stage_resource)?;
    let event_list = parse_event_list(&event_list_resource)?;
    let profile = match option(args, "--profile") {
        Some(profile_path) => {
            CutsceneOuterRuntimeProfile::decode_canonical(&fs::read(profile_path)?)?
        }
        None => bundled_gz2e01_cutscene_outer_runtime_profile()?,
    };
    let resolved = resolve_cutscene_outer_event(
        &content,
        &runtime,
        &topology,
        &package,
        topology.source.stage_archive_sha256,
        &stage_resource,
        &event_list_resource,
        &stage,
        &event_list,
        &profile,
    )?;
    write_file(&output, &resolved.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": resolved.schema,
            "output": output,
            "sha256": resolved.digest()?,
            "event_name": resolved.event_name,
            "event_finish_flags": resolved.event_finish_flags,
            "skip_cut_enabled": resolved.skip_cut_enabled,
            "skip_cut_type": resolved.skip_cut_type,
            "transitions": resolved.transitions.len(),
            "coverage": resolved.coverage,
        }))?
    );
    Ok(())
}

fn compile_cutscene_corruption_hypothesis_command(args: &[String]) -> Result<(), Box<dyn Error>> {
    let content_path = required_path(args, "--content-identity")?;
    let runtime_path = required_path(args, "--runtime-configuration")?;
    let outer_path = required_path(args, "--outer-event")?;
    let output = required_path(args, "--output")?;
    let content = ContentIdentity::decode_canonical(&fs::read(content_path)?)?;
    let runtime = RuntimeConfiguration::decode_canonical(&fs::read(runtime_path)?)?;
    let outer =
        dusklight_route_planner::cutscene_outer::ResolvedCutsceneOuterEvent::decode_canonical(
            &fs::read(outer_path)?,
        )?;
    let profile = match option(args, "--outer-profile") {
        Some(profile_path) => {
            CutsceneOuterRuntimeProfile::decode_canonical(&fs::read(profile_path)?)?
        }
        None => bundled_gz2e01_cutscene_outer_runtime_profile()?,
    };
    let hypothesis = compile_actor_corruption_hypothesis(&content, &runtime, &outer, &profile)?;
    write_file(&output, &hypothesis.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": hypothesis.schema,
            "output": output,
            "sha256": hypothesis.digest()?,
            "id": hypothesis.id,
            "producer_transition": hypothesis.producer.id,
            "unknown_requirements": hypothesis.producer.activation.unknown_requirements.len(),
            "coverage": hypothesis.coverage,
        }))?
    );
    Ok(())
}

fn extract_cutscene_wrapper(args: &[String]) -> Result<(), Box<dyn Error>> {
    let archive_path = required_path(args, "--archive")?;
    let stage_resource_name = option(args, "--stage-resource").unwrap_or_else(|| "room.dzr".into());
    let event_list_resource_name =
        option(args, "--event-list-resource").unwrap_or_else(|| "event_list.dat".into());
    let event_name = option(args, "--event-name")
        .ok_or_else(|| "missing required --event-name <name>".to_owned())?;
    let layer = option(args, "--layer")
        .ok_or_else(|| "missing required --layer <0..255>".to_owned())?
        .parse::<u8>()?;
    let output = required_path(args, "--output")?;
    let archive = fs::read(&archive_path)?;
    let stage_resource = extract_unique_rarc_resource(&archive, &stage_resource_name)?;
    let event_list_resource = extract_unique_rarc_resource(&archive, &event_list_resource_name)?;
    let stage = parse_stage_data(&stage_resource)?;
    let event_list = parse_event_list(&event_list_resource)?;
    let topology = CutsceneWrapperTopology::build(
        CutsceneWrapperSourceIdentity {
            stage_archive_sha256: Digest(Sha256::digest(&archive).into()),
            stage_resource_sha256: Digest(Sha256::digest(&stage_resource).into()),
            event_list_resource_sha256: Digest(Sha256::digest(&event_list_resource).into()),
        },
        &stage,
        &event_list,
        &event_name,
        layer,
    )?;
    write_file(&output, &topology.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": topology.schema,
            "output": output,
            "sha256": topology.digest()?,
            "event_name": topology.event_name,
            "demo_archive_name": topology.demo_archive_name,
            "package_stb_file": topology.package_stb_file,
            "normal_exit": topology.normal_exit,
            "skip_exit": topology.skip_exit,
            "coverage": topology.coverage,
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

fn extract_event_list(args: &[String]) -> Result<(), Box<dyn Error>> {
    let archive_path = required_path(args, "--archive")?;
    let resource_name = option(args, "--resource").unwrap_or_else(|| "event_list.dat".into());
    let output = required_path(args, "--output")?;
    let archive = fs::read(&archive_path)?;
    let resource = extract_unique_rarc_resource(&archive, &resource_name)?;
    let event_list = parse_event_list(&resource)?;
    let archive_sha256 = Digest(Sha256::digest(&archive).into());
    let resource_sha256 = Digest(Sha256::digest(&resource).into());
    let bytes = serde_json::to_vec_pretty(&json!({
        "schema": EXTRACTED_EVENT_LIST_SCHEMA,
        "archive": archive_path,
        "archive_sha256": archive_sha256,
        "resource": resource_name,
        "resource_sha256": resource_sha256,
        "event_list": event_list,
    }))?;
    write_file(&output, &bytes)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": EXTRACTED_EVENT_LIST_SCHEMA,
            "output": output,
            "archive_sha256": archive_sha256,
            "resource_sha256": resource_sha256,
            "events": event_list.events.len(),
            "staff": event_list.staff.len(),
            "cuts": event_list.cuts.len(),
            "data": event_list.data.len(),
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

fn serve_web_command(args: &[String]) -> Result<(), Box<dyn Error>> {
    let listen = match args {
        [] => "127.0.0.1:32170".parse::<SocketAddr>()?,
        [flag, value] if flag == "--listen" => value.parse::<SocketAddr>()?,
        _ => return Err("serve-web accepts only --listen HOST:PORT".into()),
    };
    if !listen.ip().is_loopback() {
        return Err("serve-web currently accepts only a loopback listen address".into());
    }
    println!("route planner: http://{listen}");
    serve_web(PlannerWebConfig { listen })?;
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

fn project_authorization_graph(args: &[String]) -> Result<(), Box<dyn Error>> {
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

fn compose(args: &[String]) -> Result<(), Box<dyn Error>> {
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

fn required_u32(args: &[String], name: &str) -> Result<u32, Box<dyn Error>> {
    let value = option(args, name).ok_or_else(|| format!("missing required {name} <integer>"))?;
    let parsed = if let Some(hex) = value.strip_prefix("0x") {
        u32::from_str_radix(hex, 16)
    } else {
        value.parse()
    };
    parsed.map_err(|_| format!("invalid {name} integer: {value}").into())
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
        "{}",
        [
            "Independent TP route planner:",
            "  route-planner cache-fact-pack --cache CACHE --payload PAYLOAD.json --manifest MANIFEST.json --receipt RECEIPT.json",
            "  route-planner compile-cutscene --program PROGRAM.json --output TRANSITIONS.json",
            "  route-planner compile-message-entries --bundle BUNDLE.json --message-flow-set COMPILED.json --contracts ENTRIES.json --output COMPILED_ENTRIES.json --manifest MANIFEST.json",
            "  route-planner compile-message-flows --bundle BUNDLE.json --runtime-configuration RUNTIME.json --profile PROFILE.json [--overlays OVERLAYS.json] --output COMPILED.json --manifest MANIFEST.json",
            "  route-planner compile-return-place-mechanics --content-identity CONTENT.json --runtime-configuration RUNTIME.json --output MECHANICS.json",
            "  route-planner compile-title-boundary-mechanics --content-identity CONTENT.json --runtime-configuration RUNTIME.json --output MECHANICS.json",
            "  route-planner construct-message-flows --bundle BUNDLE.json --runtime-configuration RUNTIME.json --profile PROFILE.json --output PROGRAMS.json",
            "  route-planner compose --facts FACTS.json --mechanics MECHANICS.json [--message-flow-set MESSAGE.json]... [--message-entry-set ENTRIES.json]... [--pack REFINEMENT.json]... [--route-overlay ROUTE.json]... [--what-if-overlay WHAT_IF.json]... --output CATALOG.json",
            "  route-planner diff-orig --left LEFT.json --right RIGHT.json [--left-locale LOCALE --right-locale LOCALE] --output DIFF.json",
            "  route-planner diff-state --before STATE.json --after STATE.json --boundary KIND (--catalog CATALOG.json | --facts FACTS.json) --output DIFF.json [--research]",
            "  route-planner edit-route-book --route-book BOOK.json --edits EDITS.json (--catalog CATALOG.json | --facts FACTS.json --mechanics MECHANICS.json) --output EDITED.json",
            "  route-planner extract-binary-range-evidence --dol main.dol --virtual-address ADDRESS --size BYTES --output EVIDENCE.json",
            "  route-planner extract-event-list --archive ARCHIVE.arc [--resource event_list.dat] --output EVENTS.json",
            "  route-planner extract-demo-actor-program --archive ARCHIVE.arc --resource FILE.stb --content-identity CONTENT.json --output PROGRAM.json",
            "  route-planner extract-function-evidence --dol main.dol --symbols symbols.txt --symbol EXACT_NAME --output EVIDENCE.json",
            "  route-planner extract-jstudio-stb --archive ARCHIVE.arc --resource FILE.stb --output PROGRAM.json",
            "  route-planner resolve-jstudio-stb --archive ARCHIVE.arc --resource FILE.stb --content-identity CONTENT.json [--profile PROFILE.json] --output SEMANTICS.json",
            "  route-planner resolve-cutscene-package --content-identity CONTENT.json --topology WRAPPER.json --semantics SEMANTICS.json [--profile PROFILE.json] --output PACKAGE.json",
            "  route-planner resolve-cutscene-outer --content-identity CONTENT.json --runtime-configuration RUNTIME.json --topology WRAPPER.json --package PACKAGE.json --stage-resource-file room.dzr --event-list-resource-file event_list.dat [--profile PROFILE.json] --output OUTER.json",
            "  route-planner compile-cutscene-corruption-hypothesis --content-identity CONTENT.json --runtime-configuration RUNTIME.json --outer-event OUTER.json [--outer-profile PROFILE.json] --output HYPOTHESIS.json",
            "  route-planner extract-cutscene-wrapper --archive ARCHIVE.arc [--stage-resource room.dzr] [--event-list-resource event_list.dat] --event-name NAME --layer LAYER --output WRAPPER.json",
            "  route-planner extract-message-flow --archive ARCHIVE.arc --resource FILE.bmg --output FLOW.json",
            "  route-planner extract-orig --orig ORIG_ROOT [--content-identity CONTENT.json | [--registry REGISTRY.json] [--content-id ID]] --output BUNDLE.json --manifest MANIFEST.json",
            "  route-planner extract-resource --archive ARCHIVE.arc --resource FILE --output FILE",
            "  route-planner extract-stage-data --archive ARCHIVE.arc --resource stage.dzs|room.dzr --output STAGE.json",
            "  route-planner extract-world --content-identity CONTENT.json --runtime-configuration RUNTIME.json --world-context CONTEXT.json --inventory INVENTORY.json [--inventory MORE.json] --output FACTS.json --manifest MANIFEST.json",
            "  route-planner identify-orig --orig ORIG_ROOT [--registry REGISTRY.json] [--content-id ID] --output IDENTIFICATION.json",
            "  route-planner inspect-state --state STATE.json (--catalog CATALOG.json | --facts FACTS.json) --output INSPECTION.json [--research]",
            "  route-planner list-archive-resources --archive ARCHIVE.arc --output RESOURCES.json",
            "  route-planner materialize-fact-pack --cache CACHE --manifest-sha256 SHA256 --payload PAYLOAD.json --manifest MANIFEST.json",
            "  route-planner project-authorization-graph --state STATE.json (--catalog CATALOG.json | --facts FACTS.json --mechanics MECHANICS.json) [--equivalence-set SET.json]... --output GRAPH.json [--max-depth N] [--max-states N] [--max-resolution-combinations N] [--research]",
            "  route-planner project-feasibility-diff --state STATE.json (--catalog CATALOG.json | --facts FACTS.json --mechanics MECHANICS.json) [--equivalence-set SET.json]... --output DIFF.json [--research]",
            "  route-planner project-graph (--catalog CATALOG.json | --facts FACTS.json --mechanics MECHANICS.json) [--route-book BOOK.json] --output GRAPH.json",
            "  route-planner scan-orig --orig ORIG_ROOT [--product-id ID] --output SCAN.json",
            "  route-planner state-from-snapshot --snapshot SNAPSHOT.json --output STATE.json",
            "  route-planner validate-route-book --route-book BOOK.json (--catalog CATALOG.json | --facts FACTS.json --mechanics MECHANICS.json)",
            "  route-planner solve --state STATE.json (--catalog CATALOG.json | --facts FACTS.json --mechanics MECHANICS.json) --goal ID --output REPORT.json [--route-book BOOK.json] [--max-depth N] [--max-states N] [--max-resolution-combinations N] [--upper-bound] [--research]",
            "  route-planner solve-portable --state STATE.json [--state STATE.json]... [--equivalence-set SET.json]... --route-book BOOK.json (--catalog CATALOG.json | --facts FACTS.json --mechanics MECHANICS.json) --goal ID --output REPORT.json [--max-depth N] [--max-states N] [--max-resolution-combinations N] [--upper-bound] [--research]",
            "  route-planner serve-stdio",
            "  route-planner serve-web [--listen 127.0.0.1:32170]",
        ]
        .join("\n")
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_route_planner::authorization::AuthorizationGraph;
    use dusklight_route_planner::identity::RUNTIME_CONFIGURATION_SCHEMA;
    use dusklight_route_planner::logic::FACT_CATALOG_SCHEMA;
    use dusklight_route_planner::snapshot::STATE_SNAPSHOT_SCHEMA;
    use dusklight_route_planner::state::{
        BackingAttachment, EXECUTION_ENVIRONMENT_SCHEMA, ExecutionContext, ExecutionEnvironment,
        PlayerForm, PlayerState, RuntimeFile, RuntimeFileLifecycle, RuntimeFileOrigin,
        SceneLocation,
    };
    use dusklight_route_planner::transition::MECHANICS_CATALOG_SCHEMA;
    use std::collections::BTreeMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn project_authorization_graph_command_writes_a_canonical_base_graph() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = env::temp_dir().join(format!(
            "dusklight-authorization-cli-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let state_path = root.join("state.json");
        let facts_path = root.join("facts.json");
        let mechanics_path = root.join("mechanics.json");
        let output_path = root.join("authorization.json");
        let snapshot = StateSnapshot {
            schema: STATE_SNAPSHOT_SCHEMA.into(),
            id: "snapshot.cli-start".into(),
            sequence: 0,
            environment: ExecutionEnvironment {
                schema: EXECUTION_ENVIRONMENT_SCHEMA.into(),
                runtime_configuration: RuntimeConfiguration {
                    schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
                    content_sha256: Digest([1; 32]),
                    language: "en".into(),
                    settings: BTreeMap::new(),
                },
                active_runtime_file: RuntimeFile {
                    id: "file-0".into(),
                    origin: RuntimeFileOrigin::TitleFile0,
                    backing: BackingAttachment::MemoryOnly,
                    allowed_serialization_targets: Vec::new(),
                    lifecycle: RuntimeFileLifecycle::Active,
                },
                inactive_runtime_files: Vec::new(),
                physical_slots: Vec::new(),
                physical_slot_observations: Vec::new(),
                execution_context: ExecutionContext::World,
                location: SceneLocation {
                    stage: "STAGE_A".into(),
                    room: 0,
                    layer: 0,
                    spawn: 0,
                },
                player: PlayerState {
                    form: PlayerForm::Human,
                    mount: None,
                    position: [0.0; 3],
                    rotation: [0; 3],
                    has_control: Some(true),
                    action: "idle".into(),
                },
                components: Vec::new(),
                static_world_objects: Vec::new(),
                spatial_volumes: Vec::new(),
                spatial_connections: Vec::new(),
                spatial_planes: Vec::new(),
                persisted_object_controls: Vec::new(),
                live_world_objects: Vec::new(),
            },
            semantic_observations: Vec::new(),
        };
        let state = PlannerExecutionState::new(snapshot)
            .unwrap()
            .to_document()
            .unwrap();
        let facts = FactCatalog {
            schema: FACT_CATALOG_SCHEMA.into(),
            aliases: Vec::new(),
            derived_facts: Vec::new(),
        };
        let mechanics = MechanicsCatalog {
            schema: MECHANICS_CATALOG_SCHEMA.into(),
            transitions: Vec::new(),
            obligations: Vec::new(),
            writers: Vec::new(),
            gates: Vec::new(),
            readers: Vec::new(),
            reconstruction_rules: Vec::new(),
            obstructions: Vec::new(),
            resolvers: Vec::new(),
            techniques: Vec::new(),
            microtraces: Vec::new(),
            goals: Vec::new(),
        };
        fs::write(&state_path, state.canonical_bytes().unwrap()).unwrap();
        fs::write(&facts_path, facts.canonical_bytes().unwrap()).unwrap();
        fs::write(&mechanics_path, mechanics.canonical_bytes().unwrap()).unwrap();

        project_authorization_graph(&[
            "--state".into(),
            state_path.to_string_lossy().into_owned(),
            "--facts".into(),
            facts_path.to_string_lossy().into_owned(),
            "--mechanics".into(),
            mechanics_path.to_string_lossy().into_owned(),
            "--output".into(),
            output_path.to_string_lossy().into_owned(),
            "--max-depth".into(),
            "4".into(),
            "--max-states".into(),
            "8".into(),
        ])
        .unwrap();
        let graph = AuthorizationGraph::decode_canonical(&fs::read(&output_path).unwrap()).unwrap();
        assert!(graph.traversal_complete);
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.evaluated_states, 1);
        assert!(graph.edges.is_empty());
        assert!(graph.unknown_activation_candidates.is_empty());
        assert_eq!(graph.refinement_stack_sha256, None);
        fs::remove_dir_all(root).unwrap();
    }
}
