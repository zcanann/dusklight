//! Suite-wide native/offline conformance execution.

use super::execution::execute_request;
use super::objective_suite::{
    ArtifactReference, ExpectedTerminalClass, ObjectiveCaseRole, ObjectiveSeed, ObjectiveSuite,
    ObjectiveSuiteCase,
};
use super::run_contract::{
    HarnessFidelityMode, HarnessProtocolIdentity, HarnessRunRequest, HarnessTerminalReason,
    RUN_REQUEST_SCHEMA_V2,
};
use crate::artifact::{ARTIFACT_SCHEMA_VERSION, ArtifactIdentity, BuildIdentity, Digest};
use dusklight_objectives::milestone_dsl;
use dusklight_trace::trace;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

pub const CONFORMANCE_REPORT_SCHEMA_V1: &str = "dusklight-objective-conformance-report/v1";

pub struct ConformanceConfig<'a> {
    pub repository_root: &'a Path,
    pub suite_path: &'a Path,
    pub executable: &'a Path,
    pub game_data: &'a Path,
    pub output_root: &'a Path,
    pub fidelity: HarnessFidelityMode,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConformanceReport {
    pub schema: &'static str,
    pub passed: bool,
    pub suite_id: String,
    pub suite_sha256: Digest,
    pub positive_cases: u64,
    pub negative_controls: u64,
    pub executed_attempts: usize,
    pub build: BuildIdentity,
    pub executable: ArtifactReference,
    pub game_data: ArtifactReference,
    pub output_root: PathBuf,
    pub report: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_failure: Option<String>,
    pub cases: Vec<ConformanceCaseReport>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConformanceCaseReport {
    pub id: String,
    pub role: ObjectiveCaseRole,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_for: Option<String>,
    pub expected_terminal: ExpectedTerminalClass,
    pub repetitions: u16,
    pub passed: bool,
    pub deterministic: bool,
    pub native_offline_parity: bool,
    pub attempts: Vec<ConformanceAttemptReport>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConformanceAttemptReport {
    pub attempt: u16,
    pub request: PathBuf,
    pub artifact_root: PathBuf,
    pub result: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal: Option<HarnessTerminalReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_hit_tick: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boundary_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offline_first_hit_tick: Option<u64>,
    pub native_offline_parity: bool,
    pub expected_terminal_observed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug)]
pub struct ConformanceError(String);

impl fmt::Display for ConformanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ConformanceError {}

fn conformance_error(message: impl Into<String>) -> ConformanceError {
    ConformanceError(message.into())
}

pub fn run_conformance(
    config: &ConformanceConfig<'_>,
) -> Result<ConformanceReport, ConformanceError> {
    let repository_root = config
        .repository_root
        .canonicalize()
        .map_err(|error| conformance_error(format!("cannot resolve repository root: {error}")))?;
    let suite_path = resolve_repository_file(&repository_root, config.suite_path, "suite", false)?;
    let executable_path =
        resolve_repository_file(&repository_root, config.executable, "executable", false)?;
    let game_data_path =
        resolve_repository_file(&repository_root, config.game_data, "game data", true)?;
    let output_root = resolve_new_build_root(&repository_root, config.output_root)?;

    let suite: ObjectiveSuite = read_json(&suite_path, "objective suite")?;
    let validation = suite
        .validate_files(&repository_root)
        .map_err(|error| conformance_error(format!("invalid objective suite: {error}")))?;
    if validation.positive_count != 4 {
        return Err(conformance_error(format!(
            "core conformance requires exactly four positive cases, found {}",
            validation.positive_count
        )));
    }
    let executable = artifact_reference(&repository_root, config.executable, &executable_path)?;
    let game_data = artifact_reference(&repository_root, config.game_data, &game_data_path)?;
    let build = inspect_native_build(&executable_path, &game_data_path, game_data.sha256)?;
    let protocol = conformance_protocol(&suite)?;

    fs::create_dir_all(output_root.join("requests"))
        .map_err(|error| conformance_error(format!("cannot create conformance output: {error}")))?;

    let mut cases = Vec::with_capacity(suite.cases.len());
    let mut first_failure = None;
    let mut executed_attempts = 0_usize;
    for case in &suite.cases {
        let mut attempts = Vec::with_capacity(usize::from(case.repetitions));
        for attempt in 1..=case.repetitions {
            let request_relative = PathBuf::from("build")
                .join(
                    output_root
                        .strip_prefix(repository_root.join("build"))
                        .map_err(|_| conformance_error("conformance output escaped build/"))?,
                )
                .join("requests")
                .join(&case.id)
                .join(format!("attempt-{attempt:03}.json"));
            let artifact_relative = PathBuf::from("build")
                .join(
                    output_root
                        .strip_prefix(repository_root.join("build"))
                        .map_err(|_| conformance_error("conformance output escaped build/"))?,
                )
                .join("cases")
                .join(&case.id)
                .join(format!("attempt-{attempt:03}"));
            let request_path = repository_root.join(&request_relative);
            let artifact_root = repository_root.join(&artifact_relative);
            let result_path = artifact_root.join("result.json");
            let request = build_request(
                case,
                &executable,
                &game_data,
                &build,
                &protocol,
                &artifact_relative,
                config.fidelity,
            )?;
            write_new_json(&request_path, &request)?;
            executed_attempts += 1;
            let execution = execute_request(&request, &repository_root, u32::from(attempt));
            let report = match execution {
                Ok(result) => {
                    let offline =
                        offline_first_hit(case, &result, &artifact_root, &repository_root);
                    let (offline_first_hit_tick, parity, error) = match offline {
                        Ok(tick) => (tick, tick == result.objective.first_hit_tick, None),
                        Err(error) => (
                            None,
                            false,
                            Some(format!(
                                "{}; offline validation: {error}",
                                result.detail.message
                            )),
                        ),
                    };
                    let expected_terminal_observed = terminal_matches(case, result.terminal);
                    ConformanceAttemptReport {
                        attempt,
                        request: request_relative,
                        artifact_root: artifact_relative,
                        result: repository_relative(&repository_root, &result_path, "result")?,
                        terminal: Some(result.terminal),
                        first_hit_tick: result.objective.first_hit_tick,
                        boundary_fingerprint: result
                            .objective
                            .boundary_fingerprint
                            .as_ref()
                            .map(|fingerprint| fingerprint.digest.clone()),
                        offline_first_hit_tick,
                        native_offline_parity: parity,
                        expected_terminal_observed,
                        error,
                    }
                }
                Err(error) => ConformanceAttemptReport {
                    attempt,
                    request: request_relative,
                    artifact_root: artifact_relative,
                    result: repository_relative(&repository_root, &result_path, "result")?,
                    terminal: None,
                    first_hit_tick: None,
                    boundary_fingerprint: None,
                    offline_first_hit_tick: None,
                    native_offline_parity: false,
                    expected_terminal_observed: false,
                    error: Some(error.to_string()),
                },
            };
            attempts.push(report);
        }
        let deterministic = attempts_are_deterministic(&attempts);
        let parity = attempts.iter().all(|attempt| attempt.native_offline_parity);
        let passed = deterministic
            && parity
            && attempts
                .iter()
                .all(|attempt| attempt.expected_terminal_observed && attempt.error.is_none());
        if !passed && first_failure.is_none() {
            let detail = attempts
                .iter()
                .find_map(|attempt| attempt.error.as_deref())
                .unwrap_or("terminal, determinism, or native/offline parity mismatch");
            first_failure = Some(format!("{}: {detail}", case.id));
        }
        cases.push(ConformanceCaseReport {
            id: case.id.clone(),
            role: case.role,
            control_for: case.control_for.clone(),
            expected_terminal: case.expected_terminal,
            repetitions: case.repetitions,
            passed,
            deterministic,
            native_offline_parity: parity,
            attempts,
        });
    }

    let report_relative = repository_relative(
        &repository_root,
        &output_root.join("report.json"),
        "conformance report",
    )?;
    let mut report = ConformanceReport {
        schema: CONFORMANCE_REPORT_SCHEMA_V1,
        passed: cases.iter().all(|case| case.passed),
        suite_id: suite.id,
        suite_sha256: suite.content_sha256,
        positive_cases: validation.positive_count,
        negative_controls: validation.negative_control_count,
        executed_attempts,
        build,
        executable,
        game_data,
        output_root: repository_relative(&repository_root, &output_root, "output root")?,
        report: report_relative,
        first_failure,
        cases,
    };
    if !report.passed && report.first_failure.is_none() {
        report.first_failure = Some("conformance failed without a classified case".into());
    }
    write_new_json(&output_root.join("report.json"), &report)?;
    Ok(report)
}

fn build_request(
    case: &ObjectiveSuiteCase,
    executable: &ArtifactReference,
    game_data: &ArtifactReference,
    build: &BuildIdentity,
    protocol: &HarnessProtocolIdentity,
    artifact_destination: &Path,
    fidelity: HarnessFidelityMode,
) -> Result<HarnessRunRequest, ConformanceError> {
    let settings_digest = domain_digest(
        b"dusklight.conformance-settings/v1\0",
        &serde_json::to_vec(&(
            case.logical_tick_budget,
            case.host_timeout_seconds,
            fidelity,
        ))
        .map_err(|error| conformance_error(error.to_string()))?,
    );
    let content_digest = match &case.seed {
        ObjectiveSeed::Neutral => {
            domain_digest(b"dusklight.conformance-neutral/v1\0", case.id.as_bytes())
        }
        ObjectiveSeed::Tape { artifact }
        | ObjectiveSeed::TapeSource { artifact }
        | ObjectiveSeed::Controller { artifact } => artifact.sha256,
    };
    let mut request = HarnessRunRequest {
        schema: RUN_REQUEST_SCHEMA_V2.into(),
        content_sha256: Digest::ZERO,
        id: format!("conformance-{}", case.id),
        executable: executable.clone(),
        game_data: game_data.clone(),
        build: build.clone(),
        identity: ArtifactIdentity {
            schema_version: ARTIFACT_SCHEMA_VERSION,
            content_digest,
            build: build.clone(),
            protocol_name: protocol.name.clone(),
            protocol_version: protocol.version,
            protocol_capabilities_digest: protocol.capabilities_sha256,
            scenario_id: case.id.clone(),
            region_digest: domain_digest(
                b"dusklight.conformance-region/v1\0",
                &serde_json::to_vec(&case.boot)
                    .map_err(|error| conformance_error(error.to_string()))?,
            ),
            language_assets_digest: case.objective.source.sha256,
            scenario_digest: case.scenario.sha256,
            predicate_program_digest: case.objective.program_sha256,
            action_schema_digest: case.action_schema.sha256,
            observation_schema_digest: case.observation_view.schema_sha256,
            settings_digest,
        },
        protocol: protocol.clone(),
        boot: case.boot.clone(),
        scenario: case.scenario.clone(),
        objective: case.objective.clone(),
        observation_view: case.observation_view.clone(),
        action_schema: case.action_schema.clone(),
        observation_requirements: case.observation_requirements.clone(),
        input: case.seed.clone(),
        rng_seed: 0x4455_534b_434f_4e46,
        logical_tick_budget: case.logical_tick_budget,
        host_timeout_seconds: case.host_timeout_seconds,
        fidelity,
        artifact_destination: path_string(artifact_destination, "artifact destination")?,
    };
    request
        .refresh_content_sha256()
        .map_err(|error| conformance_error(error.to_string()))?;
    Ok(request)
}

fn offline_first_hit(
    case: &ObjectiveSuiteCase,
    result: &super::run_contract::HarnessRunResult,
    artifact_root: &Path,
    repository_root: &Path,
) -> Result<Option<u64>, ConformanceError> {
    let trace_reference = result
        .artifacts
        .gameplay_trace
        .as_ref()
        .ok_or_else(|| conformance_error("run result omitted gameplay trace"))?;
    let trace_bytes = fs::read(artifact_root.join(&trace_reference.path))
        .map_err(|error| conformance_error(format!("cannot read gameplay trace: {error}")))?;
    let decoded = trace::decode(&trace_bytes)
        .map_err(|error| conformance_error(format!("cannot decode gameplay trace: {error}")))?;
    let source = fs::read_to_string(repository_root.join(&case.objective.source.path))
        .map_err(|error| conformance_error(format!("cannot read objective source: {error}")))?;
    let program = milestone_dsl::parse(&source)
        .map_err(|error| conformance_error(format!("cannot parse objective source: {error}")))?;
    let hits = milestone_dsl::evaluate_recorded_trace(&program, &decoded).map_err(|error| {
        conformance_error(format!("offline objective evaluation failed: {error}"))
    })?;
    Ok(hits
        .get(&case.objective.goal)
        .and_then(Option::as_ref)
        .map(|hit| hit.simulation_tick))
}

fn terminal_matches(case: &ObjectiveSuiteCase, terminal: HarnessTerminalReason) -> bool {
    matches!(
        (case.expected_terminal, terminal),
        (
            ExpectedTerminalClass::Reached,
            HarnessTerminalReason::Reached
        ) | (
            ExpectedTerminalClass::ObjectiveMiss,
            HarnessTerminalReason::Exhausted
        ) | (
            ExpectedTerminalClass::Unsupported,
            HarnessTerminalReason::Unsupported
        ) | (
            ExpectedTerminalClass::Impossible,
            HarnessTerminalReason::Impossible
        )
    )
}

fn attempts_are_deterministic(attempts: &[ConformanceAttemptReport]) -> bool {
    let Some(first) = attempts.first() else {
        return false;
    };
    attempts.iter().all(|attempt| {
        attempt.terminal == first.terminal
            && attempt.first_hit_tick == first.first_hit_tick
            && attempt.boundary_fingerprint == first.boundary_fingerprint
            && attempt.offline_first_hit_tick == first.offline_first_hit_tick
    })
}

fn conformance_protocol(
    suite: &ObjectiveSuite,
) -> Result<HarnessProtocolIdentity, ConformanceError> {
    let mut capabilities = BTreeSet::new();
    for case in &suite.cases {
        capabilities.extend(super::campaign::required_capabilities(case));
    }
    let mut protocol = HarnessProtocolIdentity {
        name: "dusklight-automation".into(),
        version: 2,
        capabilities_sha256: Digest::ZERO,
        capabilities: capabilities.into_iter().collect(),
    };
    protocol
        .refresh_capabilities_sha256()
        .map_err(|error| conformance_error(error.to_string()))?;
    Ok(protocol)
}

#[derive(Deserialize)]
struct NativeHello {
    protocol: NativeProtocol,
    ok: bool,
    build: NativeBuild,
}

#[derive(Deserialize)]
struct NativeProtocol {
    name: String,
    version: u16,
}

#[derive(Deserialize)]
struct NativeBuild {
    revision: String,
    aurora_revision: String,
    compiler: String,
    compiler_target: String,
    build_type: String,
    feature_digest: String,
    fidelity_profile: String,
    dirty_digest: Option<String>,
}

fn inspect_native_build(
    executable: &Path,
    game_data: &Path,
    game_digest: Digest,
) -> Result<BuildIdentity, ConformanceError> {
    let output = Command::new(executable)
        .arg("--automation-hello")
        .arg("--dvd")
        .arg(game_data)
        .output()
        .map_err(|error| conformance_error(format!("cannot inspect native build: {error}")))?;
    if !output.status.success() {
        return Err(conformance_error(format!(
            "native build inspection failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let hello: NativeHello = serde_json::from_slice(&output.stdout)
        .map_err(|error| conformance_error(format!("invalid native hello: {error}")))?;
    if !hello.ok || hello.protocol.name != "dusklight-automation" || hello.protocol.version != 2 {
        return Err(conformance_error("native hello protocol is incompatible"));
    }
    Ok(BuildIdentity {
        dusklight_commit: hello.build.revision,
        aurora_commit: hello.build.aurora_revision,
        compiler: hello.build.compiler,
        target: hello.build.compiler_target,
        profile: hello.build.build_type,
        feature_digest: hello
            .build
            .feature_digest
            .parse()
            .map_err(|error| conformance_error(format!("invalid feature digest: {error}")))?,
        game_digest,
        dirty_digest: hello
            .build
            .dirty_digest
            .map(|value| value.parse())
            .transpose()
            .map_err(|error| conformance_error(format!("invalid dirty digest: {error}")))?,
        fidelity_profile: hello.build.fidelity_profile,
    })
}

fn resolve_repository_file(
    repository_root: &Path,
    input: &Path,
    label: &str,
    allow_external_symlink: bool,
) -> Result<PathBuf, ConformanceError> {
    let joined = repository_join(repository_root, input, label)?;
    let canonical = joined
        .canonicalize()
        .map_err(|error| conformance_error(format!("cannot resolve {label}: {error}")))?;
    if !canonical.is_file() || (!allow_external_symlink && !canonical.starts_with(repository_root))
    {
        return Err(conformance_error(format!(
            "{label} must resolve to an allowed repository file"
        )));
    }
    Ok(canonical)
}

fn repository_join(
    repository_root: &Path,
    input: &Path,
    label: &str,
) -> Result<PathBuf, ConformanceError> {
    let joined = if input.is_absolute() {
        input.to_path_buf()
    } else {
        repository_root.join(input)
    };
    let lexical = if input.is_absolute() {
        input.strip_prefix(repository_root).map_err(|_| {
            conformance_error(format!("{label} must be beneath the repository root"))
        })?
    } else {
        input
    };
    if lexical.as_os_str().is_empty()
        || lexical
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(conformance_error(format!(
            "{label} must be a normalized repository path"
        )));
    }
    Ok(joined)
}

fn resolve_new_build_root(
    repository_root: &Path,
    input: &Path,
) -> Result<PathBuf, ConformanceError> {
    let output = repository_join(repository_root, input, "output root")?;
    let relative = output
        .strip_prefix(repository_root)
        .map_err(|_| conformance_error("output root escaped the repository"))?;
    if relative.components().next() != Some(Component::Normal("build".as_ref())) {
        return Err(conformance_error(
            "conformance output must be beneath build/",
        ));
    }
    if output.exists() {
        return Err(conformance_error(format!(
            "conformance output already exists: {}",
            output.display()
        )));
    }
    Ok(output)
}

fn artifact_reference(
    repository_root: &Path,
    authored_path: &Path,
    resolved_path: &Path,
) -> Result<ArtifactReference, ConformanceError> {
    let joined = repository_join(repository_root, authored_path, "artifact")?;
    let relative = joined
        .strip_prefix(repository_root)
        .map_err(|_| conformance_error("artifact escaped repository"))?;
    Ok(ArtifactReference {
        path: path_string(relative, "artifact")?,
        sha256: super::run_contract::sha256_artifact_file(resolved_path)
            .map_err(|error| conformance_error(error.to_string()))?,
    })
}

fn repository_relative(
    repository_root: &Path,
    path: &Path,
    label: &str,
) -> Result<PathBuf, ConformanceError> {
    path.strip_prefix(repository_root)
        .map(Path::to_path_buf)
        .map_err(|_| conformance_error(format!("{label} escaped repository")))
}

fn path_string(path: &Path, label: &str) -> Result<String, ConformanceError> {
    path.to_str()
        .map(|value| value.replace(std::path::MAIN_SEPARATOR, "/"))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| conformance_error(format!("{label} is not valid UTF-8")))
}

fn domain_digest(domain: &[u8], payload: &[u8]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(payload);
    Digest(hasher.finalize().into())
}

fn read_json<T: for<'de> Deserialize<'de>>(
    path: &Path,
    label: &str,
) -> Result<T, ConformanceError> {
    serde_json::from_slice(
        &fs::read(path)
            .map_err(|error| conformance_error(format!("cannot read {label}: {error}")))?,
    )
    .map_err(|error| conformance_error(format!("cannot decode {label}: {error}")))
}

fn write_new_json(path: &Path, value: &impl Serialize) -> Result<(), ConformanceError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            conformance_error(format!("cannot create {}: {error}", parent.display()))
        })?;
    }
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| conformance_error(format!("cannot encode JSON: {error}")))?;
    let mut bytes_with_newline = bytes;
    bytes_with_newline.push(b'\n');
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| conformance_error(format!("cannot create {}: {error}", path.display())))?;
    use std::io::Write as _;
    file.write_all(&bytes_with_newline)
        .map_err(|error| conformance_error(format!("cannot write {}: {error}", path.display())))?;
    file.flush()
        .map_err(|error| conformance_error(format!("cannot flush {}: {error}", path.display())))
}
