use dusklight_route_planner::artifact::Digest;
use dusklight_route_planner::binary_evidence::{
    extract_dol_function_evidence, extract_dol_range_evidence,
};
use dusklight_route_planner::builtin_refinement::{
    bundled_refinement_pack, bundled_refinement_pack_ids,
};
use dusklight_route_planner::citation::EvidenceCitationIndex;
use dusklight_route_planner::component_catalog::ComponentBoundaryCatalog;
use dusklight_route_planner::coverage_report::ExtractionCoverageReport;
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
use dusklight_route_planner::gcm::extract_gamecube_disc;
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
use dusklight_route_planner::obligation_coverage::ObligationCoverageReport;
use dusklight_route_planner::orig_diff::compare_orig_bundles;
use dusklight_route_planner::orig_discovery::{
    EXTRACTED_ORIG_BUNDLE_SCHEMA, OrigSupportStatus, SupportedBuildRegistry,
    bundled_supported_build_registry, extract_orig_bundle, scan_orig_tree,
};
use dusklight_route_planner::orig_extraction::{
    EXTRACTED_EVENT_LIST_SCHEMA, EXTRACTED_STAGE_DATA_SCHEMA, extract_unique_rarc_resource,
    list_rarc_resource_names, parse_event_list, parse_message_flow, parse_stage_data,
};
use dusklight_route_planner::orig_world::ExtractedOrigWorldInventories;
use dusklight_route_planner::refinement::{
    ComposedPlannerCatalog, RefinementDiagnostic, RefinementLayers, RefinementPack,
    diagnose_refinement_packs,
};
use dusklight_route_planner::return_place::gz2e01_tower_return_place_mechanics;
use dusklight_route_planner::return_restart_audit::ReturnRestartAudit;
use dusklight_route_planner::route_book::{RouteBook, RouteBookEditBatch};
use dusklight_route_planner::route_evidence_coverage::RouteEvidenceCoverageReport;
use dusklight_route_planner::route_observation::{
    PlannedEdgeObservationManifest, RouteObservationMatchReport,
};
use dusklight_route_planner::route_observation_validation::RouteObservationValidationReport;
use dusklight_route_planner::route_suite_coverage::{RouteSuiteCoverageReport, RouteSuiteKind};
use dusklight_route_planner::scene_change_audit::SceneChangeConsumerAudit;
use dusklight_route_planner::snapshot::StateSnapshot;
use dusklight_route_planner::solver::{ForwardSolver, SolverOptions};
use dusklight_route_planner::state::{BoundaryKind, BoundaryPolicy};
use dusklight_route_planner::title_boundary::gz2e01_reset_to_opening_mechanics;
use dusklight_route_planner::transition::MechanicsCatalog;
use dusklight_route_planner::witness_promotion::{
    WitnessPromotionRequest, promote_witnessed_actions,
};
use dusklight_route_planner::world_data::{WorldContext, WorldInventory};
use dusklight_route_planner::world_import::{EXTRACTED_WORLD_FACTS_SCHEMA, ExtractedWorldFacts};
use dusklight_route_planner_runtime::context_compare::compare_semantic_contexts;
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

