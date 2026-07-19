//! Native build inspection and authenticated request materialization.

use super::native_evidence::HarnessNativeEvidenceRequest;
use super::objective_suite::{ArtifactReference, ObjectiveBoot, ObjectiveSeed, ObjectiveSuiteCase};
use super::run_contract::{
    HarnessFidelityMode, HarnessProtocolIdentity, HarnessRunRequest, RUN_REQUEST_SCHEMA_V2,
    sha256_artifact_file,
};
use crate::artifact::{ARTIFACT_SCHEMA_VERSION, ArtifactIdentity, BuildIdentity, Digest};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug)]
pub struct NativeBuildInputs {
    pub executable: ArtifactReference,
    pub game_data: ArtifactReference,
    pub build: BuildIdentity,
}

pub struct NativeRequestConfig<'a> {
    pub case: &'a ObjectiveSuiteCase,
    pub inputs: &'a NativeBuildInputs,
    pub protocol: &'a HarnessProtocolIdentity,
    pub request_id: &'a str,
    pub artifact_destination: &'a Path,
    pub fidelity: HarnessFidelityMode,
    pub native_evidence: Option<HarnessNativeEvidenceRequest>,
    pub rng_seed: u64,
}

pub fn inspect_native_inputs(
    repository_root: &Path,
    executable: &Path,
    game_data: &Path,
) -> Result<NativeBuildInputs, NativeRequestError> {
    let repository_root = repository_root
        .canonicalize()
        .map_err(|error| request_error(format!("cannot resolve repository root: {error}")))?;
    let executable_path =
        resolve_repository_file(&repository_root, executable, "executable", false)?;
    let game_data_path = resolve_repository_file(&repository_root, game_data, "game data", true)?;
    let executable = artifact_reference(&repository_root, executable, &executable_path)?;
    let game_data = artifact_reference(&repository_root, game_data, &game_data_path)?;
    let build = inspect_native_build(&executable_path, &game_data_path, game_data.sha256)?;
    Ok(NativeBuildInputs {
        executable,
        game_data,
        build,
    })
}

pub fn protocol_for_cases(
    cases: &[ObjectiveSuiteCase],
) -> Result<HarnessProtocolIdentity, NativeRequestError> {
    let mut capabilities = BTreeSet::new();
    for case in cases {
        capabilities.extend(required_capabilities(case));
    }
    let mut protocol = HarnessProtocolIdentity {
        name: "dusklight-automation".into(),
        version: 2,
        capabilities_sha256: Digest::ZERO,
        capabilities: capabilities.into_iter().collect(),
    };
    protocol
        .refresh_capabilities_sha256()
        .map_err(|error| request_error(error.to_string()))?;
    Ok(protocol)
}

pub fn materialize_native_request(
    config: &NativeRequestConfig<'_>,
) -> Result<HarnessRunRequest, NativeRequestError> {
    let case = config.case;
    let settings_digest = domain_digest(
        b"dusklight.native-request-settings/v1\0",
        &serde_json::to_vec(&(
            case.logical_tick_budget,
            case.host_timeout_seconds,
            config.fidelity,
            config.native_evidence,
        ))
        .map_err(|error| request_error(error.to_string()))?,
    );
    let content_digest = match &case.seed {
        ObjectiveSeed::Neutral => {
            domain_digest(b"dusklight.native-request-neutral/v1\0", case.id.as_bytes())
        }
        ObjectiveSeed::Tape { artifact }
        | ObjectiveSeed::TapeSource { artifact }
        | ObjectiveSeed::Controller { artifact } => artifact.sha256,
    };
    let mut request = HarnessRunRequest {
        schema: RUN_REQUEST_SCHEMA_V2.into(),
        content_sha256: Digest::ZERO,
        id: config.request_id.into(),
        executable: config.inputs.executable.clone(),
        game_data: config.inputs.game_data.clone(),
        build: config.inputs.build.clone(),
        identity: ArtifactIdentity {
            schema_version: ARTIFACT_SCHEMA_VERSION,
            content_digest,
            build: config.inputs.build.clone(),
            protocol_name: config.protocol.name.clone(),
            protocol_version: config.protocol.version,
            protocol_capabilities_digest: config.protocol.capabilities_sha256,
            scenario_id: case.id.clone(),
            region_digest: domain_digest(
                b"dusklight.native-request-region/v1\0",
                &serde_json::to_vec(&case.boot)
                    .map_err(|error| request_error(error.to_string()))?,
            ),
            language_assets_digest: case.objective.source.sha256,
            scenario_digest: case.scenario.sha256,
            predicate_program_digest: case.objective.program_sha256,
            action_schema_digest: case.action_schema.sha256,
            observation_schema_digest: case.observation_view.schema_sha256,
            settings_digest,
        },
        protocol: config.protocol.clone(),
        boot: case.boot.clone(),
        scenario: case.scenario.clone(),
        objective: case.objective.clone(),
        observation_view: case.observation_view.clone(),
        action_schema: case.action_schema.clone(),
        observation_requirements: case.observation_requirements.clone(),
        input: case.seed.clone(),
        native_evidence: config.native_evidence,
        rng_seed: config.rng_seed,
        logical_tick_budget: case.logical_tick_budget,
        host_timeout_seconds: case.host_timeout_seconds,
        fidelity: config.fidelity,
        artifact_destination: path_string(config.artifact_destination, "artifact destination")?,
    };
    request
        .refresh_content_sha256()
        .map_err(|error| request_error(error.to_string()))?;
    Ok(request)
}

