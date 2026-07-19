use huntctl::benchmark::skybook::SkybookManifest;
use huntctl::benchmark::skybook_selection::{SkybookSelection, SkybookSelectionDisposition};
use huntctl::calibration::calibrate_fitted_q;
use huntctl::client::{CONTROL_PROTOCOL_NAME, CONTROL_PROTOCOL_VERSION, WorkerClient};
use huntctl::comparison_oracle::{ComparisonEvidence, ComparisonOracleProgram};
use huntctl::content_store::{ContentKind, ContentStore};
use huntctl::continuous_search::{ContinuousAxes, ContinuousMethod};
use huntctl::controller_compilation::{ControllerObservationProvenance, compile_static_controller};
use huntctl::controller_program::ControllerProgram;
use huntctl::corpus::Corpus;
use huntctl::dataset::{
    DATASET_SOURCE_SCHEMA_V1, DatasetBuildConfig, DatasetManifest, DatasetSourceDescriptor,
    DatasetSplit,
};
use huntctl::double_q::{ConservativeQ, ConservativeQConfig, DoubleQ, DoubleQConfig};
use huntctl::episode::{EpisodeContext, EpisodeManifest, EpisodeManifestBuild};
use huntctl::fqi::{
    FittedQ, FqiConfig, MAX_FQI_ACTIONS, MAX_FQI_BACKUP_STEPS, MAX_FQI_ITERATIONS,
    MAX_FQI_TRANSITIONS, MAX_FQI_TREE_DEPTH, MAX_FQI_TREES_PER_ACTION, Transition as FqiTransition,
};
use huntctl::harness::execution::execute_request;
use huntctl::harness::inspection::inspect_objective;
use huntctl::harness::objective_suite::ObjectiveSuite;
use huntctl::harness::run_contract::{HarnessRunRequest, HarnessRunResult};
use huntctl::learning::batch::load_fqi_batch;
use huntctl::learning::planning_priors::QBeamPriorTable;
use huntctl::low_data_baselines::{
    LocalFeature, LocalReturnConfig, NearestNeighborReturn, TabularAxis, TabularReturn,
    empirical_return_samples,
};
use huntctl::milestone_dsl;
use huntctl::motion_path::{MotionPathPlan, PathCancellationHit};
use huntctl::motion_path_golf::{MotionPathGolfSteps, golf_motion_path};
use huntctl::observation_view::{MOVEMENT_STATE_V2_ID, ObservationSpec, movement_state_v2_spec};
use huntctl::offline_rl::{
    ExploratoryExtractConfig, MOVEMENT_CATEGORICAL_FEATURES_V1, extract_exploratory_from_bytes,
    extract_exploratory_v2_from_bytes, movement_feature_schema_digest_v1,
};
use huntctl::option_execution::OptionExecution;
use huntctl::option_golf::{RollGolfSteps, golf_roll_option};
use huntctl::oracle_pipeline::OracleCompositionManifest;
use huntctl::pool::{MixedBuildPolicy, WorkerLaunch, WorkerPool};
use huntctl::reward_shaping::{PotentialShapingSpec, REWARD_REPORT_SCHEMA_V1};
use huntctl::roll_option::{RollCancellationHit, RollOptionPlan};
use huntctl::route_store::{ObjectId, RouteStore};
use huntctl::route_workbench::{
    MaterializeTarget, WorkbenchConfig, materialize_lineage, prune_thumbnails,
    serve as serve_route_workbench,
};
use huntctl::scenario_fixture::ScenarioFixture;
use huntctl::search::{
    Candidate, CandidateResult, EvaluationArtifact, EvolutionConfig, PopulationManifest,
    RESULTS_SCHEMA, SearchResults, SegmentProfile, collect_results, evolve_population,
    rank_population, write_seed_population,
};
use huntctl::search_evaluator::{
    AnchoredObjectiveConfig, AnchoredSearchRunConfig, BayesianSearchRunConfig, BeamSearchConfig,
    BootGolfConfig, BootMinimizeConfig, BoundaryFingerprint, ContinuousSearchRunConfig,
    EvaluateConfig, HarnessEvaluateConfig, ProposerTournamentConfig, SearchRunConfig,
    TournamentDefinition, evaluate_population, golf_boot, minimize_boot, run_anchored_search,
    run_bayesian_search, run_beam_search, run_continuous_search, run_proposer_tournament,
    run_search,
};
use huntctl::semantic_oracle::{
    RunOutcomeEvidence, SemanticOracleProgram, SupplementalObservations,
};
use huntctl::tape::InputTape;
use huntctl::tape_chain::{ChainSegment, concatenate};
use huntctl::tape_dsl;
use huntctl::tape_edit::{diff as diff_tapes, layer_at, resample_to_canonical};
use huntctl::tape_program::{PROGRAM_SCHEMA, TapeProgram};
use huntctl::timeline::Timeline;
use huntctl::trace_diff::SiblingTraceDiff;
use huntctl::transition_corpus::TransitionCorpus;
use huntctl::transition_evidence::{
    ImmutableEpisodeArtifact, TerminalReasonEvidence, TransitionEvidenceBuild,
    TransitionEvidenceBundle,
};
use huntctl::transport::ProcessTransport;
use huntctl::world_geometry::{KclPlc, Vec3, extract_rarc_resource, query_prism_point};
use huntctl::world_inventory::WorldInventory;
use huntctl::world_spatial::{
    Aabb3, WorldAabbQueryRequest, WorldPointQueryRequest, WorldRayQueryRequest, WorldSpatialIndex,
    WorldSurfaceFilter,
};
use huntctl::{ArtifactIdentity, BuildIdentity, CompatibilityMode, Digest, ensure_compatible};
use serde_json::{Value, json};
use sha2::{Digest as ShaDigest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::error::Error;
use std::fs;
use std::io::{self, BufRead, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

mod cli;

fn main() {
    if let Err(error) = run() {
        eprintln!("huntctl: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        return usage_error();
    };
    match command {
        "hello" => command_hello(&args[1..]),
        "ping" => command_ping(&args[1..]),
        "pool" => command_pool(&args[1..]),
        "benchmark" => command_benchmark(&args[1..]),
        "campaign" => command_campaign(&args[1..]),
        "harness" => command_harness(&args[1..]),
        "identity" => command_identity(&args[1..]),
        "corpus" => command_corpus(&args[1..]),
        "controller" => command_controller(&args[1..]),
        "milestone" => command_milestone(&args[1..]),
        "fixture" => command_fixture(&args[1..]),
        "tape" => cli::tape::command_tape(&args[1..]),
        "trace" => command_trace(&args[1..]),
        "timeline" => command_timeline(&args[1..]),
        "search" => cli::search::command_search(&args[1..]),
        "learn" => cli::learning::command_learn(&args[1..]),
        "observe" => command_observe(&args[1..]),
        "oracle" => command_oracle(&args[1..]),
        "world" => command_world(&args[1..]),
        "run" | "replay" => command_not_ready(command, &args[1..]),
        "mock-worker" => mock_worker(&args[1..]),
        "mock-search-worker" => mock_search_worker(&args[1..]),
        "mock-record-worker" => mock_record_worker(&args[1..]),
        "help" | "--help" | "-h" => {
            print_usage();
            Ok(())
        }
        _ => usage_error(),
    }
}

fn command_campaign(args: &[String]) -> Result<(), Box<dyn Error>> {
    let proposer_names = repeated_option(args, "--proposer");
    let tournament_definition_path = option(args, "--definition").map(PathBuf::from);
    let proposers = if proposer_names.is_empty()
        && let Some(path) = tournament_definition_path.as_ref()
    {
        let definition: TournamentDefinition = serde_json::from_slice(&fs::read(path)?)?;
        huntctl::harness::campaign::campaign_proposers_from_definition(&definition)?
    } else if proposer_names.is_empty() {
        vec![huntctl::harness::campaign::CampaignProposer::Scripted]
    } else {
        proposer_names
            .iter()
            .map(|name| name.parse())
            .collect::<Result<Vec<_>, _>>()?
    };
    let repository_root = option(args, "--repository-root")
        .map(PathBuf::from)
        .unwrap_or(env::current_dir()?);
    let suite = required_path(args, "--suite")?;
    let case = option(args, "--case").ok_or("missing required --case ID")?;
    let output = required_path(args, "--output")?;
    let plan_config = huntctl::harness::campaign::CampaignPlanConfig {
        repository_root: &repository_root,
        suite_path: &suite,
        case_id: &case,
        output_root: &output,
        proposers: &proposers,
    };
    if flag(args, "--dry-run") {
        let plan = huntctl::harness::campaign::resolve_campaign_plan(&plan_config)?;
        println!("{}", serde_json::to_string_pretty(&plan)?);
        return Ok(());
    }
    let request_template = required_path(args, "--run-request")?;
    let tournament_definition = tournament_definition_path
        .as_deref()
        .ok_or("campaign execution requires --definition TOURNAMENT.json")?;
    let report =
        huntctl::harness::campaign::run_campaign(&huntctl::harness::campaign::CampaignRunConfig {
            plan: plan_config,
            request_template_path: &request_template,
            tournament_definition_path: tournament_definition,
            workers: usize_option(args, "--workers", 4)?,
        })?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    if !report.passed {
        let blocker = report.first_blocker.as_ref().map(|blocker| {
            format!(
                "; first {} {}: {}; artifact: {}",
                blocker.kind,
                blocker.value,
                blocker.message,
                blocker.artifact.display()
            )
        });
        return Err(format!(
            "campaign did not meet expected terminal class{}; report: {}",
            blocker.as_deref().unwrap_or(""),
            report.plan.outputs.report.display(),
        )
        .into());
    }
    Ok(())
}

fn command_harness(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("validate-suite") => {
            let command_args = &args[1..];
            let suite_path = required_path(command_args, "--suite")?;
            let repository_root = option(command_args, "--repository-root")
                .map(PathBuf::from)
                .unwrap_or(env::current_dir()?);
            let suite: ObjectiveSuite = serde_json::from_slice(&fs::read(&suite_path)?)?;
            let report = suite.validate_files(&repository_root)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("seal-suite") => {
            let command_args = &args[1..];
            let input = required_path(command_args, "--input")?;
            let output = required_path(command_args, "--output")?;
            if output.exists() {
                return Err(format!(
                    "objective suite output already exists: {}",
                    output.display()
                )
                .into());
            }
            let repository_root = option(command_args, "--repository-root")
                .map(PathBuf::from)
                .unwrap_or(env::current_dir()?);
            let mut suite: ObjectiveSuite = serde_json::from_slice(&fs::read(&input)?)?;
            suite.refresh_content_sha256()?;
            let report = suite.validate_files(&repository_root)?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, suite.to_pretty_json()?)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("validate-run-request") => {
            let command_args = &args[1..];
            let request_path = required_path(command_args, "--request")?;
            let repository_root = option(command_args, "--repository-root")
                .map(PathBuf::from)
                .unwrap_or(env::current_dir()?);
            let request: HarnessRunRequest =
                serde_json::from_slice(&fs::read(&request_path)?)?;
            let report = request.validate_files(&repository_root)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("seal-run-request") => {
            let command_args = &args[1..];
            let input = required_path(command_args, "--input")?;
            let output = required_path(command_args, "--output")?;
            refuse_existing_output(&output, "run-request")?;
            let repository_root = option(command_args, "--repository-root")
                .map(PathBuf::from)
                .unwrap_or(env::current_dir()?);
            let mut request: HarnessRunRequest = serde_json::from_slice(&fs::read(&input)?)?;
            request.refresh_content_sha256()?;
            let report = request.validate_files(&repository_root)?;
            write_new_file(&output, request.to_pretty_json()?)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("validate-run-result") => {
            let command_args = &args[1..];
            let result_path = required_path(command_args, "--result")?;
            let request_path = required_path(command_args, "--request")?;
            let repository_root = option(command_args, "--repository-root")
                .map(PathBuf::from)
                .unwrap_or(env::current_dir()?);
            let artifact_root = required_path(command_args, "--artifact-root")?;
            let request: HarnessRunRequest =
                serde_json::from_slice(&fs::read(&request_path)?)?;
            request.validate_files(&repository_root)?;
            let result: HarnessRunResult = serde_json::from_slice(&fs::read(&result_path)?)?;
            let report = result.validate_files(&request, &artifact_root)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("seal-run-result") => {
            let command_args = &args[1..];
            let input = required_path(command_args, "--input")?;
            let output = required_path(command_args, "--output")?;
            refuse_existing_output(&output, "run-result")?;
            let request_path = required_path(command_args, "--request")?;
            let repository_root = option(command_args, "--repository-root")
                .map(PathBuf::from)
                .unwrap_or(env::current_dir()?);
            let artifact_root = required_path(command_args, "--artifact-root")?;
            let request: HarnessRunRequest =
                serde_json::from_slice(&fs::read(&request_path)?)?;
            request.validate_files(&repository_root)?;
            let mut result: HarnessRunResult = serde_json::from_slice(&fs::read(&input)?)?;
            result.refresh_content_sha256()?;
            let report = result.validate_files(&request, &artifact_root)?;
            write_new_file(&output, result.to_pretty_json()?)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("execute") => {
            let command_args = &args[1..];
            let request_path = required_path(command_args, "--request")?;
            let repository_root = option(command_args, "--repository-root")
                .map(PathBuf::from)
                .unwrap_or(env::current_dir()?);
            let attempt = u32_option(command_args, "--attempt", 1)?;
            let request: HarnessRunRequest =
                serde_json::from_slice(&fs::read(&request_path)?)?;
            let result = execute_request(&request, &repository_root, attempt)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(())
        }
        Some("inspect-objective") => {
            let command_args = &args[1..];
            let request_path = required_path(command_args, "--request")?;
            let repository_root = option(command_args, "--repository-root")
                .map(PathBuf::from)
                .unwrap_or(env::current_dir()?);
            let request: HarnessRunRequest =
                serde_json::from_slice(&fs::read(&request_path)?)?;
            let result_path = option(command_args, "--result").map(PathBuf::from);
            let result: Option<HarnessRunResult> = result_path
                .as_ref()
                .map(|path| -> Result<_, Box<dyn Error>> {
                    Ok(serde_json::from_slice(&fs::read(path)?)?)
                })
                .transpose()?;
            let artifact_root = option(command_args, "--artifact-root").map(PathBuf::from);
            if result.is_some() != artifact_root.is_some() {
                return Err(
                    "harness inspect-objective requires --result and --artifact-root together"
                        .into(),
                );
            }
            let inspection = inspect_objective(
                &request,
                &repository_root,
                result.as_ref().zip(artifact_root.as_deref()),
            )?;
            print!("{inspection}");
            Ok(())
        }
        _ => Err("harness command: validate-suite|seal-suite|validate-run-request|seal-run-request|validate-run-result|seal-run-result|execute|inspect-objective (use --help for arguments)".into()),
    }
}

fn refuse_existing_output(path: &Path, label: &str) -> Result<(), Box<dyn Error>> {
    if path.exists() {
        return Err(format!("harness {label} output already exists: {}", path.display()).into());
    }
    Ok(())
}

fn write_new_file(path: &Path, bytes: Vec<u8>) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(&bytes)?;
    file.flush()?;
    Ok(())
}

fn command_identity(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("compare") => {
            let compare_args = &args[1..];
            let mode: CompatibilityMode = option(compare_args, "--mode")
                .ok_or("identity compare requires --mode MODE")?
                .parse()?;
            let expected_path = required_path(compare_args, "--expected")?;
            let actual_path = required_path(compare_args, "--actual")?;
            let expected: ArtifactIdentity =
                serde_json::from_slice(&fs::read(&expected_path)?)?;
            let actual: ArtifactIdentity = serde_json::from_slice(&fs::read(&actual_path)?)?;
            expected.validate().map_err(|error| {
                format!(
                    "invalid expected identity {}: {error}",
                    expected_path.display()
                )
            })?;
            actual.validate().map_err(|error| {
                format!(
                    "invalid actual identity {}: {error}",
                    actual_path.display()
                )
            })?;
            ensure_compatible(mode, &expected, &actual)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": "huntctl-identity-comparison/v1",
                    "mode": mode.as_str(),
                    "compatible": true,
                    "expected": expected_path,
                    "actual": actual_path,
                }))?
            );
            Ok(())
        }
        _ => Err("identity command: compare --mode replay|trace-merge|model-training|checkpoint-restore|cross-build-comparison|cross-fidelity-comparison --expected EXPECTED.json --actual ACTUAL.json".into()),
    }
}

fn command_benchmark(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("import-skybook") => {
            let import_args = &args[1..];
            let source = required_path(import_args, "--source")?;
            let output = required_path(import_args, "--output")?;
            if output.exists() {
                return Err(format!("Skybook manifest already exists: {}", output.display()).into());
            }
            let revision = clean_git_revision(&source, "_posts")?;
            if let Some(expected) = option(import_args, "--revision")
                && expected != revision
            {
                return Err(format!(
                    "Skybook checkout revision {revision} does not match requested {expected}"
                )
                .into());
            }
            let repository = option(import_args, "--repository")
                .unwrap_or_else(|| "https://github.com/qwertyquerty/skybook".into());
            let manifest =
                SkybookManifest::import_directory(&source, &repository, &revision)?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, manifest.to_pretty_json()?)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": manifest.schema,
                    "source_revision": manifest.source.git_revision,
                    "page_count": manifest.source.post_count,
                    "categorized_glitch_count": manifest.source.categorized_glitch_count,
                    "content_digest": manifest.content_sha256,
                    "output": output,
                }))?
            );
            Ok(())
        }
        Some("validate-skybook-selection") => {
            let selection_args = &args[1..];
            let manifest_path = required_path(selection_args, "--manifest")?;
            let selection_path = required_path(selection_args, "--selection")?;
            let manifest: SkybookManifest =
                serde_json::from_slice(&fs::read(&manifest_path)?)?;
            let selection: SkybookSelection =
                serde_json::from_slice(&fs::read(&selection_path)?)?;
            selection.validate_against(&manifest)?;
            let selected = selection
                .entries
                .iter()
                .filter(|entry| entry.disposition == SkybookSelectionDisposition::Selected)
                .count();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": selection.schema,
                    "content_digest": selection.content_sha256,
                    "source_revision": selection.source_git_revision,
                    "approved_by": selection.approved_by,
                    "selected_page_count": selected,
                    "entry_count": selection.entries.len(),
                    "selection": selection_path,
                }))?
            );
            Ok(())
        }
        _ => Err("benchmark command:\n  import-skybook --source CHECKOUT --output MANIFEST.json [--revision FULL_GIT_REVISION] [--repository URL]\n  validate-skybook-selection --manifest MANIFEST.json --selection SELECTION.json".into()),
    }
}

