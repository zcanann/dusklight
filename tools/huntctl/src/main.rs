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

const MAX_LEARN_INPUT_CORPORA: usize = 64;

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
        "tape" => command_tape(&args[1..]),
        "trace" => command_trace(&args[1..]),
        "timeline" => command_timeline(&args[1..]),
        "search" => command_search(&args[1..]),
        "learn" => command_learn(&args[1..]),
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
    if !flag(args, "--dry-run") {
        return Err(
            "campaign execution is not enabled yet; use --dry-run to inspect the resolved plan"
                .into(),
        );
    }
    let proposer_names = repeated_option(args, "--proposer");
    let proposers = if proposer_names.is_empty() {
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
    let plan = huntctl::harness::campaign::resolve_campaign_plan(
        &huntctl::harness::campaign::CampaignPlanConfig {
            repository_root: &repository_root,
            suite_path: &suite,
            case_id: &case,
            output_root: &output,
            proposers: &proposers,
        },
    )?;
    println!("{}", serde_json::to_string_pretty(&plan)?);
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
        _ => Err("identity command: compare --mode replay|trace-merge|model-training|checkpoint-restore|cross-build-comparison --expected EXPECTED.json --actual ACTUAL.json".into()),
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

fn command_conservative_q(learn_args: &[String]) -> Result<(), Box<dyn Error>> {
    let direct_inputs = repeated_option(learn_args, "--input");
    let dataset_path = option(learn_args, "--dataset").map(PathBuf::from);
    if dataset_path.is_some() && !direct_inputs.is_empty() {
        return Err("learn cql accepts either --dataset or --input, not both".into());
    }
    let dataset_manifest: Option<DatasetManifest> = dataset_path
        .as_ref()
        .map(|path| -> Result<_, Box<dyn Error>> {
            let manifest: DatasetManifest = serde_json::from_slice(&fs::read(path)?)?;
            manifest.validate()?;
            Ok(manifest)
        })
        .transpose()?;
    let inputs = if let Some(manifest) = &dataset_manifest {
        manifest
            .entries
            .iter()
            .filter(|entry| entry.split == DatasetSplit::Train)
            .map(|entry| entry.transition_corpus.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
    } else {
        direct_inputs
    };
    let training = load_fqi_batch(&inputs, "CQL training", MAX_LEARN_INPUT_CORPORA)?;
    let expected_corpus_digests = dataset_manifest.as_ref().map(|manifest| {
        manifest
            .entries
            .iter()
            .filter(|entry| entry.split == DatasetSplit::Train)
            .map(|entry| entry.corpus_sha256)
            .collect::<Vec<_>>()
    });
    if expected_corpus_digests
        .as_ref()
        .is_some_and(|expected| expected != &training.corpus_digests)
    {
        return Err("CQL corpus content differs from dataset manifest".into());
    }
    let defaults = ConservativeQConfig::default();
    let config = ConservativeQConfig {
        double_q: DoubleQConfig {
            epochs: usize_option(learn_args, "--epochs", defaults.double_q.epochs)?,
            hidden_width: usize_option(
                learn_args,
                "--hidden-width",
                defaults.double_q.hidden_width,
            )?,
            learning_rate: option(learn_args, "--learning-rate")
                .map(|value| value.parse::<f64>())
                .transpose()?
                .unwrap_or(defaults.double_q.learning_rate),
            discount: option(learn_args, "--discount")
                .map(|value| value.parse::<f64>())
                .transpose()?
                .unwrap_or(defaults.double_q.discount),
            target_sync_steps: usize_option(
                learn_args,
                "--target-sync-steps",
                defaults.double_q.target_sync_steps,
            )?,
            gradient_clip: option(learn_args, "--gradient-clip")
                .map(|value| value.parse::<f64>())
                .transpose()?
                .unwrap_or(defaults.double_q.gradient_clip),
            seed: u64_option(learn_args, "--seed", defaults.double_q.seed)?,
        },
        conservative_weight: option(learn_args, "--conservative-weight")
            .map(|value| value.parse::<f64>())
            .transpose()?
            .unwrap_or(defaults.conservative_weight),
        temperature: option(learn_args, "--temperature")
            .map(|value| value.parse::<f64>())
            .transpose()?
            .unwrap_or(defaults.temperature),
    };
    let action_support = training.transitions.iter().fold(
        BTreeMap::<u32, usize>::new(),
        |mut counts, transition| {
            *counts.entry(transition.action).or_default() += 1;
            counts
        },
    );
    if action_support.len() > MAX_FQI_ACTIONS {
        return Err(format!(
            "CQL supports at most {MAX_FQI_ACTIONS} distinct actions; received {}",
            action_support.len()
        )
        .into());
    }
    let actions = action_support.keys().copied().collect::<Vec<_>>();
    let model = ConservativeQ::fit(
        training.feature_count,
        &actions,
        &training.transitions,
        &config,
    )?;
    let query_index = usize_option(learn_args, "--query-transition", 0)?;
    let query_transition = training
        .transitions
        .get(query_index)
        .ok_or("--query-transition is outside the merged transition batch")?;
    let query_side = option(learn_args, "--query-side").unwrap_or_else(|| "state".into());
    let query_state = match query_side.as_str() {
        "state" => &query_transition.state,
        "next-state" => &query_transition.next_state,
        _ => return Err("--query-side must be state or next-state".into()),
    };
    let ranking = model
        .rank_actions(query_state)?
        .into_iter()
        .map(|estimate| {
            json!({
                "action": estimate.action,
                "mean_q": estimate.mean,
                "critic_a": estimate.critic_a,
                "critic_b": estimate.critic_b,
                "critic_disagreement": estimate.critic_disagreement,
                "support": action_support[&estimate.action],
            })
        })
        .collect::<Vec<_>>();
    let model_output = option(learn_args, "--model-output").map(PathBuf::from);
    let mut model_content_blob = None;
    let mut model_artifact_store = None;
    if let Some(path) = &model_output {
        if path.exists() {
            return Err(format!("CQL model output already exists: {}", path.display()).into());
        }
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        let bytes = model.artifact_bytes(
            training.feature_schema,
            training.action_schema,
            dataset_manifest
                .as_ref()
                .map(|manifest| manifest.dataset_sha256),
            &training.corpus_digests,
            &config,
        )?;
        fs::write(path, &bytes)?;
        let store_path = option(learn_args, "--artifact-store")
            .map(PathBuf::from)
            .unwrap_or_else(|| path.parent().unwrap_or(Path::new(".")).join("content"));
        model_content_blob =
            Some(ContentStore::initialize(&store_path)?.put_bytes(&bytes, ContentKind::Model)?);
        model_artifact_store = Some(store_path);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": "dusklight-conservative-q-ranking/v1",
            "feature_schema": training.feature_schema,
            "action_schema": training.action_schema,
            "input_corpora": inputs,
            "training_corpus_sha256": training.corpus_digests,
            "training_dataset": dataset_path,
            "training_dataset_sha256": dataset_manifest.as_ref().map(|manifest| manifest.dataset_sha256),
            "transition_count": training.transitions.len(),
            "episode_groups": training.episode_groups.iter().copied().collect::<BTreeSet<_>>().len(),
            "query_transition": query_index,
            "query_side": query_side,
            "config": config,
            "gradient_updates": model.gradient_updates(),
            "target_synchronizations": model.target_synchronizations(),
            "conservative_updates": model.conservative_updates(),
            "mean_conservative_gap": model.mean_conservative_gap(),
            "conservative_objective": "temperature_logsumexp_all_actions_minus_observed_action",
            "model_output": model_output,
            "model_artifact_store": model_artifact_store,
            "model_content_blob": model_content_blob,
            "ranking": ranking,
            "promotion_authority": false,
            "limitations": [
                "CQL reduces but does not prove safety for state-local unsupported actions",
                "numeric normalization does not provide categorical embeddings or missingness masks",
                "critic disagreement is not calibrated uncertainty",
                "rankings are proposals and require native predicate and cold replay proof"
            ]
        }))?
    );
    Ok(())
}