fn required_capabilities(case: &ObjectiveSuiteCase) -> Vec<String> {
    let mut capabilities = BTreeSet::from([
        "gameplay-trace-v5".to_string(),
        "milestone-program-v1.5".to_string(),
        "scenario-fixture-v1".to_string(),
        "typed-fact-response-v1".to_string(),
    ]);
    capabilities.insert(match case.boot {
        ObjectiveBoot::Process => "process-boot".into(),
        ObjectiveBoot::Stage { .. } => "stage-boot".into(),
    });
    capabilities.insert(match case.seed {
        ObjectiveSeed::Neutral | ObjectiveSeed::Tape { .. } | ObjectiveSeed::TapeSource { .. } => {
            "input-tape-v3".into()
        }
        ObjectiveSeed::Controller { .. } => "input-controller-v1.4".into(),
    });
    for family in &case.observation_requirements.families {
        capabilities.insert(format!(
            "observation-family:{}/v{}",
            family.id, family.minimum_version
        ));
    }
    capabilities.into_iter().collect()
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
) -> Result<BuildIdentity, NativeRequestError> {
    let output = Command::new(executable)
        .arg("--automation-hello")
        .arg("--dvd")
        .arg(game_data)
        .output()
        .map_err(|error| request_error(format!("cannot inspect native build: {error}")))?;
    if !output.status.success() {
        return Err(request_error(format!(
            "native build inspection failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let hello: NativeHello = serde_json::from_slice(&output.stdout)
        .map_err(|error| request_error(format!("invalid native hello: {error}")))?;
    if !hello.ok || hello.protocol.name != "dusklight-automation" || hello.protocol.version != 2 {
        return Err(request_error("native hello protocol is incompatible"));
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
            .map_err(|error| request_error(format!("invalid feature digest: {error}")))?,
        game_digest,
        dirty_digest: hello
            .build
            .dirty_digest
            .map(|value| value.parse())
            .transpose()
            .map_err(|error| request_error(format!("invalid dirty digest: {error}")))?,
        fidelity_profile: hello.build.fidelity_profile,
    })
}

fn resolve_repository_file(
    repository_root: &Path,
    input: &Path,
    label: &str,
    allow_external_symlink: bool,
) -> Result<PathBuf, NativeRequestError> {
    let joined = repository_join(repository_root, input, label)?;
    let canonical = joined
        .canonicalize()
        .map_err(|error| request_error(format!("cannot resolve {label}: {error}")))?;
    if !canonical.is_file() || (!allow_external_symlink && !canonical.starts_with(repository_root))
    {
        return Err(request_error(format!(
            "{label} must resolve to an allowed repository file"
        )));
    }
    Ok(canonical)
}

fn repository_join(
    repository_root: &Path,
    input: &Path,
    label: &str,
) -> Result<PathBuf, NativeRequestError> {
    let joined = if input.is_absolute() {
        input.to_path_buf()
    } else {
        repository_root.join(input)
    };
    let lexical = if input.is_absolute() {
        input
            .strip_prefix(repository_root)
            .map_err(|_| request_error(format!("{label} must be beneath the repository root")))?
    } else {
        input
    };
    if lexical.as_os_str().is_empty()
        || lexical
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(request_error(format!(
            "{label} must be a normalized repository path"
        )));
    }
    Ok(joined)
}

fn artifact_reference(
    repository_root: &Path,
    authored_path: &Path,
    resolved_path: &Path,
) -> Result<ArtifactReference, NativeRequestError> {
    let joined = repository_join(repository_root, authored_path, "artifact")?;
    let relative = joined
        .strip_prefix(repository_root)
        .map_err(|_| request_error("artifact escaped repository"))?;
    Ok(ArtifactReference {
        path: path_string(relative, "artifact")?,
        sha256: sha256_artifact_file(resolved_path)
            .map_err(|error| request_error(error.to_string()))?,
    })
}

fn path_string(path: &Path, label: &str) -> Result<String, NativeRequestError> {
    path.to_str()
        .map(|value| value.replace(std::path::MAIN_SEPARATOR, "/"))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| request_error(format!("{label} is not valid UTF-8")))
}

fn domain_digest(domain: &[u8], payload: &[u8]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(payload);
    Digest(hasher.finalize().into())
}

#[derive(Debug)]
pub struct NativeRequestError(String);

impl fmt::Display for NativeRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeRequestError {}

fn request_error(message: impl Into<String>) -> NativeRequestError {
    NativeRequestError(message.into())
}