#[path = "main/catalog_commands.rs"]
mod catalog_commands;
#[path = "main/extraction_commands.rs"]
mod extraction_commands;
#[path = "main/planning_commands.rs"]
mod planning_commands;
use catalog_commands::*;
use extraction_commands::*;
use planning_commands::*;

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
        Some("catalog-state-boundaries") => catalog_state_boundaries(&args[1..]),
        Some("compile-cutscene") => compile_cutscene(&args[1..]),
        Some("compile-cutscene-corruption-hypothesis") => {
            compile_cutscene_corruption_hypothesis_command(&args[1..])
        }
        Some("compile-message-entries") => compile_message_entries(&args[1..]),
        Some("compile-message-flows") => compile_message_flows(&args[1..]),
        Some("compile-return-place-mechanics") => compile_return_place_mechanics(&args[1..]),
        Some("compile-title-boundary-mechanics") => compile_title_boundary_mechanics(&args[1..]),
        Some("construct-message-flows") => construct_message_flows(&args[1..]),
        Some("construct-world-inventories") => construct_world_inventories(&args[1..]),
        Some("compose") => compose(&args[1..]),
        Some("compare-semantic-contexts") => compare_semantic_contexts_command(&args[1..]),
        Some("diff-orig") => diff_orig(&args[1..]),
        Some("diagnose-refinement-packs") => diagnose_refinement_packs_command(&args[1..]),
        Some("extract-binary-range-evidence") => extract_binary_range_evidence(&args[1..]),
        Some("extract-event-list") => extract_event_list(&args[1..]),
        Some("extract-demo-actor-program") => extract_demo_actor_program(&args[1..]),
        Some("extract-function-evidence") => extract_function_evidence(&args[1..]),
        Some("extract-gcm") => extract_gcm(&args[1..]),
        Some("extract-jstudio-stb") => extract_jstudio_stb(&args[1..]),
        Some("resolve-jstudio-stb") => resolve_jstudio_stb(&args[1..]),
        Some("resolve-cutscene-package") => resolve_cutscene_package_command(&args[1..]),
        Some("resolve-cutscene-outer") => resolve_cutscene_outer_command(&args[1..]),
        Some("extract-cutscene-wrapper") => extract_cutscene_wrapper(&args[1..]),
        Some("extract-message-flow") => extract_message_flow(&args[1..]),
        Some("extract-native-world") => extract_native_world(&args[1..]),
        Some("extract-orig") => extract_orig(&args[1..]),
        Some("extract-resource") => extract_resource(&args[1..]),
        Some("extract-stage-data") => extract_stage_data(&args[1..]),
        Some("extract-world") => extract_world(&args[1..]),
        Some("export-builtin-refinement-pack") => export_builtin_refinement_pack(&args[1..]),
        Some("export-evidence-citations") => export_evidence_citations(&args[1..]),
        Some("export-refinement-pack") => export_refinement_pack(&args[1..]),
        Some("edit-route-book") => edit_route_book(&args[1..]),
        Some("inspect-state") => inspect_state_command(&args[1..]),
        Some("identify-orig") => identify_orig(&args[1..]),
        Some("list-archive-resources") => list_archive_resources(&args[1..]),
        Some("list-builtin-refinement-packs") => list_builtin_refinement_packs(),
        Some("materialize-fact-pack") => materialize_fact_pack(&args[1..]),
        Some("diff-state") => diff_state_command(&args[1..]),
        Some("project-graph") => project_graph(&args[1..]),
        Some("project-authorization-graph") => project_authorization_graph(&args[1..]),
        Some("project-feasibility-diff") => project_feasibility_diff(&args[1..]),
        Some("report-extraction-coverage") => report_extraction_coverage(&args[1..]),
        Some("report-obligation-coverage") => report_obligation_coverage(&args[1..]),
        Some("report-route-evidence-coverage") => report_route_evidence_coverage(&args[1..]),
        Some("report-route-suite-coverage") => report_route_suite_coverage(&args[1..]),
        Some("match-route-observations") => match_route_observations(&args[1..]),
        Some("validate-route-observations") => validate_route_observations(&args[1..]),
        Some("promote-witnessed-actions") => promote_witnessed_actions_command(&args[1..]),
        Some("serve-stdio") => serve_stdio(&args[1..]),
        Some("serve-web") => serve_web_command(&args[1..]),
        Some("state-from-snapshot") => state_from_snapshot(&args[1..]),
        Some("validate-route-book") => validate_route_book(&args[1..]),
        Some("solve") => solve(&args[1..]),
        Some("solve-portable") => solve_portable(&args[1..]),
        Some("scan-orig") => scan_orig(&args[1..]),
        Some("audit-scene-change-consumers") => audit_scene_change_consumers(&args[1..]),
        Some("validate-scene-change-consumer-audit") => {
            validate_scene_change_consumer_audit(&args[1..])
        }
        Some("audit-return-restart-writers") => audit_return_restart_writers(&args[1..]),
        Some("refresh-return-restart-audit-sources") => {
            refresh_return_restart_audit_sources(&args[1..])
        }
        Some("validate-return-restart-audit") => validate_return_restart_audit(&args[1..]),
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
        max_plans: usize_option(args, "--max-plans", defaults.max_plans)?,
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
            "  route-planner audit-scene-change-consumers --source-root SOURCE --content-identity CONTENT.json --output AUDIT.json",
            "  route-planner validate-scene-change-consumer-audit --input AUDIT.json",
            "  route-planner audit-return-restart-writers --repository-root REPOSITORY --bundle BUNDLE.json --output AUDIT.json",
            "  route-planner refresh-return-restart-audit-sources --repository-root REPOSITORY --input AUDIT.json --output AUDIT.json",
            "  route-planner validate-return-restart-audit --input AUDIT.json",
            "  route-planner cache-fact-pack --cache CACHE --payload PAYLOAD.json --manifest MANIFEST.json --receipt RECEIPT.json",
            "  route-planner catalog-state-boundaries --state STATE.json --policy POLICY.json [--policy POLICY.json]... --output CATALOG.json",
            "  route-planner compile-cutscene --program PROGRAM.json --output TRANSITIONS.json",
            "  route-planner compile-message-entries --bundle BUNDLE.json --message-flow-set COMPILED.json --contracts ENTRIES.json --output COMPILED_ENTRIES.json --manifest MANIFEST.json",
            "  route-planner compile-message-flows --bundle BUNDLE.json --runtime-configuration RUNTIME.json --profile PROFILE.json [--overlays OVERLAYS.json] --output COMPILED.json --manifest MANIFEST.json",
            "  route-planner compile-return-place-mechanics --content-identity CONTENT.json --runtime-configuration RUNTIME.json --output MECHANICS.json",
            "  route-planner compile-title-boundary-mechanics --content-identity CONTENT.json --runtime-configuration RUNTIME.json --output MECHANICS.json",
            "  route-planner construct-message-flows --bundle BUNDLE.json --runtime-configuration RUNTIME.json --profile PROFILE.json --output PROGRAMS.json",
            "  route-planner construct-world-inventories --bundle BUNDLE.json --output INVENTORIES.json",
            "  route-planner compose --facts FACTS.json --mechanics MECHANICS.json [--message-flow-set MESSAGE.json]... [--message-entry-set ENTRIES.json]... [--pack REFINEMENT.json]... [--route-overlay ROUTE.json]... [--what-if-overlay WHAT_IF.json]... --output CATALOG.json",
            "  route-planner compare-semantic-contexts --left-state STATE.json --left-catalog CATALOG.json [--left-equivalence-set SET.json]... --right-state STATE.json --right-catalog CATALOG.json [--right-equivalence-set SET.json]... --output REPORT.json [--research]",
            "  route-planner diff-orig --left LEFT.json --right RIGHT.json [--left-locale LOCALE --right-locale LOCALE] --output DIFF.json",
            "  route-planner diagnose-refinement-packs --pack PACK.json [--pack PACK.json]... [--output REPORT.json]",
            "  route-planner diff-state --before STATE.json --after STATE.json --boundary KIND (--catalog CATALOG.json | --facts FACTS.json) --output DIFF.json [--research]",
            "  route-planner edit-route-book --route-book BOOK.json --edits EDITS.json (--catalog CATALOG.json | --facts FACTS.json --mechanics MECHANICS.json) --output EDITED.json",
            "  route-planner extract-binary-range-evidence --dol main.dol --virtual-address ADDRESS --size BYTES --output EVIDENCE.json",
            "  route-planner extract-event-list --archive ARCHIVE.arc [--resource event_list.dat] --output EVENTS.json",
            "  route-planner extract-demo-actor-program --archive ARCHIVE.arc --resource FILE.stb --content-identity CONTENT.json --output PROGRAM.json",
            "  route-planner extract-function-evidence --dol main.dol --symbols symbols.txt --symbol EXACT_NAME --output EVIDENCE.json",
            "  route-planner extract-gcm --iso DISC.iso --output NEW_EXTRACTED_ROOT",
            "  route-planner extract-jstudio-stb --archive ARCHIVE.arc --resource FILE.stb --output PROGRAM.json",
            "  route-planner resolve-jstudio-stb --archive ARCHIVE.arc --resource FILE.stb --content-identity CONTENT.json [--profile PROFILE.json] --output SEMANTICS.json",
            "  route-planner resolve-cutscene-package --content-identity CONTENT.json --topology WRAPPER.json --semantics SEMANTICS.json [--profile PROFILE.json] --output PACKAGE.json",
            "  route-planner resolve-cutscene-outer --content-identity CONTENT.json --runtime-configuration RUNTIME.json --topology WRAPPER.json --package PACKAGE.json --stage-resource-file room.dzr --event-list-resource-file event_list.dat [--profile PROFILE.json] --output OUTER.json",
            "  route-planner compile-cutscene-corruption-hypothesis --content-identity CONTENT.json --runtime-configuration RUNTIME.json --outer-event OUTER.json [--outer-profile PROFILE.json] --output HYPOTHESIS.json",
            "  route-planner extract-cutscene-wrapper --archive ARCHIVE.arc [--stage-resource room.dzr] [--event-list-resource event_list.dat] --event-name NAME --layer LAYER --output WRAPPER.json",
            "  route-planner extract-message-flow --archive ARCHIVE.arc --resource FILE.bmg --output FLOW.json",
            "  route-planner extract-native-world --content-identity CONTENT.json --runtime-configuration RUNTIME.json --inventories INVENTORIES.json --output FACTS.json --manifest MANIFEST.json",
            "  route-planner extract-orig --orig ORIG_ROOT [--content-identity CONTENT.json | [--registry REGISTRY.json] [--content-id ID]] --output BUNDLE.json --manifest MANIFEST.json",
            "  route-planner extract-resource --archive ARCHIVE.arc --resource FILE --output FILE",
            "  route-planner extract-stage-data --archive ARCHIVE.arc --resource stage.dzs|room.dzr --output STAGE.json",
            "  route-planner extract-world --content-identity CONTENT.json --runtime-configuration RUNTIME.json --world-context CONTEXT.json --inventory INVENTORY.json [--inventory MORE.json] --output FACTS.json --manifest MANIFEST.json",
            "  route-planner export-builtin-refinement-pack --id ID --output PACK.json",
            "  route-planner export-evidence-citations --catalog CATALOG.json --input DRAFT.json --output CITATIONS.json",
            "  route-planner export-refinement-pack --input DRAFT.json --output PACK.json",
            "  route-planner identify-orig --orig ORIG_ROOT [--registry REGISTRY.json] [--content-id ID] --output IDENTIFICATION.json",
            "  route-planner inspect-state --state STATE.json (--catalog CATALOG.json | --facts FACTS.json) --output INSPECTION.json [--research]",
            "  route-planner list-archive-resources --archive ARCHIVE.arc --output RESOURCES.json",
            "  route-planner list-builtin-refinement-packs",
            "  route-planner materialize-fact-pack --cache CACHE --manifest-sha256 SHA256 --payload PAYLOAD.json --manifest MANIFEST.json",
            "  route-planner project-authorization-graph --state STATE.json (--catalog CATALOG.json | --facts FACTS.json --mechanics MECHANICS.json) [--equivalence-set SET.json]... --output GRAPH.json [--max-depth N] [--max-states N] [--max-resolution-combinations N] [--research]",
            "  route-planner project-feasibility-diff --state STATE.json (--catalog CATALOG.json | --facts FACTS.json --mechanics MECHANICS.json) [--equivalence-set SET.json]... --output DIFF.json [--research]",
            "  route-planner project-graph (--catalog CATALOG.json | --facts FACTS.json --mechanics MECHANICS.json) [--route-book BOOK.json] --output GRAPH.json",
            "  route-planner report-extraction-coverage --manifest MANIFEST.json [--manifest MANIFEST.json]... --output REPORT.json",
            "  route-planner report-obligation-coverage (--catalog CATALOG.json | --mechanics MECHANICS.json) --output REPORT.json",
            "  route-planner report-route-evidence-coverage --catalog CATALOG.json --route-book BOOK.json [--route-book BOOK.json]... --output REPORT.json [--minimum-route-count N]",
            "  route-planner report-route-suite-coverage --catalog CATALOG.json [--glitchless BOOK.json]... [--hundred-percent BOOK.json]... [--any-percent BOOK.json]... [--hypothetical BOOK.json]... --output REPORT.json",
            "  route-planner match-route-observations --catalog CATALOG.json --route-book BOOK.json --manifest OBSERVATIONS.json --snapshot SNAPSHOT.json [--snapshot SNAPSHOT.json]... --output REPORT.json",
            "  route-planner validate-route-observations --catalog CATALOG.json --route-book BOOK.json --matches MATCHES.json --snapshot SNAPSHOT.json [--snapshot SNAPSHOT.json]... [--equivalence-set SET.json]... [--research] --output REPORT.json",
            "  route-planner promote-witnessed-actions --catalog CATALOG.json --validation VALIDATION.json --request REQUEST.json --output PACK.json --receipt RECEIPT.json",
            "  route-planner scan-orig --orig ORIG_ROOT [--product-id ID] --output SCAN.json",
            "  route-planner state-from-snapshot --snapshot SNAPSHOT.json --output STATE.json",
            "  route-planner validate-route-book --route-book BOOK.json (--catalog CATALOG.json | --facts FACTS.json --mechanics MECHANICS.json)",
            "  route-planner solve --state STATE.json (--catalog CATALOG.json | --facts FACTS.json --mechanics MECHANICS.json) --goal ID --output REPORT.json [--route-book BOOK.json] [--max-depth N] [--max-states N] [--max-resolution-combinations N] [--max-plans N] [--upper-bound] [--research]",
            "  route-planner solve-portable --state STATE.json [--state STATE.json]... [--equivalence-set SET.json]... --route-book BOOK.json (--catalog CATALOG.json | --facts FACTS.json --mechanics MECHANICS.json) --goal ID --output REPORT.json [--max-depth N] [--max-states N] [--max-resolution-combinations N] [--max-plans N] [--upper-bound] [--research]",
            "  route-planner serve-stdio",
            "  route-planner serve-web [--listen 127.0.0.1:32170] [--projects DIR]",
        ]
        .join("\n")
    );
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