fn command_learn(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("cql") => command_conservative_q(&args[1..]),
        Some("iql") => cli::learning::command_iql(&args[1..]),
        Some("ensemble-q") => cli::learning::command_ensemble_q(&args[1..]),
        Some("prioritized-q") => cli::learning::command_prioritized_q(&args[1..]),
        Some("ablate-q") => cli::learning::command_q_ablation(&args[1..]),
        Some("option-values") => cli::learning::command_option_values(&args[1..]),
        Some("diff-episodes") => {
            let learn_args = &args[1..];
            let success_trace_path = required_path(learn_args, "--success-trace")?;
            let failure_trace_path = required_path(learn_args, "--failure-trace")?;
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(
                    format!("trace diff output already exists: {}", output.display()).into(),
                );
            }
            let success_evidence_path = option(learn_args, "--success-evidence").map(PathBuf::from);
            let failure_evidence_path = option(learn_args, "--failure-evidence").map(PathBuf::from);
            if success_evidence_path.is_some() != failure_evidence_path.is_some() {
                return Err(
                    "--success-evidence and --failure-evidence must be supplied together".into(),
                );
            }
            let success_bytes = fs::read(&success_trace_path)?;
            let failure_bytes = fs::read(&failure_trace_path)?;
            let success_trace = huntctl::trace::decode(&success_bytes)?;
            let failure_trace = huntctl::trace::decode(&failure_bytes)?;
            let success_evidence: Option<TransitionEvidenceBundle> = success_evidence_path
                .as_ref()
                .map(|path| fs::read(path).map_err(Box::<dyn Error>::from))
                .transpose()?
                .map(|bytes| serde_json::from_slice(&bytes))
                .transpose()?;
            let failure_evidence: Option<TransitionEvidenceBundle> = failure_evidence_path
                .as_ref()
                .map(|path| fs::read(path).map_err(Box::<dyn Error>::from))
                .transpose()?
                .map(|bytes| serde_json::from_slice(&bytes))
                .transpose()?;
            let report = SiblingTraceDiff::compare(
                &success_trace,
                &success_bytes,
                &failure_trace,
                &failure_bytes,
                success_evidence.as_ref(),
                failure_evidence.as_ref(),
            )?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, serde_json::to_vec_pretty(&report)?)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("dataset") => {
            let learn_args = &args[1..];
            let source_paths = repeated_option(learn_args, "--source");
            if source_paths.is_empty() {
                return Err("learn dataset requires at least one --source SOURCE.json".into());
            }
            let output = required_path(learn_args, "--output")?;
            if output.exists() {
                return Err(format!("dataset output already exists: {}", output.display()).into());
            }
            let mut sources = Vec::with_capacity(source_paths.len());
            for source_path in &source_paths {
                let source_path = PathBuf::from(source_path);
                let descriptor: DatasetSourceDescriptor =
                    serde_json::from_slice(&fs::read(&source_path)?)?;
                sources.push(descriptor.load(source_path.parent().unwrap_or(Path::new(".")))?);
            }
            let validation_percent =
                u8::try_from(usize_option(learn_args, "--validation-percent", 10)?)?;
            let test_percent = u8::try_from(usize_option(learn_args, "--test-percent", 10)?)?;
            let manifest = DatasetManifest::build(
                &sources,
                &DatasetBuildConfig {
                    validation_percent,
                    test_percent,
                    withheld_objectives: repeated_option(learn_args, "--withheld-objective")
                        .into_iter()
                        .collect(),
                },
            )?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let bytes = serde_json::to_vec_pretty(&manifest)?;
            fs::write(&output, &bytes)?;
            let artifact_store = option(learn_args, "--artifact-store")
                .map(PathBuf::from)
                .unwrap_or_else(|| output.parent().unwrap_or(Path::new(".")).join("content"));
            let content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&bytes, ContentKind::DatasetManifest)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": manifest.schema,
                    "dataset_sha256": manifest.dataset_sha256,
                    "frozen_withheld_sha256": manifest.frozen_withheld_sha256,
                    "output": output,
                    "artifact_store": artifact_store,
                    "content_blob": content_blob,
                    "report": manifest.report,
                    "leakage": manifest.leakage,
                    "normalization_schemas": manifest.normalization.len(),
                }))?
            );
            Ok(())
        }
        Some("extract-trace") => {
            let learn_args = &args[1..];
            let trace_path = required_path(learn_args, "--trace")?;
            let tape_path = required_path(learn_args, "--tape")?;
            let episode_context_path = required_path(learn_args, "--episode-context")?;
            let output = required_path(learn_args, "--output")?;
            let start_tape_frame: u64 = option(learn_args, "--start-frame")
                .ok_or("missing required --start-frame N")?
                .parse()?;
            let end_tape_frame: u64 = option(learn_args, "--end-frame")
                .ok_or("missing required --end-frame N")?
                .parse()?;
            let trace_bytes = fs::read(&trace_path)?;
            let tape_bytes = fs::read(&tape_path)?;
            let episode_context: EpisodeContext =
                serde_json::from_slice(&fs::read(&episode_context_path)?)?;
            episode_context.validate()?;
            let episode_digest = if let Some(value) = option(learn_args, "--episode-digest") {
                value.parse::<Digest>()?
            } else {
                let mut hasher = Sha256::new();
                hasher.update(b"dusklight.exploratory-offline-episode/v1\0");
                hasher.update((trace_bytes.len() as u64).to_le_bytes());
                hasher.update(&trace_bytes);
                hasher.update((tape_bytes.len() as u64).to_le_bytes());
                hasher.update(&tape_bytes);
                Digest(hasher.finalize().into())
            };
            let end_is_terminal = learn_args.iter().any(|arg| arg == "--terminal");
            let feature_view =
                option(learn_args, "--view").unwrap_or_else(|| "movement-state/v1".into());
            let extract_config = ExploratoryExtractConfig {
                episode_digest,
                start_tape_frame,
                end_tape_frame,
                start_reference: None,
                terminal_reference: None,
                end_is_terminal,
            };
            let corpus = match feature_view.as_str() {
                "movement-state/v1" => {
                    extract_exploratory_from_bytes(&trace_bytes, &tape_bytes, extract_config)?
                }
                MOVEMENT_STATE_V2_ID => {
                    extract_exploratory_v2_from_bytes(&trace_bytes, &tape_bytes, extract_config)?
                }
                _ => {
                    return Err(format!(
                        "unknown --view {feature_view:?}; expected movement-state/v1 or {MOVEMENT_STATE_V2_ID}"
                    )
                    .into());
                }
            };
            let decoded_trace = huntctl::trace::decode(&trace_bytes)?;
            let decoded_tape = InputTape::decode(&tape_bytes)?.tape;
            let transition_evidence = TransitionEvidenceBundle::build(TransitionEvidenceBuild {
                corpus: &corpus,
                trace: &decoded_trace,
                tape: &decoded_tape,
                trace_sha256: Digest(Sha256::digest(&trace_bytes).into()),
                tape_sha256: Digest(Sha256::digest(&tape_bytes).into()),
                start_tape_frame,
                end_tape_frame,
                terminal_reason: end_is_terminal
                    .then_some(TerminalReasonEvidence::DeclaredExtractionBoundary),
            })?;
            let transition_evidence_bytes = serde_json::to_vec_pretty(&transition_evidence)?;
            let trace_sha256 = Digest(Sha256::digest(&trace_bytes).into());
            let tape_sha256 = Digest(Sha256::digest(&tape_bytes).into());
            let episode_manifest = EpisodeManifest::build(EpisodeManifestBuild {
                context: &episode_context,
                boot: &decoded_tape.boot,
                corpus: &corpus,
                query_view_id: &feature_view,
                tape_sha256,
                trace_sha256,
                transition_evidence_sha256: Digest(
                    Sha256::digest(&transition_evidence_bytes).into(),
                ),
            })?;
            let compression_level: i32 = option(learn_args, "--compression-level")
                .map(|value| value.parse())
                .transpose()?
                .unwrap_or(3);
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let content_digest = corpus.write_zstd_file(&output, compression_level)?;
            let artifact_store = option(learn_args, "--artifact-store")
                .map(PathBuf::from)
                .unwrap_or_else(|| output.parent().unwrap_or(Path::new(".")).join("content"));
            let trace_content_blob = ContentStore::initialize(&artifact_store)?
                .put_bytes(&trace_bytes, ContentKind::GameplayTrace)?;
            let transition_evidence_path =
                PathBuf::from(format!("{}.evidence.json", output.display()));
            fs::write(&transition_evidence_path, transition_evidence_bytes)?;
            let episode_manifest_path = PathBuf::from(format!("{}.episode.json", output.display()));
            fs::write(
                &episode_manifest_path,
                serde_json::to_vec_pretty(&episode_manifest)?,
            )?;
            let dataset_source_path =
                PathBuf::from(format!("{}.dataset-source.json", output.display()));
            fs::write(
                &dataset_source_path,
                serde_json::to_vec_pretty(&DatasetSourceDescriptor {
                    schema: DATASET_SOURCE_SCHEMA_V1.into(),
                    source_id: episode_manifest.episode_sha256.to_string(),
                    episode_manifest: fs::canonicalize(&episode_manifest_path)?,
                    transition_corpus: fs::canonicalize(&output)?,
                    absolute_tape: fs::canonicalize(&tape_path)?,
                    transition_evidence: fs::canonicalize(&transition_evidence_path)?,
                    gameplay_trace: fs::canonicalize(&trace_path)?,
                    route_family: episode_manifest.objective.id.clone(),
                    screenshot_sha256: Vec::new(),
                    checkpoint_sha256: Vec::new(),
                })?,
            )?;
            let observation_spec = if feature_view == MOVEMENT_STATE_V2_ID {
                let spec = movement_state_v2_spec();
                let path = PathBuf::from(format!("{}.observation.json", output.display()));
                fs::write(&path, spec.canonical_bytes()?)?;
                Some(path)
            } else {
                None
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": "dusklight-exploratory-extraction/v1",
                    "authoritative": false,
                    "limitations": [
                        "the batch contains observed behavior, not counterfactual actions",
                        "explicit frame bounds are not native milestone proof",
                        "--terminal records a declared extraction boundary, not inferred objective proof",
                        "the observation view is objective-specific and not a complete process state"
                    ],
                    "trace": trace_path,
                    "trace_content_blob": trace_content_blob,
                    "artifact_store": artifact_store,
                    "tape": tape_path,
                    "output": output,
                    "transition_evidence": transition_evidence_path,
                    "episode_context": episode_context_path,
                    "episode_manifest": episode_manifest_path,
                    "dataset_source": dataset_source_path,
                    "input_identity": episode_manifest.input_identity_sha256,
                    "episode_identity": episode_manifest.episode_sha256,
                    "feature_view": feature_view,
                    "observation_spec": observation_spec,
                    "episode_digest": episode_digest,
                    "content_digest": content_digest,
                    "feature_schema": corpus.feature_schema,
                    "action_schema": corpus.action_schema,
                    "feature_count": corpus.feature_count,
                    "transitions": corpus.transitions.len(),
                    "start_frame": start_tape_frame,
                    "end_frame": end_tape_frame,
                    "terminal": end_is_terminal,
                }))?
            );
            Ok(())
        }
        Some("inspect-episode") => {
            let input = required_path(&args[1..], "--input")?;
            let artifact: ImmutableEpisodeArtifact = serde_json::from_slice(&fs::read(&input)?)?;
            artifact.validate()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": artifact.schema,
                    "content_sha256": artifact.content_sha256,
                    "episode_sha256": artifact.episode_sha256,
                    "objective": artifact.objective,
                    "terminal": artifact.terminal,
                    "terminal_detail": artifact.terminal_detail,
                    "realized_tape_sha256": artifact.realized_tape_sha256,
                    "gameplay_trace_sha256": artifact.gameplay_trace_sha256,
                    "transition_corpus_sha256": artifact.transition_corpus_sha256,
                    "transition_evidence_sha256": artifact.transition_evidence_sha256,
                    "steps": artifact.steps.len(),
                    "lineage": artifact.lineage,
                }))?
            );
            Ok(())
        }
        Some("inspect") => {
            let corpus = TransitionCorpus::read_zstd_file(required_path(&args[1..], "--input")?)?;
            let mut action_counts = BTreeMap::<u32, usize>::new();
            let mut terminal_transitions = 0_usize;
            for transition in &corpus.transitions {
                *action_counts
                    .entry(transition.action.action_id)
                    .or_default() += 1;
                terminal_transitions += usize::from(transition.terminal);
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": "dusklight-transition-inspection/v1",
                    "content_digest": corpus.content_digest()?,
                    "feature_schema": corpus.feature_schema,
                    "action_schema": corpus.action_schema,
                    "feature_count": corpus.feature_count,
                    "transitions": corpus.transitions.len(),
                    "terminal_transitions": terminal_transitions,
                    "action_counts": action_counts,
                }))?
            );
            Ok(())
        }
        Some("baseline") => {
            let learn_args = &args[1..];
            let inputs = repeated_option(learn_args, "--input");
            if inputs.is_empty() || inputs.len() > MAX_LEARN_INPUT_CORPORA {
                return Err(format!(
                    "learn baseline requires 1..={MAX_LEARN_INPUT_CORPORA} --input corpora"
                )
                .into());
            }
            let method = option(learn_args, "--method")
                .ok_or("learn baseline requires --method nearest-neighbor|tabular")?;
            let discount = option(learn_args, "--discount")
                .map(|value| value.parse::<f32>())
                .transpose()?
                .unwrap_or(1.0);
            let mut feature_schema = None;
            let mut action_schema = None;
            let mut feature_count = None;
            let mut transitions = Vec::new();
            let mut episode_groups = Vec::new();
            let mut next_episode_group = 0_u64;
            for input in &inputs {
                let corpus = TransitionCorpus::read_zstd_file(input)?;
                if feature_schema.is_some_and(|value| value != corpus.feature_schema)
                    || action_schema.is_some_and(|value| value != corpus.action_schema)
                    || feature_count.is_some_and(|value| value != corpus.feature_count)
                {
                    return Err("baseline corpora use incompatible schemas".into());
                }
                feature_schema = Some(corpus.feature_schema);
                action_schema = Some(corpus.action_schema);
                feature_count = Some(corpus.feature_count);
                let mut ended_terminal = false;
                for transition in corpus.transitions {
                    let terminal = transition.terminal;
                    transitions.push(FqiTransition {
                        state: transition.state,
                        action: transition.action.action_id,
                        duration: transition.duration_ticks,
                        reward: transition.reward,
                        next_state: transition.next_state,
                        terminal,
                    });
                    episode_groups.push(next_episode_group);
                    ended_terminal = terminal;
                    if terminal {
                        next_episode_group = next_episode_group
                            .checked_add(1)
                            .ok_or("baseline episode-group count overflowed")?;
                    }
                }
                if !ended_terminal {
                    next_episode_group = next_episode_group
                        .checked_add(1)
                        .ok_or("baseline episode-group count overflowed")?;
                }
            }
            let query_index = usize_option(learn_args, "--query-transition", 0)?;
            let query = transitions
                .get(query_index)
                .ok_or("--query-transition is outside the merged transition batch")?;
            let query_side = option(learn_args, "--query-side").unwrap_or_else(|| "state".into());
            let query_state = match query_side.as_str() {
                "state" => &query.state,
                "next-state" => &query.next_state,
                _ => return Err("--query-side must be state or next-state".into()),
            };
            let samples = empirical_return_samples(&transitions, &episode_groups, discount)?;
            let (ranking, configuration) = match method.as_str() {
                "nearest-neighbor" => {
                    let declared = repeated_option(learn_args, "--feature");
                    let categorical = if feature_schema == Some(movement_feature_schema_digest_v1())
                    {
                        MOVEMENT_CATEGORICAL_FEATURES_V1.to_vec()
                    } else if feature_schema == Some(movement_state_v2_spec().digest()?) {
                        movement_state_v2_spec().categorical_features()
                    } else {
                        Vec::new()
                    };
                    let features = if declared.is_empty() {
                        if categorical.is_empty() {
                            return Err("unknown schema requires repeated --feature INDEX:SCALE:continuous|categorical".into());
                        }
                        (0..feature_count.unwrap() as usize)
                            .map(|index| LocalFeature {
                                index,
                                scale: 1.0,
                                categorical: categorical.contains(&index),
                            })
                            .collect::<Vec<_>>()
                    } else {
                        declared
                            .iter()
                            .map(|value| -> Result<LocalFeature, Box<dyn Error>> {
                                let parts = value.split(':').collect::<Vec<_>>();
                                if parts.len() != 3
                                    || !matches!(parts[2], "continuous" | "categorical")
                                {
                                    return Err(
                                        "--feature syntax is INDEX:SCALE:continuous|categorical"
                                            .into(),
                                    );
                                }
                                Ok(LocalFeature {
                                    index: parts[0].parse()?,
                                    scale: parts[1].parse()?,
                                    categorical: parts[2] == "categorical",
                                })
                            })
                            .collect::<Result<Vec<_>, _>>()?
                    };
                    let neighbors = usize_option(learn_args, "--neighbors", 8)?;
                    let model = NearestNeighborReturn::fit(
                        samples,
                        LocalReturnConfig {
                            neighbors,
                            features: features.clone(),
                        },
                    )?;
                    (
                        model.rank(query_state)?,
                        json!({
                            "neighbors": neighbors,
                            "features": features.iter().map(|feature| json!({
                                "index": feature.index,
                                "scale": feature.scale,
                                "categorical": feature.categorical,
                            })).collect::<Vec<_>>(),
                        }),
                    )
                }
                "tabular" => {
                    let axes = repeated_option(learn_args, "--axis")
                        .iter()
                        .map(|value| -> Result<TabularAxis, Box<dyn Error>> {
                            let parts = value.split(':').collect::<Vec<_>>();
                            if parts.len() != 3 {
                                return Err("--axis syntax is INDEX:ORIGIN:WIDTH".into());
                            }
                            Ok(TabularAxis {
                                index: parts[0].parse()?,
                                origin: parts[1].parse()?,
                                width: parts[2].parse()?,
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let model = TabularReturn::fit(&samples, axes.clone())?;
                    (
                        model.rank(query_state)?,
                        json!({
                            "axes": axes.iter().map(|axis| json!({
                                "index": axis.index,
                                "origin": axis.origin,
                                "width": axis.width,
                            })).collect::<Vec<_>>(),
                        }),
                    )
                }
                _ => return Err("--method must be nearest-neighbor or tabular".into()),
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": "dusklight-low-data-baseline/v1",
                    "method": method,
                    "feature_schema": feature_schema,
                    "action_schema": action_schema,
                    "input_corpora": inputs,
                    "episode_groups": episode_groups.iter().copied().collect::<BTreeSet<_>>().len(),
                    "transitions": transitions.len(),
                    "per_tick_discount": discount,
                    "query_transition": query_index,
                    "query_side": query_side,
                    "configuration": configuration,
                    "ranking": ranking,
                    "limitations": [
                        "empirical observed returns only; no counterfactual inference",
                        "a nonterminal episode end is truncated and receives no cross-episode bootstrap",
                        "rankings are proposal heuristics and require native rollout proof"
                    ]
                }))?
            );
            Ok(())
        }
        Some("calibrate") => {
            let learn_args = &args[1..];
            let dataset_path = option(learn_args, "--dataset").map(PathBuf::from);
            let explicit_training = repeated_option(learn_args, "--training");
            let explicit_held_out = repeated_option(learn_args, "--held-out");
            if dataset_path.is_some()
                == (!explicit_training.is_empty() || !explicit_held_out.is_empty())
            {
                return Err("learn calibrate requires either --dataset or both --training/--held-out corpora".into());
            }
            let mut dataset_identity = None;
            let mut held_out_split = None;
            let mut expected_dataset_corpus_digests = None;
            let (training_paths, held_out_paths) = if let Some(path) = &dataset_path {
                let manifest: DatasetManifest = serde_json::from_slice(&fs::read(path)?)?;
                manifest.validate()?;
                let split = match option(learn_args, "--split")
                    .unwrap_or_else(|| "test".into())
                    .as_str()
                {
                    "validation" => DatasetSplit::Validation,
                    "test" => DatasetSplit::Test,
                    "withheld" => DatasetSplit::Withheld,
                    _ => return Err("--split must be validation, test, or withheld".into()),
                };
                let training = manifest
                    .entries
                    .iter()
                    .filter(|entry| entry.split == DatasetSplit::Train)
                    .map(|entry| entry.transition_corpus.to_string_lossy().into_owned())
                    .collect::<Vec<_>>();
                let held_out = manifest
                    .entries
                    .iter()
                    .filter(|entry| entry.split == split)
                    .map(|entry| entry.transition_corpus.to_string_lossy().into_owned())
                    .collect::<Vec<_>>();
                expected_dataset_corpus_digests = Some((
                    manifest
                        .entries
                        .iter()
                        .filter(|entry| entry.split == DatasetSplit::Train)
                        .map(|entry| entry.corpus_sha256)
                        .collect::<Vec<_>>(),
                    manifest
                        .entries
                        .iter()
                        .filter(|entry| entry.split == split)
                        .map(|entry| entry.corpus_sha256)
                        .collect::<Vec<_>>(),
                ));
                dataset_identity = Some(manifest.dataset_sha256);
                held_out_split = Some(split);
                (training, held_out)
            } else {
                if explicit_training.is_empty() || explicit_held_out.is_empty() {
                    return Err(
                        "explicit calibration requires both --training and --held-out".into(),
                    );
                }
                (explicit_training, explicit_held_out)
            };
            let training_files = training_paths
                .iter()
                .map(fs::canonicalize)
                .collect::<Result<BTreeSet<_>, _>>()?;
            let held_out_files = held_out_paths
                .iter()
                .map(fs::canonicalize)
                .collect::<Result<BTreeSet<_>, _>>()?;
            if !training_files.is_disjoint(&held_out_files) {
                return Err("training and held-out calibration files overlap".into());
            }
            let training = load_fqi_batch(
                &training_paths,
                "calibration training",
                MAX_LEARN_INPUT_CORPORA,
            )?;
            let held_out = load_fqi_batch(
                &held_out_paths,
                "calibration held-out",
                MAX_LEARN_INPUT_CORPORA,
            )?;
            if expected_dataset_corpus_digests.as_ref().is_some_and(
                |(expected_training, expected_held_out)| {
                    expected_training != &training.corpus_digests
                        || expected_held_out != &held_out.corpus_digests
                },
            ) {
                return Err("calibration corpus content differs from dataset manifest".into());
            }
            if training.feature_schema != held_out.feature_schema
                || training.action_schema != held_out.action_schema
                || training.feature_count != held_out.feature_count
                || !training
                    .corpus_digests
                    .iter()
                    .all(|digest| !held_out.corpus_digests.contains(digest))
            {
                return Err(
                    "calibration requires compatible schemas and content-disjoint held-out corpora"
                        .into(),
                );
            }
            let mut config = FqiConfig {
                iterations: usize_option(learn_args, "--iterations", 24)?,
                backup_steps: usize_option(learn_args, "--n-step", 1)?,
                trees_per_action: usize_option(learn_args, "--trees", 31)?,
                max_tree_depth: usize_option(learn_args, "--max-depth", 8)?,
                seed: u64_option(learn_args, "--seed", FqiConfig::default().seed)?,
                discount: option(learn_args, "--discount")
                    .map(|value| value.parse::<f32>())
                    .transpose()?
                    .unwrap_or(FqiConfig::default().discount),
                ..FqiConfig::default()
            };
            if config.iterations == 0
                || config.iterations > MAX_FQI_ITERATIONS
                || config.backup_steps == 0
                || config.backup_steps > MAX_FQI_BACKUP_STEPS
                || config.trees_per_action == 0
                || config.trees_per_action > MAX_FQI_TREES_PER_ACTION
                || config.max_tree_depth > MAX_FQI_TREE_DEPTH
            {
                return Err("invalid bounded calibration FQI configuration".into());
            }
            let declared_categorical = repeated_option(learn_args, "--categorical-feature")
                .into_iter()
                .map(|value| value.parse::<usize>())
                .collect::<Result<Vec<_>, _>>()?;
            let declared_all_continuous = learn_args.iter().any(|arg| arg == "--all-continuous");
            if declared_all_continuous && !declared_categorical.is_empty() {
                return Err(
                    "--all-continuous and --categorical-feature cannot be used together".into(),
                );
            }
            if training.feature_schema == movement_feature_schema_digest_v1() {
                if declared_all_continuous || !declared_categorical.is_empty() {
                    return Err(
                        "the authenticated movement schema owns its categorical feature map".into(),
                    );
                }
                config.categorical_features = MOVEMENT_CATEGORICAL_FEATURES_V1.to_vec();
            } else if training.feature_schema == movement_state_v2_spec().digest()? {
                if declared_all_continuous || !declared_categorical.is_empty() {
                    return Err(
                        "the authenticated movement schema owns its categorical feature map".into(),
                    );
                }
                config.categorical_features = movement_state_v2_spec().categorical_features();
            } else if declared_all_continuous {
                config.categorical_features.clear();
            } else if !declared_categorical.is_empty() {
                config.categorical_features = declared_categorical;
            } else {
                return Err("unknown feature schema: declare --all-continuous or repeat --categorical-feature N".into());
            }
            let actions = training
                .transitions
                .iter()
                .map(|transition| transition.action)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            if actions.is_empty() || actions.len() > MAX_FQI_ACTIONS {
                return Err("calibration training action support is outside bounds".into());
            }
            let model = FittedQ::fit_with_episode_groups(
                training.feature_count,
                &actions,
                &training.transitions,
                &training.episode_groups,
                &config,
            )?;
            let held_out_samples = empirical_return_samples(
                &held_out.transitions,
                &held_out.episode_groups,
                config.discount,
            )?;
            let calibration = calibrate_fitted_q(&model, &held_out_samples)?;
            let output_path = required_path(learn_args, "--output")?;
            if output_path.exists() {
                return Err(format!(
                    "calibration output already exists: {}",
                    output_path.display()
                )
                .into());
            }
            let report = json!({
                "schema": "dusklight-held-out-fqi-calibration/v1",
                "dataset": dataset_path,
                "dataset_sha256": dataset_identity,
                "held_out_split": held_out_split,
                "training_corpora": training_paths,
                "training_corpus_sha256": training.corpus_digests,
                "held_out_corpora": held_out_paths,
                "held_out_corpus_sha256": held_out.corpus_digests,
                "feature_schema": training.feature_schema,
                "action_schema": training.action_schema,
                "training_episode_groups": training.episode_groups.iter().copied().collect::<BTreeSet<_>>().len(),
                "held_out_episode_groups": held_out.episode_groups.iter().copied().collect::<BTreeSet<_>>().len(),
                "config": config,
                "calibration": calibration,
                "promotion_authority": false,
                "limitations": [
                    "exact-state proposal win rate is measured only where held-out actions are comparable",
                    "unsupported held-out actions and proposed actions remain explicit OOD diagnostics",
                    "calibration is analysis evidence and cannot replace native predicate or cold replay proof"
                ]
            });
            if let Some(parent) = output_path
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output_path, serde_json::to_vec_pretty(&report)?)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("double-q") => {
            let learn_args = &args[1..];
            let direct_inputs = repeated_option(learn_args, "--input");
            let dataset_path = option(learn_args, "--dataset").map(PathBuf::from);
            if dataset_path.is_some() && !direct_inputs.is_empty() {
                return Err("learn double-q accepts either --dataset or --input, not both".into());
            }
            let dataset_manifest: Option<DatasetManifest> = dataset_path
                .as_ref()
                .map(|path| -> Result<_, Box<dyn Error>> {
                    let manifest: DatasetManifest = serde_json::from_slice(&fs::read(path)?)?;
                    manifest.validate()?;
                    Ok(manifest)
                })
                .transpose()?;
            let inputs = if let Some(manifest) = &dataset_manifest {
                manifest
                    .entries
                    .iter()
                    .filter(|entry| entry.split == DatasetSplit::Train)
                    .map(|entry| entry.transition_corpus.to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
            } else {
                direct_inputs
            };
            let training = load_fqi_batch(&inputs, "Double-Q training", MAX_LEARN_INPUT_CORPORA)?;
            let expected_corpus_digests = dataset_manifest.as_ref().map(|manifest| {
                manifest
                    .entries
                    .iter()
                    .filter(|entry| entry.split == DatasetSplit::Train)
                    .map(|entry| entry.corpus_sha256)
                    .collect::<Vec<_>>()
            });
            if expected_corpus_digests
                .as_ref()
                .is_some_and(|expected| expected != &training.corpus_digests)
            {
                return Err("Double-Q corpus content differs from dataset manifest".into());
            }
            let config = DoubleQConfig {
                epochs: usize_option(learn_args, "--epochs", DoubleQConfig::default().epochs)?,
                hidden_width: usize_option(
                    learn_args,
                    "--hidden-width",
                    DoubleQConfig::default().hidden_width,
                )?,
                learning_rate: option(learn_args, "--learning-rate")
                    .map(|value| value.parse::<f64>())
                    .transpose()?
                    .unwrap_or(DoubleQConfig::default().learning_rate),
                discount: option(learn_args, "--discount")
                    .map(|value| value.parse::<f64>())
                    .transpose()?
                    .unwrap_or(DoubleQConfig::default().discount),
                target_sync_steps: usize_option(
                    learn_args,
                    "--target-sync-steps",
                    DoubleQConfig::default().target_sync_steps,
                )?,
                gradient_clip: option(learn_args, "--gradient-clip")
                    .map(|value| value.parse::<f64>())
                    .transpose()?
                    .unwrap_or(DoubleQConfig::default().gradient_clip),
                seed: u64_option(learn_args, "--seed", DoubleQConfig::default().seed)?,
            };
            let action_support = training.transitions.iter().fold(
                BTreeMap::<u32, usize>::new(),
                |mut counts, transition| {
                    *counts.entry(transition.action).or_default() += 1;
                    counts
                },
            );
            if action_support.len() > MAX_FQI_ACTIONS {
                return Err(format!(
                    "Double-Q supports at most {MAX_FQI_ACTIONS} distinct actions; received {}",
                    action_support.len()
                )
                .into());
            }
            let actions = action_support.keys().copied().collect::<Vec<_>>();
            let model = DoubleQ::fit(
                training.feature_count,
                &actions,
                &training.transitions,
                &config,
            )?;
            let query_index = usize_option(learn_args, "--query-transition", 0)?;
            let query_transition = training
                .transitions
                .get(query_index)
                .ok_or("--query-transition is outside the merged transition batch")?;
            let query_side = option(learn_args, "--query-side").unwrap_or_else(|| "state".into());
            let query_state = match query_side.as_str() {
                "state" => &query_transition.state,
                "next-state" => &query_transition.next_state,
                _ => return Err("--query-side must be state or next-state".into()),
            };
            let ranking = model
                .rank_actions(query_state)?
                .into_iter()
                .map(|estimate| {
                    json!({
                        "action": estimate.action,
                        "mean_q": estimate.mean,
                        "critic_a": estimate.critic_a,
                        "critic_b": estimate.critic_b,
                        "critic_disagreement": estimate.critic_disagreement,
                        "support": action_support[&estimate.action],
                    })
                })
                .collect::<Vec<_>>();
            let model_output = option(learn_args, "--model-output").map(PathBuf::from);
            let mut model_content_blob = None;
            let mut model_artifact_store = None;
            if let Some(path) = &model_output {
                if path.exists() {
                    return Err(format!(
                        "Double-Q model output already exists: {}",
                        path.display()
                    )
                    .into());
                }
                if let Some(parent) = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    fs::create_dir_all(parent)?;
                }
                let bytes = model.artifact_bytes(
                    training.feature_schema,
                    training.action_schema,
                    dataset_manifest
                        .as_ref()
                        .map(|manifest| manifest.dataset_sha256),
                    &training.corpus_digests,
                    &config,
                )?;
                fs::write(path, &bytes)?;
                let store_path = option(learn_args, "--artifact-store")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| path.parent().unwrap_or(Path::new(".")).join("content"));
                model_content_blob = Some(
                    ContentStore::initialize(&store_path)?.put_bytes(&bytes, ContentKind::Model)?,
                );
                model_artifact_store = Some(store_path);
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": "dusklight-double-q-ranking/v1",
                    "feature_schema": training.feature_schema,
                    "action_schema": training.action_schema,
                    "input_corpora": inputs,
                    "training_corpus_sha256": training.corpus_digests,
                    "training_dataset": dataset_path,
                    "training_dataset_sha256": dataset_manifest.as_ref().map(|manifest| manifest.dataset_sha256),
                    "transition_count": training.transitions.len(),
                    "episode_groups": training.episode_groups.iter().copied().collect::<BTreeSet<_>>().len(),
                    "query_transition": query_index,
                    "query_side": query_side,
                    "config": config,
                    "gradient_updates": model.gradient_updates(),
                    "target_synchronizations": model.target_synchronizations(),
                    "target_evaluation": "online_selects_opposite_frozen_target_evaluates",
                    "sample_order": "deterministic_seeded_epoch_shuffle",
                    "model_output": model_output,
                    "model_artifact_store": model_artifact_store,
                    "model_content_blob": model_content_blob,
                    "ranking": ranking,
                    "promotion_authority": false,
                    "limitations": [
                        "offline Double-Q can overvalue actions outside dataset support; use support diagnostics and the conservative learner",
                        "numeric normalization does not provide categorical embeddings or missingness masks",
                        "critic disagreement is not calibrated uncertainty",
                        "rankings are proposals and require native predicate and cold replay proof"
                    ]
                }))?
            );
            Ok(())
        }
        Some("fit") => {
            let learn_args = &args[1..];
            let direct_inputs = repeated_option(learn_args, "--input");
            let dataset_path = option(learn_args, "--dataset").map(PathBuf::from);
            if dataset_path.is_some() && !direct_inputs.is_empty() {
                return Err("learn fit accepts either --dataset or --input, not both".into());
            }
            let dataset_manifest: Option<DatasetManifest> = dataset_path
                .as_ref()
                .map(|path| -> Result<_, Box<dyn Error>> {
                    let manifest: DatasetManifest = serde_json::from_slice(&fs::read(path)?)?;
                    manifest.validate()?;
                    Ok(manifest)
                })
                .transpose()?;
            let inputs = if let Some(manifest) = &dataset_manifest {
                manifest
                    .entries
                    .iter()
                    .filter(|entry| entry.split == huntctl::dataset::DatasetSplit::Train)
                    .map(|entry| entry.transition_corpus.to_string_lossy().into_owned())
                    .collect()
            } else {
                direct_inputs
            };
            if inputs.is_empty() {
                return Err(
                    "learn fit requires training entries in --dataset or at least one --input FILE"
                        .into(),
                );
            }
            if inputs.len() > MAX_LEARN_INPUT_CORPORA {
                return Err(format!(
                    "learn fit accepts at most {MAX_LEARN_INPUT_CORPORA} input corpora; received {}",
                    inputs.len()
                )
                .into());
            }
            let mut config = FqiConfig {
                iterations: usize_option(learn_args, "--iterations", 24)?,
                backup_steps: usize_option(learn_args, "--n-step", 1)?,
                trees_per_action: usize_option(learn_args, "--trees", 31)?,
                max_tree_depth: usize_option(learn_args, "--max-depth", 8)?,
                seed: u64_option(learn_args, "--seed", FqiConfig::default().seed)?,
                discount: option(learn_args, "--discount")
                    .map(|value| value.parse::<f32>())
                    .transpose()?
                    .unwrap_or(FqiConfig::default().discount),
                ..FqiConfig::default()
            };
            if config.iterations > MAX_FQI_ITERATIONS {
                return Err(format!(
                    "--iterations must not exceed {MAX_FQI_ITERATIONS}; received {}",
                    config.iterations
                )
                .into());
            }
            if config.backup_steps == 0 || config.backup_steps > MAX_FQI_BACKUP_STEPS {
                return Err(format!(
                    "--n-step must be within 1..={MAX_FQI_BACKUP_STEPS}; received {}",
                    config.backup_steps
                )
                .into());
            }
            if config.trees_per_action > MAX_FQI_TREES_PER_ACTION {
                return Err(format!(
                    "--trees must not exceed {MAX_FQI_TREES_PER_ACTION}; received {}",
                    config.trees_per_action
                )
                .into());
            }
            if config.max_tree_depth > MAX_FQI_TREE_DEPTH {
                return Err(format!(
                    "--max-depth must not exceed {MAX_FQI_TREE_DEPTH}; received {}",
                    config.max_tree_depth
                )
                .into());
            }
            let mut feature_schema = None;
            let mut action_schema = None;
            let mut feature_count = None;
            let mut transitions = Vec::new();
            let mut episode_groups = Vec::new();
            let mut next_episode_group = 0_u64;
            let mut training_corpus_sha256 = Vec::new();
            let mut action_support = BTreeMap::<u32, usize>::new();
            let shaping_path = option(learn_args, "--shaping").map(PathBuf::from);
            let shaping_report_path = option(learn_args, "--shaping-report").map(PathBuf::from);
            if shaping_path.is_some() != shaping_report_path.is_some() {
                return Err(
                    "--shaping SPEC.json and --shaping-report REPORT.json must be supplied together"
                        .into(),
                );
            }
            let shaping_spec: Option<PotentialShapingSpec> = if let Some(path) = &shaping_path {
                Some(serde_json::from_slice(&fs::read(path)?)?)
            } else {
                None
            };
            let mut shaping_records = Vec::new();
            for input in &inputs {
                let corpus = TransitionCorpus::read_zstd_file(input)?;
                training_corpus_sha256.push(corpus.content_digest()?);
                if feature_schema.is_some_and(|value| value != corpus.feature_schema)
                    || action_schema.is_some_and(|value| value != corpus.action_schema)
                    || feature_count.is_some_and(|value| value != corpus.feature_count)
                {
                    return Err(
                        "transition corpora use incompatible feature or action schemas".into(),
                    );
                }
                feature_schema = Some(corpus.feature_schema);
                action_schema = Some(corpus.action_schema);
                feature_count = Some(corpus.feature_count);
                if let Some(spec) = &shaping_spec {
                    if spec.feature_schema != corpus.feature_schema {
                        return Err(format!(
                            "shaping feature schema {} does not match corpus feature schema {}",
                            spec.feature_schema, corpus.feature_schema
                        )
                        .into());
                    }
                    spec.validate(corpus.feature_count as usize)?;
                }
                let merged_count = transitions
                    .len()
                    .checked_add(corpus.transitions.len())
                    .ok_or("learn fit merged transition count overflow")?;
                if merged_count > MAX_FQI_TRANSITIONS {
                    return Err(format!(
                        "learn fit accepts at most {MAX_FQI_TRANSITIONS} merged transitions; received at least {merged_count}"
                    )
                    .into());
                }
                transitions.reserve(corpus.transitions.len());
                let mut ended_terminal = false;
                for (transition_index, transition) in corpus.transitions.into_iter().enumerate() {
                    let action = transition.action.action_id;
                    let terminal = transition.terminal;
                    if !action_support.contains_key(&action)
                        && action_support.len() >= MAX_FQI_ACTIONS
                    {
                        return Err(format!(
                            "learn fit accepts at most {MAX_FQI_ACTIONS} distinct actions; encountered action {action} after reaching the limit"
                        )
                        .into());
                    }
                    *action_support.entry(action).or_default() += 1;
                    let reward = if let Some(spec) = &shaping_spec {
                        let breakdown = spec.shape_reward(
                            corpus.feature_count as usize,
                            &transition.state,
                            &transition.next_state,
                            transition.reward,
                            transition.duration_ticks,
                            terminal,
                            config.discount,
                        )?;
                        let training_reward = breakdown.training_reward;
                        shaping_records.push(json!({
                            "input_corpus": input,
                            "transition": transition_index,
                            "source_reference": transition.source.digest,
                            "next_reference": transition.next.digest,
                            "reward": breakdown,
                        }));
                        training_reward
                    } else {
                        transition.reward
                    };
                    transitions.push(FqiTransition {
                        state: transition.state,
                        action,
                        duration: transition.duration_ticks,
                        reward,
                        next_state: transition.next_state,
                        terminal,
                    });
                    episode_groups.push(next_episode_group);
                    ended_terminal = terminal;
                    if terminal {
                        next_episode_group = next_episode_group
                            .checked_add(1)
                            .ok_or("learn fit episode-group count overflowed")?;
                    }
                }
                if !ended_terminal {
                    next_episode_group = next_episode_group
                        .checked_add(1)
                        .ok_or("learn fit episode-group count overflowed")?;
                }
            }
            let declared_categorical = repeated_option(learn_args, "--categorical-feature")
                .into_iter()
                .map(|value| value.parse::<usize>())
                .collect::<Result<Vec<_>, _>>()?;
            let declared_all_continuous = learn_args.iter().any(|arg| arg == "--all-continuous");
            if declared_all_continuous && !declared_categorical.is_empty() {
                return Err(
                    "--all-continuous and --categorical-feature cannot be used together".into(),
                );
            }
            if feature_schema == Some(movement_feature_schema_digest_v1()) {
                if declared_all_continuous || !declared_categorical.is_empty() {
                    return Err(
                        "the authenticated movement schema owns its categorical feature map; do not override it"
                            .into(),
                    );
                }
                config.categorical_features = MOVEMENT_CATEGORICAL_FEATURES_V1.to_vec();
            } else if feature_schema == Some(movement_state_v2_spec().digest()?) {
                if declared_all_continuous || !declared_categorical.is_empty() {
                    return Err(
                        "the authenticated movement schema owns its categorical feature map; do not override it"
                            .into(),
                    );
                }
                config.categorical_features = movement_state_v2_spec().categorical_features();
            } else if declared_all_continuous {
                config.categorical_features.clear();
            } else if !declared_categorical.is_empty() {
                config.categorical_features = declared_categorical;
            } else {
                return Err(
                    "unknown feature schema: declare --all-continuous or repeat --categorical-feature N"
                        .into(),
                );
            }
            let actions: Vec<u32> = action_support.keys().copied().collect();
            let query_index = usize_option(learn_args, "--query-transition", 0)?;
            let query_transition = transitions
                .get(query_index)
                .ok_or("--query-transition is outside the merged transition batch")?;
            let query_side = option(learn_args, "--query-side").unwrap_or_else(|| "state".into());
            let query_state = match query_side.as_str() {
                "state" => query_transition.state.clone(),
                "next-state" => query_transition.next_state.clone(),
                _ => return Err("--query-side must be state or next-state".into()),
            };
            let learned_feature_count =
                feature_count.ok_or("transition corpus has no feature width")? as usize;
            let shaping_identity = shaping_spec
                .as_ref()
                .map(|spec| spec.identity(learned_feature_count))
                .transpose()?;
            if let (Some(spec), Some(path)) = (&shaping_spec, &shaping_report_path) {
                if path.exists() {
                    return Err(format!(
                        "shaping reward report already exists: {}",
                        path.display()
                    )
                    .into());
                }
                if let Some(parent) = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    fs::create_dir_all(parent)?;
                }
                fs::write(
                    path,
                    serde_json::to_vec_pretty(&json!({
                        "schema": REWARD_REPORT_SCHEMA_V1,
                        "shaping_identity": shaping_identity,
                        "shaping_spec": spec,
                        "feature_schema": feature_schema,
                        "action_schema": action_schema,
                        "per_tick_discount": config.discount,
                        "proposal_signal_only": true,
                        "terminal_objective": "unchanged_external_predicate",
                        "input_corpora": &inputs,
                        "transitions": shaping_records,
                    }))?,
                )?;
            }
            let model = FittedQ::fit_with_episode_groups(
                learned_feature_count,
                &actions,
                &transitions,
                &episode_groups,
                &config,
            )?;
            let model_output = option(learn_args, "--model-output").map(PathBuf::from);
            let mut model_content_blob = None;
            let mut model_artifact_store = None;
            if let Some(path) = &model_output {
                if path.exists() {
                    return Err(format!("model output already exists: {}", path.display()).into());
                }
                if let Some(parent) = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    fs::create_dir_all(parent)?;
                }
                let bytes = model.artifact_bytes(
                    feature_schema.ok_or("transition corpus has no feature schema")?,
                    action_schema.ok_or("transition corpus has no action schema")?,
                    dataset_manifest
                        .as_ref()
                        .map(|manifest| manifest.dataset_sha256),
                    &training_corpus_sha256,
                    &config,
                )?;
                fs::write(path, &bytes)?;
                let store_path = option(learn_args, "--artifact-store")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| path.parent().unwrap_or(Path::new(".")).join("content"));
                model_content_blob = Some(
                    ContentStore::initialize(&store_path)?.put_bytes(&bytes, ContentKind::Model)?,
                );
                model_artifact_store = Some(store_path);
            }
            let ranking: Vec<_> = model
                .rank_actions(&query_state)?
                .into_iter()
                .map(|estimate| {
                    json!({
                        "action": estimate.action,
                        "mean_q": estimate.mean,
                        "ensemble_variance": estimate.variance,
                        "support": action_support[&estimate.action],
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": "dusklight-fitted-q-ranking/v1",
                    "feature_schema": feature_schema,
                    "action_schema": action_schema,
                    "input_corpora": inputs,
                    "training_dataset": dataset_path,
                    "training_dataset_sha256": dataset_manifest.as_ref().map(|manifest| manifest.dataset_sha256),
                    "transition_count": transitions.len(),
                    "episode_groups": episode_groups.iter().copied().collect::<BTreeSet<_>>().len(),
                    "bootstrap_unit": model.bootstrap_unit(),
                    "query_transition": query_index,
                    "query_side": query_side,
                    "per_tick_discount": config.discount,
                    "potential_shaping": shaping_identity,
                    "reward_report": shaping_report_path,
                    "model_output": model_output,
                    "model_artifact_store": model_artifact_store,
                    "model_content_blob": model_content_blob,
                    "iterations": config.iterations,
                    "backup_steps": config.backup_steps,
                    "trees_per_action": config.trees_per_action,
                    "categorical_features": config.categorical_features,
                    "seed": config.seed,
                    "ranking": ranking,
                }))?
            );
            Ok(())
        }
        Some("benchmark") => {
            const ADVANCE: u32 = 3;
            const WAIT: u32 = 9;
            let mut transitions = Vec::new();
            for nuisance in [-1.0, 1.0] {
                transitions.extend([
                    FqiTransition {
                        state: vec![0.0, nuisance],
                        action: ADVANCE,
                        duration: 1,
                        reward: 0.0,
                        next_state: vec![1.0, nuisance],
                        terminal: false,
                    },
                    FqiTransition {
                        state: vec![0.0, nuisance],
                        action: WAIT,
                        duration: 1,
                        reward: -1.0,
                        next_state: vec![0.0, nuisance],
                        terminal: false,
                    },
                    FqiTransition {
                        state: vec![1.0, nuisance],
                        action: ADVANCE,
                        duration: 1,
                        reward: 10.0,
                        next_state: vec![2.0, nuisance],
                        terminal: true,
                    },
                    FqiTransition {
                        state: vec![1.0, nuisance],
                        action: WAIT,
                        duration: 1,
                        reward: -1.0,
                        next_state: vec![1.0, nuisance],
                        terminal: false,
                    },
                ]);
            }
            let config = FqiConfig {
                iterations: 16,
                trees_per_action: 7,
                max_tree_depth: 3,
                features_per_split: 2,
                discount: 0.9,
                bootstrap: false,
                ..FqiConfig::default()
            };
            let model = FittedQ::fit(2, &[WAIT, ADVANCE], &transitions, &config)?;
            let held_out = [[0.0, 0.0], [1.0, 0.0]];
            let selected: Vec<u32> = held_out
                .iter()
                .map(|state| model.best_action(state).map(|estimate| estimate.action))
                .collect::<Result<_, _>>()?;
            let passed = selected == [ADVANCE, ADVANCE];
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": "dusklight-fqi-benchmark/v1",
                    "training_transitions": transitions.len(),
                    "held_out_states": held_out,
                    "selected_actions": selected,
                    "expected_actions": [ADVANCE, ADVANCE],
                    "passed": passed,
                }))?
            );
            if !passed {
                return Err("fitted-Q benchmark failed its fixed acceptance threshold".into());
            }
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

