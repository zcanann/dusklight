//! Native execution adapter for authenticated core-harness requests.

use super::objective_suite::{ArtifactReference, ObjectiveBoot, ObjectiveSeed};
use super::observation_contract::{
    OBSERVATION_INVENTORY_SCHEMA_V1, ObservationFamilyAvailability, ObservationFamilyStatus,
    ObservationInventory,
};
use super::run_contract::{
    HarnessBoundaryFingerprint, HarnessFidelityMode, HarnessObjectiveResult, HarnessRunArtifacts,
    HarnessRunRequest, HarnessRunResult, HarnessRunTiming, HarnessTerminalDetail,
    HarnessTerminalReason, HarnessWorkerIdentity, RUN_RESULT_SCHEMA_V2,
};
use crate::artifact::Digest;
use crate::controller_program::ControllerProgram;
use crate::milestone_dsl;
use crate::scenario_fixture::ScenarioFixture;
use crate::tape::{InputFrame, InputTape, TapeBoot, WaitCondition};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const VALID_GOAL_MISS_EXIT_CODE: i32 = 2;
const CONTROLLER_PROTOCOL_FAILURE_EXIT_CODE: i32 = 3;
const FULL_TRACE_MAXIMUM_TICKS: u64 = 131_071;

/// Dispatches one authenticated request to its execution adapter.
pub fn execute_request(
    request: &HarnessRunRequest,
    repository_root: &Path,
    attempt: u32,
) -> Result<HarnessRunResult, HarnessExecutionError> {
    execute_native_request(request, repository_root, attempt)
}