fn clean_git_revision(checkout: &Path, imported_path: &str) -> Result<String, Box<dyn Error>> {
    let revision_output = Command::new("git")
        .arg("-C")
        .arg(checkout)
        .args(["rev-parse", "HEAD"])
        .output()?;
    if !revision_output.status.success() {
        return Err(format!(
            "cannot resolve Git revision for {}: {}",
            checkout.display(),
            String::from_utf8_lossy(&revision_output.stderr).trim()
        )
        .into());
    }
    let revision = String::from_utf8(revision_output.stdout)?.trim().to_owned();
    let status_output = Command::new("git")
        .arg("-C")
        .arg(checkout)
        .args([
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--",
            imported_path,
        ])
        .output()?;
    if !status_output.status.success() {
        return Err(format!(
            "cannot inspect Git state for {}: {}",
            checkout.display(),
            String::from_utf8_lossy(&status_output.stderr).trim()
        )
        .into());
    }
    let dirty = String::from_utf8(status_output.stdout)?;
    if !dirty.trim().is_empty() {
        return Err(format!(
            "refusing to import dirty Skybook {imported_path} content at {revision}:\n{}",
            dirty.trim_end()
        )
        .into());
    }
    Ok(revision)
}

fn command_oracle(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("evaluate") => {
            let oracle_args = &args[1..];
            let program_path = required_path(oracle_args, "--program")?;
            let trace_path = required_path(oracle_args, "--trace")?;
            let program: SemanticOracleProgram =
                serde_json::from_slice(&fs::read(&program_path)?)?;
            let trace = huntctl::trace::decode(&fs::read(&trace_path)?)?;
            let mut supplemental: SupplementalObservations =
                if let Some(path) = option(oracle_args, "--supplemental") {
                    serde_json::from_slice(&fs::read(path)?)?
                } else {
                    SupplementalObservations::default()
                };
            if let Some(path) = option(oracle_args, "--run-outcome") {
                if supplemental.run_outcome.is_some() {
                    return Err(
                        "run outcome was supplied in both --supplemental and --run-outcome".into(),
                    );
                }
                supplemental.run_outcome = Some(serde_json::from_slice::<RunOutcomeEvidence>(
                    &fs::read(path)?,
                )?);
            }
            let report = program.evaluate(&trace, &supplemental)?;
            let encoded = serde_json::to_vec_pretty(&report)?;
            if let Some(path) = option(oracle_args, "--output").map(PathBuf::from) {
                if let Some(parent) = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    fs::create_dir_all(parent)?;
                }
                fs::write(path, &encoded)?;
            }
            println!("{}", String::from_utf8(encoded)?);
            Ok(())
        }
        Some("compare") => {
            let oracle_args = &args[1..];
            let program: ComparisonOracleProgram = serde_json::from_slice(&fs::read(
                required_path(oracle_args, "--program")?,
            )?)?;
            let evidence: ComparisonEvidence = serde_json::from_slice(&fs::read(required_path(
                oracle_args,
                "--evidence",
            )?)?)?;
            let report = program.evaluate(&evidence)?;
            let encoded = serde_json::to_vec_pretty(&report)?;
            if let Some(path) = option(oracle_args, "--output").map(PathBuf::from) {
                if let Some(parent) = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    fs::create_dir_all(parent)?;
                }
                fs::write(path, &encoded)?;
            }
            println!("{}", String::from_utf8(encoded)?);
            Ok(())
        }
        Some("compose") => {
            let oracle_args = &args[1..];
            let manifest: OracleCompositionManifest = serde_json::from_slice(&fs::read(
                required_path(oracle_args, "--manifest")?,
            )?)?;
            let evidence = manifest.compose()?;
            let encoded = serde_json::to_vec_pretty(&evidence)?;
            if let Some(path) = option(oracle_args, "--output").map(PathBuf::from) {
                if let Some(parent) = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    fs::create_dir_all(parent)?;
                }
                fs::write(path, &encoded)?;
            }
            println!("{}", String::from_utf8(encoded)?);
            Ok(())
        }
        _ => Err("oracle command: evaluate --program ORACLES.json --trace RUN.trace [--supplemental OBSERVATIONS.json] [--run-outcome OUTCOME.json] [--output REPORT.json] | compose --manifest COMPOSITION.json [--output EVIDENCE.json] | compare --program ORACLES.json --evidence COMPARISON.json [--output REPORT.json]".into()),
    }
}