struct SearchExecutionConfig {
    game: PathBuf,
    dvd: PathBuf,
    working_directory: PathBuf,
    game_args_prefix: Vec<String>,
    timeout: Duration,
    harness: Option<HarnessEvaluateConfig>,
}

fn search_execution_config(args: &[String]) -> Result<SearchExecutionConfig, Box<dyn Error>> {
    if let Some(path) = option(args, "--run-request") {
        if option(args, "--game").is_some()
            || option(args, "--dvd").is_some()
            || option(args, "--working-directory").is_some()
            || option(args, "--timeout-ms").is_some()
            || option(args, "--timeout-seconds").is_some()
            || !repeated_option(args, "--game-arg").is_empty()
        {
            return Err("--run-request is the sole execution authority; do not combine it with --game, --dvd, --working-directory, --game-arg, or timeout options".into());
        }
        let repository_root = fs::canonicalize(
            option(args, "--repository-root")
                .map(PathBuf::from)
                .unwrap_or(std::env::current_dir()?),
        )?;
        let request_template: HarnessRunRequest = serde_json::from_slice(&fs::read(path)?)?;
        request_template.validate_files(&repository_root)?;
        return Ok(SearchExecutionConfig {
            game: repository_root.join(&request_template.executable.path),
            dvd: repository_root.join(&request_template.game_data.path),
            working_directory: repository_root.clone(),
            game_args_prefix: Vec::new(),
            timeout: Duration::from_secs(u64::from(request_template.host_timeout_seconds)),
            harness: Some(HarnessEvaluateConfig {
                repository_root,
                request_template,
            }),
        });
    }
    Ok(SearchExecutionConfig {
        game: required_path(args, "--game")?,
        dvd: required_path(args, "--dvd")?,
        working_directory: option(args, "--working-directory")
            .map(PathBuf::from)
            .unwrap_or(std::env::current_dir()?),
        game_args_prefix: repeated_option(args, "--game-arg"),
        timeout: timeout_option(args)?,
        harness: None,
    })
}