/// Executes a tape- or controller-backed request in one isolated native
/// process and seals its typed result beneath the authenticated destination.
fn execute_native_request(
    request: &HarnessRunRequest,
    repository_root: &Path,
    attempt: u32,
) -> Result<HarnessRunResult, HarnessExecutionError> {
    request
        .validate_files(repository_root)
        .map_err(|error| execution_error(format!("invalid run request: {error}")))?;
    if attempt == 0 {
        return Err(execution_error("harness attempt must be nonzero"));
    }

    let repository_root = repository_root.canonicalize().map_err(|error| {
        execution_error(format!(
            "cannot resolve repository root {}: {error}",
            repository_root.display()
        ))
    })?;
    let scenario = read_scenario(request, &repository_root)?;
    let prepared_input = prepare_native_input(request, &repository_root, scenario)?;
    let expected_boot = prepared_input.boot().clone();
    let planned_ticks = prepared_input.planned_ticks();
    if planned_ticks > request.logical_tick_budget {
        return Err(execution_error(format!(
            "input requires {planned_ticks} ticks but request budget is {}",
            request.logical_tick_budget
        )));
    }

    let objective_source = fs::read(repository_root.join(&request.objective.source.path))
        .map_err(|error| execution_error(format!("cannot read objective source: {error}")))?;
    let objective_text = std::str::from_utf8(&objective_source)
        .map_err(|error| execution_error(format!("objective source is not UTF-8: {error}")))?;
    let compiled = milestone_dsl::compile_source(objective_text)
        .map_err(|error| execution_error(format!("cannot compile objective program: {error}")))?;
    if Digest(compiled.program_sha256) != request.objective.program_sha256 {
        return Err(execution_error(
            "compiled objective identity changed after request validation",
        ));
    }

    let artifact_root = create_artifact_root(&repository_root, &request.artifact_destination)?;
    let paths = ExecutionPaths::new(&artifact_root);
    fs::create_dir(&paths.state).map_err(|error| {
        execution_error(format!("cannot create isolated automation state: {error}"))
    })?;
    fs::create_dir(&paths.renderer_cache).map_err(|error| {
        execution_error(format!("cannot create isolated renderer cache: {error}"))
    })?;
    if let Some(tape) = prepared_input.launch_tape() {
        write_new_file(
            &paths.input,
            &tape.encode().map_err(|error| {
                execution_error(format!("cannot encode materialized input tape: {error}"))
            })?,
        )?;
    }
    write_new_file(&paths.objective_program, &compiled.bytes)?;

    let stdout = File::create(&paths.stdout)
        .map_err(|error| execution_error(format!("cannot create stdout artifact: {error}")))?;
    let stderr = File::create(&paths.stderr)
        .map_err(|error| execution_error(format!("cannot create stderr artifact: {error}")))?;
    let executable = repository_root.join(&request.executable.path);
    let game_data = repository_root.join(&request.game_data.path);
    let mut command = Command::new(executable);
    command
        .current_dir(&repository_root)
        .arg("--dvd")
        .arg(game_data);
    match &prepared_input {
        PreparedNativeInput::Tape { .. } => {
            command
                .arg("--input-tape")
                .arg(&paths.input)
                .arg("--input-tape-end")
                .arg("release")
                .arg("--exit-after-tape");
        }
        PreparedNativeInput::Controller {
            artifact,
            stage_boot_tape,
            ..
        } => {
            if stage_boot_tape.is_some() {
                command
                    .arg("--input-tape")
                    .arg(&paths.input)
                    .arg("--input-tape-end")
                    .arg("release");
            }
            command
                .arg("--input-controller")
                .arg(artifact)
                .arg("--exit-after-controller");
        }
    }
    command
        .arg("--realized-input-tape")
        .arg(&paths.realized_input)
        .arg("--automation-data-root")
        .arg(&paths.state)
        .arg("--automation-tick-budget")
        .arg(request.logical_tick_budget.to_string())
        .arg("--renderer-cache-root")
        .arg(&paths.renderer_cache)
        .arg("--milestone-program")
        .arg(&paths.objective_program)
        .arg("--milestones")
        .arg(&request.objective.goal)
        .arg("--milestone-goal")
        .arg(&request.objective.goal)
        .arg("--milestone-result")
        .arg(&paths.objective_result)
        .arg("--gameplay-trace")
        .arg(&paths.gameplay_trace)
        .arg("--gameplay-trace-channels")
        .arg("all")
        .arg("--cvar")
        .arg("game.instantSaves=true")
        .arg("--cvar")
        .arg("backend.cardFileType=1")
        .arg("--cvar")
        .arg("backend.wasPresetChosen=true")
        .arg("--cvar")
        .arg("game.enableMenuPointer=false")
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    if request.logical_tick_budget > FULL_TRACE_MAXIMUM_TICKS {
        command
            .arg("--gameplay-trace-retention")
            .arg("64,64")
            .arg("--gameplay-trace-triggers")
            .arg("predicate,crash");
    }
    match request.fidelity {
        HarnessFidelityMode::Headless => {
            command.arg("--headless");
        }
        HarnessFidelityMode::UnpacedHeadful => {
            command.arg("--unpaced");
        }
        HarnessFidelityMode::RealtimeHeadful => {
            command.arg("--fixed-step");
        }
    }

    let started = Instant::now();
    let execution = match command.spawn() {
        Ok(mut child) => {
            let timeout = Duration::from_secs(u64::from(request.host_timeout_seconds));
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => break NativeExecution::Exited(status),
                    Ok(None) if started.elapsed() < timeout => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Ok(None) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break NativeExecution::TimedOut;
                    }
                    Err(error) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break NativeExecution::WaitFailed(error.to_string());
                    }
                }
            }
        }
        Err(error) => NativeExecution::LaunchFailed(error.to_string()),
    };
    let elapsed_millis = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    let native_goal = read_native_goal(&paths.objective_result, request, &expected_boot).ok();
    if let Some(goal) = &native_goal
        && goal.reached
        && let Some(tape_frame) = goal.tape_frame
    {
        trim_realized_tape(&paths.realized_input, &expected_boot, tape_frame)?;
    }
    let controller_planned_ticks = prepared_input.controller_planned_ticks();
    // Drop the large in-memory proposal before reading proof artifacts.
    drop(prepared_input);

    let realized_valid = validate_realized_tape(&paths.realized_input, &expected_boot).ok();
    let trace_valid = validate_gameplay_trace(&paths.gameplay_trace, &expected_boot).ok();
    let unsupported_detail = trace_valid
        .as_ref()
        .map(|trace| request.unsupported_observation_detail(&trace.inventory))
        .transpose()
        .map_err(|error| execution_error(format!("cannot assess observation support: {error}")))?
        .flatten();
    let outcome = classify_execution(
        &execution,
        native_goal.as_ref(),
        realized_valid,
        trace_valid.as_ref().map(|trace| trace.ticks),
        unsupported_detail,
        controller_planned_ticks,
    );
    let result = build_result(
        request,
        attempt,
        &paths,
        &outcome,
        elapsed_millis,
        native_goal.as_ref(),
        realized_valid,
    )?;
    result
        .validate_files(request, &artifact_root)
        .map_err(|error| {
            execution_error(format!("executor produced an invalid result: {error}"))
        })?;
    write_new_file(
        &paths.result,
        &result
            .to_pretty_json()
            .map_err(|error| execution_error(format!("cannot encode run result: {error}")))?,
    )?;
    Ok(result)
}