fn command_fixture(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("compile") if args.len() == 3 => {
            let fixture: ScenarioFixture = serde_json::from_slice(&fs::read(&args[1])?)?;
            let bytes = fixture.encode()?;
            fs::write(&args[2], &bytes)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": fixture.schema,
                    "name": fixture.name,
                    "encoded_bytes": bytes.len(),
                    "output": args[2]
                }))?
            );
            Ok(())
        }
        Some("inspect") if args.len() == 2 => {
            let fixture = ScenarioFixture::decode(&fs::read(&args[1])?)?;
            println!("{}", serde_json::to_string_pretty(&fixture)?);
            Ok(())
        }
        _ => Err(
            "fixture commands: compile SOURCE.json OUTPUT.fixture, inspect INPUT.fixture".into(),
        ),
    }
}

fn command_observe(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("spec") if args.get(1).map(String::as_str) == Some(MOVEMENT_STATE_V2_ID) => {
            let spec = movement_state_v2_spec();
            let bytes = spec.canonical_bytes()?;
            if let Some(output) = option(&args[2..], "--output") {
                let output = PathBuf::from(output);
                if let Some(parent) = output
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&output, &bytes)?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "output": output,
                        "id": spec.id,
                        "digest": spec.digest()?,
                        "feature_count": spec.feature_count(),
                    }))?
                );
            } else {
                println!("{}", serde_json::to_string_pretty(&spec)?);
            }
            Ok(())
        }
        Some("inspect") if args.len() == 2 => {
            let spec: ObservationSpec = serde_json::from_slice(&fs::read(&args[1])?)?;
            spec.validate()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "path": args[1],
                    "id": spec.id,
                    "objective": spec.objective,
                    "phase": spec.phase,
                    "digest": spec.digest()?,
                    "feature_count": spec.feature_count(),
                    "categorical_features": spec.categorical_features(),
                    "channels": spec.channels,
                    "features": spec.features,
                }))?
            );
            Ok(())
        }
        _ => usage_error(),
    }
}

fn command_world(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("inventory") => {
            let stage_dir = required_path(&args[1..], "--stage-dir")?;
            let stage = option(&args[1..], "--stage").ok_or("missing required --stage ID")?;
            let output = required_path(&args[1..], "--output")?;
            let inventory = WorldInventory::build(&stage_dir, &stage)?;
            let bytes = inventory.canonical_bytes()?;
            let digest = inventory.digest()?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &bytes)?;
            let artifact_store = option(&args[1..], "--artifact-store")
                .map(PathBuf::from)
                .unwrap_or_else(|| output.parent().unwrap_or(Path::new(".")).join("content"));
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::WorldInventory)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": inventory.schema,
                    "stage": inventory.stage,
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                    "sha256": digest,
                    "bytes": bytes.len(),
                    "sources": inventory.sources.len(),
                    "chunks": inventory.chunks.len(),
                    "placements": inventory.placements.len(),
                    "player_spawns": inventory.player_spawns.len(),
                    "exits": inventory.exits.len(),
                    "collisions": inventory.collisions.len(),
                    "load_triggers": inventory.load_triggers.len(),
                }))?
            );
            Ok(())
        }
        Some("spatial-index") => {
            let stage_dir = required_path(&args[1..], "--stage-dir")?;
            let stage = option(&args[1..], "--stage").ok_or("missing required --stage ID")?;
            let output = required_path(&args[1..], "--output")?;
            let inventory = WorldInventory::build(&stage_dir, &stage)?;
            let index = WorldSpatialIndex::build(&inventory)?;
            let bytes = index.artifact().canonical_bytes()?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &bytes)?;
            let artifact_store = option(&args[1..], "--artifact-store")
                .map(PathBuf::from)
                .unwrap_or_else(|| output.parent().unwrap_or(Path::new(".")).join("content"));
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::WorldInventory)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": index.artifact().schema,
                    "stage": inventory.stage,
                    "inventory_sha256": inventory.digest()?,
                    "spatial_index_sha256": index.artifact_digest()?,
                    "bytes": bytes.len(),
                    "rooms": index.artifact().rooms.len(),
                    "indexed_surfaces": index.artifact().rooms.iter()
                        .map(|room| room.primitive_ids.len()).sum::<usize>(),
                    "excluded_surfaces": index.artifact().excluded.len(),
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                }))?
            );
            Ok(())
        }
        Some("query") => command_world_query(&args[1..]),
        Some("kcl") => {
            let prism_index: u16 = option(&args[1..], "--prism")
                .ok_or("missing required --prism INDEX")?
                .parse()?;
            let archive_path = option(&args[1..], "--archive").map(PathBuf::from);
            let kcl_path = option(&args[1..], "--kcl").map(PathBuf::from);
            let plc_path = option(&args[1..], "--plc").map(PathBuf::from);

            let (kcl, plc, source) = match (archive_path, kcl_path, plc_path) {
                (Some(archive), None, None) => {
                    let kcl_name =
                        option(&args[1..], "--kcl-name").unwrap_or_else(|| "room.kcl".into());
                    let plc_name =
                        option(&args[1..], "--plc-name").unwrap_or_else(|| "room.plc".into());
                    let archive_bytes = fs::read(&archive)?;
                    let kcl = extract_rarc_resource(&archive_bytes, &kcl_name)?;
                    let plc = extract_rarc_resource(&archive_bytes, &plc_name)?;
                    let source = json!({
                        "kind": "rarc",
                        "archive": archive,
                        "kcl_resource": kcl_name,
                        "plc_resource": plc_name,
                    });
                    (kcl, plc, source)
                }
                (None, Some(kcl), Some(plc)) => {
                    let source = json!({
                        "kind": "loose_files",
                        "kcl": kcl,
                        "plc": plc,
                    });
                    (fs::read(&kcl)?, fs::read(&plc)?, source)
                }
                _ => {
                    return Err(
                        "world kcl requires either --archive PATH or both --kcl PATH --plc PATH"
                            .into(),
                    );
                }
            };
            let inspection = KclPlc::parse(&kcl, &plc)?.inspect_prism(prism_index)?;
            let point_query = option(&args[1..], "--point")
                .map(|value| parse_world_point(&value))
                .transpose()?
                .map(|point| query_prism_point(&inspection.prism, point))
                .transpose()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "source": source,
                    "inspection": inspection,
                    "point_query": point_query,
                }))?
            );
            Ok(())
        }
        _ => usage_error(),
    }
}

fn command_world_query(args: &[String]) -> Result<(), Box<dyn Error>> {
    let operation = args
        .first()
        .map(String::as_str)
        .ok_or("world query requires point, aabb, or ray as its operation")?;
    let query_args = &args[1..];
    let stage_dir = required_path(query_args, "--stage-dir")?;
    let stage = option(query_args, "--stage").ok_or("missing required --stage ID")?;
    let room: i8 = option(query_args, "--room")
        .ok_or("missing required --room N (coordinates are room-scoped)")?
        .parse()?;
    let limit = usize_option(query_args, "--limit", 8)?;
    let filter = WorldSurfaceFilter {
        room,
        load_triggers_only: flag(query_args, "--load-triggers-only"),
        trigger_stable_id: option(query_args, "--trigger-id"),
        destination_stage: option(query_args, "--destination-stage"),
        destination_room: option(query_args, "--destination-room")
            .map(|value| value.parse())
            .transpose()?,
        destination_point: option(query_args, "--destination-point")
            .map(|value| value.parse())
            .transpose()?,
    };
    let inventory = WorldInventory::build(&stage_dir, &stage)?;
    let index = WorldSpatialIndex::build(&inventory)?;
    match operation {
        "point" => {
            let point = parse_world_vec3(
                &option(query_args, "--point").ok_or("missing required --point X,Y,Z")?,
                "--point",
            )?;
            let max_distance = option(query_args, "--max-distance")
                .map(|value| value.parse())
                .transpose()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&index.point_query(WorldPointQueryRequest {
                    point,
                    max_distance,
                    limit,
                    filter,
                })?)?
            );
        }
        "aabb" => {
            let min = parse_world_vec3(
                &option(query_args, "--min").ok_or("missing required --min X,Y,Z")?,
                "--min",
            )?;
            let max = parse_world_vec3(
                &option(query_args, "--max").ok_or("missing required --max X,Y,Z")?,
                "--max",
            )?;
            println!(
                "{}",
                serde_json::to_string_pretty(&index.aabb_query(WorldAabbQueryRequest {
                    bounds: Aabb3::new(min, max)?,
                    limit,
                    filter,
                })?)?
            );
        }
        "ray" => {
            let origin = parse_world_vec3(
                &option(query_args, "--origin").ok_or("missing required --origin X,Y,Z")?,
                "--origin",
            )?;
            let direction = parse_world_vec3(
                &option(query_args, "--direction").ok_or("missing required --direction X,Y,Z")?,
                "--direction",
            )?;
            let max_distance: f32 = option(query_args, "--max-distance")
                .ok_or("missing required --max-distance DISTANCE")?
                .parse()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&index.ray_query(WorldRayQueryRequest {
                    origin,
                    direction,
                    max_distance,
                    limit,
                    filter,
                })?)?
            );
        }
        _ => return Err("world query operation must be point, aabb, or ray".into()),
    }
    Ok(())
}

fn parse_world_point(value: &str) -> Result<Vec3, Box<dyn Error>> {
    parse_world_vec3(value, "--point")
}

fn parse_world_vec3(value: &str, option_name: &str) -> Result<Vec3, Box<dyn Error>> {
    let components = value.split(',').collect::<Vec<_>>();
    if components.len() != 3 {
        return Err(format!("{option_name} must be exactly X,Y,Z").into());
    }
    let point = Vec3 {
        x: components[0].parse()?,
        y: components[1].parse()?,
        z: components[2].parse()?,
    };
    if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
        return Err(format!("{option_name} components must be finite").into());
    }
    Ok(point)
}

fn command_milestone(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("compile") if args.len() == 3 => {
            let source = fs::read_to_string(&args[1])?;
            let compiled = milestone_dsl::compile_source(&source)?;
            let output = PathBuf::from(&args[2]);
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &compiled.bytes)?;
            println!(
                "wrote {} milestones ({} bytes, sha256 {}) to {}",
                compiled.definitions.len(),
                compiled.bytes.len(),
                Digest(compiled.program_sha256),
                output.display()
            );
            Ok(())
        }
        Some("inspect") if args.len() == 2 => {
            let decoded = milestone_dsl::decode(&fs::read(&args[1])?)?;
            let definitions = decoded
                .definitions
                .iter()
                .zip(&decoded.program.definitions)
                .map(|(definition, ast)| -> Result<_, milestone_dsl::BinaryError> {
                    let projections = ast
                        .projections
                        .iter()
                        .map(|projection| {
                            Ok(json!({
                                "name": projection.name,
                                "identity": Digest(milestone_dsl::value_projection_identity(projection)?),
                                "items": projection.items,
                            }))
                        })
                        .collect::<Result<Vec<_>, milestone_dsl::BinaryError>>()?;
                    Ok(json!({
                        "id": definition.name,
                        "sha256": Digest(definition.sha256),
                        "value_projections": projections,
                    }))
                })
                .collect::<Result<Vec<_>, _>>()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "format": "DMSP",
                    "program_sha256": Digest(decoded.program_sha256),
                    "definitions": definitions,
                    "source": milestone_dsl::format(&decoded.program)?,
                }))?
            );
            Ok(())
        }
        Some("format") if args.len() == 2 => {
            let source = fs::read_to_string(&args[1])?;
            println!(
                "{}",
                milestone_dsl::format(&milestone_dsl::parse(&source)?)?
            );
            Ok(())
        }
        _ => usage_error(),
    }
}