fn command_search(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("evaluate") => {
            let search_args = &args[1..];
            let population = required_path(search_args, "--population")?;
            let output = required_path(search_args, "--output")?;
            let results = option(search_args, "--results")
                .map(PathBuf::from)
                .unwrap_or_else(|| output.join("results.json"));
            let execution = search_execution_config(search_args)?;
            let report = evaluate_population(&EvaluateConfig {
                population_path: population,
                game: execution.game,
                dvd: execution.dvd,
                output_root: output,
                results_path: results,
                working_directory: execution.working_directory,
                game_args_prefix: execution.game_args_prefix,
                workers: usize_option(search_args, "--workers", 4)?,
                repetitions: u32_option(search_args, "--repetitions", 3)?,
                timeout: execution.timeout,
                harness: execution.harness,
            })?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Some("run-route") => {
            let search_args = &args[1..];
            let timeline_path = required_path(search_args, "--timeline")?;
            let timeline = load_timeline(&timeline_path)?;
            let artifact_root = timeline_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."));
            timeline.validate_artifacts(Some(artifact_root))?;
            let segment_name = option(search_args, "--segment")
                .ok_or("missing required --segment TIMELINE_SEGMENT")?;
            let segment = timeline
                .segments
                .get(&segment_name)
                .ok_or_else(|| format!("unknown timeline segment {segment_name:?}"))?;
            if !matches!(
                segment.profile,
                SegmentProfile::Fsp103ToFsp104 | SegmentProfile::LinkControlToTunnelCrawlStart
            ) {
                return Err(format!(
                    "route search requires an anchored movement profile, got {}",
                    segment.profile.as_str()
                )
                .into());
            }
            let lineage = option(search_args, "--lineage").unwrap_or_else(|| "main".into());
            let parent_segment = segment
                .parent
                .as_ref()
                .ok_or("anchored route search requires a child segment with an explicit parent")?;
            let prefix = materialize_lineage(
                &timeline,
                artifact_root,
                &lineage,
                MaterializeTarget::ThroughSegment(parent_segment.clone()),
            )?;
            let through_goal = huntctl::route_workbench::materialize_segment_chain(
                &timeline,
                artifact_root,
                &segment.id,
            )?;
            if through_goal.steps.len() != prefix.steps.len() + 1
                || through_goal.steps.last().map(|step| step.segment.as_str())
                    != Some(segment_name.as_str())
                || through_goal.steps[..prefix.steps.len()]
                    .iter()
                    .map(|step| step.segment.as_str())
                    .ne(prefix.steps.iter().map(|step| step.segment.as_str()))
                || through_goal.tape.frames.len() <= prefix.tape.frames.len()
            {
                return Err(format!(
                    "segment {segment_name:?} is not an exact structural child of parent {parent_segment:?} on lineage {lineage:?}"
                )
                .into());
            }
            let source_segment_id = prefix
                .steps
                .last()
                .map(|step| step.segment.as_str())
                .ok_or("anchored route search requires a nonempty immutable prefix")?;
            let source_fingerprint = timeline.segments[source_segment_id].end_fingerprint.clone();
            let suffix = InputTape {
                tick_rate_numerator: through_goal.tape.tick_rate_numerator,
                tick_rate_denominator: through_goal.tape.tick_rate_denominator,
                frames: through_goal.tape.frames[prefix.tape.frames.len()..].to_vec(),
                ..InputTape::default()
            };
            let observed_candidate = Candidate::from_absolute_tape(segment.profile, &suffix)?;
            let seed_candidate = if let Some(path) = option(search_args, "--candidate") {
                let candidate: Candidate = serde_json::from_slice(&fs::read(path)?)?;
                candidate.validate()?;
                if candidate.segment != segment.profile {
                    return Err("route-search candidate profile does not match the segment".into());
                }
                candidate
            } else {
                observed_candidate
            };

            let output = required_path(search_args, "--output")?;
            let execution = search_execution_config(search_args)?;
            let game = execution.game;
            let dvd = execution.dvd;
            let working_directory = execution.working_directory;
            if !game.is_file() || !dvd.is_file() || !working_directory.is_dir() {
                return Err(
                    "route search requires existing game/DVD files and working directory".into(),
                );
            }
            let size = usize_option(search_args, "--size", 16)?;
            let generations = u32_option(search_args, "--generations", 2)?;
            let elite_count = usize_option(search_args, "--elites", (size / 4).max(1))?;
            let workers = usize_option(search_args, "--workers", 4)?;
            let repetitions = u32_option(search_args, "--repetitions", 3)?;
            let timeout = execution.timeout;
            let rng_seed = u64_option(search_args, "--rng-seed", 1)?;
            if !execution.game_args_prefix.is_empty() {
                return Err(
                    "route search does not accept --game-arg; its execution contract is fixed"
                        .into(),
                );
            }
            if generations == 0
                || size == 0
                || elite_count == 0
                || elite_count > size
                || workers == 0
                || repetitions == 0
            {
                return Err(
                    "route search counts and elite bounds must be nonzero and valid".into(),
                );
            }
            let output_name = output
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or("route-search output requires a UTF-8 final path component")?;
            let objective_root = output.with_file_name(format!("{output_name}.objective"));
            if objective_root.exists() {
                return Err(format!(
                    "route-search objective directory already exists: {}",
                    objective_root.display()
                )
                .into());
            }
            fs::create_dir_all(&objective_root)?;
            let prefix_path = objective_root.join("prefix.tape");
            fs::write(&prefix_path, prefix.tape.encode()?)?;
            let source_path = artifact_root.join(
                timeline
                    .predicate_program
                    .as_ref()
                    .ok_or("route search requires predicate_program")?,
            );
            let compiled = milestone_dsl::compile_source(&fs::read_to_string(&source_path)?)?;
            let program_path = objective_root.join("milestones.dmsp");
            fs::write(&program_path, &compiled.bytes)?;

            let select_goal = |segment_id: &str,
                               requested: Option<String>,
                               option_name: &str|
             -> Result<&huntctl::timeline::Goal, Box<dyn Error>> {
                let available = timeline
                    .goals
                    .values()
                    .filter(|goal| {
                        goal.segment == segment_id
                            || timeline
                                .proofs
                                .iter()
                                .any(|proof| proof.segment == segment_id && proof.goal == goal.id)
                    })
                    .collect::<Vec<_>>();
                if let Some(id) = requested {
                    let goal = timeline
                        .goals
                        .get(&id)
                        .ok_or_else(|| format!("unknown route goal {id:?}"))?;
                    if !available.iter().any(|candidate| candidate.id == goal.id) {
                        return Err(format!(
                            "segment {segment_id:?} neither defines nor proves goal {id:?}"
                        )
                        .into());
                    }
                    return Ok(goal);
                }
                if available.len() != 1 {
                    return Err(format!(
                        "segment {segment_id:?} defines or proves {} goals; select one with {option_name}",
                        available.len()
                    )
                    .into());
                }
                Ok(available[0])
            };
            let source_goal = select_goal(
                parent_segment,
                option(search_args, "--source-goal"),
                "--source-goal GOAL",
            )?;
            let target_goal =
                select_goal(&segment_name, option(search_args, "--goal"), "--goal GOAL")?;

            let summary = run_anchored_search(&AnchoredSearchRunConfig {
                search: SearchRunConfig {
                    segment: segment.profile,
                    seed_candidate: Some(seed_candidate),
                    game: game.clone(),
                    dvd: dvd.clone(),
                    output_root: output,
                    working_directory,
                    game_args_prefix: Vec::new(),
                    generations,
                    population_size: size,
                    elite_count,
                    workers,
                    repetitions,
                    timeout,
                    rng_seed,
                    harness: execution.harness,
                },
                objective: AnchoredObjectiveConfig {
                    segment: segment.profile,
                    prefix_tape: prefix_path,
                    milestone_program: program_path,
                    game,
                    dvd,
                    source_milestone: source_goal.predicate.clone(),
                    source_boundary_fingerprint: source_fingerprint,
                    goal_milestone: target_goal.predicate.clone(),
                },
            })?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            Ok(())
        }
        Some("run") => {
            let search_args = &args[1..];
            let segment: SegmentProfile = option(search_args, "--segment")
                .ok_or("missing required --segment ID")?
                .parse()?;
            let seed_candidate = if let Some(path) = option(search_args, "--candidate") {
                let candidate: Candidate = serde_json::from_slice(&fs::read(path)?)?;
                candidate.validate()?;
                if candidate.segment != segment {
                    return Err("candidate segment does not match --segment".into());
                }
                Some(candidate)
            } else {
                None
            };
            let output = required_path(search_args, "--output")?;
            let size = usize_option(search_args, "--size", 16)?;
            let execution = search_execution_config(search_args)?;
            let summary = run_search(&SearchRunConfig {
                segment,
                seed_candidate,
                game: execution.game,
                dvd: execution.dvd,
                output_root: output,
                working_directory: execution.working_directory,
                game_args_prefix: execution.game_args_prefix,
                generations: u32_option(search_args, "--generations", 2)?,
                population_size: size,
                elite_count: usize_option(search_args, "--elites", (size / 4).max(1))?,
                workers: usize_option(search_args, "--workers", 4)?,
                repetitions: u32_option(search_args, "--repetitions", 3)?,
                timeout: execution.timeout,
                rng_seed: u64_option(search_args, "--rng-seed", 1)?,
                harness: execution.harness,
            })?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            Ok(())
        }
        Some("beam") => {
            let search_args = &args[1..];
            let seed_candidate: Candidate =
                serde_json::from_slice(&fs::read(required_path(search_args, "--candidate")?)?)?;
            seed_candidate.validate()?;
            let options: Vec<huntctl::search::MacroAction> =
                serde_json::from_slice(&fs::read(required_path(search_args, "--options")?)?)?;
            let q_priors: Option<QBeamPriorTable> = option(search_args, "--q-priors")
                .map(|path| {
                    fs::read(path)
                        .map_err(Box::<dyn Error>::from)
                        .and_then(|bytes| {
                            serde_json::from_slice(&bytes).map_err(Box::<dyn Error>::from)
                        })
                })
                .transpose()?;
            let summary = run_beam_search(&BeamSearchConfig {
                segment: seed_candidate.segment,
                seed_candidate,
                options,
                q_priors,
                game: required_path(search_args, "--game")?,
                dvd: required_path(search_args, "--dvd")?,
                output_root: required_path(search_args, "--output")?,
                working_directory: option(search_args, "--working-directory")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_dir()?),
                game_args_prefix: repeated_option(search_args, "--game-arg"),
                beam_width: usize_option(search_args, "--beam-width", 8)?,
                maximum_depth: u32_option(search_args, "--maximum-depth", 8)?,
                candidate_budget: usize_option(search_args, "--candidate-budget", 1_000)?,
                workers: usize_option(search_args, "--workers", 4)?,
                repetitions: u32_option(search_args, "--repetitions", 3)?,
                timeout: timeout_option(search_args)?,
            })?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            Ok(())
        }
        Some("continuous") => {
            let search_args = &args[1..];
            let seed_candidate: Candidate =
                serde_json::from_slice(&fs::read(required_path(search_args, "--candidate")?)?)?;
            seed_candidate.validate()?;
            let axes: ContinuousAxes =
                serde_json::from_slice(&fs::read(required_path(search_args, "--axes")?)?)?;
            let method: ContinuousMethod = option(search_args, "--method")
                .ok_or("missing required --method cem|cma-es")?
                .parse()?;
            let population_size = usize_option(search_args, "--population", 32)?;
            let summary = run_continuous_search(&ContinuousSearchRunConfig {
                method,
                seed_candidate,
                axes,
                game: required_path(search_args, "--game")?,
                dvd: required_path(search_args, "--dvd")?,
                output_root: required_path(search_args, "--output")?,
                working_directory: option(search_args, "--working-directory")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_dir()?),
                game_args_prefix: repeated_option(search_args, "--game-arg"),
                generations: u32_option(search_args, "--generations", 10)?,
                population_size,
                elite_count: usize_option(search_args, "--elites", (population_size / 4).max(1))?,
                initial_sigma: option(search_args, "--initial-sigma")
                    .map(|value| value.parse())
                    .transpose()?
                    .unwrap_or(0.25),
                candidate_budget: usize_option(search_args, "--candidate-budget", 10_000)?,
                rng_seed: u64_option(search_args, "--rng-seed", 1)?,
                workers: usize_option(search_args, "--workers", 4)?,
                repetitions: u32_option(search_args, "--repetitions", 3)?,
                timeout: timeout_option(search_args)?,
            })?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            Ok(())
        }
        Some("bayesian") => {
            let search_args = &args[1..];
            let seed_candidate: Candidate =
                serde_json::from_slice(&fs::read(required_path(search_args, "--candidate")?)?)?;
            seed_candidate.validate()?;
            let axes: ContinuousAxes =
                serde_json::from_slice(&fs::read(required_path(search_args, "--axes")?)?)?;
            let parse_f64 = |name: &str, default: f64| -> Result<f64, Box<dyn Error>> {
                Ok(option(search_args, name)
                    .map(|value| value.parse())
                    .transpose()?
                    .unwrap_or(default))
            };
            let summary = run_bayesian_search(&BayesianSearchRunConfig {
                seed_candidate,
                axes,
                game: required_path(search_args, "--game")?,
                dvd: required_path(search_args, "--dvd")?,
                output_root: required_path(search_args, "--output")?,
                working_directory: option(search_args, "--working-directory")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_dir()?),
                game_args_prefix: repeated_option(search_args, "--game-arg"),
                generations: u32_option(search_args, "--generations", 20)?,
                batch_size: usize_option(search_args, "--batch-size", 4)?,
                initial_samples: usize_option(search_args, "--initial-samples", 8)?,
                acquisition_pool: usize_option(search_args, "--acquisition-pool", 2_048)?,
                length_scale: parse_f64("--length-scale", 0.2)?,
                observation_noise: parse_f64("--observation-noise", 1.0e-6)?,
                exploration: parse_f64("--exploration", 0.01)?,
                candidate_budget: usize_option(search_args, "--candidate-budget", 80)?,
                rng_seed: u64_option(search_args, "--rng-seed", 1)?,
                workers: usize_option(search_args, "--workers", 4)?,
                repetitions: u32_option(search_args, "--repetitions", 3)?,
                timeout: timeout_option(search_args)?,
            })?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            Ok(())
        }
        Some("tournament") => {
            let search_args = &args[1..];
            let definition_path = required_path(search_args, "--definition")?;
            let definition: TournamentDefinition =
                serde_json::from_slice(&fs::read(&definition_path)?)?;
            let definition_directory = fs::canonicalize(
                definition_path
                    .parent()
                    .ok_or("tournament definition has no parent directory")?,
            )?;
            let summary = run_proposer_tournament(&ProposerTournamentConfig {
                definition,
                definition_directory,
                game: required_path(search_args, "--game")?,
                dvd: required_path(search_args, "--dvd")?,
                output_root: required_path(search_args, "--output")?,
                working_directory: option(search_args, "--working-directory")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_dir()?),
                game_args_prefix: repeated_option(search_args, "--game-arg"),
                workers: usize_option(search_args, "--workers", 4)?,
                repetitions: u32_option(search_args, "--repetitions", 3)?,
                timeout: timeout_option(search_args)?,
            })?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            Ok(())
        }
        Some("minimize-boot") => {
            let search_args = &args[1..];
            let candidate: Candidate =
                serde_json::from_slice(&fs::read(required_path(search_args, "--candidate")?)?)?;
            candidate.validate()?;
            let summary = minimize_boot(&BootMinimizeConfig {
                candidate,
                game: required_path(search_args, "--game")?,
                dvd: required_path(search_args, "--dvd")?,
                output_root: required_path(search_args, "--output")?,
                working_directory: option(search_args, "--working-directory")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_dir()?),
                game_args_prefix: repeated_option(search_args, "--game-arg"),
                workers: usize_option(search_args, "--workers", 4)?,
                repetitions: u32_option(search_args, "--repetitions", 3)?,
                timeout: timeout_option(search_args)?,
            })?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            Ok(())
        }
        Some("golf-boot") => {
            let search_args = &args[1..];
            let candidate: Candidate =
                serde_json::from_slice(&fs::read(required_path(search_args, "--candidate")?)?)?;
            candidate.validate()?;
            let summary = golf_boot(&BootGolfConfig {
                candidate,
                game: required_path(search_args, "--game")?,
                dvd: required_path(search_args, "--dvd")?,
                output_root: required_path(search_args, "--output")?,
                working_directory: option(search_args, "--working-directory")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_dir()?),
                game_args_prefix: repeated_option(search_args, "--game-arg"),
                workers: usize_option(search_args, "--workers", 4)?,
                repetitions: u32_option(search_args, "--repetitions", 3)?,
                timeout: timeout_option(search_args)?,
            })?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            Ok(())
        }
        Some("golf-option") => {
            let search_args = &args[1..];
            let plan: RollOptionPlan =
                serde_json::from_slice(&fs::read(required_path(search_args, "--plan")?)?)?;
            let execution: OptionExecution =
                serde_json::from_slice(&fs::read(required_path(search_args, "--execution")?)?)?;
            let tape_path = required_path(search_args, "--tape")?;
            let tape = InputTape::decode(&fs::read(&tape_path)?)?.tape;
            let cancellation_tick = option(search_args, "--cancellation-tick")
                .map(|value| value.parse::<u32>())
                .transpose()?;
            let condition_index = option(search_args, "--condition-index")
                .map(|value| value.parse::<u32>())
                .transpose()?;
            let cancellation = match (cancellation_tick, condition_index) {
                (Some(tick), Some(condition_index)) => Some(RollCancellationHit {
                    tick,
                    condition_index,
                }),
                (None, None) => None,
                _ => {
                    return Err(
                        "--cancellation-tick and --condition-index must be supplied together"
                            .into(),
                    );
                }
            };
            let steps = RollGolfSteps {
                heading_degrees: u16::try_from(u32_option(search_args, "--heading-step", 1)?)?,
                magnitude: u8::try_from(u32_option(search_args, "--magnitude-step", 1)?)?,
                duration_ticks: u32_option(search_args, "--duration-step", 1)?,
                phase_ticks: u32_option(search_args, "--phase-step", 1)?,
                button_ticks: u32_option(search_args, "--button-step", 1)?,
                cancellation_ticks: u32_option(search_args, "--cancellation-step", 1)?,
            };
            let proposals = golf_roll_option(&plan, cancellation, &execution, &tape, steps)?;
            let output = required_path(search_args, "--output")?;
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)?;
            }
            let manifest = json!({
                "schema": "dusklight-option-relative-golf-manifest/v1",
                "seed_option_id": execution.option_id,
                "seed_tape": tape_path,
                "steps": steps,
                "proposal_count": proposals.len(),
                "proposals": proposals,
            });
            fs::write(&output, serde_json::to_vec_pretty(&manifest)?)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
            Ok(())
        }
        Some("golf-path") => {
            let search_args = &args[1..];
            let plan: MotionPathPlan =
                serde_json::from_slice(&fs::read(required_path(search_args, "--plan")?)?)?;
            let execution: OptionExecution =
                serde_json::from_slice(&fs::read(required_path(search_args, "--execution")?)?)?;
            let tape_path = required_path(search_args, "--tape")?;
            let tape = InputTape::decode(&fs::read(&tape_path)?)?.tape;
            let cancellation_tick = option(search_args, "--cancellation-tick")
                .map(|value| value.parse::<u32>())
                .transpose()?;
            let condition_index = option(search_args, "--condition-index")
                .map(|value| value.parse::<u32>())
                .transpose()?;
            let cancellation = match (cancellation_tick, condition_index) {
                (Some(tick), Some(condition_index)) => Some(PathCancellationHit {
                    tick,
                    condition_index,
                }),
                (None, None) => None,
                _ => {
                    return Err(
                        "--cancellation-tick and --condition-index must be supplied together"
                            .into(),
                    );
                }
            };
            let steps = MotionPathGolfSteps {
                point_units: u16::try_from(u32_option(search_args, "--point-step", 1)?)?,
                duration_ticks: u32_option(search_args, "--duration-step", 1)?,
                phase_units: u32_option(search_args, "--phase-step", 1)?,
                cancellation_ticks: u32_option(search_args, "--cancellation-step", 1)?,
            };
            let proposals = golf_motion_path(&plan, cancellation, &execution, &tape, steps)?;
            let output = required_path(search_args, "--output")?;
            if output.exists() {
                return Err(
                    format!("path-golf output already exists: {}", output.display()).into(),
                );
            }
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            let manifest = json!({
                "schema": "dusklight-motion-path-relative-golf-manifest/v1",
                "seed_option_id": execution.option_id,
                "seed_tape": tape_path,
                "steps": steps,
                "proposal_count": proposals.len(),
                "proposals": proposals,
            });
            fs::write(&output, serde_json::to_vec_pretty(&manifest)?)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
            Ok(())
        }
        Some("import-tape") => {
            let search_args = &args[1..];
            let segment: SegmentProfile = option(search_args, "--segment")
                .ok_or("missing required --segment ID")?
                .parse()?;
            let tape_path = required_path(search_args, "--tape")?;
            let output = required_path(search_args, "--output")?;
            let tape = InputTape::decode(&fs::read(&tape_path)?)?.tape;
            let candidate = Candidate::from_absolute_tape(segment, &tape)?;
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, serde_json::to_vec_pretty(&candidate)?)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "candidate_id": candidate.id()?,
                    "candidate": output,
                    "source_tape": tape_path,
                    "frames": candidate.frame_count(),
                    "lossless": candidate.compile()? == tape,
                }))?
            );
            Ok(())
        }
        Some("seed") => {
            let search_args = &args[1..];
            let segment: SegmentProfile = option(search_args, "--segment")
                .ok_or("missing required --segment ID")?
                .parse()?;
            let output = required_path(search_args, "--output")?;
            let size = usize_option(search_args, "--size", 16)?;
            let rng_seed = u64_option(search_args, "--rng-seed", 1)?;
            let candidate = if let Some(path) = option(search_args, "--candidate") {
                let candidate: Candidate = serde_json::from_slice(&fs::read(path)?)?;
                if candidate.segment != segment {
                    return Err("candidate segment does not match --segment".into());
                }
                candidate
            } else {
                Candidate::baseline(segment)
            };
            let manifest = write_seed_population(&output, candidate, size, rng_seed)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
            Ok(())
        }
        Some("evolve") => {
            let search_args = &args[1..];
            let population = required_path(search_args, "--population")?;
            let results: SearchResults =
                serde_json::from_slice(&fs::read(required_path(search_args, "--results")?)?)?;
            let output = required_path(search_args, "--output")?;
            let size = usize_option(search_args, "--size", 16)?;
            let elites = usize_option(search_args, "--elites", (size / 4).max(1))?;
            let rng_seed = u64_option(search_args, "--rng-seed", 1)?;
            let manifest = evolve_population(
                &population,
                &results,
                &output,
                EvolutionConfig {
                    population_size: size,
                    elite_count: elites,
                    rng_seed,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
            Ok(())
        }
        Some("rank") => {
            let search_args = &args[1..];
            let manifest: PopulationManifest =
                serde_json::from_slice(&fs::read(required_path(search_args, "--population")?)?)?;
            let results: SearchResults =
                serde_json::from_slice(&fs::read(required_path(search_args, "--results")?)?)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&rank_population(&manifest, &results)?)?
            );
            Ok(())
        }
        Some("collect") => {
            let search_args = &args[1..];
            let manifest: PopulationManifest =
                serde_json::from_slice(&fs::read(required_path(search_args, "--population")?)?)?;
            let inputs = repeated_option(search_args, "--input");
            if inputs.is_empty() {
                return Err("search collect requires at least one --input FILE".into());
            }
            let artifacts = inputs
                .iter()
                .map(|path| serde_json::from_slice(&fs::read(path)?).map_err(Into::into))
                .collect::<Result<Vec<EvaluationArtifact>, Box<dyn Error>>>()?;
            let results = collect_results(&manifest, artifacts)?;
            let output = required_path(search_args, "--output")?;
            fs::write(&output, serde_json::to_vec_pretty(&results)?)?;
            println!("{}", serde_json::to_string_pretty(&results)?);
            Ok(())
        }
        Some("inspect") if args.len() == 2 => {
            let candidate: Candidate = serde_json::from_slice(&fs::read(&args[1])?)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "candidate_id": candidate.id()?,
                    "segment": candidate.segment,
                    "target": candidate.segment.target(),
                    "target_depth": candidate.segment.target_depth(),
                    "action_count": candidate.actions.len(),
                    "frame_count": candidate.frame_count(),
                    "ancestry": candidate.ancestry,
                }))?
            );
            Ok(())
        }
        Some("mock-evaluate") => {
            let search_args = &args[1..];
            let population_path = required_path(search_args, "--population")?;
            let output = required_path(search_args, "--output")?;
            let attempts = u32::try_from(usize_option(search_args, "--attempts", 3)?)?;
            if attempts == 0 {
                return Err("--attempts must be greater than zero".into());
            }
            let manifest: PopulationManifest = serde_json::from_slice(&fs::read(population_path)?)?;
            let candidates = manifest
                .members
                .iter()
                .map(|member| {
                    (
                        member.candidate_id.clone(),
                        CandidateResult {
                            goal_reached: Some(true),
                            milestone_depth: manifest.segment.target_depth(),
                            attempts,
                            successes: attempts,
                            first_hit_ticks: vec![member.frame_count; attempts as usize],
                            risk_events: None,
                            boundary_compatibility: huntctl::search::BoundaryCompatibility::Unknown,
                        },
                    )
                })
                .collect();
            let results = SearchResults {
                schema: RESULTS_SCHEMA.into(),
                segment: manifest.segment,
                boot: manifest.boot,
                candidates,
            };
            fs::write(&output, serde_json::to_vec_pretty(&results)?)?;
            println!("{}", serde_json::to_string_pretty(&results)?);
            Ok(())
        }
        _ => usage_error(),
    }
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