#[derive(Debug)]
struct ExecutionPaths {
    root: PathBuf,
    state: PathBuf,
    renderer_cache: PathBuf,
    input: PathBuf,
    objective_program: PathBuf,
    realized_input: PathBuf,
    gameplay_trace: PathBuf,
    objective_result: PathBuf,
    stdout: PathBuf,
    stderr: PathBuf,
    result: PathBuf,
}

impl ExecutionPaths {
    fn new(root: &Path) -> Self {
        Self {
            root: root.to_owned(),
            state: root.join("state"),
            renderer_cache: root.join("renderer-cache"),
            input: root.join("input.tape"),
            objective_program: root.join("objective.dmsp"),
            realized_input: root.join("realized.tape"),
            gameplay_trace: root.join("gameplay.trace"),
            objective_result: root.join("objective.json"),
            stdout: root.join("stdout.txt"),
            stderr: root.join("stderr.txt"),
            result: root.join("result.json"),
        }
    }
}

enum PreparedNativeInput {
    Tape {
        tape: InputTape,
    },
    Controller {
        artifact: PathBuf,
        boot: TapeBoot,
        stage_boot_tape: Option<Box<InputTape>>,
        duration_ticks: u64,
    },
}

impl PreparedNativeInput {
    fn boot(&self) -> &TapeBoot {
        match self {
            Self::Tape { tape } => &tape.boot,
            Self::Controller { boot, .. } => boot,
        }
    }

    fn planned_ticks(&self) -> u64 {
        match self {
            Self::Tape { tape } => u64::try_from(tape.frames.len()).unwrap_or(u64::MAX),
            Self::Controller { duration_ticks, .. } => *duration_ticks,
        }
    }

    fn launch_tape(&self) -> Option<&InputTape> {
        match self {
            Self::Tape { tape } => Some(tape),
            Self::Controller {
                stage_boot_tape, ..
            } => stage_boot_tape.as_deref(),
        }
    }

    fn controller_planned_ticks(&self) -> Option<u64> {
        match self {
            Self::Tape { .. } => None,
            Self::Controller { duration_ticks, .. } => Some(*duration_ticks),
        }
    }
}

enum NativeExecution {
    Exited(ExitStatus),
    TimedOut,
    LaunchFailed(String),
    WaitFailed(String),
}

struct NativeGoal {
    reached: bool,
    first_hit_tick: Option<u64>,
    tape_frame: Option<u64>,
    fingerprint: Option<HarnessBoundaryFingerprint>,
}

struct ClassifiedExecution {
    terminal: HarnessTerminalReason,
    message: String,
    proof_complete: bool,
    unsupported_detail: Option<HarnessTerminalDetail>,
}

struct ValidatedGameplayTrace {
    ticks: u64,
    inventory: ObservationInventory,
}

#[derive(Deserialize)]
struct NativeMilestoneResult {
    schema: NativeSchema,
    boot: Option<TapeBoot>,
    boot_origin_established: Option<bool>,
    goal: Option<String>,
    goal_reached: bool,
    program_digest: Option<String>,
    milestones: Vec<NativeMilestone>,
}

#[derive(Deserialize)]
struct NativeSchema {
    name: String,
    version: u32,
}

#[derive(Deserialize)]
struct NativeMilestone {
    id: String,
    hit: bool,
    sim_tick: Option<u64>,
    tape_frame: Option<u64>,
    evidence: Option<NativeEvidence>,
}

#[derive(Deserialize)]
struct NativeEvidence {
    boundary_fingerprint: HarnessBoundaryFingerprint,
}

fn read_scenario(
    request: &HarnessRunRequest,
    root: &Path,
) -> Result<ScenarioFixture, HarnessExecutionError> {
    serde_json::from_slice(
        &fs::read(root.join(&request.scenario.path))
            .map_err(|error| execution_error(format!("cannot read scenario fixture: {error}")))?,
    )
    .map_err(|error| execution_error(format!("cannot decode scenario fixture: {error}")))
}

