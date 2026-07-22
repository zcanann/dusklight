//! Tape authoring, replay, proof, minimization, and recording adapters.

use crate::{
    flag, option, repeated_option, required_path, timeout_option, u32_option, usize_option,
};
use huntctl::Digest;
use huntctl::harness::run_contract::sha256_artifact_file;
use huntctl::native_fidelity::FIXED_AUTOMATION_CVARS;
use huntctl::scenario_fixture::ScenarioFixture;
use huntctl::search_evaluator::BoundaryFingerprint;
use huntctl::tape::InputTape;
use huntctl::tape_chain::{ChainSegment, concatenate};
use huntctl::tape_dsl;
use huntctl::tape_edit::{diff as diff_tapes, layer_at, resample_to_canonical};
use huntctl::tape_program::TapeProgram;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct TemporaryCardRoot {
    path: PathBuf,
}

impl TemporaryCardRoot {
    fn create(state_root: &Path) -> Result<Self, Box<dyn Error>> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = state_root.join(format!(
            ".memory-card-session-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self { path })
    }
}

impl Drop for TemporaryCardRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub(crate) fn command_tape(args: &[String]) -> Result<(), Box<dyn Error>> {
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
    let card_fixture = option(args, "--card-fixture").map(PathBuf::from);
    if let Some(path) = &card_fixture
        && !path.is_dir()
    {
        return Err(format!("card fixture is not a directory: {}", path.display()).into());
    }
    let decoded = InputTape::decode(&fs::read(&input)?)?;
    if decoded.tape.frames.is_empty() {
        return Err("tape run requires at least one input frame".into());
    }
    let logical_tick_budget = u64::try_from(decoded.tape.frames.len())
        .map_err(|_| "tape run input length does not fit u64")?;
    fs::create_dir_all(&state_root)?;
    let card_root = TemporaryCardRoot::create(&state_root)?;
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
        .arg("--automation-card-root")
        .arg(&card_root.path)
        .arg("--renderer-cache-root")
        .arg(&renderer_cache);
    for cvar in FIXED_AUTOMATION_CVARS {
        command.arg("--cvar").arg(cvar);
    }
    command.arg("--fixed-step").arg("--exit-after-tape");
    if let Some(path) = &card_fixture {
        command.arg("--automation-card-fixture").arg(path);
    }
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
            "card_fixture": card_fixture,
            "milestone_result": milestones.is_some().then_some(milestone_result),
            "gameplay_trace": gameplay_trace,
        }))?
    );
    if !status.success() {
        return Err(format!("tape run exited with {:?}", status.code()).into());
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
struct TapeMinimizeProof {
    sim_tick: u64,
    tape_frame: u64,
    fingerprint: BoundaryFingerprint,
}

const TAPE_REPLAY_TERMINAL_CLASS: &str = "reached";
const TAPE_REPLAY_FIDELITY_PROFILE: &str = "headless-fixed-step-unpaced-30hz/v1";
const TAPE_REPLAY_CVARS: [&str; 5] = FIXED_AUTOMATION_CVARS;

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
    validate_proof_tape(&tape, "tape prove")?;

    let game_args = repeated_option(args, "--game-arg");
    validate_replay_game_args("tape prove", &game_args)?;
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
        "game_sha256": sha256_artifact_file(&game)?,
        "dvd": dvd,
        "dvd_sha256": sha256_artifact_file(&dvd)?,
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

fn validate_replay_game_args(command: &str, arguments: &[String]) -> Result<(), Box<dyn Error>> {
    const OWNED_OPTIONS: &[&str] = &[
        "--automation-data-root",
        "--automation-tick-budget",
        "--dvd",
        "--deterministic-time-start",
        "--exit-after-tape",
        "--fixed-step",
        "--fixed-step-speed-percent",
        "--headless",
        "--input-controller",
        "--input-tape",
        "--input-tape-end",
        "--input-tape-fast-forward-frames",
        "--input-tape-fast-forward-visible",
        "--milestone-goal",
        "--milestone-program",
        "--milestone-result",
        "--milestones",
        "--renderer-cache-root",
        "--unpaced",
    ];
    if let Some(argument) = arguments.iter().find(|argument| {
        OWNED_OPTIONS
            .iter()
            .any(|option| argument == option || argument.starts_with(&format!("{option}=")))
    }) {
        return Err(format!(
            "{command} owns replay option {argument}; a controller, alternate tape, fidelity override, or proof override cannot enter the replay launch"
        )
        .into());
    }
    Ok(())
}

fn validate_proof_tape(tape: &InputTape, command: &str) -> Result<(), Box<dyn Error>> {
    if tape.frames.is_empty() {
        return Err(format!("{command} requires at least one frame").into());
    }
    if tape.tick_rate_numerator != 30 || tape.tick_rate_denominator != 1 {
        return Err(format!("{command} requires a canonical 30/1 input tape").into());
    }
    if tape
        .frames
        .iter()
        .any(|frame| frame.wait_condition != huntctl::tape::WaitCondition::None)
    {
        return Err(format!("{command} requires absolute input without reactive waits").into());
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
    let source_bytes = fs::read(&input)?;
    let source = InputTape::decode(&source_bytes)?.tape;
    validate_proof_tape(&source, "tape minimize")?;
    let timeout = timeout_option(args)?;
    let game_args = repeated_option(args, "--game-arg");
    validate_replay_game_args("tape minimize", &game_args)?;
    fs::create_dir_all(&work_root)?;
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }

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

    let minimized_bytes = current.encode()?;
    fs::write(&output, &minimized_bytes)?;
    let summary = json!({
        "schema": "dusklight-tape-minimization-proof/v2",
        "boot": current.boot,
        "source_boot": source.boot,
        "goal": goal,
        "source_tape": input,
        "source_tape_sha256": Digest(Sha256::digest(&source_bytes).into()),
        "minimized_tape": output,
        "minimized_tape_sha256": Digest(Sha256::digest(&minimized_bytes).into()),
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
        "fidelity": {
            "profile": TAPE_REPLAY_FIDELITY_PROFILE,
            "headless": true,
            "fixed_step": true,
            "unpaced": true,
            "logical_hz": 30,
            "cvars": TAPE_REPLAY_CVARS,
        },
        "source_frames": source.frames.len(),
        "minimized_frames": current.frames.len(),
        "source_active_frames": source_active_frames,
        "minimized_active_frames": tape_active_frames(&current).len(),
        "evaluated_candidates": evaluation_index,
        "repetitions": repetitions,
        "proof": {
            "terminal_class": TAPE_REPLAY_TERMINAL_CLASS,
            "fidelity_profile": TAPE_REPLAY_FIDELITY_PROFILE,
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
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        for cvar in TAPE_REPLAY_CVARS {
            command.arg("--cvar").arg(cvar);
        }
        command
            .arg("--headless")
            .arg("--fixed-step")
            .arg("--exit-after-tape");
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
                let quarantine = write_replay_quarantine(
                    &root,
                    tape,
                    game,
                    dvd,
                    goal,
                    milestone_program,
                    game_args,
                    repetitions,
                    repetition,
                    "goal_reachability_disagreement",
                    accepted.as_ref(),
                    None,
                    false,
                )?;
                return Err(format!(
                    "minimization repetitions disagree on goal reachability; quarantined at {}",
                    quarantine.display()
                )
                .into());
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
            let quarantine = write_replay_quarantine(
                &root,
                tape,
                game,
                dvd,
                goal,
                milestone_program,
                game_args,
                repetitions,
                repetition,
                "goal_reachability_disagreement",
                None,
                None,
                true,
            )?;
            return Err(format!(
                "minimization repetitions disagree on goal reachability; quarantined at {}",
                quarantine.display()
            )
            .into());
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
        let supported_boundary = (fingerprint.schema == "dusklight.milestone-boundary/v4"
            && fingerprint.canonical_encoding == "little-endian-fixed-v4")
            || (fingerprint.schema == "dusklight.milestone-boundary/v5"
                && fingerprint.canonical_encoding == "little-endian-fixed-v5")
            || (fingerprint.schema == "dusklight.milestone-boundary/v6"
                && fingerprint.canonical_encoding == "little-endian-fixed-v6");
        if !supported_boundary || fingerprint.algorithm != "xxh3-128" {
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
            let quarantine = write_replay_quarantine(
                &root,
                tape,
                game,
                dvd,
                goal,
                milestone_program,
                game_args,
                repetitions,
                repetition,
                "exact_proof_disagreement",
                accepted.as_ref(),
                Some(&proof),
                true,
            )?;
            return Err(format!(
                "minimization repetitions produced contradictory exact proofs; quarantined at {}",
                quarantine.display()
            )
            .into());
        }
        accepted = Some(proof);
    }
    Ok(if missed { None } else { accepted })
}

#[allow(clippy::too_many_arguments)]
fn write_replay_quarantine(
    root: &Path,
    tape: &InputTape,
    game: &Path,
    dvd: &Path,
    goal: &str,
    milestone_program: Option<&Path>,
    game_args: &[String],
    repetitions_expected: u32,
    contradictory_repetition: u32,
    reason: &str,
    prior_proof: Option<&TapeMinimizeProof>,
    current_proof: Option<&TapeMinimizeProof>,
    current_goal_reached: bool,
) -> Result<PathBuf, Box<dyn Error>> {
    let tape_bytes = tape.encode()?;
    let quarantine_path = root.join("quarantine.json");
    let retained_trials = (1..=contradictory_repetition)
        .map(|repetition| root.join(format!("repeat-{repetition:03}")))
        .collect::<Vec<_>>();
    let quarantine = json!({
        "schema": "dusklight-replay-quarantine/v1",
        "reason": reason,
        "promotion_allowed": false,
        "candidate": {
            "tape": root.join("candidate.tape"),
            "tape_sha256": Digest(Sha256::digest(&tape_bytes).into()),
        },
        "build": {
            "game": game,
            "game_sha256": Digest(Sha256::digest(fs::read(game)?).into()),
            "dvd": dvd,
            "dvd_sha256": Digest(Sha256::digest(fs::read(dvd)?).into()),
            "game_args": game_args,
            "fidelity_profile": TAPE_REPLAY_FIDELITY_PROFILE,
            "cvars": TAPE_REPLAY_CVARS,
        },
        "scenario": {
            "boot": tape.boot,
            "goal": goal,
            "milestone_program": milestone_program,
            "milestone_program_sha256": milestone_program
                .map(fs::read)
                .transpose()?
                .map(|bytes| Digest(Sha256::digest(bytes).into())),
        },
        "repetitions_expected": repetitions_expected,
        "contradictory_repetition": contradictory_repetition,
        "prior_goal_reached": prior_proof.is_some(),
        "current_goal_reached": current_goal_reached,
        "prior_proof": prior_proof,
        "current_proof": current_proof,
        "retained_trials": retained_trials,
    });
    fs::write(&quarantine_path, serde_json::to_vec_pretty(&quarantine)?)?;
    Ok(quarantine_path)
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
        .arg(&renderer_cache);
    for cvar in FIXED_AUTOMATION_CVARS {
        command.arg("--cvar").arg(cvar);
    }
    command.arg("--fixed-step");

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