fn command_tape(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("inspect") if args.len() == 2 || (args.len() == 3 && args[2] == "--frames") => {
            let bytes = fs::read(&args[1])?;
            let decoded = InputTape::decode(&bytes)?;
            if args.get(2).is_some_and(|value| value == "--frames") {
                println!("{}", serde_json::to_string_pretty(&decoded)?);
            } else {
                let owned_ports = decoded
                    .tape
                    .frames
                    .iter()
                    .fold(0, |mask, frame| mask | frame.owned_ports);
                let mut wait_conditions = BTreeMap::new();
                for frame in &decoded.tape.frames {
                    if frame.wait_condition != huntctl::tape::WaitCondition::None {
                        *wait_conditions
                            .entry(frame.wait_condition.as_str())
                            .or_insert(0_usize) += 1;
                    }
                }
                let wait_frame_count: usize = wait_conditions.values().sum();
                let minimum_tick_count = decoded.tape.frames.len() - wait_frame_count;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "format": "DUSKTAPE",
                        "source_version": decoded.source_version,
                        "boot": decoded.tape.boot,
                        "tick_rate": {
                            "numerator": decoded.tape.tick_rate_numerator,
                            "denominator": decoded.tape.tick_rate_denominator
                        },
                        "nominal_frame_count": decoded.tape.frames.len(),
                        "owned_ports_union": owned_ports,
                        "wait_frame_count": wait_frame_count,
                        "wait_conditions": wait_conditions,
                        "minimum_tick_count": minimum_tick_count,
                        "minimum_duration_seconds": minimum_tick_count as f64
                            * decoded.tape.tick_rate_denominator as f64
                            / decoded.tape.tick_rate_numerator as f64
                    }))?
                );
            }
            Ok(())
        }
        Some("compile") if args.len() == 3 || args.len() == 5 => {
            if args.len() == 5 && args[3] != "--fixture" {
                return Err("tape compile optional argument is --fixture FIXTURE.json".into());
            }
            let source = fs::read_to_string(&args[1])?;
            let program = if source.trim_start().starts_with('{') {
                TapeProgram::from_json(&source)?
            } else {
                tape_dsl::parse(&source)?
            };
            let mut compiled = program.compile()?;
            if let Some(path) = option(&args[3..], "--fixture") {
                let fixture: ScenarioFixture = serde_json::from_slice(&fs::read(path)?)?;
                fixture.validate()?;
                match &mut compiled.tape.boot {
                    huntctl::tape::TapeBoot::Stage {
                        fixture: target, ..
                    } => {
                        if target.is_some() {
                            return Err("tape boot already contains a scenario fixture".into());
                        }
                        *target = Some(fixture);
                    }
                    huntctl::tape::TapeBoot::Process => {
                        return Err("--fixture requires a stage-boot tape".into());
                    }
                }
            }
            let bytes = compiled.tape.encode()?;
            fs::write(&args[2], &bytes)?;
            let marker_path = format!("{}.markers.json", args[2]);
            fs::write(
                &marker_path,
                serde_json::to_vec_pretty(&json!({
                    "schema": "dusktape-markers/v1",
                    "tape": args[2],
                    "markers": compiled.markers
                }))?,
            )?;
            println!(
                "wrote {} frames ({} bytes) to {}; markers: {}",
                compiled.tape.frames.len(),
                bytes.len(),
                args[2],
                marker_path
            );
            Ok(())
        }
        Some("run") if args.len() >= 2 => command_tape_run(&args[1..]),
        Some("prove") if args.len() >= 2 => command_tape_prove(&args[1..]),
        Some("record") if args.len() >= 3 => command_tape_record(&args[1..]),
        Some("minimize") if args.len() >= 3 => command_tape_minimize(&args[1..]),
        Some("concat") if args.len() >= 4 => {
            let output = PathBuf::from(&args[1]);
            let mut segments = Vec::with_capacity(args.len() - 2);
            for input in &args[2..] {
                let tape = InputTape::decode(&fs::read(input)?)?.tape;
                segments.push(ChainSegment::all(tape));
            }
            let chained = concatenate(segments)?;
            let bytes = chained.tape.encode()?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &bytes)?;
            println!(
                "concatenated {} tapes into {} frames ({} bytes) at {}",
                args.len() - 2,
                chained.tape.frames.len(),
                bytes.len(),
                output.display()
            );
            Ok(())
        }
        Some("slice") if args.len() == 7 && args[3] == "--start" && args[5] == "--frames" => {
            let input = PathBuf::from(&args[1]);
            let output = PathBuf::from(&args[2]);
            let start = args[4].parse::<usize>()?;
            let frame_count = args[6].parse::<usize>()?;
            if frame_count == 0 {
                return Err("tape slice --frames must be greater than zero".into());
            }
            let mut tape = InputTape::decode(&fs::read(&input)?)?.tape;
            let end = start
                .checked_add(frame_count)
                .ok_or("tape slice range overflows")?;
            if end > tape.frames.len() {
                return Err(format!(
                    "tape slice range {start}..{end} exceeds {} frames",
                    tape.frames.len()
                )
                .into());
            }
            tape.frames = tape.frames[start..end].to_vec();
            let bytes = tape.encode()?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &bytes)?;
            println!(
                "wrote frames {start}..{end} ({} frames, {} bytes) to {}",
                tape.frames.len(),
                bytes.len(),
                output.display()
            );
            Ok(())
        }
        Some("layer") if args.len() == 6 && args[4] == "--start" => {
            let base_path = PathBuf::from(&args[1]);
            let overlay_path = PathBuf::from(&args[2]);
            let output = PathBuf::from(&args[3]);
            let start = args[5].parse::<usize>()?;
            let base = InputTape::decode(&fs::read(&base_path)?)?.tape;
            let overlay = InputTape::decode(&fs::read(&overlay_path)?)?.tape;
            let overlay_frames = overlay.frames.len();
            let layered = layer_at(base, overlay, start)?;
            let bytes = layered.encode()?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &bytes)?;
            println!(
                "layered {} frames at {start} into {} frames ({} bytes) at {}",
                overlay_frames,
                layered.frames.len(),
                bytes.len(),
                output.display()
            );
            Ok(())
        }
        Some("resample") if args.len() == 3 => {
            let input = PathBuf::from(&args[1]);
            let output = PathBuf::from(&args[2]);
            let source = InputTape::decode(&fs::read(&input)?)?.tape;
            let source_rate = (
                source.tick_rate_numerator,
                source.tick_rate_denominator,
            );
            let source_frames = source.frames.len();
            let resampled = resample_to_canonical(source)?;
            let bytes = resampled.encode()?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, &bytes)?;
            println!(
                "resampled {source_frames} frames at {}/{} Hz to {} frames at 30/1 Hz ({} bytes) at {}",
                source_rate.0,
                source_rate.1,
                resampled.frames.len(),
                bytes.len(),
                output.display()
            );
            Ok(())
        }
        Some("diff") if args.len() == 3 => {
            let left = InputTape::decode(&fs::read(&args[1])?)?.tape;
            let right = InputTape::decode(&fs::read(&args[2])?)?.tape;
            println!("{}", serde_json::to_string_pretty(&diff_tapes(&left, &right))?);
            Ok(())
        }
        _ => Err("tape commands: inspect, compile, run, record, minimize, concat, slice, layer, resample, diff".into()),
    }
}