fn command_controller(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("compile") if args.len() == 3 => {
            let source = fs::read_to_string(&args[1])?;
            let program = ControllerProgram::parse(&source)?;
            let bytes = program.encode()?;
            let output = PathBuf::from(&args[2]);
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &bytes)?;
            println!(
                "wrote {} frames, {} layers ({} bytes) to {}",
                program.duration_frames,
                program.layers.len(),
                bytes.len(),
                output.display()
            );
            Ok(())
        }
        Some("inspect") if args.len() == 2 => {
            let bytes = fs::read(&args[1])?;
            let program = ControllerProgram::decode(&bytes)?;
            let version_major = u16::from_le_bytes(bytes[8..10].try_into()?);
            let version_minor = u16::from_le_bytes(bytes[10..12].try_into()?);
            let provenance = ControllerObservationProvenance::for_program(&program);
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "format": "DUSKCTRL",
                    "version": { "major": version_major, "minor": version_minor },
                    "duration_frames": program.duration_frames,
                    "layer_count": program.layers.len(),
                    "static_tape_compilable": provenance.is_static(),
                    "observation_provenance": provenance,
                    "layers": program.layers,
                }))?
            );
            Ok(())
        }
        Some("flatten") if args.len() == 3 => {
            let program = ControllerProgram::decode(&fs::read(&args[1])?)?;
            let tape = compile_static_controller(&program)?;
            let output = PathBuf::from(&args[2]);
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, tape.encode()?)?;
            println!(
                "flattened {} controller frames to {}",
                tape.frames.len(),
                output.display()
            );
            Ok(())
        }
        _ => usage_error(),
    }
}

fn command_timeline(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("parse") if args.len() == 2 => {
            let path = PathBuf::from(&args[1]);
            let timeline = load_timeline(&path)?;
            timeline.validate_artifacts(path.parent())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "valid": true,
                    "timeline": timeline.name,
                    "goals": timeline.goals.len(),
                    "proofs": timeline.proofs.len(),
                    "segments": timeline.segments.len(),
                    "continuations": timeline.continuations.len(),
                    "branches": timeline.branches.len(),
                }))?
            );
            Ok(())
        }
        Some("inspect") if args.len() == 2 => {
            let path = PathBuf::from(&args[1]);
            let timeline = load_timeline(&path)?;
            timeline.validate_artifacts(path.parent())?;
            println!("{}", serde_json::to_string_pretty(&timeline.inspect()?)?);
            Ok(())
        }
        Some("status") => {
            let timeline_args = &args[1..];
            let path = required_path(timeline_args, "--timeline")?;
            let timeline = load_timeline(&path)?;
            timeline.validate_artifacts(path.parent())?;
            let selections = timeline_selections(timeline_args)?;
            let status = timeline.status(
                option(timeline_args, "--continuation").as_deref(),
                &selections,
            )?;
            let output = serde_json::to_vec_pretty(&status)?;
            if let Some(path) = option(timeline_args, "--output") {
                fs::write(path, &output)?;
            }
            println!("{}", String::from_utf8(output)?);
            Ok(())
        }
        Some("rebase-compatible") => {
            let timeline_args = &args[1..];
            let path = required_path(timeline_args, "--timeline")?;
            let timeline = load_timeline(&path)?;
            timeline.validate_artifacts(path.parent())?;
            let continuation = option(timeline_args, "--continuation")
                .ok_or("missing required --continuation NAME")?;
            let name = option(timeline_args, "--name")
                .ok_or("missing required --name NEW_CONTINUATION")?;
            let selections = timeline_selections(timeline_args)?;
            if selections.is_empty() {
                return Err(
                    "rebase-compatible requires at least one --select ORIGINAL=REPLACEMENT".into(),
                );
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&timeline.rebase_compatible(
                    &continuation,
                    &selections,
                    &name,
                )?)?
            );
            Ok(())
        }
        Some("workbench") => command_timeline_workbench(&args[1..]),
        Some("store") => command_timeline_store(&args[1..]),
        Some("prune-thumbnails") => {
            let timeline_args = &args[1..];
            let path = required_path(timeline_args, "--timeline")?;
            let timeline = load_timeline(&path)?;
            let repository_root = option(timeline_args, "--repository-root")
                .map(PathBuf::from)
                .unwrap_or(env::current_dir()?);
            let state_root = required_path(timeline_args, "--state-root")?;
            let report = prune_thumbnails(
                &timeline,
                &path,
                &repository_root,
                &state_root,
                flag(timeline_args, "--apply"),
            )?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        _ => usage_error(),
    }
}

fn command_timeline_workbench(args: &[String]) -> Result<(), Box<dyn Error>> {
    let timeline_path = required_path(args, "--timeline")?;
    let game = required_path(args, "--game")?;
    let dvd = option(args, "--dvd")
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(configured_dvd_path)?;
    let state_root = option(args, "--state-root")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("build/automation-state/route-workbench"));
    let port = option(args, "--port")
        .map(|value| value.parse::<u16>())
        .transpose()?
        .unwrap_or(0);
    let working_directory = env::current_dir()?;
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let address = listener.local_addr()?;
    let url = format!("http://{address}/");

    println!("Route Workbench: {url}");
    println!("Timeline: {}", timeline_path.display());
    println!("Ctrl+C stops the workbench; launched playback sessions keep running.");
    if !args.iter().any(|arg| arg == "--no-open") {
        open_browser(&url)?;
    }

    serve_route_workbench(
        listener,
        WorkbenchConfig {
            timeline_path,
            repository_root: working_directory.clone(),
            working_directory,
            game,
            dvd,
            state_root,
        },
    )?;
    Ok(())
}

fn open_browser(url: &str) -> Result<(), Box<dyn Error>> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let brave = ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
            .into_iter()
            .filter_map(env::var_os)
            .map(PathBuf::from)
            .map(|root| root.join("BraveSoftware/Brave-Browser/Application/brave.exe"))
            .find(|path| path.is_file());
        if let Some(brave) = brave {
            let mut command = Command::new(brave);
            command.args(["--new-tab", url]);
            command
        } else {
            let mut command = Command::new("cmd");
            command.args(["/C", "start", "", url]);
            command
        }
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(url);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    };
    command.spawn()?;
    Ok(())
}

fn configured_dvd_path() -> Result<PathBuf, Box<dyn Error>> {
    let app_data = env::var_os("APPDATA")
        .map(PathBuf::from)
        .ok_or("--dvd is required when APPDATA is unavailable")?;
    let config_path = app_data.join("TwilitRealm/Dusklight/config.json");
    let config: Value = serde_json::from_slice(&fs::read(&config_path).map_err(|error| {
        format!(
            "--dvd was omitted and the Dusklight config {} could not be read: {error}",
            config_path.display()
        )
    })?)?;
    config
        .get("backend.isoPath")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            format!(
                "--dvd was omitted and {} has no backend.isoPath",
                config_path.display()
            )
            .into()
        })
}

fn command_timeline_store(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("init") if args.len() == 2 => {
            RouteStore::initialize(&args[1])?;
            println!("initialized {}", args[1]);
            Ok(())
        }
        Some("import") => {
            let store_args = &args[1..];
            let root = required_path(store_args, "--store")?;
            let path = required_path(store_args, "--timeline")?;
            let reference = option(store_args, "--ref").ok_or("missing required --ref NAME")?;
            let timeline = load_timeline(&path)?;
            let result = RouteStore::open(root)?.import_timeline(
                &timeline,
                path.parent().unwrap_or_else(|| std::path::Path::new(".")),
                &reference,
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(())
        }
        Some("fork") => {
            let store_args = &args[1..];
            let store = RouteStore::open(required_path(store_args, "--store")?)?;
            let from = option(store_args, "--from").ok_or("missing required --from REF")?;
            let to = option(store_args, "--to").ok_or("missing required --to REF")?;
            let id = if let Some(lineage) = option(store_args, "--lineage") {
                store.fork_lineage(&from, &lineage, &to)?
            } else {
                store.fork(&from, &to)?
            };
            println!("{id}");
            Ok(())
        }
        Some("import-evaluation") => {
            let store_args = &args[1..];
            let store = RouteStore::open(required_path(store_args, "--store")?)?;
            let path = required_path(store_args, "--evaluation")?;
            let segment =
                option(store_args, "--segment").ok_or("missing required --segment NAME")?;
            let fingerprint = option(store_args, "--fingerprint")
                .ok_or("missing required --fingerprint VALUE")?;
            let reference = option(store_args, "--ref");
            let id =
                store.import_evaluation(&path, &segment, &fingerprint, reference.as_deref())?;
            println!("{id}");
            Ok(())
        }
        Some("append") => {
            let store_args = &args[1..];
            let store = RouteStore::open(required_path(store_args, "--store")?)?;
            let reference = option(store_args, "--ref").ok_or("missing required --ref REF")?;
            let path = required_path(store_args, "--timeline")?;
            let continuation = option(store_args, "--continuation")
                .ok_or("missing required --continuation NAME")?;
            let timeline = load_timeline(&path)?;
            let id = store.append_lineage(
                &reference,
                &timeline,
                &continuation,
                path.parent().unwrap_or_else(|| std::path::Path::new(".")),
            )?;
            println!("{id}");
            Ok(())
        }
        Some("replay-repair") => {
            let store_args = &args[1..];
            let store = RouteStore::open(required_path(store_args, "--store")?)?;
            let from = option(store_args, "--from").ok_or("missing required --from REF")?;
            let to = option(store_args, "--to").ok_or("missing required --to REF")?;
            let path = required_path(store_args, "--timeline")?;
            let continuation = option(store_args, "--continuation")
                .ok_or("missing required --continuation NAME")?;
            let timeline = load_timeline(&path)?;
            let id = store.replay_repair(
                &from,
                &to,
                &timeline,
                &continuation,
                path.parent().unwrap_or_else(|| std::path::Path::new(".")),
            )?;
            println!("{id}");
            Ok(())
        }
        Some("promote") => {
            let store_args = &args[1..];
            let store = RouteStore::open(required_path(store_args, "--store")?)?;
            let reference = option(store_args, "--ref").ok_or("missing required --ref REF")?;
            let object: ObjectId = option(store_args, "--object")
                .ok_or("missing required --object ID")?
                .parse()?;
            store.promote(&reference, &object)?;
            println!("{object}");
            Ok(())
        }
        Some("resolve") => {
            let store_args = &args[1..];
            let store = RouteStore::open(required_path(store_args, "--store")?)?;
            let reference = option(store_args, "--ref").ok_or("missing required --ref REF")?;
            println!("{}", store.resolve_ref(&reference)?);
            Ok(())
        }
        Some("show") => {
            let store_args = &args[1..];
            let store = RouteStore::open(required_path(store_args, "--store")?)?;
            let object: ObjectId = option(store_args, "--object")
                .ok_or("missing required --object ID")?
                .parse()?;
            println!("{}", serde_json::to_string_pretty(&store.read(&object)?)?);
            Ok(())
        }
        Some("verify") => {
            let store_args = &args[1..];
            let store = RouteStore::open(required_path(store_args, "--store")?)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"valid": true, "objects": store.verify()?}))?
            );
            Ok(())
        }
        Some("gc") => {
            let store_args = &args[1..];
            let store = RouteStore::open(required_path(store_args, "--store")?)?;
            let apply = store_args.iter().any(|arg| arg == "--apply");
            println!("{}", serde_json::to_string_pretty(&store.gc(apply)?)?);
            Ok(())
        }
        _ => usage_error(),
    }
}

fn load_timeline(path: impl AsRef<std::path::Path>) -> Result<Timeline, Box<dyn Error>> {
    Ok(Timeline::parse(&fs::read_to_string(path)?)?)
}

