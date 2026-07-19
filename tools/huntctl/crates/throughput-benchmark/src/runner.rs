use crate::report::{
    ColdProcessBenchmarkAttempt, ColdProcessBenchmarkHost, ColdProcessBenchmarkReport,
    comparison_issue, summarize,
};
use crate::{
    COLD_PROCESS_BENCHMARK_SCHEMA, COLD_PROCESS_MODE, ColdProcessBenchmarkError, MAX_REPETITIONS,
    benchmark_error,
};
use dusklight_harness_contracts::artifact::Digest;
use dusklight_harness_contracts::run_contract::{HarnessRunRequest, HarnessRunResult};
use dusklight_harness_runtime::execution::execute_request;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
pub struct ColdProcessBenchmarkConfig<'a> {
    pub request_template: &'a HarnessRunRequest,
    pub repository_root: &'a Path,
    pub artifact_destination_root: &'a str,
    pub repetitions: u32,
}

pub fn run_cold_process_benchmark(
    config: &ColdProcessBenchmarkConfig<'_>,
) -> Result<ColdProcessBenchmarkReport, ColdProcessBenchmarkError> {
    if !(2..=MAX_REPETITIONS).contains(&config.repetitions) {
        return Err(benchmark_error(format!(
            "cold-process repetitions must be between 2 and {MAX_REPETITIONS}"
        )));
    }
    if config.artifact_destination_root.is_empty()
        || config.artifact_destination_root.ends_with('/')
    {
        return Err(benchmark_error(
            "cold-process artifact root must be a canonical relative path",
        ));
    }
    config
        .request_template
        .validate_files(config.repository_root)
        .map_err(|error| benchmark_error(format!("invalid request template: {error}")))?;

    let recorded_unix_millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| benchmark_error(format!("host clock predates Unix epoch: {error}")))?
        .as_millis();
    let host = capture_host()?;
    let benchmark_root =
        create_benchmark_root(config.repository_root, config.artifact_destination_root)?;
    let request_root = benchmark_root.join("requests");
    fs::create_dir(&request_root)
        .map_err(|error| benchmark_error(format!("cannot create request directory: {error}")))?;

    let mut attempts = Vec::with_capacity(config.repetitions as usize);
    for attempt in 1..=config.repetitions {
        let artifact_destination =
            format!("{}/attempt-{attempt:03}", config.artifact_destination_root);
        let request_relative = format!(
            "{}/requests/attempt-{attempt:03}.json",
            config.artifact_destination_root
        );
        let result_relative = format!("{artifact_destination}/result.json");
        let mut request = config.request_template.clone();
        request.artifact_destination = artifact_destination.clone();
        request
            .refresh_content_sha256()
            .map_err(|error| benchmark_error(format!("cannot seal attempt request: {error}")))?;
        request
            .validate_files(config.repository_root)
            .map_err(|error| benchmark_error(format!("invalid attempt request: {error}")))?;
        write_new_file(
            &config.repository_root.join(&request_relative),
            &request.to_pretty_json().map_err(|error| {
                benchmark_error(format!("cannot encode attempt request: {error}"))
            })?,
        )?;

        let started = Instant::now();
        let result = execute_request(&request, config.repository_root, attempt)
            .map_err(|error| benchmark_error(format!("attempt {attempt} failed: {error}")))?;
        let end_to_end_micros = started.elapsed().as_micros().max(1);
        attempts.push(attempt_record(
            attempt,
            &request_relative,
            &artifact_destination,
            &result_relative,
            &request,
            &result,
            end_to_end_micros,
        )?);
    }

    let comparison_issue = comparison_issue(&attempts);
    let mut report = ColdProcessBenchmarkReport {
        schema: COLD_PROCESS_BENCHMARK_SCHEMA.into(),
        content_sha256: Digest::ZERO,
        mode: COLD_PROCESS_MODE.into(),
        recorded_unix_millis,
        host,
        template_request_sha256: config.request_template.content_sha256,
        artifact_destination_root: config.artifact_destination_root.into(),
        repetitions: config.repetitions,
        comparable: comparison_issue.is_none(),
        comparison_issue,
        summary: summarize(&attempts)?,
        attempts,
    };
    report.refresh_content_sha256()?;
    report.validate()?;
    Ok(report)
}