fn command_tape_run(args: &[String]) -> Result<(), Box<dyn Error>> {
    let input = PathBuf::from(args.first().ok_or("tape run requires INPUT.tape")?);
    let game = required_path(args, "--game")?;
    let dvd = required_path(args, "--dvd")?;
    let state_root = required_path(args, "--state-root")?;
    let decoded = InputTape::decode(&fs::read(&input)?)?;
    if decoded.tape.frames.is_empty() {
        return Err("tape run requires at least one input frame".into());
    }
    let logical_tick_budget = u64::try_from(decoded.tape.frames.len())
        .map_err(|_| "tape run input length does not fit u64")?;
    fs::create_dir_all(&state_root)?;
    let renderer_cache = state_root
        .parent()
        .unwrap_or_else(|| std::path::Path::new(""))
        .join("renderer-cache");
    fs::create_dir_all(&renderer_cache)?;
    let milestone_result = option(args, "--milestone-result")
        .map(PathBuf::from)
        .unwrap_or_else(|| state_root.join("milestones.json"));
    let milestone_goal = option(args, "--milestone-goal");
    let milestones = option(args, "--milestones").or_else(|| milestone_goal.clone());
    let gameplay_trace = option(args, "--gameplay-trace").map(PathBuf::from);
    let gameplay_trace_channels = option(args, "--gameplay-trace-channels");
    if gameplay_trace_channels.is_some() && gameplay_trace.is_none() {
        return Err("--gameplay-trace-channels requires --gameplay-trace FILE".into());
    }
    if milestones.is_some()
        && let Some(parent) = milestone_result.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    if let Some(parent) = gameplay_trace
        .as_ref()
        .and_then(|path| path.parent())
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }

    let mut command = Command::new(&game);
    command
        .args(repeated_option(args, "--game-arg"))
        .arg("--dvd")
        .arg(&dvd)
        .arg("--input-tape")
        .arg(&input)
        .arg("--automation-tick-budget")
        .arg(logical_tick_budget.to_string())
        .arg("--automation-data-root")
        .arg(&state_root)
        .arg("--renderer-cache-root")
        .arg(&renderer_cache)
        .arg("--cvar")
        .arg("game.instantSaves=true")
        .arg("--cvar")
        .arg("backend.cardFileType=1")
        .arg("--cvar")
        .arg("backend.wasPresetChosen=true")
        .arg("--cvar")
        .arg("game.enableMenuPointer=false")
        .arg("--fixed-step")
        .arg("--exit-after-tape");
    if !flag(args, "--headful") {
        command.arg("--headless");
    }
    if let Some(program) = option(args, "--milestone-program") {
        command.arg("--milestone-program").arg(program);
    }
    if let Some(milestones) = &milestones {
        command.arg("--milestones").arg(milestones);
    }
    if let Some(goal) = &milestone_goal {
        command.arg("--milestone-goal").arg(goal);
    }
    if milestones.is_some() {
        command.arg("--milestone-result").arg(&milestone_result);
    }
    if let Some(path) = &gameplay_trace {
        command.arg("--gameplay-trace").arg(path);
    }
    if let Some(channels) = &gameplay_trace_channels {
        command.arg("--gameplay-trace-channels").arg(channels);
    }

    let timeout = timeout_option(args)?;
    let started = Instant::now();
    let mut child = command.spawn()?;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= timeout {
            child.kill()?;
            let _ = child.wait();
            return Err(format!(
                "tape run timed out after {:.3} seconds",
                timeout.as_secs_f64()
            )
            .into());
        }
        thread::sleep(Duration::from_millis(10));
    };
    if status.success()
        && let Some(path) = &gameplay_trace
    {
        let trace = huntctl::trace::decode(&fs::read(path)?)?;
        if trace.boot != decoded.tape.boot {
            return Err(format!(
                "gameplay trace boot origin {:?} does not match tape origin {:?}",
                trace.boot, decoded.tape.boot
            )
            .into());
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": "huntctl-tape-run/v1",
            "boot": decoded.tape.boot,
            "source_version": decoded.source_version,
            "frames": decoded.tape.frames.len(),
            "exit_code": status.code(),
            "elapsed_millis": started.elapsed().as_millis(),
            "state_root": state_root,
            "milestone_result": milestones.is_some().then_some(milestone_result),
            "gameplay_trace": gameplay_trace,
        }))?
    );
    if !status.success() {
        return Err(format!("tape run exited with {:?}", status.code()).into());
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TapeMinimizeProof {
    sim_tick: u64,
    tape_frame: u64,
    fingerprint: BoundaryFingerprint,
}

fn command_tape_prove(args: &[String]) -> Result<(), Box<dyn Error>> {
    let input = PathBuf::from(args.first().ok_or("tape prove requires INPUT.tape")?);
    let game = required_path(args, "--game")?;
    let dvd = required_path(args, "--dvd")?;
    let work_root = required_path(args, "--state-root")?;
    let goal = option(args, "--milestone-goal").ok_or("tape prove requires --milestone-goal ID")?;
    let milestone_program = option(args, "--milestone-program").map(PathBuf::from);
    let repetitions = u32_option(args, "--repetitions", 2)?;
    if repetitions < 2 {
        return Err("tape prove requires at least two repetitions".into());
    }
    if work_root.exists() && fs::read_dir(&work_root)?.next().is_some() {
        return Err(format!(
            "tape prove --state-root must be new or empty: {}",
            work_root.display()
        )
        .into());
    }
    let proof_path = option(args, "--proof")
        .map(PathBuf::from)
        .unwrap_or_else(|| work_root.join("cold-replay.proof.json"));
    if proof_path.exists() {
        return Err(format!("cold-replay proof already exists: {}", proof_path.display()).into());
    }

    let tape_bytes = fs::read(&input)?;
    let tape = InputTape::decode(&tape_bytes)?.tape;
    if tape.frames.is_empty() {
        return Err("tape prove requires at least one frame".into());
    }
    if tape.tick_rate_numerator != 30 || tape.tick_rate_denominator != 1 {
        return Err("tape prove requires a canonical 30/1 input tape".into());
    }
    if tape
        .frames
        .iter()
        .any(|frame| frame.wait_condition != huntctl::tape::WaitCondition::None)
    {
        return Err("tape prove requires absolute input without reactive waits".into());
    }

    let game_args = repeated_option(args, "--game-arg");
    validate_cold_replay_game_args(&game_args)?;
    fs::create_dir_all(&work_root)?;
    if let Some(parent) = proof_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let timeout = timeout_option(args)?;
    let mut evaluation_index = 0_u64;
    let proof = evaluate_minimize_tape(
        &tape,
        &game,
        &dvd,
        &work_root,
        &goal,
        milestone_program.as_deref(),
        &game_args,
        repetitions,
        timeout,
        &mut evaluation_index,
    )?
    .ok_or("cold replay did not reach the requested milestone goal")?;
    let summary = json!({
        "schema": "dusklight-cold-replay-proof/v1",
        "input_tape": input,
        "input_tape_sha256": Digest(Sha256::digest(&tape_bytes).into()),
        "boot": tape.boot,
        "goal": goal,
        "milestone_program": milestone_program,
        "milestone_program_sha256": milestone_program
            .as_deref()
            .map(fs::read)
            .transpose()?
            .map(|bytes| Digest(Sha256::digest(bytes).into())),
        "game": game,
        "game_sha256": Digest(Sha256::digest(fs::read(&game)?).into()),
        "dvd": dvd,
        "dvd_sha256": Digest(Sha256::digest(fs::read(&dvd)?).into()),
        "game_args": game_args,
        "repetitions": repetitions,
        "controller_in_loop": false,
        "model_in_loop": false,
        "proof": {
            "sim_tick": proof.sim_tick,
            "tape_frame": proof.tape_frame,
            "boundary_fingerprint": proof.fingerprint,
        },
        "evidence_root": work_root,
    });
    fs::write(&proof_path, serde_json::to_vec_pretty(&summary)?)?;
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

fn validate_cold_replay_game_args(arguments: &[String]) -> Result<(), Box<dyn Error>> {
    const OWNED_OPTIONS: &[&str] = &[
        "--automation-data-root",
        "--automation-tick-budget",
        "--dvd",
        "--exit-after-tape",
        "--fixed-step",
        "--headless",
        "--input-controller",
        "--input-tape",
        "--input-tape-end",
        "--milestone-goal",
        "--milestone-program",
        "--milestone-result",
        "--milestones",
        "--renderer-cache-root",
    ];
    if let Some(argument) = arguments.iter().find(|argument| {
        OWNED_OPTIONS
            .iter()
            .any(|option| argument == option || argument.starts_with(&format!("{option}=")))
    }) {
        return Err(format!(
            "tape prove owns replay option {argument}; a controller, alternate tape, or proof override cannot enter the cold-replay launch"
        )
        .into());
    }
    Ok(())
}

fn command_tape_minimize(args: &[String]) -> Result<(), Box<dyn Error>> {
    let input = PathBuf::from(args.first().ok_or("tape minimize requires INPUT.tape")?);
    let output = PathBuf::from(args.get(1).ok_or("tape minimize requires OUTPUT.tape")?);
    let game = required_path(args, "--game")?;
    let dvd = required_path(args, "--dvd")?;
    let work_root = required_path(args, "--state-root")?;
    let goal =
        option(args, "--milestone-goal").ok_or("tape minimize requires --milestone-goal ID")?;
    let milestone_program = option(args, "--milestone-program").map(PathBuf::from);
    let repetitions = u32_option(args, "--repetitions", 2)?;
    if repetitions < 2 {
        return Err("tape minimize requires at least two repetitions".into());
    }
    if output.exists() {
        return Err(format!("minimized tape already exists: {}", output.display()).into());
    }
    let proof_path = output.with_extension("proof.json");
    if proof_path.exists() {
        return Err(format!(
            "minimization proof already exists: {}",
            proof_path.display()
        )
        .into());
    }
    if work_root.exists() && fs::read_dir(&work_root)?.next().is_some() {
        return Err(format!(
            "tape minimize --state-root must be new or empty: {}",
            work_root.display()
        )
        .into());
    }
    fs::create_dir_all(&work_root)?;
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }

    let source = InputTape::decode(&fs::read(&input)?)?.tape;
    if source.frames.is_empty() {
        return Err("tape minimize requires at least one frame".into());
    }
    let timeout = timeout_option(args)?;
    let game_args = repeated_option(args, "--game-arg");
    let mut evaluation_index = 0_u64;
    let target = evaluate_minimize_tape(
        &source,
        &game,
        &dvd,
        &work_root,
        &goal,
        milestone_program.as_deref(),
        &game_args,
        repetitions,
        timeout,
        &mut evaluation_index,
    )?
    .ok_or("source tape does not reach the requested milestone goal")?;
    let source_active_frames = tape_active_frames(&source).len();
    let mut current = source.clone();

    let mut granularity = 2_usize;
    loop {
        let active = tape_active_frames(&current);
        if active.is_empty() {
            break;
        }
        let partitions = granularity.min(active.len());
        let mut accepted = None;
        for partition in 0..partitions {
            let start = active.len() * partition / partitions;
            let end = active.len() * (partition + 1) / partitions;
            let mut candidate = current.clone();
            neutralize_tape_frames(&mut candidate, &active[start..end]);
            if evaluate_minimize_tape(
                &candidate,
                &game,
                &dvd,
                &work_root,
                &goal,
                milestone_program.as_deref(),
                &game_args,
                repetitions,
                timeout,
                &mut evaluation_index,
            )?
            .is_some_and(|proof| proof == target)
            {
                accepted = Some(candidate);
                break;
            }
        }
        if let Some(candidate) = accepted {
            current = candidate;
            granularity = 2;
        } else if partitions == active.len() {
            break;
        } else {
            granularity = (partitions * 2).min(active.len());
        }
    }

    loop {
        let active = tape_active_frames(&current);
        let mut accepted = None;
        for frame in active {
            let mut candidate = current.clone();
            neutralize_tape_frames(&mut candidate, &[frame]);
            if evaluate_minimize_tape(
                &candidate,
                &game,
                &dvd,
                &work_root,
                &goal,
                milestone_program.as_deref(),
                &game_args,
                repetitions,
                timeout,
                &mut evaluation_index,
            )?
            .is_some_and(|proof| proof == target)
            {
                accepted = Some(candidate);
                break;
            }
        }
        let Some(candidate) = accepted else {
            break;
        };
        current = candidate;
    }

    let required_frames = usize::try_from(target.tape_frame)?
        .checked_add(1)
        .ok_or("goal tape frame overflows")?;
    if required_frames > current.frames.len() {
        return Err("goal tape frame lies outside the source tape".into());
    }
    current.frames.truncate(required_frames);
    let final_proof = evaluate_minimize_tape(
        &current,
        &game,
        &dvd,
        &work_root,
        &goal,
        milestone_program.as_deref(),
        &game_args,
        repetitions,
        timeout,
        &mut evaluation_index,
    )?
    .ok_or("trimmed minimized tape no longer reaches the goal")?;
    if final_proof != target {
        return Err("trimmed minimized tape changed the exact goal proof".into());
    }

    fs::write(&output, current.encode()?)?;
    let summary = json!({
        "schema": "huntctl-tape-minimization/v1",
        "boot": current.boot,
        "goal": goal,
        "source_tape": input,
        "minimized_tape": output,
        "source_frames": source.frames.len(),
        "minimized_frames": current.frames.len(),
        "source_active_frames": source_active_frames,
        "minimized_active_frames": tape_active_frames(&current).len(),
        "evaluated_candidates": evaluation_index,
        "repetitions": repetitions,
        "proof": {
            "sim_tick": target.sim_tick,
            "tape_frame": target.tape_frame,
            "boundary_fingerprint": target.fingerprint,
        },
        "evidence_root": work_root,
    });
    fs::write(&proof_path, serde_json::to_vec_pretty(&summary)?)?;
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

fn tape_active_frames(tape: &InputTape) -> Vec<usize> {
    tape.frames
        .iter()
        .enumerate()
        .filter_map(|(index, frame)| {
            frame
                .pads
                .iter()
                .any(|pad| *pad != huntctl::tape::RawPadState::default())
                .then_some(index)
        })
        .collect()
}

fn neutralize_tape_frames(tape: &mut InputTape, frames: &[usize]) {
    for &index in frames {
        tape.frames[index]
            .pads
            .fill(huntctl::tape::RawPadState::default());
    }
}

#[allow(clippy::too_many_arguments)]
fn evaluate_minimize_tape(
    tape: &InputTape,
    game: &Path,
    dvd: &Path,
    work_root: &Path,
    goal: &str,
    milestone_program: Option<&Path>,
    game_args: &[String],
    repetitions: u32,
    timeout: Duration,
    evaluation_index: &mut u64,
) -> Result<Option<TapeMinimizeProof>, Box<dyn Error>> {
    let evaluation = *evaluation_index;
    *evaluation_index = evaluation
        .checked_add(1)
        .ok_or("minimization evaluation count overflowed")?;
    let root = work_root.join(format!("candidate-{evaluation:06}"));
    fs::create_dir_all(&root)?;
    let tape_path = root.join("candidate.tape");
    fs::write(&tape_path, tape.encode()?)?;
    let logical_tick_budget = u64::try_from(tape.frames.len())
        .map_err(|_| "minimization candidate length does not fit u64")?;
    let mut accepted: Option<TapeMinimizeProof> = None;
    let mut missed = false;
    for repetition in 1..=repetitions {
        let trial = root.join(format!("repeat-{repetition:03}"));
        let state = trial.join("state");
        let renderer_cache = trial.join("renderer-cache");
        let result_path = trial.join("milestones.json");
        fs::create_dir_all(&state)?;
        fs::create_dir_all(&renderer_cache)?;
        let stdout = fs::File::create(trial.join("stdout.txt"))?;
        let stderr = fs::File::create(trial.join("stderr.txt"))?;
        let mut command = Command::new(game);
        command
            .args(game_args)
            .arg("--dvd")
            .arg(dvd)
            .arg("--input-tape")
            .arg(&tape_path)
            .arg("--input-tape-end")
            .arg("hold")
            .arg("--automation-tick-budget")
            .arg(logical_tick_budget.to_string())
            .arg("--automation-data-root")
            .arg(&state)
            .arg("--renderer-cache-root")
            .arg(&renderer_cache)
            .arg("--milestones")
            .arg(goal)
            .arg("--milestone-goal")
            .arg(goal)
            .arg("--milestone-result")
            .arg(&result_path)
            .arg("--cvar")
            .arg("game.instantSaves=true")
            .arg("--cvar")
            .arg("backend.cardFileType=1")
            .arg("--cvar")
            .arg("backend.wasPresetChosen=true")
            .arg("--cvar")
            .arg("game.enableMenuPointer=false")
            .arg("--headless")
            .arg("--fixed-step")
            .arg("--exit-after-tape")
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        if let Some(program) = milestone_program {
            command.arg("--milestone-program").arg(program);
        }
        let started = Instant::now();
        let mut child = command.spawn()?;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if started.elapsed() >= timeout {
                child.kill()?;
                let _ = child.wait();
                return Err(format!(
                    "minimization candidate {evaluation} repeat {repetition} timed out"
                )
                .into());
            }
            thread::sleep(Duration::from_millis(10));
        };
        if status.code() == Some(2) {
            if accepted.is_some() {
                return Err("minimization repetitions disagree on goal reachability".into());
            }
            missed = true;
            continue;
        }
        if !status.success() {
            return Err(format!(
                "minimization candidate {evaluation} repeat {repetition} exited with {:?}",
                status.code()
            )
            .into());
        }
        let result: Value = serde_json::from_slice(&fs::read(&result_path)?)?;
        if missed {
            return Err("minimization repetitions disagree on goal reachability".into());
        }
        if result["schema"]["name"] != "dusklight.automation.milestones"
            || result["schema"]["version"] != 5
            || result["boot_origin_established"] != true
        {
            return Err("minimization received an unauthenticated milestone result".into());
        }
        let result_boot: huntctl::tape::TapeBoot = serde_json::from_value(result["boot"].clone())?;
        if result_boot != tape.boot {
            return Err("minimization result boot origin does not match its tape".into());
        }
        let milestone = result["milestones"]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["id"] == goal))
            .ok_or("minimization result omitted the requested goal")?;
        if milestone["hit"] != true || result["goal_reached"] != true {
            return Err("successful minimization process did not report a goal hit".into());
        }
        let fingerprint: BoundaryFingerprint =
            serde_json::from_value(milestone["evidence"]["boundary_fingerprint"].clone())?;
        if fingerprint.schema != "dusklight.milestone-boundary/v4"
            || fingerprint.canonical_encoding != "little-endian-fixed-v4"
            || fingerprint.algorithm != "xxh3-128"
        {
            return Err("minimization received an unsupported boundary fingerprint".into());
        }
        let proof = TapeMinimizeProof {
            sim_tick: milestone["sim_tick"]
                .as_u64()
                .ok_or("goal hit omitted sim_tick")?,
            tape_frame: milestone["tape_frame"]
                .as_u64()
                .ok_or("goal hit omitted tape_frame")?,
            fingerprint,
        };
        if accepted.as_ref().is_some_and(|prior| prior != &proof) {
            return Err("minimization repetitions produced contradictory exact proofs".into());
        }
        accepted = Some(proof);
    }
    Ok(if missed { None } else { accepted })
}