fn prepare_native_input(
    request: &HarnessRunRequest,
    root: &Path,
    scenario: ScenarioFixture,
) -> Result<PreparedNativeInput, HarnessExecutionError> {
    match &request.input {
        ObjectiveSeed::Controller { artifact } => {
            validate_process_scenario(request, &scenario)?;
            let boot = objective_boot(request, Some(scenario));
            let artifact_path = root.join(&artifact.path);
            let program =
                ControllerProgram::decode(&fs::read(&artifact_path).map_err(|error| {
                    execution_error(format!("cannot read seed controller: {error}"))
                })?)
                .map_err(|error| {
                    execution_error(format!("cannot decode seed controller: {error}"))
                })?;
            let duration_ticks = u64::from(program.duration_frames);
            let stage_boot_tape = matches!(boot, TapeBoot::Stage { .. }).then(|| {
                Box::new(InputTape {
                    boot: boot.clone(),
                    ..InputTape::default()
                })
            });
            Ok(PreparedNativeInput::Controller {
                artifact: artifact_path,
                boot,
                stage_boot_tape,
                duration_ticks,
            })
        }
        _ => {
            let tape = materialize_tape(request, root, scenario)?;
            if tape.tick_rate_numerator != 30 || tape.tick_rate_denominator != 1 {
                return Err(execution_error(
                    "native harness execution requires a canonical 30/1 input tape",
                ));
            }
            if tape.frames.is_empty() {
                return Err(execution_error(
                    "native harness execution requires at least one input frame",
                ));
            }
            if tape
                .frames
                .iter()
                .any(|frame| frame.wait_condition != WaitCondition::None)
            {
                return Err(execution_error(
                    "authenticated tape execution requires absolute input without reactive waits",
                ));
            }
            Ok(PreparedNativeInput::Tape { tape })
        }
    }
}

fn materialize_tape(
    request: &HarnessRunRequest,
    root: &Path,
    scenario: ScenarioFixture,
) -> Result<InputTape, HarnessExecutionError> {
    validate_process_scenario(request, &scenario)?;
    let expected_boot = objective_boot(request, Some(scenario));
    let mut tape = match &request.input {
        ObjectiveSeed::Neutral => InputTape {
            boot: expected_boot.clone(),
            frames: vec![
                InputFrame {
                    owned_ports: 0x0f,
                    ..InputFrame::default()
                };
                usize::try_from(request.logical_tick_budget).map_err(|_| {
                    execution_error("neutral input budget does not fit host address space")
                })?
            ],
            ..InputTape::default()
        },
        ObjectiveSeed::Tape { artifact } => {
            InputTape::decode(
                &fs::read(root.join(&artifact.path))
                    .map_err(|error| execution_error(format!("cannot read seed tape: {error}")))?,
            )
            .map_err(|error| execution_error(format!("cannot decode seed tape: {error}")))?
            .tape
        }
        ObjectiveSeed::TapeSource { artifact } => {
            let source = fs::read_to_string(root.join(&artifact.path)).map_err(|error| {
                execution_error(format!("cannot read seed tape source: {error}"))
            })?;
            crate::tape_dsl::parse(&source)
                .map_err(|error| execution_error(format!("cannot parse seed tape: {error}")))?
                .compile()
                .map_err(|error| execution_error(format!("cannot compile seed tape: {error}")))?
                .tape
        }
        ObjectiveSeed::Controller { .. } => {
            return Err(execution_error(
                "controller input must use the reactive harness executor",
            ));
        }
    };
    if !same_boot_origin(&tape.boot, &expected_boot) {
        return Err(execution_error(
            "materialized input boot disagrees with the run request",
        ));
    }
    tape.boot = expected_boot;
    tape.validate()
        .map_err(|error| execution_error(format!("materialized tape is invalid: {error}")))?;
    Ok(tape)
}

fn validate_process_scenario(
    request: &HarnessRunRequest,
    scenario: &ScenarioFixture,
) -> Result<(), HarnessExecutionError> {
    if matches!(request.boot, ObjectiveBoot::Process)
        && (scenario.form.is_some()
            || scenario.health.is_some()
            || !scenario.rng.is_empty()
            || scenario.video_mode.is_some()
            || !scenario.inventory.is_empty()
            || !scenario.equipment.is_empty()
            || !scenario.flags.is_empty()
            || !scenario.settings.is_empty())
    {
        return Err(execution_error(
            "process boot cannot apply a stateful scenario fixture; use an explicit stage boot",
        ));
    }
    Ok(())
}

