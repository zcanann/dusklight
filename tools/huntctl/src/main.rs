use huntctl::client::{CONTROL_PROTOCOL_NAME, CONTROL_PROTOCOL_VERSION, WorkerClient};
use huntctl::corpus::Corpus;
use huntctl::pool::{MixedBuildPolicy, WorkerLaunch, WorkerPool};
use huntctl::search::{
    Candidate, CandidateResult, EvaluationArtifact, EvolutionConfig, PopulationManifest,
    RESULTS_SCHEMA, SearchResults, SegmentProfile, collect_results, evolve_population,
    rank_population, write_seed_population,
};
use huntctl::tape::InputTape;
use huntctl::tape_dsl;
use huntctl::tape_program::{PROGRAM_SCHEMA, TapeProgram};
use huntctl::transport::ProcessTransport;
use huntctl::{BuildIdentity, Digest};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

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
        "tape" => command_tape(&args[1..]),
        "trace" => command_trace(&args[1..]),
        "search" => command_search(&args[1..]),
        "run" | "replay" => command_not_ready(command, &args[1..]),
        "mock-worker" => mock_worker(&args[1..]),
        "help" | "--help" | "-h" => {
            print_usage();
            Ok(())
        }
        _ => usage_error(),
    }
}

fn command_search(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
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

fn usage_error<T>() -> Result<T, Box<dyn Error>> {
    print_usage();
    Err("invalid command line".into())
}

fn print_usage() {
    eprintln!(
        "Usage:\n  huntctl hello --worker PATH [--worker-arg ARG]...\n  huntctl ping --worker PATH [--worker-arg ARG]...\n  huntctl pool health --worker PATH [--worker-arg ARG]... [--workers N] [--checks N] [--allow-mixed-builds]\n  huntctl tape inspect INPUT.tape [--frames]\n  huntctl tape compile PROGRAM.tas OUTPUT.tape\n  huntctl trace inspect INPUT.trace\n  huntctl trace timeline INPUT.trace\n  huntctl trace compare INPUT.trace INPUT.trace...\n  huntctl search seed --segment ID --output DIR [--candidate FILE] [--size N] [--rng-seed N]\n  huntctl search collect --population MANIFEST --input EVALUATION.json... --output RESULTS.json\n  huntctl search evolve --population MANIFEST --results RESULTS --output DIR [--size N] [--elites N] [--rng-seed N]\n  huntctl search rank --population MANIFEST --results RESULTS\n  huntctl search inspect CANDIDATE.json\n  huntctl search mock-evaluate --population MANIFEST --output RESULTS.json [--attempts N]\n  huntctl corpus init ROOT\n  huntctl corpus ingest ROOT --tape INPUT.tape --scenario ID --build BUILD.json [--scenario-json METADATA.json]\n  huntctl corpus list ROOT\n  huntctl corpus show ROOT ARTIFACT_SHA256\n  huntctl corpus verify ROOT\n  huntctl run --worker PATH\n  huntctl replay --worker PATH\n  huntctl mock-worker [--mock-revision REVISION]\n\nSearch segment IDs: boot_to_fsp103, fsp103_to_fsp104\nTAS DSL: dusktape 1 (legacy JSON schema: {PROGRAM_SCHEMA})"
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

fn success_response(response_type: &str) -> Value {
    json!({
        "protocol": {"name": CONTROL_PROTOCOL_NAME, "version": CONTROL_PROTOCOL_VERSION},
        "type": response_type, "ok": true
    })
}