fn timeline_selections(args: &[String]) -> Result<BTreeMap<String, String>, Box<dyn Error>> {
    let mut output = BTreeMap::new();
    for selection in repeated_option(args, "--select") {
        let (original, replacement) = selection
            .split_once('=')
            .ok_or("--select must be ORIGINAL_SEGMENT=REPLACEMENT_SEGMENT")?;
        if original.is_empty() || replacement.is_empty() {
            return Err("--select segment IDs must be nonempty".into());
        }
        if output
            .insert(original.to_owned(), replacement.to_owned())
            .is_some()
        {
            return Err(format!("duplicate selection for segment {original}").into());
        }
    }
    Ok(output)
}

fn command_trace(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("inspect") if args.len() == 2 => {
            let summary = huntctl::trace::decode_and_summarize(&fs::read(&args[1])?)?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            Ok(())
        }
        Some("facts") if args.len() == 2 || args.len() == 4 => {
            if args.len() == 4 && args[2] != "--boundary-index" {
                return usage_error();
            }
            let decoded = huntctl::trace::decode(&fs::read(&args[1])?)?;
            let requested_boundary = option(&args[2..], "--boundary-index")
                .map(|value| value.parse::<u64>())
                .transpose()?;
            let facts = decoded
                .records
                .iter()
                .filter(|record| {
                    requested_boundary.is_none_or(|index| record.boundary_index == index)
                })
                .map(huntctl::trace_typed_facts::typed_facts_from_trace_record)
                .collect::<Vec<_>>();
            if requested_boundary.is_some() && facts.is_empty() {
                return Err("trace has no record at the requested boundary index".into());
            }
            for response in &facts {
                response.validate()?;
            }
            println!("{}", serde_json::to_string_pretty(&facts)?);
            Ok(())
        }
        Some("timeline") if args.len() == 2 => {
            let decoded = huntctl::trace::decode(&fs::read(&args[1])?)?;
            let mut prior: Option<&huntctl::trace::TraceRecord> = None;
            let records: Vec<_> = decoded
                .records
                .iter()
                .filter(|record| {
                    let changed = prior.is_none_or(|previous| {
                        record.stage_name != previous.stage_name
                            || record.room != previous.room
                            || record.layer != previous.layer
                            || record.point != previous.point
                            || record.player_present() != previous.player_present()
                            || record.player_is_link() != previous.player_is_link()
                            || record.event_running() != previous.event_running()
                            || record.event_id != previous.event_id
                            || record.event_status != previous.event_status
                            || record.player_proc_id != previous.player_proc_id
                    });
                    let input = record.buttons != 0 || record.stick_x != 0 || record.stick_y != 0;
                    prior = Some(record);
                    changed || input
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&records)?);
            Ok(())
        }
        Some("compare") if args.len() >= 3 => {
            let mut rows: Vec<Value> = args[1..]
                .iter()
                .map(|path| {
                    let summary = huntctl::trace::decode_and_summarize(&fs::read(path)?)?;
                    let milestone_count = [
                        summary.first_playable.is_some(),
                        summary.route_control.is_some(),
                        summary.first_loading_trigger.is_some(),
                        summary.first_loading_transition.is_some(),
                        summary.post_load_playable.is_some(),
                        summary.first_post_load_event.is_some(),
                        summary.intro_cutscene.is_some(),
                    ]
                    .into_iter()
                    .filter(|reached| *reached)
                    .count();
                    let score_tick = summary
                        .intro_cutscene
                        .as_ref()
                        .or(summary.first_post_load_event.as_ref())
                        .or(summary.post_load_playable.as_ref())
                        .or(summary.first_loading_transition.as_ref())
                        .or(summary.first_loading_trigger.as_ref())
                        .or(summary.route_control.as_ref())
                        .or(summary.first_playable.as_ref())
                        .map(|milestone| milestone.simulation_tick)
                        .unwrap_or(u64::MAX);
                    Ok::<_, Box<dyn Error>>(json!({
                        "path": path,
                        "milestones_reached": milestone_count,
                        "score_tick": score_tick,
                        "summary": summary,
                    }))
                })
                .collect::<Result<_, _>>()?;
            rows.sort_by(|left, right| {
                let left_count = left["milestones_reached"].as_u64().unwrap();
                let right_count = right["milestones_reached"].as_u64().unwrap();
                right_count.cmp(&left_count).then_with(|| {
                    left["score_tick"]
                        .as_u64()
                        .unwrap()
                        .cmp(&right["score_tick"].as_u64().unwrap())
                })
            });
            println!("{}", serde_json::to_string_pretty(&rows)?);
            Ok(())
        }
        _ => usage_error(),
    }
}