fn objective_boot(request: &HarnessRunRequest, scenario: Option<ScenarioFixture>) -> TapeBoot {
    match &request.boot {
        ObjectiveBoot::Process => TapeBoot::Process,
        ObjectiveBoot::Stage {
            stage,
            room,
            point,
            layer,
            save_slot,
        } => TapeBoot::Stage {
            stage: stage.clone(),
            room: *room,
            point: *point,
            layer: *layer,
            save_slot: *save_slot,
            fixture: scenario,
        },
    }
}

fn same_boot_origin(left: &TapeBoot, right: &TapeBoot) -> bool {
    match (left, right) {
        (TapeBoot::Process, TapeBoot::Process) => true,
        (
            TapeBoot::Stage {
                stage: left_stage,
                room: left_room,
                point: left_point,
                layer: left_layer,
                save_slot: left_save,
                ..
            },
            TapeBoot::Stage {
                stage: right_stage,
                room: right_room,
                point: right_point,
                layer: right_layer,
                save_slot: right_save,
                ..
            },
        ) => {
            left_stage == right_stage
                && left_room == right_room
                && left_point == right_point
                && left_layer == right_layer
                && left_save == right_save
        }
        _ => false,
    }
}

fn create_artifact_root(root: &Path, relative: &str) -> Result<PathBuf, HarnessExecutionError> {
    let destination = root.join(relative);
    if destination.exists() {
        return Err(execution_error(format!(
            "artifact destination already exists: {}",
            destination.display()
        )));
    }
    let parent = destination
        .parent()
        .ok_or_else(|| execution_error("artifact destination has no parent"))?;
    fs::create_dir_all(parent).map_err(|error| {
        execution_error(format!("cannot create artifact parent directory: {error}"))
    })?;
    let canonical_parent = parent.canonicalize().map_err(|error| {
        execution_error(format!("cannot resolve artifact parent directory: {error}"))
    })?;
    if !canonical_parent.starts_with(root) {
        return Err(execution_error(
            "artifact destination escapes the repository through a symlink",
        ));
    }
    fs::create_dir(&destination)
        .map_err(|error| execution_error(format!("cannot create artifact destination: {error}")))?;
    destination
        .canonicalize()
        .map_err(|error| execution_error(format!("cannot resolve artifact destination: {error}")))
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), HarnessExecutionError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| execution_error(format!("cannot create {}: {error}", path.display())))?;
    file.write_all(bytes)
        .and_then(|()| file.flush())
        .map_err(|error| execution_error(format!("cannot write {}: {error}", path.display())))
}

fn read_native_goal(
    path: &Path,
    request: &HarnessRunRequest,
    expected_boot: &TapeBoot,
) -> Result<NativeGoal, HarnessExecutionError> {
    let native: NativeMilestoneResult =
        serde_json::from_slice(&fs::read(path).map_err(|error| {
            execution_error(format!("cannot read native objective result: {error}"))
        })?)
        .map_err(|error| {
            execution_error(format!("cannot decode native objective result: {error}"))
        })?;
    if native.schema.name != "dusklight.automation.milestones"
        || native.schema.version != 5
        || native.boot_origin_established != Some(true)
        || native.boot.as_ref() != Some(expected_boot)
        || native.goal.as_deref() != Some(request.objective.goal.as_str())
        || native.program_digest.as_deref() != Some(&request.objective.program_sha256.to_string())
        || native.milestones.len() != 1
        || native.milestones[0].id != request.objective.goal
        || native.milestones[0].hit != native.goal_reached
    {
        return Err(execution_error(
            "native objective result disagrees with the authenticated request",
        ));
    }
    let goal = &native.milestones[0];
    if goal.hit {
        let tick = goal
            .sim_tick
            .ok_or_else(|| execution_error("reached objective omitted its first-hit tick"))?;
        let evidence = goal
            .evidence
            .as_ref()
            .ok_or_else(|| execution_error("reached objective omitted boundary evidence"))?;
        if tick >= request.logical_tick_budget {
            return Err(execution_error(
                "native objective hit lies outside the logical-tick budget",
            ));
        }
        Ok(NativeGoal {
            reached: true,
            first_hit_tick: Some(tick),
            tape_frame: goal.tape_frame,
            fingerprint: Some(evidence.boundary_fingerprint.clone()),
        })
    } else if goal.sim_tick.is_none() && goal.tape_frame.is_none() && goal.evidence.is_none() {
        Ok(NativeGoal {
            reached: false,
            first_hit_tick: None,
            tape_frame: None,
            fingerprint: None,
        })
    } else {
        Err(execution_error(
            "unreached objective contains contradictory hit evidence",
        ))
    }
}