fn command_tape_record(args: &[String]) -> Result<(), Box<dyn Error>> {
    let seed_path = PathBuf::from(args.first().ok_or("tape record requires SEED.tape")?);
    let output_path = PathBuf::from(args.get(1).ok_or("tape record requires OUTPUT.tape")?);
    if output_path.exists() {
        return Err(format!("recording output already exists: {}", output_path.display()).into());
    }
    let game = required_path(args, "--game")?;
    let dvd = required_path(args, "--dvd")?;
    let state_root = required_path(args, "--state-root")?;
    let seed = InputTape::decode(&fs::read(&seed_path)?)?.tape;
    if seed.frames.is_empty() {
        return Err("tape record seed requires at least one input frame".into());
    }
    fs::create_dir_all(&state_root)?;
    let renderer_cache = state_root
        .parent()
        .unwrap_or_else(|| std::path::Path::new(""))
        .join("renderer-cache");
    fs::create_dir_all(&renderer_cache)?;
    let continuation_path = state_root.join("huntctl-recorded-continuation.tape");
    if continuation_path.exists() {
        return Err(format!(
            "recording continuation already exists: {}; use a fresh state root",
            continuation_path.display()
        )
        .into());
    }
    let capacity = usize_option(args, "--capacity", 1_080_000)?;
    if capacity == 0 {
        return Err("tape record --capacity must be greater than zero".into());
    }

    let mut command = Command::new(&game);
    command
        .args(repeated_option(args, "--game-arg"))
        .arg("--dvd")
        .arg(&dvd)
        .arg("--input-tape")
        .arg(&seed_path)
        .arg("--input-tape-end")
        .arg("release")
        .arg("--record-input-tape")
        .arg(&continuation_path)
        .arg("--record-input-capacity")
        .arg(capacity.to_string())
        .arg("--automation-data-root")
        .arg(&state_root)
        .arg("--renderer-cache-root")
        .arg(&renderer_cache)
        .arg("--cvar")
        .arg("game.instantSaves=true")
        .arg("--cvar")
        .arg("backend.cardFileType=1")
        .arg("--cvar")
        .arg("backend.wasPresetChosen=true")
        .arg("--cvar")
        .arg("game.enableMenuPointer=false")
        .arg("--fixed-step");

    let timeout = timeout_option(args)?;
    let started = Instant::now();
    let mut child = command.spawn()?;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= timeout {
            child.kill()?;
            let _ = child.wait();
            return Err(format!(
                "tape recording timed out after {:.3} seconds",
                timeout.as_secs_f64()
            )
            .into());
        }
        thread::sleep(Duration::from_millis(10));
    };
    if !status.success() {
        return Err(format!("tape recording exited with {:?}", status.code()).into());
    }
    let continuation = InputTape::decode(&fs::read(&continuation_path)?)?.tape;
    if continuation.boot != huntctl::tape::TapeBoot::Process {
        return Err("native continuation unexpectedly declared its own boot origin".into());
    }
    let continuation_frames = continuation.frames.len();
    let composed = concatenate(vec![
        ChainSegment::all(seed),
        ChainSegment::all(continuation),
    ])?
    .tape;
    if let Some(parent) = output_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output_path, composed.encode()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": "huntctl-tape-recording/v1",
            "boot": composed.boot,
            "seed_frames": composed.frames.len() - continuation_frames,
            "recorded_frames": continuation_frames,
            "total_frames": composed.frames.len(),
            "output": output_path,
            "native_continuation": continuation_path,
            "elapsed_millis": started.elapsed().as_millis(),
        }))?
    );
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
        "Campaign planning:\n  huntctl campaign --suite SUITE.json --case ID --output build/DIR --dry-run [--repository-root DIR] [--proposer scripted|random|structured|learned]...\n"
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
        "\nRun identity:\n  huntctl identity compare --mode replay|trace-merge|model-training|checkpoint-restore|cross-build-comparison --expected EXPECTED.json --actual ACTUAL.json"
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
        "  huntctl search tournament --definition TOURNAMENT.json --game PATH --dvd PATH --output DIR [--workers N] [--repetitions N]\n",
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
