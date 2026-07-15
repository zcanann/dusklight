use huntctl::client::{CONTROL_PROTOCOL_NAME, CONTROL_PROTOCOL_VERSION, WorkerClient};
use huntctl::controller_program::ControllerProgram;
use huntctl::corpus::Corpus;
use huntctl::fqi::{
    FittedQ, FqiConfig, MAX_FQI_ACTIONS, MAX_FQI_ITERATIONS, MAX_FQI_TRANSITIONS,
    MAX_FQI_TREE_DEPTH, MAX_FQI_TREES_PER_ACTION, Transition as FqiTransition,
};
use huntctl::milestone_dsl;
use huntctl::offline_rl::{
    ExploratoryExtractConfig, MOVEMENT_CATEGORICAL_FEATURES_V1, extract_exploratory_from_bytes,
    movement_feature_schema_digest_v1,
};
use huntctl::pool::{MixedBuildPolicy, WorkerLaunch, WorkerPool};
use huntctl::route_store::{ObjectId, RouteStore};
use huntctl::route_workbench::{
    MaterializeTarget, WorkbenchConfig, materialize_lineage, serve as serve_route_workbench,
};
use huntctl::search::{
    Candidate, CandidateResult, EvaluationArtifact, EvolutionConfig, PopulationManifest,
    RESULTS_SCHEMA, SearchResults, SegmentProfile, collect_results, evolve_population,
    rank_population, write_seed_population,
};
use huntctl::search_evaluator::{
    AnchoredObjectiveConfig, AnchoredSearchRunConfig, BootGolfConfig, BootMinimizeConfig,
    EvaluateConfig, SearchRunConfig, evaluate_population, golf_boot, minimize_boot,
    run_anchored_search, run_search,
};
use huntctl::tape::InputTape;
use huntctl::tape_chain::{ChainSegment, concatenate};
use huntctl::tape_dsl;
use huntctl::tape_program::{PROGRAM_SCHEMA, TapeProgram};
use huntctl::timeline::Timeline;
use huntctl::transition_corpus::TransitionCorpus;
use huntctl::transport::ProcessTransport;
use huntctl::{BuildIdentity, Digest};
use serde_json::{Value, json};
use sha2::{Digest as ShaDigest, Sha256};
use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fs;
use std::io::{self, BufRead, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

const MAX_LEARN_INPUT_CORPORA: usize = 64;

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
        "corpus" => command_corpus(&args[1..]),
        "controller" => command_controller(&args[1..]),
        "milestone" => command_milestone(&args[1..]),
        "tape" => command_tape(&args[1..]),
        "trace" => command_trace(&args[1..]),
        "timeline" => command_timeline(&args[1..]),
        "search" => command_search(&args[1..]),
        "learn" => command_learn(&args[1..]),
        "run" | "replay" => command_not_ready(command, &args[1..]),
        "mock-worker" => mock_worker(&args[1..]),
        "mock-search-worker" => mock_search_worker(&args[1..]),
        "help" | "--help" | "-h" => {
            print_usage();
            Ok(())
        }
        _ => usage_error(),
    }
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
                .map(|definition| {
                    json!({
                        "id": definition.name,
                        "sha256": Digest(definition.sha256),
                    })
                })
                .collect::<Vec<_>>();
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
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "format": "DUSKCTRL",
                    "version": { "major": version_major, "minor": version_minor },
                    "duration_frames": program.duration_frames,
                    "layer_count": program.layers.len(),
                    "layers": program.layers,
                }))?
            );
            Ok(())
        }
        _ => usage_error(),
    }
}