fn trim_realized_tape(
    path: &Path,
    expected_boot: &TapeBoot,
    final_frame: u64,
) -> Result<(), HarnessExecutionError> {
    let Ok(bytes) = fs::read(path) else {
        return Ok(());
    };
    let Ok(decoded) = InputTape::decode(&bytes) else {
        return Ok(());
    };
    if &decoded.tape.boot != expected_boot {
        return Ok(());
    }
    let mut tape = decoded.tape;
    let keep = usize::try_from(final_frame.saturating_add(1)).unwrap_or(usize::MAX);
    tape.frames.truncate(keep);
    fs::write(
        path,
        tape.encode().map_err(|error| {
            execution_error(format!("cannot encode consumed realized tape: {error}"))
        })?,
    )
    .map_err(|error| execution_error(format!("cannot trim realized tape: {error}")))
}

fn validate_realized_tape(
    path: &Path,
    expected_boot: &TapeBoot,
) -> Result<u64, HarnessExecutionError> {
    let tape = InputTape::decode(
        &fs::read(path)
            .map_err(|error| execution_error(format!("cannot read realized tape: {error}")))?,
    )
    .map_err(|error| execution_error(format!("cannot decode realized tape: {error}")))?
    .tape;
    if &tape.boot != expected_boot
        || tape.tick_rate_numerator != 30
        || tape.tick_rate_denominator != 1
        || tape
            .frames
            .iter()
            .any(|frame| frame.wait_condition != WaitCondition::None)
    {
        return Err(execution_error(
            "realized tape is not an absolute replay of the requested boot",
        ));
    }
    u64::try_from(tape.frames.len())
        .map_err(|_| execution_error("realized tape length does not fit the tick counter"))
}

fn validate_gameplay_trace(
    path: &Path,
    expected_boot: &TapeBoot,
) -> Result<ValidatedGameplayTrace, HarnessExecutionError> {
    let trace = crate::trace::decode(
        &fs::read(path)
            .map_err(|error| execution_error(format!("cannot read gameplay trace: {error}")))?,
    )
    .map_err(|error| execution_error(format!("cannot decode gameplay trace: {error}")))?;
    if &trace.boot != expected_boot || trace.capacity_exhausted || trace.records.is_empty() {
        return Err(execution_error(
            "gameplay trace is incomplete or names a different boot",
        ));
    }
    let ticks = u64::try_from(trace.records.len())
        .map_err(|_| execution_error("gameplay trace length does not fit the tick counter"))?;
    let mut families = Vec::new();
    for channel in crate::trace::TraceChannel::ALL {
        let Some(format) = trace.channel_formats.get(&channel) else {
            continue;
        };
        let statuses: Vec<_> = trace
            .records
            .iter()
            .map(|record| {
                record
                    .channel_status
                    .get(&channel)
                    .copied()
                    .unwrap_or(crate::trace::TraceChannelStatus::NotSampled)
            })
            .collect();
        let status = if statuses.contains(&crate::trace::TraceChannelStatus::Truncated) {
            ObservationFamilyStatus::Truncated
        } else if statuses.contains(&crate::trace::TraceChannelStatus::Unavailable) {
            ObservationFamilyStatus::Unavailable
        } else if statuses.contains(&crate::trace::TraceChannelStatus::NotSampled) {
            ObservationFamilyStatus::NotSampled
        } else if statuses.contains(&crate::trace::TraceChannelStatus::Present) {
            ObservationFamilyStatus::Present
        } else {
            ObservationFamilyStatus::Absent
        };
        families.push(ObservationFamilyAvailability {
            id: channel.name().into(),
            version: Some(format.version),
            status,
        });
    }
    families.sort_by(|left, right| left.id.cmp(&right.id));
    let inventory = ObservationInventory {
        schema: OBSERVATION_INVENTORY_SCHEMA_V1.into(),
        families,
    };
    inventory.validate().map_err(|error| {
        execution_error(format!("invalid trace observation inventory: {error}"))
    })?;
    Ok(ValidatedGameplayTrace { ticks, inventory })
}