fn attempt_record(
    attempt: u32,
    request_path: &str,
    artifact_destination: &str,
    result_path: &str,
    request: &HarnessRunRequest,
    result: &HarnessRunResult,
    end_to_end_micros: u128,
) -> Result<ColdProcessBenchmarkAttempt, ColdProcessBenchmarkError> {
    let native_process_micros = u128::from(result.timing.host_elapsed_millis) * 1_000;
    let native_phases = result.timing.native_phases.clone().ok_or_else(|| {
        benchmark_error(format!(
            "attempt {attempt} omitted authenticated native lifecycle timing"
        ))
    })?;
    Ok(ColdProcessBenchmarkAttempt {
        attempt,
        request: request_path.into(),
        request_sha256: request.content_sha256,
        artifact_destination: artifact_destination.into(),
        result: result_path.into(),
        result_sha256: result.content_sha256,
        terminal: result.terminal,
        objective_reached: result.objective.reached,
        first_hit_tick: result.objective.first_hit_tick,
        boundary_fingerprint: result.objective.boundary_fingerprint.clone(),
        realized_input_sha256: result
            .artifacts
            .realized_input
            .as_ref()
            .map(|artifact| artifact.sha256),
        gameplay_trace_sha256: result
            .artifacts
            .gameplay_trace
            .as_ref()
            .map(|artifact| artifact.sha256),
        objective_evidence_sha256: result
            .objective
            .evidence
            .as_ref()
            .map(|artifact| artifact.sha256),
        artifacts_complete: result.artifacts.complete,
        logical_ticks: result.timing.logical_ticks,
        consumed_input_ticks: result.timing.consumed_input_ticks,
        native_process_millis: result.timing.host_elapsed_millis,
        end_to_end_micros,
        harness_outside_process_micros: end_to_end_micros.saturating_sub(native_process_micros),
        native_phases,
    })
}

fn capture_host() -> Result<ColdProcessBenchmarkHost, ColdProcessBenchmarkError> {
    let logical_cpu_count = std::thread::available_parallelism()
        .map_err(|error| benchmark_error(format!("cannot query logical CPU count: {error}")))?
        .get();
    let operating_system_version = if cfg!(target_os = "macos") {
        command_value("sw_vers", &["-productVersion"])
    } else {
        command_value("uname", &["-sr"])
    };
    Ok(ColdProcessBenchmarkHost {
        operating_system: std::env::consts::OS.into(),
        architecture: std::env::consts::ARCH.into(),
        operating_system_version,
        hardware_model: cfg!(target_os = "macos")
            .then(|| command_value("sysctl", &["-n", "hw.model"]))
            .flatten(),
        cpu_model: cfg!(target_os = "macos")
            .then(|| command_value("sysctl", &["-n", "machdep.cpu.brand_string"]))
            .flatten(),
        logical_cpu_count,
        memory_bytes: cfg!(target_os = "macos")
            .then(|| command_value("sysctl", &["-n", "hw.memsize"]))
            .flatten()
            .and_then(|value| value.parse().ok()),
    })
}

fn command_value(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.into())
}

fn create_benchmark_root(
    repository_root: &Path,
    relative: &str,
) -> Result<PathBuf, ColdProcessBenchmarkError> {
    let repository_root = repository_root
        .canonicalize()
        .map_err(|error| benchmark_error(format!("cannot resolve repository root: {error}")))?;
    let destination = repository_root.join(relative);
    if destination.exists() {
        return Err(benchmark_error(format!(
            "cold-process artifact root already exists: {}",
            destination.display()
        )));
    }
    let parent = destination
        .parent()
        .ok_or_else(|| benchmark_error("cold-process artifact root has no parent"))?;
    fs::create_dir_all(parent).map_err(|error| {
        benchmark_error(format!("cannot create benchmark parent directory: {error}"))
    })?;
    let canonical_parent = parent.canonicalize().map_err(|error| {
        benchmark_error(format!(
            "cannot resolve benchmark parent directory: {error}"
        ))
    })?;
    if !canonical_parent.starts_with(&repository_root) {
        return Err(benchmark_error(
            "cold-process artifact root escapes the repository through a symlink",
        ));
    }
    fs::create_dir(&destination)
        .map_err(|error| benchmark_error(format!("cannot create benchmark root: {error}")))?;
    destination
        .canonicalize()
        .map_err(|error| benchmark_error(format!("cannot resolve benchmark root: {error}")))
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), ColdProcessBenchmarkError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| benchmark_error(format!("cannot create {}: {error}", path.display())))?;
    file.write_all(bytes)
        .and_then(|()| file.flush())
        .map_err(|error| benchmark_error(format!("cannot write {}: {error}", path.display())))
}