fn command_learn(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("extract-trace") => {
            let learn_args = &args[1..];
            let trace_path = required_path(learn_args, "--trace")?;
            let tape_path = required_path(learn_args, "--tape")?;
            let output = required_path(learn_args, "--output")?;
            let start_tape_frame: u64 = option(learn_args, "--start-frame")
                .ok_or("missing required --start-frame N")?
                .parse()?;
            let end_tape_frame: u64 = option(learn_args, "--end-frame")
                .ok_or("missing required --end-frame N")?
                .parse()?;
            let trace_bytes = fs::read(&trace_path)?;
            let tape_bytes = fs::read(&tape_path)?;
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
            let corpus = extract_exploratory_from_bytes(
                &trace_bytes,
                &tape_bytes,
                ExploratoryExtractConfig {
                    episode_digest,
                    start_tape_frame,
                    end_tape_frame,
                    start_reference: None,
                    terminal_reference: None,
                    end_is_terminal,
                },
            )?;
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
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schema": "dusklight-exploratory-extraction/v1",
                    "authoritative": false,
                    "limitations": [
                        "trace-v1 omits per-tick RNG and collision state",
                        "the batch contains observed behavior, not counterfactual actions",
                        "explicit frame bounds are not native milestone proof"
                    ],
                    "trace": trace_path,
                    "tape": tape_path,
                    "output": output,
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
        Some("fit") => {
            let learn_args = &args[1..];
            let inputs = repeated_option(learn_args, "--input");
            if inputs.is_empty() {
                return Err("learn fit requires at least one --input FILE".into());
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
                trees_per_action: usize_option(learn_args, "--trees", 31)?,
                max_tree_depth: usize_option(learn_args, "--max-depth", 8)?,
                seed: u64_option(learn_args, "--seed", FqiConfig::default().seed)?,
                ..FqiConfig::default()
            };
            if config.iterations > MAX_FQI_ITERATIONS {
                return Err(format!(
                    "--iterations must not exceed {MAX_FQI_ITERATIONS}; received {}",
                    config.iterations
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
            let mut action_support = BTreeMap::<u32, usize>::new();
            for input in &inputs {
                let corpus = TransitionCorpus::read_zstd_file(input)?;
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
                for transition in corpus.transitions {
                    let action = transition.action.action_id;
                    if !action_support.contains_key(&action)
                        && action_support.len() >= MAX_FQI_ACTIONS
                    {
                        return Err(format!(
                            "learn fit accepts at most {MAX_FQI_ACTIONS} distinct actions; encountered action {action} after reaching the limit"
                        )
                        .into());
                    }
                    *action_support.entry(action).or_default() += 1;
                    transitions.push(FqiTransition {
                        state: transition.state,
                        action,
                        duration: transition.duration_ticks,
                        reward: transition.reward,
                        next_state: transition.next_state,
                        terminal: transition.terminal,
                    });
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
            let model = FittedQ::fit(
                feature_count.ok_or("transition corpus has no feature width")? as usize,
                &actions,
                &transitions,
                &config,
            )?;
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
                    "transition_count": transitions.len(),
                    "query_transition": query_index,
                    "query_side": query_side,
                    "iterations": config.iterations,
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

fn command_search(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("evaluate") => {
            let search_args = &args[1..];
            let population = required_path(search_args, "--population")?;
            let game = required_path(search_args, "--game")?;
            let dvd = required_path(search_args, "--dvd")?;
            let output = required_path(search_args, "--output")?;
            let results = option(search_args, "--results")
                .map(PathBuf::from)
                .unwrap_or_else(|| output.join("results.json"));
            let working_directory = option(search_args, "--working-directory")
                .map(PathBuf::from)
                .unwrap_or(std::env::current_dir()?);
            let report = evaluate_population(&EvaluateConfig {
                population_path: population,
                game,
                dvd,
                output_root: output,
                results_path: results,
                working_directory,
                game_args_prefix: repeated_option(search_args, "--game-arg"),
                workers: usize_option(search_args, "--workers", 4)?,
                repetitions: u32_option(search_args, "--repetitions", 3)?,
                timeout: timeout_option(search_args)?,
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
            if segment.profile != SegmentProfile::LinkControlToTunnelCrawlStart {
                return Err(format!(
                    "route search currently requires profile {}, got {}",
                    SegmentProfile::LinkControlToTunnelCrawlStart.as_str(),
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
            let through_goal = materialize_lineage(
                &timeline,
                artifact_root,
                &lineage,
                MaterializeTarget::ThroughSegment(segment.id.clone()),
            )?;
            if through_goal.steps.len() != prefix.steps.len() + 1
                || through_goal.steps.last().map(|step| step.segment.as_str())
                    != Some(segment_name.as_str())
                || through_goal.tape.frames.len() <= prefix.tape.frames.len()
            {
                return Err(format!(
                    "lineage {lineage:?} does not contain {segment_name:?} immediately after parent segment {parent_segment:?}"
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
            };
            let seed_candidate = Candidate::from_absolute_tape(segment.profile, &suffix)?;

            let output = required_path(search_args, "--output")?;
            let game = required_path(search_args, "--game")?;
            let dvd = required_path(search_args, "--dvd")?;
            let working_directory = option(search_args, "--working-directory")
                .map(PathBuf::from)
                .unwrap_or(std::env::current_dir()?);
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
            let timeout = timeout_option(search_args)?;
            let rng_seed = u64_option(search_args, "--rng-seed", 1)?;
            if !repeated_option(search_args, "--game-arg").is_empty() {
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
                },
                objective: AnchoredObjectiveConfig {
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
            let summary = run_search(&SearchRunConfig {
                segment,
                seed_candidate,
                game: required_path(search_args, "--game")?,
                dvd: required_path(search_args, "--dvd")?,
                output_root: output,
                working_directory: option(search_args, "--working-directory")
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_dir()?),
                game_args_prefix: repeated_option(search_args, "--game-arg"),
                generations: u32_option(search_args, "--generations", 2)?,
                population_size: size,
                elite_count: usize_option(search_args, "--elites", (size / 4).max(1))?,
                workers: usize_option(search_args, "--workers", 4)?,
                repetitions: u32_option(search_args, "--repetitions", 3)?,
                timeout: timeout_option(search_args)?,
                rng_seed: u64_option(search_args, "--rng-seed", 1)?,
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
                            milestone_depth: manifest.segment.target_depth(),
                            attempts,
                            successes: attempts,
                            first_hit_ticks: vec![member.frame_count; attempts as usize],
                        },
                    )
                })
                .collect();
            let results = SearchResults {
                schema: RESULTS_SCHEMA.into(),
                segment: manifest.segment,
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
        _ => usage_error(),
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
        Some("compile") if args.len() == 3 => {
            let source = fs::read_to_string(&args[1])?;
            let program = if source.trim_start().starts_with('{') {
                TapeProgram::from_json(&source)?
            } else {
                tape_dsl::parse(&source)?
            };
            let compiled = program.compile()?;
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
        _ => usage_error(),
    }
}

fn command_hello(args: &[String]) -> Result<(), Box<dyn Error>> {
    let (program, worker_args) = worker_spec(args)?;
    let mut client = WorkerClient::new(ProcessTransport::spawn(program, &worker_args)?);
    let hello = client.handshake()?.clone();
    println!(
        "protocol={CONTROL_PROTOCOL_NAME}/{} version={} revision={} platform={}/{} pointer_bits={} dirty={}",
        CONTROL_PROTOCOL_VERSION,
        hello.build.version,
        hello.build.revision,
        hello.build.platform,
        hello.build.architecture,
        hello.build.pointer_bits,
        hello.build.dirty
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
    Err(format!("{command} is unavailable (engine_session={}, input_tape={}, batch_run={}); protocol v1 currently exposes bootstrap control only",
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
    eprintln!(
        "Usage:\n  huntctl hello --worker PATH [--worker-arg ARG]...\n  huntctl ping --worker PATH [--worker-arg ARG]...\n  huntctl pool health --worker PATH [--worker-arg ARG]... [--workers N] [--checks N] [--allow-mixed-builds]\n  huntctl controller compile SOURCE.duskctl OUTPUT.dctl\n  huntctl controller inspect INPUT.dctl\n  huntctl milestone compile SOURCE.milestones OUTPUT.dmsp\n  huntctl milestone inspect INPUT.dmsp\n  huntctl milestone format SOURCE.milestones\n  huntctl tape inspect INPUT.tape [--frames]\n  huntctl tape compile PROGRAM.tas OUTPUT.tape\n  huntctl tape concat OUTPUT.tape INPUT.tape INPUT.tape...\n  huntctl trace inspect INPUT.trace\n  huntctl trace timeline INPUT.trace\n  huntctl trace compare INPUT.trace INPUT.trace...\n  huntctl timeline parse ROUTE.timeline\n  huntctl timeline inspect ROUTE.timeline\n  huntctl timeline status --timeline FILE [--continuation NAME] [--select ORIGINAL_SEGMENT=REPLACEMENT_SEGMENT]... [--output FILE]\n  huntctl timeline rebase-compatible --timeline FILE --continuation NAME --select ORIGINAL_SEGMENT=REPLACEMENT_SEGMENT --name NEW_NAME\n  huntctl timeline store init ROOT\n  huntctl timeline store import --store ROOT --timeline FILE --ref REF\n  huntctl timeline store import-evaluation --store ROOT --evaluation FILE --segment NAME --fingerprint VALUE [--ref REF]\n  huntctl timeline store fork --store ROOT --from REF --to REF [--lineage NAME]\n  huntctl timeline store append --store ROOT --ref REF --timeline FILE --continuation NAME\n  huntctl timeline store replay-repair --store ROOT --from REF --to REF --timeline FILE --continuation NAME\n  huntctl timeline store promote --store ROOT --ref REF --object ID\n  huntctl timeline store resolve|show|verify|gc ...\n  huntctl search seed --segment ID --output DIR [--candidate FILE] [--size N] [--rng-seed N]\n  huntctl search collect --population MANIFEST --input EVALUATION.json... --output RESULTS.json\n  huntctl search evolve --population MANIFEST --results RESULTS --output DIR [--size N] [--elites N] [--rng-seed N]\n  huntctl search rank --population MANIFEST --results RESULTS\n  huntctl search inspect CANDIDATE.json\n  huntctl search mock-evaluate --population MANIFEST --output RESULTS.json [--attempts N]\n  huntctl corpus init ROOT\n  huntctl corpus ingest ROOT --tape INPUT.tape --scenario ID --build BUILD.json [--scenario-json METADATA.json]\n  huntctl corpus list ROOT\n  huntctl corpus show ROOT ARTIFACT_SHA256\n  huntctl corpus verify ROOT\n  huntctl run --worker PATH\n  huntctl replay --worker PATH\n  huntctl mock-worker [--mock-revision REVISION]\n\nSearch segment IDs: boot_to_fsp103, fsp103_to_fsp104\nTAS DSL: dusktape 1 (legacy JSON schema: {PROGRAM_SCHEMA})"
    );
    eprintln!(
        "\nRoute workbench:\n  huntctl timeline workbench --timeline FILE --game PATH [--dvd PATH] [--state-root DIR] [--port N] [--no-open]"
    );
    eprintln!("\nTape editing:\n  huntctl tape slice INPUT.tape OUTPUT.tape --start N --frames N");
    eprintln!(
        "\nNative search:\n  huntctl search evaluate --population MANIFEST --game PATH --dvd PATH --output DIR [--results FILE] [--workers N] [--repetitions N] [--timeout-seconds N]\n  huntctl search run --segment ID [--candidate FILE] --game PATH --dvd PATH --output DIR [--generations N] [--size N] [--elites N] [--workers N] [--repetitions N]\n  huntctl search minimize-boot --candidate FILE --game PATH --dvd PATH --output DIR [--workers N] [--repetitions N]\n  huntctl search golf-boot --candidate FILE --game PATH --dvd PATH --output DIR [--workers N] [--repetitions N]\n  huntctl search import-tape --segment ID --tape INPUT.tape --output CANDIDATE.json"
    );
    eprintln!(
        "  huntctl search run-route --timeline FILE --lineage NAME --segment TIMELINE_SEGMENT --game PATH --dvd PATH --output DIR [--generations N] [--size N] [--elites N] [--workers N] [--repetitions N]"
    );
    eprintln!(
        "\nNative fitted Q:\n  huntctl learn benchmark\n  huntctl learn extract-trace --trace INPUT.trace --tape INPUT.tape --start-frame N --end-frame N --output BATCH.dtcz [--terminal]\n  huntctl learn inspect --input BATCH.dtcz\n  huntctl learn fit --input BATCH.dtcz [--input MORE.dtcz] [--query-transition N] [--query-side state|next-state] [--iterations N] [--trees N] [--max-depth N] [--seed N] [--all-continuous | --categorical-feature N ...]"
    );
}

fn mock_worker(args: &[String]) -> Result<(), Box<dyn Error>> {
    let revision = option(args, "--mock-revision").unwrap_or_else(|| "mock".into());
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
                    "version": "mock", "describe": revision, "revision": revision, "branch": "test",
                    "source_date": "1970-01-01", "build_type": "test", "platform": env::consts::OS,
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
    let state_root = option(args, "--automation-data-root").unwrap_or_default();
    let second_attempt = state_root.contains("attempt-002");
    let unstable_miss = mode == "unstable-goal" && second_attempt;
    let coordinate_golf_tick = if mode == "coordinate-golf" {
        let tape_path = required_path(args, "--input-tape")?;
        let tape = InputTape::decode(&fs::read(tape_path)?)?.tape;
        let pulse_timestamps: Vec<_> = tape
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
        mode != "miss" && !unstable_miss
    };
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
                    "boundary_fingerprint": {
                        "schema": "dusklight.milestone-boundary/v1",
                        "algorithm": "xxh3-128",
                        "canonical_encoding": "little-endian-fixed-v1",
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
                "version": 1
            },
            "goal": goal,
            "goal_reached": hit_goal,
            "milestones": milestones
        }))?,
    )?;
    if mode == "miss" || unstable_miss || (mode == "coordinate-golf" && !hit_goal) {
        return Err("mock worker goal miss".into());
    }
    Ok(())
}

fn success_response(response_type: &str) -> Value {
    json!({
        "protocol": {"name": CONTROL_PROTOCOL_NAME, "version": CONTROL_PROTOCOL_VERSION},
        "type": response_type, "ok": true
    })
}