fn classify_execution(
    execution: &NativeExecution,
    goal: Option<&NativeGoal>,
    realized_ticks: Option<u64>,
    trace_ticks: Option<u64>,
    unsupported_detail: Option<HarnessTerminalDetail>,
    controller_planned_ticks: Option<u64>,
) -> ClassifiedExecution {
    match execution {
        NativeExecution::TimedOut => ClassifiedExecution {
            terminal: HarnessTerminalReason::HostTimeout,
            message: "native process exceeded the host timeout".into(),
            proof_complete: false,
            unsupported_detail: None,
        },
        NativeExecution::LaunchFailed(message) => ClassifiedExecution {
            terminal: HarnessTerminalReason::Rejected,
            message: format!("native process could not launch: {message}"),
            proof_complete: false,
            unsupported_detail: None,
        },
        NativeExecution::WaitFailed(message) => ClassifiedExecution {
            terminal: HarnessTerminalReason::WorkerCrashed,
            message: format!("native process status could not be observed: {message}"),
            proof_complete: false,
            unsupported_detail: None,
        },
        NativeExecution::Exited(status) => {
            let complete = goal.is_some() && realized_ticks.is_some() && trace_ticks.is_some();
            match (status.code(), goal.map(|goal| goal.reached), complete) {
                (Some(VALID_GOAL_MISS_EXIT_CODE), Some(false), _)
                    if controller_planned_ticks.is_some_and(|planned| {
                        realized_ticks.is_some_and(|realized| realized < planned)
                    }) =>
                {
                    ClassifiedExecution {
                        terminal: HarnessTerminalReason::TargetLost,
                        message: "exact controller target disappeared before the planned action completed"
                            .into(),
                        proof_complete: false,
                        unsupported_detail: None,
                    }
                }
                (Some(0 | VALID_GOAL_MISS_EXIT_CODE), _, true) if unsupported_detail.is_some() => {
                    ClassifiedExecution {
                        terminal: HarnessTerminalReason::Unsupported,
                        message: "objective observations were unavailable or incomplete".into(),
                        proof_complete: false,
                        unsupported_detail,
                    }
                }
                (Some(0), Some(true), true) => ClassifiedExecution {
                    terminal: HarnessTerminalReason::Reached,
                    message: "objective reached with complete replay proof".into(),
                    proof_complete: true,
                    unsupported_detail: None,
                },
                (Some(VALID_GOAL_MISS_EXIT_CODE), Some(false), true) => ClassifiedExecution {
                    terminal: HarnessTerminalReason::Exhausted,
                    message: "input exhausted without reaching the objective".into(),
                    proof_complete: true,
                    unsupported_detail: None,
                },
                (Some(0 | VALID_GOAL_MISS_EXIT_CODE), _, _) => ClassifiedExecution {
                    terminal: HarnessTerminalReason::ProtocolFailure,
                    message: "native process omitted or contradicted required proof artifacts"
                        .into(),
                    proof_complete: false,
                    unsupported_detail: None,
                },
                (Some(CONTROLLER_PROTOCOL_FAILURE_EXIT_CODE), _, _) => ClassifiedExecution {
                    terminal: HarnessTerminalReason::ProtocolFailure,
                    message: "controller did not return one valid action for its pre-input request"
                        .into(),
                    proof_complete: false,
                    unsupported_detail: None,
                },
                (code, _, _) => ClassifiedExecution {
                    terminal: HarnessTerminalReason::GameCrashed,
                    message: format!("native process exited unexpectedly with {code:?}"),
                    proof_complete: false,
                    unsupported_detail: None,
                },
            }
        }
    }
}