fn command_corpus(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("init") if args.len() == 2 => {
            let corpus = Corpus::initialize(&args[1])?;
            println!("initialized {}", corpus.root().display());
            Ok(())
        }
        Some("ingest") if args.len() >= 2 => {
            let corpus = Corpus::open(&args[1])?;
            let tape_path = required_path(args, "--tape")?;
            let build_path = required_path(args, "--build")?;
            let scenario = option(args, "--scenario").ok_or("missing required --scenario ID")?;
            let build: BuildIdentity = serde_json::from_slice(&fs::read(build_path)?)?;
            let metadata = if let Some(path) = option(args, "--scenario-json") {
                serde_json::from_slice(&fs::read(path)?)?
            } else {
                json!({})
            };
            let result = corpus.ingest(&fs::read(tape_path)?, build, scenario, metadata)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "artifact_id": result.artifact_id,
                    "tape_digest": result.tape_digest,
                    "created": result.created
                }))?
            );
            Ok(())
        }
        Some("list") if args.len() == 2 => {
            let artifacts: Vec<Value> = Corpus::open(&args[1])?
                .list()?
                .into_iter()
                .map(|artifact| {
                    json!({
                        "artifact_id": artifact.artifact_id,
                        "scenario": artifact.manifest.scenario.id,
                        "frame_count": artifact.manifest.frame_count,
                        "tape_digest": artifact.manifest.tape.digest
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&artifacts)?);
            Ok(())
        }
        Some("show") if args.len() == 3 => {
            let artifact_id: Digest = args[2].parse()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&Corpus::open(&args[1])?.show(artifact_id)?)?
            );
            Ok(())
        }
        Some("verify") if args.len() == 2 => {
            println!(
                "{}",
                serde_json::to_string_pretty(&Corpus::open(&args[1])?.verify()?)?
            );
            Ok(())
        }
        Some("query") => {
            let inputs = repeated_option(&args[1..], "--input")
                .into_iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>();
            let action = option(&args[1..], "--action")
                .map(|value| value.parse())
                .transpose()?;
            let terminal = option(&args[1..], "--terminal")
                .map(|value| value.parse::<bool>())
                .transpose()?;
            let minimum_reward = option(&args[1..], "--minimum-reward")
                .map(|value| value.parse::<f32>())
                .transpose()?;
            let rows = huntctl::corpus_ops::query(
                &inputs,
                action,
                terminal,
                minimum_reward,
                usize_option(&args[1..], "--limit", 1000)?,
            )?;
            println!("{}", serde_json::to_string_pretty(&rows)?);
            Ok(())
        }
        Some("compare") => {
            let report = huntctl::corpus_ops::compare(
                &required_path(&args[1..], "--left")?,
                &required_path(&args[1..], "--right")?,
            )?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("merge" | "compact") => {
            let inputs = repeated_option(&args[1..], "--input")
                .into_iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>();
            let output = required_path(&args[1..], "--output")?;
            let level = if args[0] == "compact" { 19 } else { 3 };
            let report = huntctl::corpus_ops::merge(&inputs, &output, level)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("shard") => {
            let report = huntctl::corpus_ops::shard(
                &required_path(&args[1..], "--input")?,
                &required_path(&args[1..], "--output-directory")?,
                usize_option(&args[1..], "--maximum-transitions", 100_000)?,
                3,
            )?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("refeature") => {
            let descriptor = huntctl::corpus_ops::refeature(
                &required_path(&args[1..], "--source")?,
                &required_path(&args[1..], "--output")?,
                &option(&args[1..], "--view").unwrap_or_else(|| MOVEMENT_STATE_V2_ID.into()),
            )?;
            println!("{}", serde_json::to_string_pretty(&descriptor)?);
            Ok(())
        }
        Some("validate-transitions") => {
            let inputs = repeated_option(&args[1..], "--input");
            let mut reports = Vec::new();
            for input in inputs {
                let corpus = TransitionCorpus::read_zstd_file(&input)?;
                reports.push(json!({
                    "input": input,
                    "content_sha256": corpus.content_digest()?,
                    "feature_schema": corpus.feature_schema,
                    "action_schema": corpus.action_schema,
                    "transitions": corpus.transitions.len(),
                }));
            }
            println!("{}", serde_json::to_string_pretty(&reports)?);
            Ok(())
        }
        Some("quarantine") => {
            let inputs = repeated_option(&args[1..], "--input")
                .into_iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>();
            let report = huntctl::corpus_ops::quarantine_invalid(
                &inputs,
                &required_path(&args[1..], "--quarantine-root")?,
                !args[1..].iter().any(|argument| argument == "--apply"),
            )?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("gc-content") => {
            let corpus_args = &args[1..];
            let store_root = required_path(corpus_args, "--store")?;
            let trash_root = required_path(corpus_args, "--trash-root")?;
            let mut referenced = BTreeSet::new();
            for value in repeated_option(corpus_args, "--reference") {
                referenced.insert(value.parse::<Digest>()?);
            }
            let manifests = repeated_option(corpus_args, "--manifest");
            if manifests.is_empty() && referenced.is_empty() {
                return Err(
                    "corpus gc-content requires at least one --manifest or --reference".into(),
                );
            }
            for manifest in manifests {
                let bytes = fs::read(&manifest)?;
                referenced.insert(Digest(Sha256::digest(&bytes).into()));
                collect_json_digests(&serde_json::from_slice(&bytes)?, &mut referenced);
            }
            let report = ContentStore::initialize(store_root)?.garbage_collect(
                &referenced,
                &trash_root,
                !flag(corpus_args, "--apply"),
            )?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("export-arrow") => {
            let corpus_args = &args[1..];
            let inputs = repeated_option(corpus_args, "--input")
                .into_iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>();
            let report = huntctl::corpus_ops::export_arrow(
                &inputs,
                &required_path(corpus_args, "--output")?,
            )?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        _ => usage_error(),
    }
}

fn collect_json_digests(value: &Value, output: &mut BTreeSet<Digest>) {
    match value {
        Value::String(value) => {
            if let Ok(digest) = value.parse() {
                output.insert(digest);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_json_digests(value, output);
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                collect_json_digests(value, output);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn command_pool(args: &[String]) -> Result<(), Box<dyn Error>> {
    if args.first().map(String::as_str) != Some("health") {
        return usage_error();
    }
    let pool_args = &args[1..];
    let (program, worker_args) = worker_spec(pool_args)?;
    let worker_count: usize = option(pool_args, "--workers")
        .unwrap_or_else(|| "1".into())
        .parse()?;
    let check_count: usize = option(pool_args, "--checks")
        .unwrap_or_else(|| worker_count.to_string())
        .parse()?;
    if worker_count == 0 {
        return Err("--workers must be greater than zero".into());
    }
    let policy = if pool_args.iter().any(|arg| arg == "--allow-mixed-builds") {
        MixedBuildPolicy::AllowMixed
    } else {
        MixedBuildPolicy::RequireIdentical
    };
    let launches = (0..worker_count)
        .map(|index| WorkerLaunch {
            label: format!("worker-{index}"),
            program: program.clone(),
            args: worker_args.clone(),
        })
        .collect();
    let start = WorkerPool::spawn(launches, policy);
    let startup_failures: Vec<Value> = start
        .failures
        .iter()
        .map(|failure| {
            json!({
                "index": failure.index, "label": failure.label,
                "kind": format!("{:?}", failure.kind), "message": failure.message
            })
        })
        .collect();
    let mut pool = start.pool;
    let active_workers = pool.worker_count();
    let health = pool.health_jobs(check_count);
    let jobs: Vec<Value> = health
        .jobs
        .iter()
        .map(|job| {
            json!({
                "job_id": job.job_id, "worker_index": job.worker_index,
                "worker_label": job.worker_label, "ok": job.is_ok(),
                "latency_micros": job.latency_micros, "error": job.error
            })
        })
        .collect();
    let shutdown = pool.shutdown();
    let shutdown_results: Vec<Value> = shutdown
        .iter()
        .map(|result| {
            json!({
                "worker_index": result.worker_index, "worker_label": result.worker_label,
                "ok": result.error.is_none(), "error": result.error
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "policy": format!("{policy:?}"), "requested_workers": worker_count,
            "active_workers": active_workers, "startup_failures": startup_failures,
            "health_jobs": jobs, "shutdown": shutdown_results
        }))?
    );
    if !start.failures.is_empty()
        || !health.all_ok()
        || shutdown.iter().any(|result| result.error.is_some())
    {
        return Err("worker-pool health check reported failures".into());
    }
    Ok(())
}

fn command_hello(args: &[String]) -> Result<(), Box<dyn Error>> {
    let (program, worker_args) = worker_spec(args)?;
    let mut client = WorkerClient::new(ProcessTransport::spawn(program, &worker_args)?);
    let hello = client.handshake()?.clone();
    println!(
        "protocol={CONTROL_PROTOCOL_NAME}/{} version={} revision={} dirty_digest={} aurora={} compiler={} target={} config={} features={} fidelity={} platform={}/{} pointer_bits={}",
        CONTROL_PROTOCOL_VERSION,
        hello.build.version,
        hello.build.revision,
        hello.build.dirty_digest,
        hello.build.aurora_revision,
        hello.build.compiler,
        hello.build.compiler_target,
        hello.build.build_type,
        hello.build.feature_digest,
        hello.build.fidelity_profile,
        hello.build.platform,
        hello.build.architecture,
        hello.build.pointer_bits
    );
    println!(
        "persistent={} engine_session={} headless={} input_tape={} batch_run={} commands={}",
        hello.capabilities.persistent_control,
        hello.capabilities.engine_session,
        hello.capabilities.headless,
        hello.capabilities.input_tape,
        hello.capabilities.batch_run,
        hello.capabilities.commands.join(",")
    );
    client.shutdown()?;
    Ok(())
}

fn command_ping(args: &[String]) -> Result<(), Box<dyn Error>> {
    let (program, worker_args) = worker_spec(args)?;
    let mut client = WorkerClient::new(ProcessTransport::spawn(program, &worker_args)?);
    client.handshake()?;
    client.ping()?;
    println!("pong");
    client.shutdown()?;
    Ok(())
}

fn command_not_ready(command: &str, args: &[String]) -> Result<(), Box<dyn Error>> {
    let (program, worker_args) = worker_spec(args)?;
    let mut client = WorkerClient::new(ProcessTransport::spawn(program, &worker_args)?);
    let capabilities = client.handshake()?.capabilities.clone();
    client.shutdown()?;
    Err(format!("{command} is unavailable (engine_session={}, input_tape={}, batch_run={}); protocol v{CONTROL_PROTOCOL_VERSION} currently exposes bootstrap control only",
        capabilities.engine_session, capabilities.input_tape, capabilities.batch_run).into())
}

fn worker_spec(args: &[String]) -> Result<(PathBuf, Vec<String>), Box<dyn Error>> {
    Ok((
        required_path(args, "--worker")?,
        repeated_option(args, "--worker-arg"),
    ))
}

fn option(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
}

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|argument| argument == name)
}

fn repeated_option(args: &[String], name: &str) -> Vec<String> {
    args.windows(2)
        .filter(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
        .collect()
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

fn u64_option(args: &[String], name: &str, default: u64) -> Result<u64, Box<dyn Error>> {
    Ok(option(args, name)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(default))
}

fn u32_option(args: &[String], name: &str, default: u32) -> Result<u32, Box<dyn Error>> {
    Ok(option(args, name)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(default))
}

fn timeout_option(args: &[String]) -> Result<Duration, Box<dyn Error>> {
    if let Some(milliseconds) = option(args, "--timeout-ms") {
        return Ok(Duration::from_millis(milliseconds.parse()?));
    }
    Ok(Duration::from_secs(
        option(args, "--timeout-seconds")
            .map(|value| value.parse())
            .transpose()?
            .unwrap_or(300),
    ))
}

fn usage_error<T>() -> Result<T, Box<dyn Error>> {
    print_usage();
    Err("invalid command line".into())
}

fn print_usage() {
    eprintln!("Trace typed facts:\n  huntctl trace facts INPUT.trace [--boundary-index N]\n");
    eprintln!(
        "Objective campaigns:\n  huntctl campaign --suite SUITE.json --case ID --output build/DIR --dry-run [--repository-root DIR] [--proposer scripted|random|structured|learned]...\n  huntctl campaign --suite SUITE.json --case ID --output build/DIR --run-request REQUEST.json --definition TOURNAMENT.json [--repository-root DIR] [--workers N]\n"
    );
    eprintln!(
        "Usage:\n  huntctl hello --worker PATH [--worker-arg ARG]...\n  huntctl ping --worker PATH [--worker-arg ARG]...\n  huntctl pool health --worker PATH [--worker-arg ARG]... [--workers N] [--checks N] [--allow-mixed-builds]\n  huntctl controller compile SOURCE.duskctl OUTPUT.dctl\n  huntctl controller inspect INPUT.dctl\n  huntctl controller flatten INPUT.dctl OUTPUT.tape\n  huntctl milestone compile SOURCE.milestones OUTPUT.dmsp\n  huntctl milestone inspect INPUT.dmsp\n  huntctl milestone format SOURCE.milestones\n  huntctl tape inspect INPUT.tape [--frames]\n  huntctl tape compile PROGRAM.tas OUTPUT.tape\n  huntctl tape run INPUT.tape --game PATH --dvd PATH --state-root DIR [--milestone-program FILE] [--milestones IDS] [--milestone-goal ID] [--milestone-result FILE] [--gameplay-trace FILE] [--gameplay-trace-channels LIST] [--headful] [--timeout-seconds N] [--game-arg ARG]...\n  huntctl tape prove INPUT.tape --game PATH --dvd PATH --state-root DIR --milestone-goal ID [--milestone-program FILE] [--proof FILE] [--repetitions N] [--timeout-seconds N] [--game-arg ARG]...\n  huntctl tape concat OUTPUT.tape INPUT.tape INPUT.tape...\n  huntctl trace inspect INPUT.trace\n  huntctl trace timeline INPUT.trace\n  huntctl trace compare INPUT.trace INPUT.trace...\n  huntctl timeline parse ROUTE.timeline\n  huntctl timeline inspect ROUTE.timeline\n  huntctl timeline status --timeline FILE [--continuation NAME] [--select ORIGINAL_SEGMENT=REPLACEMENT_SEGMENT]... [--output FILE]\n  huntctl timeline rebase-compatible --timeline FILE --continuation NAME --select ORIGINAL_SEGMENT=REPLACEMENT_SEGMENT --name NEW_NAME\n  huntctl timeline store init ROOT\n  huntctl timeline store import --store ROOT --timeline FILE --ref REF\n  huntctl timeline store import-evaluation --store ROOT --evaluation FILE --segment NAME --fingerprint VALUE [--ref REF]\n  huntctl timeline store fork --store ROOT --from REF --to REF [--lineage NAME]\n  huntctl timeline store append --store ROOT --ref REF --timeline FILE --continuation NAME\n  huntctl timeline store replay-repair --store ROOT --from REF --to REF --timeline FILE --continuation NAME\n  huntctl timeline store promote --store ROOT --ref REF --object ID\n  huntctl timeline store resolve|show|verify|gc ...\n  huntctl search seed --segment ID --output DIR [--candidate FILE] [--size N] [--rng-seed N]\n  huntctl search collect --population MANIFEST --input EVALUATION.json... --output RESULTS.json\n  huntctl search evolve --population MANIFEST --results RESULTS --output DIR [--size N] [--elites N] [--rng-seed N]\n  huntctl search rank --population MANIFEST --results RESULTS\n  huntctl search inspect CANDIDATE.json\n  huntctl search mock-evaluate --population MANIFEST --output RESULTS.json [--attempts N]\n  huntctl corpus init ROOT\n  huntctl corpus ingest ROOT --tape INPUT.tape --scenario ID --build BUILD.json [--scenario-json METADATA.json]\n  huntctl corpus list ROOT\n  huntctl corpus show ROOT ARTIFACT_SHA256\n  huntctl corpus verify ROOT\n  huntctl run --worker PATH\n  huntctl replay --worker PATH\n  huntctl mock-worker [--mock-revision REVISION]\n\nSearch segment IDs: boot_to_fsp103, fsp103_to_fsp104\nTAS DSL: dusktape 1 (legacy JSON schema: {PROGRAM_SCHEMA})"
    );
    eprintln!(
        "\nBenchmark metadata:\n  huntctl benchmark import-skybook --source CHECKOUT --output MANIFEST.json [--revision FULL_GIT_REVISION] [--repository URL]\n  huntctl benchmark validate-skybook-selection --manifest MANIFEST.json --selection SELECTION.json"
    );
    eprintln!(
        "\nCore harness:\n  huntctl harness validate-suite --suite SUITE.json [--repository-root DIR]\n  huntctl harness seal-suite --input DRAFT.json --output SUITE.json [--repository-root DIR]\n  huntctl harness validate-run-request --request REQUEST.json [--repository-root DIR]\n  huntctl harness seal-run-request --input DRAFT.json --output REQUEST.json [--repository-root DIR]\n  huntctl harness validate-run-result --result RESULT.json --request REQUEST.json --artifact-root DIR [--repository-root DIR]\n  huntctl harness seal-run-result --input DRAFT.json --output RESULT.json --request REQUEST.json --artifact-root DIR [--repository-root DIR]\n  huntctl harness execute --request REQUEST.json [--repository-root DIR] [--attempt N]\n  huntctl harness inspect-objective --request REQUEST.json [--result RESULT.json --artifact-root DIR] [--repository-root DIR]"
    );
    eprintln!(
        "\nRun identity:\n  huntctl identity compare --mode replay|trace-merge|model-training|checkpoint-restore|cross-build-comparison|cross-fidelity-comparison --expected EXPECTED.json --actual ACTUAL.json"
    );
    eprintln!(
        "\nRoute workbench:\n  huntctl timeline workbench --timeline FILE --game PATH [--dvd PATH] [--state-root DIR] [--port N] [--no-open]"
    );
    eprintln!(
        "  huntctl timeline prune-thumbnails --timeline FILE --state-root DIR [--repository-root DIR] [--apply]"
    );
    eprintln!(
        "\nScenario fixtures:\n  huntctl fixture compile SOURCE.json OUTPUT.fixture\n  huntctl fixture inspect INPUT.fixture\n  huntctl tape compile PROGRAM.tas OUTPUT.tape [--fixture FIXTURE.json]"
    );
    eprintln!(
        "\nTape editing:\n  huntctl tape slice INPUT.tape OUTPUT.tape --start N --frames N\n  huntctl tape layer BASE.tape OVERLAY.tape OUTPUT.tape --start N\n  huntctl tape resample INPUT.tape OUTPUT.tape\n  huntctl tape diff LEFT.tape RIGHT.tape"
    );
    eprintln!(
        "\nTransition corpus lifecycle:\n  huntctl corpus query --input BATCH.dtcz [--input MORE.dtcz] [--action N] [--terminal BOOL] [--minimum-reward R] [--limit N]\n  huntctl corpus compare --left LEFT.dtcz --right RIGHT.dtcz\n  huntctl corpus merge|compact --input BATCH.dtcz [--input MORE.dtcz] --output OUTPUT.dtcz\n  huntctl corpus shard --input BATCH.dtcz --output-directory DIR [--maximum-transitions N]\n  huntctl corpus refeature --source SOURCE.json --output OUTPUT.dtcz [--view movement-state/v1|movement-state/v2]\n  huntctl corpus validate-transitions --input BATCH.dtcz [--input MORE.dtcz]\n  huntctl corpus quarantine --input BATCH.dtcz [--input MORE.dtcz] --quarantine-root DIR [--apply]\n  huntctl corpus gc-content --store ROOT --trash-root DIR (--manifest ROOT.json | --reference SHA256)... [--apply]\n  huntctl corpus export-arrow --input BATCH.dtcz [--input MORE.dtcz] --output ANALYSIS.arrow"
    );
    eprintln!(
        "\nTape recording:\n  huntctl tape record SEED.tape OUTPUT.tape --game PATH --dvd PATH --state-root DIR [--capacity N] [--timeout-seconds N] [--game-arg ARG]..."
    );
    eprintln!(concat!(
        "\nNative search:\n",
        "  huntctl search evaluate --population MANIFEST --output DIR (--run-request REQUEST.json [--repository-root DIR] | --game PATH --dvd PATH) [--results FILE] [--workers N] [--repetitions N]\n",
        "  huntctl search run --segment ID [--candidate FILE] --output DIR (--run-request REQUEST.json [--repository-root DIR] | --game PATH --dvd PATH) [--generations N] [--size N] [--elites N] [--workers N] [--repetitions N]\n",
        "  huntctl search beam --candidate SEED.json --options OPTIONS.json [--q-priors PRIORS.json] --game PATH --dvd PATH --output DIR [--beam-width N] [--maximum-depth N] [--candidate-budget N] [--workers N] [--repetitions N]\n",
        "  huntctl search continuous --method cem|cma-es --candidate SEED.json --axes AXES.json --game PATH --dvd PATH --output DIR [--generations N] [--population N] [--elites N] [--initial-sigma S] [--candidate-budget N] [--rng-seed N]\n",
        "  huntctl search bayesian --candidate SEED.json --axes AXES.json --game PATH --dvd PATH --output DIR [--generations N] [--batch-size N] [--initial-samples N] [--acquisition-pool N] [--length-scale L] [--observation-noise N] [--exploration X] [--candidate-budget N] [--rng-seed N]\n",
        "  huntctl search tournament --definition TOURNAMENT.json --output DIR (--run-request REQUEST.json [--repository-root DIR] | --game PATH --dvd PATH) [--workers N] [--repetitions N]\n",
        "  huntctl search minimize-boot --candidate FILE --game PATH --dvd PATH --output DIR [--workers N] [--repetitions N]\n",
        "  huntctl search golf-boot --candidate FILE --game PATH --dvd PATH --output DIR [--workers N] [--repetitions N]\n",
        "  huntctl search golf-option --plan ROLL.json --execution EXECUTION.json --tape INPUT.tape --output PROPOSALS.json [--cancellation-tick N --condition-index N] [--heading-step N] [--magnitude-step N] [--duration-step N] [--phase-step N] [--button-step N] [--cancellation-step N]\n",
        "  huntctl search golf-path --plan PATH.json --execution EXECUTION.json --tape INPUT.tape --output PROPOSALS.json [--cancellation-tick N --condition-index N] [--point-step N] [--duration-step N] [--phase-step N] [--cancellation-step N]\n",
        "  huntctl search import-tape --segment ID --tape INPUT.tape --output CANDIDATE.json"
    ));
    eprintln!(
        "  huntctl search run-route --timeline FILE --lineage NAME --segment TIMELINE_SEGMENT [--candidate FILE] --output DIR (--run-request REQUEST.json [--repository-root DIR] | --game PATH --dvd PATH) [--generations N] [--size N] [--elites N] [--workers N] [--repetitions N]"
    );
    eprintln!(
        "\nObservation views:\n  huntctl observe spec movement-state/v2 [--output SPEC.json]\n  huntctl observe inspect SPEC.json\n\nNative fitted Q:\n  huntctl learn benchmark\n  huntctl learn extract-trace --trace INPUT.trace --tape INPUT.tape --episode-context CONTEXT.json --start-frame N --end-frame N --output BATCH.dtcz [--artifact-store ROOT] [--view movement-state/v1|movement-state/v2] [--terminal]\n  huntctl learn dataset --source SOURCE.json [--source MORE.json] --output DATASET.json [--withheld-objective ID] [--validation-percent N] [--test-percent N] [--artifact-store ROOT]\n  huntctl learn diff-episodes --success-trace SUCCESS.trace --failure-trace FAILURE.trace --output DIFF.json [--success-evidence SUCCESS.json --failure-evidence FAILURE.json]\n  huntctl learn inspect --input BATCH.dtcz\n  huntctl learn baseline --method nearest-neighbor|tabular --input BATCH.dtcz [--input MORE.dtcz] [--query-transition N] [--query-side state|next-state] [--discount D] [--neighbors N --feature INDEX:SCALE:continuous|categorical ...] [--axis INDEX:ORIGIN:WIDTH ...]\n  huntctl learn calibrate (--dataset DATASET.json [--split validation|test|withheld] | --training TRAIN.dtcz --held-out TEST.dtcz) --output REPORT.json [--iterations N] [--n-step N] [--trees N] [--max-depth N] [--seed N] [--discount D] [--all-continuous | --categorical-feature N ...]\n  huntctl learn double-q (--dataset DATASET.json | --input BATCH.dtcz [--input MORE.dtcz]) [--model-output MODEL.json] [--artifact-store ROOT] [--query-transition N] [--query-side state|next-state] [--epochs N] [--hidden-width N] [--learning-rate R] [--target-sync-steps N] [--gradient-clip V] [--seed N] [--discount D]\n  huntctl learn fit (--dataset DATASET.json | --input BATCH.dtcz [--input MORE.dtcz]) [--model-output MODEL.json] [--artifact-store ROOT] [--query-transition N] [--query-side state|next-state] [--iterations N] [--n-step N] [--trees N] [--max-depth N] [--seed N] [--discount D] [--shaping SPEC.json --shaping-report REPORT.json] [--all-continuous | --categorical-feature N ...]"
    );
    eprintln!("  huntctl learn inspect-episode --input IMMUTABLE-EPISODE.json");
    eprintln!(
        "  huntctl learn cql (--dataset DATASET.json | --input BATCH.dtcz [--input MORE.dtcz]) [--model-output MODEL.json] [--artifact-store ROOT] [--query-transition N] [--query-side state|next-state] [--epochs N] [--hidden-width N] [--learning-rate R] [--target-sync-steps N] [--conservative-weight A] [--temperature T] [--gradient-clip V] [--seed N] [--discount D]"
    );
    eprintln!(
        "  huntctl learn iql (--dataset DATASET.json | --input BATCH.dtcz [--input MORE.dtcz]) [--model-output MODEL.json] [--artifact-store ROOT] [--query-transition N] [--query-side state|next-state] [--epochs N] [--hidden-width N] [--learning-rate R] [--discount D] [--expectile T] [--advantage-beta B] [--max-advantage-weight W] [--target-sync-steps N] [--gradient-clip G] [--seed N]"
    );
    eprintln!(
        "  huntctl learn ensemble-q (--dataset DATASET.json | --input BATCH.dtcz [--input MORE.dtcz]) [--model-output MODEL.json] [--artifact-store ROOT] [--query-transition N] [--query-side state|next-state] [--members N] [--epochs N] [--hidden-width N] [--learning-rate R] [--discount D] [--target-sync-steps N] [--gradient-clip G] [--seed N] [--critic-seed N]"
    );
    eprintln!(
        "  huntctl learn prioritized-q (--dataset DATASET.json | --input BATCH.dtcz [--input MORE.dtcz]) [--model-output MODEL.json] [--artifact-store ROOT] [--query-transition N] [--query-side state|next-state] [--epochs N] [--hidden-width N] [--learning-rate R] [--discount D] [--target-sync-steps N] [--gradient-clip G] [--seed N] [--priority-alpha A] [--importance-beta-start B] [--importance-beta-end B] [--priority-epsilon E] [--importance-weight-cap W] [--replay-seed N]"
    );
    eprintln!(
        "  huntctl learn ablate-q --component dueling-heads|n-step|distributional-values|noisy-exploration (--dataset DATASET.json [--split validation|test|withheld] | --training TRAIN.dtcz --held-out TEST.dtcz) --output REPORT.json [--epochs N] [--hidden-width N] [--learning-rate R] [--discount D] [--target-sync-steps N] [--gradient-clip G] [--seed N] [--n-step N] [--distribution-atoms N] [--distribution-min V] [--distribution-max V] [--noisy-stddev V]"
    );
    eprintln!(
        "  huntctl learn option-values --input BATCH.json --model-output MODEL.json [--artifact-store ROOT] [--query-sample N] [--query-side state|next-state] [--iterations N] [--trees N] [--max-depth N] [--min-samples-leaf N] [--features-per-split N] [--max-thresholds N] [--categorical-feature INDEX] [--discount D] [--seed N]"
    );
    eprintln!(
        "\nSemantic oracles:\n  huntctl oracle evaluate --program ORACLES.json --trace RUN.trace [--supplemental OBSERVATIONS.json] [--run-outcome OUTCOME.json] [--output REPORT.json]\n  huntctl oracle compose --manifest COMPOSITION.json [--output EVIDENCE.json]\n  huntctl oracle compare --program ORACLES.json --evidence COMPARISON.json [--output REPORT.json]"
    );
    eprintln!(
        "\nOffline world geometry:\n  huntctl world inventory --stage-dir STAGE_DIR --stage STAGE_ID --output INVENTORY.json [--artifact-store ROOT]\n  huntctl world spatial-index --stage-dir STAGE_DIR --stage STAGE_ID --output INDEX.json [--artifact-store ROOT]\n  huntctl world query point --stage-dir STAGE_DIR --stage STAGE_ID --room N --point X,Y,Z [--max-distance D] [--limit K] [FILTERS]\n  huntctl world query aabb --stage-dir STAGE_DIR --stage STAGE_ID --room N --min X,Y,Z --max X,Y,Z [--limit K] [FILTERS]\n  huntctl world query ray --stage-dir STAGE_DIR --stage STAGE_ID --room N --origin X,Y,Z --direction X,Y,Z --max-distance D [--limit K] [FILTERS]\n  FILTERS: [--load-triggers-only] [--trigger-id ID] [--destination-stage ID] [--destination-room N] [--destination-point N]\n  huntctl world kcl --archive ROOM.arc --prism INDEX [--point X,Y,Z] [--kcl-name room.kcl] [--plc-name room.plc]\n  huntctl world kcl --kcl room.kcl --plc room.plc --prism INDEX [--point X,Y,Z]"
    );
}

fn mock_worker(args: &[String]) -> Result<(), Box<dyn Error>> {
    let revision_label = option(args, "--mock-revision").unwrap_or_else(|| "mock".into());
    let mut revision = format!("{:x}", Sha256::digest(revision_label.as_bytes()));
    revision.truncate(40);
    let aurora_revision = "2".repeat(40);
    let feature_digest = "3".repeat(64);
    let fidelity_profile =
        option(args, "--mock-fidelity-profile").unwrap_or_else(|| "observe_only".into());
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let request: Value = serde_json::from_str(&line?)?;
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let command = request.get("command").and_then(Value::as_str).unwrap_or("");
        let mut response = match command {
            "hello" => json!({
                "protocol": {"name": CONTROL_PROTOCOL_NAME, "version": CONTROL_PROTOCOL_VERSION},
                "type": "hello", "ok": true,
                "build": {
                    "version": "mock", "describe": revision_label, "revision": revision, "branch": "test",
                    "dirty_digest": "", "source_date": "1970-01-01", "aurora_revision": aurora_revision,
                    "compiler": "mock-compiler", "compiler_target": "mock-target", "build_type": "test",
                    "feature_switches": "mock=ON", "feature_digest": feature_digest, "fidelity_profile": fidelity_profile, "platform": env::consts::OS,
                    "architecture": env::consts::ARCH, "pointer_bits": usize::BITS, "dirty": false
                },
                "capabilities": {
                    "persistent_control": true, "engine_session": false, "headless": false,
                    "scenario_load": false, "input_tape": false, "batch_run": false,
                    "commands": ["hello", "ping", "shutdown"]
                }
            }),
            "ping" => success_response("pong"),
            "shutdown" => success_response("shutdown"),
            _ => json!({
                "protocol": {"name": CONTROL_PROTOCOL_NAME, "version": CONTROL_PROTOCOL_VERSION},
                "type": "error", "ok": false,
                "error": {"code": "unknown_command", "message": "unsupported command"}
            }),
        };
        response["id"] = id;
        writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
        stdout.flush()?;
        if command == "shutdown" {
            break;
        }
    }
    Ok(())
}

fn mock_search_worker(args: &[String]) -> Result<(), Box<dyn Error>> {
    let mode = option(args, "--mock-mode").unwrap_or_else(|| "hit".into());
    if mode == "timeout" {
        std::thread::sleep(Duration::from_secs(30));
        return Ok(());
    }
    let result_path = required_path(args, "--milestone-result")?;
    if mode == "malformed" {
        fs::write(result_path, b"{}")?;
        return Ok(());
    }
    let goal = option(args, "--milestone-goal").ok_or("mock worker missing milestone goal")?;
    let requested = option(args, "--milestones").ok_or("mock worker missing milestone list")?;
    let logical_tick_budget = option(args, "--automation-tick-budget")
        .ok_or("mock worker missing automation tick budget")?
        .parse::<u64>()?;
    if logical_tick_budget == 0 {
        return Err("mock worker received a zero automation tick budget".into());
    }
    if mode == "protocol-failure" || mode == "game-crash" {
        writeln!(io::stdout(), "mock native partial stdout before {mode}")?;
        writeln!(io::stderr(), "mock native partial stderr before {mode}")?;
        io::stdout().flush()?;
        io::stderr().flush()?;
        std::process::exit(if mode == "protocol-failure" { 3 } else { 86 });
    }
    let state_root = option(args, "--automation-data-root").unwrap_or_default();
    let input_tape = option(args, "--input-tape")
        .map(|path| -> Result<_, Box<dyn Error>> { Ok(InputTape::decode(&fs::read(path)?)?.tape) })
        .transpose()?
        .unwrap_or_default();
    let controller_duration = option(args, "--input-controller")
        .map(|path| -> Result<_, Box<dyn Error>> {
            Ok(ControllerProgram::decode(&fs::read(path)?)?.duration_frames)
        })
        .transpose()?;
    if input_tape.frames.is_empty() && controller_duration.is_none() {
        return Err("mock worker requires an input tape or controller".into());
    }
    let planned_ticks = controller_duration
        .map(u64::from)
        .unwrap_or(input_tape.frames.len() as u64);
    if planned_ticks > logical_tick_budget {
        return Err(format!(
            "mock worker input requires {planned_ticks} ticks but received budget {logical_tick_budget}"
        )
        .into());
    }
    if let Some(path) = option(args, "--gameplay-trace") {
        fs::write(path, mock_gameplay_trace(&input_tape.boot))?;
    }
    let program_digest = option(args, "--milestone-program")
        .map(|path| -> Result<_, Box<dyn Error>> {
            Ok(Digest(milestone_dsl::decode(&fs::read(path)?)?.program_sha256).to_string())
        })
        .transpose()?;
    let second_attempt = state_root.contains("attempt-002") || state_root.contains("repeat-002");
    let unstable_miss = mode == "unstable-goal" && second_attempt;
    let coordinate_golf_tick = if mode == "coordinate-golf" {
        let pulse_timestamps: Vec<_> = input_tape
            .frames
            .iter()
            .enumerate()
            .filter_map(|(index, frame)| (frame.pads[0].buttons != 0).then_some(index))
            .collect();
        match pulse_timestamps.as_slice() {
            [10, 20] | [10, 19] => Some(100_u64),
            [9, 19] => Some(90_u64),
            _ => None,
        }
    } else {
        None
    };
    let hit_goal = if mode == "coordinate-golf" {
        coordinate_golf_tick.is_some()
    } else {
        mode != "miss" && mode != "target-lost" && !unstable_miss
    };
    if let Some(path) = option(args, "--realized-input-tape") {
        let realized = if let Some(duration) = controller_duration {
            let frame_count = if hit_goal {
                1
            } else if mode == "target-lost" {
                duration.saturating_sub(1)
            } else {
                duration
            };
            InputTape {
                boot: input_tape.boot.clone(),
                frames: vec![
                    huntctl::tape::InputFrame {
                        owned_ports: 1,
                        ..huntctl::tape::InputFrame::default()
                    };
                    usize::try_from(frame_count)?
                ],
                ..InputTape::default()
            }
        } else {
            input_tape.clone()
        };
        fs::write(path, realized.encode()?)?;
    }
    let milestones: Vec<Value> = requested
        .split(',')
        .map(|id| {
            let hit = hit_goal
                || ((mode == "miss" || unstable_miss)
                    && id == "gameplay-ready-f-sp103"
                    && goal == "entered-f-sp104");
            let base_tick = match id {
                "gameplay-ready-f-sp103" => coordinate_golf_tick.unwrap_or(77),
                "exit-f-sp103-to-f-sp104" => 572,
                "entered-f-sp104" => 603,
                _ => 0,
            };
            let tick = base_tick + u64::from(mode == "unstable-tick" && second_attempt);
            let tape_frame = tick + u64::from(mode == "unstable-frame" && second_attempt);
            let mut digest_character = match id {
                "gameplay-ready-f-sp103" => "1",
                "exit-f-sp103-to-f-sp104" => "2",
                "entered-f-sp104" => "3",
                _ => "0",
            };
            if mode == "unstable-fingerprint" && second_attempt {
                digest_character = "a";
            }
            json!({
                "id": id,
                "hit": hit,
                "sim_tick": hit.then_some(tick),
                "tape_frame": hit.then_some(tape_frame),
                "evidence": hit.then(|| json!({
                    "boot": input_tape.boot,
                    "boundary_fingerprint": {
                        "schema": "dusklight.milestone-boundary/v4",
                        "algorithm": "xxh3-128",
                        "canonical_encoding": "little-endian-fixed-v4",
                        "digest": digest_character.repeat(32)
                    }
                }))
            })
        })
        .collect();
    fs::write(
        result_path,
        serde_json::to_vec_pretty(&json!({
            "schema": {
                "name": "dusklight.automation.milestones",
                "version": 5
            },
            "boot": input_tape.boot,
            "boot_origin_established": true,
            "goal": goal,
            "goal_reached": hit_goal,
            "program_digest": program_digest,
            "milestones": milestones
        }))?,
    )?;
    if mode == "miss"
        || mode == "target-lost"
        || unstable_miss
        || (mode == "coordinate-golf" && !hit_goal)
    {
        std::process::exit(2);
    }
    Ok(())
}

fn mock_gameplay_trace(boot: &huntctl::tape::TapeBoot) -> Vec<u8> {
    let mut bytes = vec![0_u8; 225];
    bytes[..8].copy_from_slice(b"DUSKTRCE");
    bytes[8..10].copy_from_slice(&4_u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&128_u16.to_le_bytes());
    bytes[12..16].copy_from_slice(&30_u32.to_le_bytes());
    bytes[16..20].copy_from_slice(&1_u32.to_le_bytes());
    bytes[20..28].copy_from_slice(&1_u64.to_le_bytes());
    bytes[28..32].copy_from_slice(&1_u32.to_le_bytes());
    bytes[32..34].copy_from_slice(&1_u16.to_le_bytes());
    bytes[34..36].copy_from_slice(&64_u16.to_le_bytes());
    bytes[36..44].copy_from_slice(&128_u64.to_le_bytes());
    bytes[44..52].copy_from_slice(&192_u64.to_le_bytes());
    bytes[52..60].copy_from_slice(&1_u64.to_le_bytes());
    if let huntctl::tape::TapeBoot::Stage {
        stage,
        room,
        point,
        layer,
        save_slot,
        fixture,
        ..
    } = boot
    {
        bytes[64] = 1;
        bytes[65] = save_slot.unwrap_or(0);
        bytes[66] = *room as u8;
        bytes[67] = *layer as u8;
        bytes[68..70].copy_from_slice(&point.to_le_bytes());
        bytes[70] = stage.len() as u8;
        bytes[72..72 + stage.len()].copy_from_slice(stage.as_bytes());
        if let Some(fixture) = fixture {
            let encoded = fixture.encode().expect("validated mock fixture");
            let fixture_offset = bytes.len() as u64;
            bytes[88..96].copy_from_slice(&fixture_offset.to_le_bytes());
            bytes[96..100].copy_from_slice(&(encoded.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&encoded);
        }
    }
    bytes[128..130].copy_from_slice(&0_u16.to_le_bytes());
    bytes[130..132].copy_from_slice(&1_u16.to_le_bytes());
    bytes[132..136].copy_from_slice(&3_u32.to_le_bytes());
    bytes[136..140].copy_from_slice(&32_u32.to_le_bytes());
    bytes[140..144].copy_from_slice(&1_u32.to_le_bytes());
    bytes[144..152].copy_from_slice(&192_u64.to_le_bytes());
    bytes[152..160].copy_from_slice(&1_u64.to_le_bytes());
    bytes[160..168].copy_from_slice(&193_u64.to_le_bytes());
    bytes[168..176].copy_from_slice(&32_u64.to_le_bytes());
    bytes[192] = 1;
    bytes[193..201].copy_from_slice(&1_u64.to_le_bytes());
    bytes[201..209].copy_from_slice(&0_u64.to_le_bytes());
    bytes[209..217].copy_from_slice(&u64::MAX.to_le_bytes());
    bytes[217..221].copy_from_slice(&1_u32.to_le_bytes());
    bytes[221] = 2;
    bytes[222] = 1;
    bytes
}

fn mock_record_worker(args: &[String]) -> Result<(), Box<dyn Error>> {
    let output = required_path(args, "--record-input-tape")?;
    let tape = InputTape {
        frames: vec![huntctl::tape::InputFrame::default()],
        ..InputTape::default()
    };
    fs::write(output, tape.encode()?)?;
    Ok(())
}

fn success_response(response_type: &str) -> Value {
    json!({
        "protocol": {"name": CONTROL_PROTOCOL_NAME, "version": CONTROL_PROTOCOL_VERSION},
        "type": response_type, "ok": true
    })
}