fn build_result(
    request: &HarnessRunRequest,
    attempt: u32,
    paths: &ExecutionPaths,
    outcome: &ClassifiedExecution,
    elapsed_millis: u64,
    native_goal: Option<&NativeGoal>,
    realized_ticks: Option<u64>,
) -> Result<HarnessRunResult, HarnessExecutionError> {
    let realized = artifact_if_file(&paths.root, &paths.realized_input)?;
    let trace = artifact_if_file(&paths.root, &paths.gameplay_trace)?;
    let objective = artifact_if_file(&paths.root, &paths.objective_result)?;
    let stdout = artifact_if_file(&paths.root, &paths.stdout)?;
    let stderr = artifact_if_file(&paths.root, &paths.stderr)?;
    let goal = (outcome.terminal == HarnessTerminalReason::Reached)
        .then_some(native_goal)
        .flatten();
    let evidence = goal.and_then(|_| objective.clone());
    let first_hit_tick = goal.and_then(|goal| goal.first_hit_tick);
    let logical_ticks = first_hit_tick
        .map(|tick| tick.saturating_add(1))
        .or(realized_ticks)
        .unwrap_or(0)
        .min(request.logical_tick_budget);
    let mut result = HarnessRunResult {
        schema: RUN_RESULT_SCHEMA_V2.into(),
        content_sha256: Digest::ZERO,
        request_id: request.id.clone(),
        request_sha256: request.content_sha256,
        identity: request.identity.clone(),
        attempt,
        worker: HarnessWorkerIdentity {
            id: "local-native-worker".into(),
            build: request.build.clone(),
            protocol: request.protocol.clone(),
        },
        terminal: outcome.terminal,
        detail: outcome
            .unsupported_detail
            .clone()
            .unwrap_or_else(|| HarnessTerminalDetail {
                message: outcome.message.clone(),
                missing_query_facts: Vec::new(),
                missing_capabilities: Vec::new(),
                observation_issues: Vec::new(),
            }),
        objective: HarnessObjectiveResult {
            reached: goal.is_some(),
            first_hit_tick,
            evidence,
            boundary_fingerprint: goal.and_then(|goal| goal.fingerprint.clone()),
        },
        artifacts: HarnessRunArtifacts {
            realized_input: realized,
            gameplay_trace: trace,
            objective_result: objective,
            stdout,
            stderr,
            complete: outcome.proof_complete,
        },
        timing: HarnessRunTiming {
            logical_ticks,
            consumed_input_ticks: realized_ticks.unwrap_or(0).min(logical_ticks),
            host_elapsed_millis: elapsed_millis,
        },
    };
    result
        .refresh_content_sha256()
        .map_err(|error| execution_error(format!("cannot seal run result: {error}")))?;
    Ok(result)
}

fn artifact_if_file(
    root: &Path,
    path: &Path,
) -> Result<Option<ArtifactReference>, HarnessExecutionError> {
    if !path.is_file() {
        return Ok(None);
    }
    let relative = path.strip_prefix(root).map_err(|_| {
        execution_error(format!("artifact {} escapes its run root", path.display()))
    })?;
    let relative = relative
        .to_str()
        .ok_or_else(|| execution_error("artifact path is not UTF-8"))?
        .replace(std::path::MAIN_SEPARATOR, "/");
    let bytes = fs::read(path)
        .map_err(|error| execution_error(format!("cannot hash {}: {error}", path.display())))?;
    Ok(Some(ArtifactReference {
        path: relative,
        sha256: Digest(Sha256::digest(bytes).into()),
    }))
}

#[derive(Debug)]
pub struct HarnessExecutionError(String);

impl fmt::Display for HarnessExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for HarnessExecutionError {}

fn execution_error(message: impl Into<String>) -> HarnessExecutionError {
    HarnessExecutionError(message.into())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::harness::observation_contract::{
        ObservationAdmissionIssue, ObservationAdmissionIssueKind,
    };
    use std::os::unix::process::ExitStatusExt;

    fn unsupported_detail() -> HarnessTerminalDetail {
        HarnessTerminalDetail {
            message: "required observation families are unsupported".into(),
            missing_query_facts: vec!["player.exists".into()],
            missing_capabilities: Vec::new(),
            observation_issues: vec![ObservationAdmissionIssue {
                family: "player_motion".into(),
                minimum_version: 1,
                actual_version: Some(1),
                kind: ObservationAdmissionIssueKind::Truncated,
            }],
        }
    }

    #[test]
    fn complete_native_exit_cannot_hide_unsupported_observations() {
        let goal = NativeGoal {
            reached: true,
            first_hit_tick: Some(0),
            tape_frame: Some(0),
            fingerprint: None,
        };
        let outcome = classify_execution(
            &NativeExecution::Exited(ExitStatus::from_raw(0)),
            Some(&goal),
            Some(1),
            Some(1),
            Some(unsupported_detail()),
            None,
        );
        assert_eq!(outcome.terminal, HarnessTerminalReason::Unsupported);
        assert!(!outcome.proof_complete);
        assert!(outcome.unsupported_detail.is_some());
    }

    #[test]
    fn host_timeout_takes_precedence_over_observation_admission() {
        let outcome = classify_execution(
            &NativeExecution::TimedOut,
            None,
            None,
            None,
            Some(unsupported_detail()),
            None,
        );
        assert_eq!(outcome.terminal, HarnessTerminalReason::HostTimeout);
        assert!(outcome.unsupported_detail.is_none());
    }
}
