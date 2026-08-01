//! Managed persistent native suffix-worker sessions.

use crate::native_suffix_result::{
    NativeSuffixBatchResult, NativeTerminalBinding, ValidatedNativeSuffixBatch,
};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::native_fidelity::FIXED_AUTOMATION_CVARS;
use dusklight_automation_contracts::tape::{InputTape, TapeBoot};
use dusklight_evidence::native_episode_shard::NativeEpisodeShard;
use dusklight_learning::frozen_inference::FrozenInferenceModel;
use dusklight_learning::native_frozen_policy_reinference::{
    NativeFrozenPolicyReinferenceReport, verify_native_frozen_policy_reinference,
};
use dusklight_learning::native_frozen_policy_suffix_batch::NativeFrozenPolicySuffixBatch;
use dusklight_search::suffix_batch::{
    NATIVE_CACHED_SUFFIX_BATCH_SCHEMA, NATIVE_REACTIVE_SUFFIX_BATCH_SCHEMA,
    NATIVE_SUFFIX_BATCH_SCHEMA, NativeSuffixBatch,
};
use dusklight_worker_protocol::client::{BatchComplete, ClientError, HelloResponse, WorkerClient};
use dusklight_worker_protocol::transport::ProcessTransport;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

const MAXIMUM_PERSISTENT_BATCH_TICKS: usize = 4_096;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeHeadlessAuditComparators {
    pub gpu_frame_submission: bool,
    pub cpu_renderer_submission: bool,
    pub presentation_lifecycle: bool,
    pub imgui_frame_lifecycle: bool,
    pub host_pacing: bool,
    pub host_audio_device: bool,
    #[serde(default)]
    pub suppress_cpu_draw_traversal: bool,
    #[serde(default)]
    pub suppress_deterministic_audio_emulation: bool,
    #[serde(default)]
    pub suppress_game_audio_update: bool,
}

impl NativeHeadlessAuditComparators {
    /// Headless farming defaults that have passed native subsystem parity.
    /// Audit runs pass explicit comparator sets and retain unsuppressed
    /// treatments for comparison.
    pub fn production() -> Self {
        Self {
            suppress_deterministic_audio_emulation: true,
            suppress_game_audio_update: true,
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug)]
pub struct NativeSuffixWorkerLaunch {
    pub executable: PathBuf,
    pub game_data: PathBuf,
    pub input_tape: PathBuf,
    pub milestone_program: PathBuf,
    pub card_fixture: PathBuf,
    pub card_fixture_sha256: Digest,
    pub working_directory: PathBuf,
    pub state_root: PathBuf,
    pub world_context_sha256: Digest,
    pub terminal: NativeTerminalBinding,
    pub initial_batch: PathBuf,
    pub initial_result: PathBuf,
    pub initial_winner_tape: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct NativeFrozenPolicyWorkerLaunch {
    pub executable: PathBuf,
    pub game_data: PathBuf,
    pub input_tape: PathBuf,
    pub milestone_program: PathBuf,
    pub card_fixture: PathBuf,
    pub card_fixture_sha256: Digest,
    pub working_directory: PathBuf,
    pub state_root: PathBuf,
    pub world_context_sha256: Digest,
    pub terminal: NativeTerminalBinding,
    pub initial_batch: PathBuf,
    pub initial_result: PathBuf,
}

/// Content identities already authenticated by the enclosing sealed execution
/// binding. Passing these avoids re-reading immutable executable and game-data
/// files once per concurrently launched worker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeSuffixPrevalidatedFileIdentities {
    pub executable_sha256: Digest,
    pub game_data_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSuffixWorkerIdentity {
    pub executable_sha256: Digest,
    pub game_data_sha256: Digest,
    pub input_tape_sha256: Digest,
    pub milestone_program_sha256: Digest,
    pub card_fixture_sha256: Digest,
    pub world_context_sha256: Digest,
    pub source_frame: u64,
    pub source_boundary_fingerprint: String,
    pub checkpoint_validation_kind: String,
    pub checkpoint_validation_ticks: u64,
    pub maximum_ticks: u64,
    pub terminal: NativeTerminalBinding,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSuffixWorkerLaunchTiming {
    pub spawn_call_micros: u64,
    pub handshake_micros: u64,
    pub initial_batch_wait_micros: u64,
    pub artifact_validation_micros: u64,
    pub total_micros: u64,
}

pub struct NativeSuffixWorkerSession {
    client: WorkerClient<ProcessTransport>,
    hello: HelloResponse,
    identity: NativeSuffixWorkerIdentity,
    terminal: NativeTerminalBinding,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedNativeFrozenPolicyBatch {
    pub execution: ValidatedNativeSuffixBatch,
    pub reinference: NativeFrozenPolicyReinferenceReport,
}

struct PreparedLaunch {
    executable: PathBuf,
    working_directory: PathBuf,
    args: Vec<String>,
    batch: NativeSuffixBatch,
    result: PathBuf,
    identity: NativeSuffixWorkerIdentity,
    terminal: NativeTerminalBinding,
}

struct PreparedFrozenLaunch {
    executable: PathBuf,
    working_directory: PathBuf,
    args: Vec<String>,
    batch: NativeFrozenPolicySuffixBatch,
    model_bytes: Vec<u8>,
    result: PathBuf,
    identity: NativeSuffixWorkerIdentity,
    terminal: NativeTerminalBinding,
}

impl NativeSuffixWorkerSession {
    pub fn launch(
        config: &NativeSuffixWorkerLaunch,
    ) -> Result<(Self, ValidatedNativeSuffixBatch), NativeSuffixWorkerError> {
        Self::launch_prevalidated(config, None, false)
            .map(|(session, validated, _)| (session, validated))
    }

    pub fn launch_with_prevalidated_files(
        config: &NativeSuffixWorkerLaunch,
        identities: NativeSuffixPrevalidatedFileIdentities,
    ) -> Result<(Self, ValidatedNativeSuffixBatch), NativeSuffixWorkerError> {
        Self::launch_prevalidated(config, Some(identities), false)
            .map(|(session, validated, _)| (session, validated))
    }

    pub fn launch_compact_with_prevalidated_files(
        config: &NativeSuffixWorkerLaunch,
        identities: NativeSuffixPrevalidatedFileIdentities,
    ) -> Result<(Self, ValidatedNativeSuffixBatch), NativeSuffixWorkerError> {
        Self::launch_prevalidated(config, Some(identities), true)
            .map(|(session, validated, _)| (session, validated))
    }

    pub fn launch_profiled_with_prevalidated_files(
        config: &NativeSuffixWorkerLaunch,
        identities: NativeSuffixPrevalidatedFileIdentities,
    ) -> Result<
        (
            Self,
            ValidatedNativeSuffixBatch,
            NativeSuffixWorkerLaunchTiming,
        ),
        NativeSuffixWorkerError,
    > {
        Self::launch_prevalidated(config, Some(identities), false)
    }

    fn launch_prevalidated(
        config: &NativeSuffixWorkerLaunch,
        identities: Option<NativeSuffixPrevalidatedFileIdentities>,
        require_compact_batch_run: bool,
    ) -> Result<
        (
            Self,
            ValidatedNativeSuffixBatch,
            NativeSuffixWorkerLaunchTiming,
        ),
        NativeSuffixWorkerError,
    > {
        Self::launch_prevalidated_with_comparators(
            config,
            identities,
            NativeHeadlessAuditComparators::production(),
            require_compact_batch_run,
        )
    }

    pub fn launch_profiled_with_audit_comparators(
        config: &NativeSuffixWorkerLaunch,
        identities: NativeSuffixPrevalidatedFileIdentities,
        comparators: NativeHeadlessAuditComparators,
    ) -> Result<
        (
            Self,
            ValidatedNativeSuffixBatch,
            NativeSuffixWorkerLaunchTiming,
        ),
        NativeSuffixWorkerError,
    > {
        Self::launch_prevalidated_with_comparators(config, Some(identities), comparators, false)
    }

    fn launch_prevalidated_with_comparators(
        config: &NativeSuffixWorkerLaunch,
        identities: Option<NativeSuffixPrevalidatedFileIdentities>,
        comparators: NativeHeadlessAuditComparators,
        require_compact_batch_run: bool,
    ) -> Result<
        (
            Self,
            ValidatedNativeSuffixBatch,
            NativeSuffixWorkerLaunchTiming,
        ),
        NativeSuffixWorkerError,
    > {
        let total_started = Instant::now();
        let prepared = prepare_launch(config, identities, comparators)?;
        let spawn_started = Instant::now();
        let transport = ProcessTransport::spawn_in(
            &prepared.executable,
            &prepared.args,
            Some(&prepared.working_directory),
        )
        .map_err(|source| {
            worker_message(format!(
                "cannot launch native suffix worker {}: {source}",
                prepared.executable.display()
            ))
        })?;
        let spawn_call_micros = elapsed_micros(spawn_started);
        let mut client = WorkerClient::new(transport);
        let handshake_started = Instant::now();
        let hello = client.handshake().map_err(worker_error)?.clone();
        let handshake_micros = elapsed_micros(handshake_started);
        if !hello.capabilities.persistent_control || !hello.capabilities.batch_run {
            return Err(worker_message(
                "native child does not advertise persistent suffix-batch capability",
            ));
        }
        if require_compact_batch_run && !hello.capabilities.compact_batch_run {
            return Err(worker_message(
                "native child does not advertise compact persistent suffix-batch capability",
            ));
        }
        let initial_batch_started = Instant::now();
        let complete = client.await_initial_batch().map_err(worker_error)?;
        let initial_batch_wait_micros = elapsed_micros(initial_batch_started);
        let validation_started = Instant::now();
        let validated = validate_completed_batch(
            &complete,
            &prepared.result,
            &prepared.batch,
            &prepared.terminal,
        )?;
        let artifact_validation_micros = elapsed_micros(validation_started);
        let session = Self {
            client,
            hello,
            identity: prepared.identity,
            terminal: prepared.terminal,
        };
        Ok((
            session,
            validated,
            NativeSuffixWorkerLaunchTiming {
                spawn_call_micros,
                handshake_micros,
                initial_batch_wait_micros,
                artifact_validation_micros,
                total_micros: elapsed_micros(total_started),
            },
        ))
    }

    pub fn launch_frozen(
        config: &NativeFrozenPolicyWorkerLaunch,
    ) -> Result<(Self, ValidatedNativeFrozenPolicyBatch), NativeSuffixWorkerError> {
        let prepared = prepare_frozen_launch(config)?;
        let transport = ProcessTransport::spawn_in(
            &prepared.executable,
            &prepared.args,
            Some(&prepared.working_directory),
        )
        .map_err(worker_error)?;
        let mut client = WorkerClient::new(transport);
        let hello = client.handshake().map_err(worker_error)?.clone();
        if !hello.capabilities.persistent_control || !hello.capabilities.batch_run {
            return Err(worker_message(
                "native child does not advertise persistent frozen-policy batch capability",
            ));
        }
        let complete = client.await_initial_batch().map_err(worker_error)?;
        let validated = validate_completed_frozen_batch(
            &complete,
            &prepared.result,
            &prepared.batch,
            &prepared.model_bytes,
            &prepared.terminal,
        )?;
        let session = Self {
            client,
            hello,
            identity: prepared.identity,
            terminal: prepared.terminal,
        };
        Ok((session, validated))
    }

    pub fn hello(&self) -> &HelloResponse {
        &self.hello
    }

    pub fn identity(&self) -> &NativeSuffixWorkerIdentity {
        &self.identity
    }

    pub fn run_batch(
        &mut self,
        batch_path: &Path,
        result_path: &Path,
        winner_tape_path: Option<&Path>,
    ) -> Result<ValidatedNativeSuffixBatch, NativeSuffixWorkerError> {
        let batch_path = canonical_file(batch_path, "suffix batch")?;
        let batch: NativeSuffixBatch =
            serde_json::from_slice(&fs::read(&batch_path).map_err(worker_error)?)
                .map_err(worker_error)?;
        self.run_prepared_batch(&batch_path, result_path, winner_tape_path, &batch)
    }

    /// Runs a transport-encoded batch while retaining the full request as the
    /// Rust-side validation authority. This avoids decoding the compact tactic
    /// envelope back into a second object graph.
    pub fn run_prepared_batch(
        &mut self,
        batch_path: &Path,
        result_path: &Path,
        winner_tape_path: Option<&Path>,
        batch: &NativeSuffixBatch,
    ) -> Result<ValidatedNativeSuffixBatch, NativeSuffixWorkerError> {
        let batch_path = canonical_file(batch_path, "suffix batch")?;
        validate_batch_identity(&batch, &self.identity)?;
        let result_path = prepare_new_result_output(result_path, "suffix result")?;
        let winner_tape_path = winner_tape_path
            .map(|path| prepare_new_output(path, "suffix winner tape"))
            .transpose()?;
        let complete = self
            .client
            .run_batch(
                path_text(&batch_path, "suffix batch")?,
                path_text(&result_path, "suffix result")?,
                winner_tape_path
                    .as_deref()
                    .map(|path| path_text(path, "suffix winner tape"))
                    .transpose()?,
            )
            .map_err(worker_client_error)?;
        validate_completed_batch(&complete, &result_path, &batch, &self.terminal)
    }

    pub fn run_frozen_batch(
        &mut self,
        batch_path: &Path,
        result_path: &Path,
    ) -> Result<ValidatedNativeFrozenPolicyBatch, NativeSuffixWorkerError> {
        let batch_path = canonical_file(batch_path, "frozen policy suffix batch")?;
        let batch: NativeFrozenPolicySuffixBatch =
            serde_json::from_slice(&fs::read(&batch_path).map_err(worker_error)?)
                .map_err(worker_error)?;
        let model_path = canonical_frozen_model(&batch)?;
        let model_bytes = fs::read(&model_path).map_err(worker_error)?;
        validate_frozen_batch_identity(&batch, &model_bytes, &self.identity, &self.terminal)?;
        let result_path = prepare_new_result_output(result_path, "frozen policy suffix result")?;
        let complete = self
            .client
            .run_batch(
                path_text(&batch_path, "frozen policy suffix batch")?,
                path_text(&result_path, "frozen policy suffix result")?,
                None,
            )
            .map_err(worker_client_error)?;
        validate_completed_frozen_batch(
            &complete,
            &result_path,
            &batch,
            &model_bytes,
            &self.terminal,
        )
    }

    pub(crate) fn suspend_process(&mut self) -> Result<(), NativeSuffixWorkerError> {
        self.client.suspend_process().map_err(worker_error)
    }

    pub(crate) fn resume_process(&mut self) -> Result<(), NativeSuffixWorkerError> {
        self.client.resume_process().map_err(worker_error)
    }

    pub(crate) fn process_cpu_micros(&self) -> Result<Option<u64>, NativeSuffixWorkerError> {
        self.client.process_cpu_micros().map_err(worker_error)
    }

    pub fn shutdown(mut self) -> Result<(), NativeSuffixWorkerError> {
        self.client.shutdown().map_err(worker_error)
    }
}

mod launch_preparation;
use launch_preparation::{
    prepare_frozen_launch, prepare_launch, validate_completed_batch,
    validate_completed_frozen_batch,
};

/// Revalidates a completed native batch from sealed request/result artifacts.
/// This lets a resumed campaign adopt a result written before the journal
/// boundary without rerunning those candidates.
pub fn validate_native_suffix_artifacts(
    batch: &NativeSuffixBatch,
    result_path: &Path,
    terminal: &NativeTerminalBinding,
) -> Result<ValidatedNativeSuffixBatch, NativeSuffixWorkerError> {
    validate_batch_shape(batch)?;
    let result_path = canonical_file(result_path, "native suffix result")?;
    let result: NativeSuffixBatchResult =
        serde_json::from_slice(&fs::read(&result_path).map_err(worker_error)?)
            .map_err(worker_error)?;
    let validated = result
        .validate_against(batch, terminal)
        .map_err(worker_error)?;
    let episode_path = canonical_file(
        Path::new(&validated.episode_shard_path),
        "native episode shard",
    )?;
    if Path::new(&validated.episode_shard_path) != episode_path {
        return Err(worker_message(
            "native suffix result episode shard path is not canonical",
        ));
    }
    Ok(validated)
}

pub fn validate_native_frozen_policy_artifacts(
    batch: &NativeFrozenPolicySuffixBatch,
    model_bytes: &[u8],
    result_path: &Path,
    terminal: &NativeTerminalBinding,
) -> Result<ValidatedNativeFrozenPolicyBatch, NativeSuffixWorkerError> {
    batch.validate(model_bytes).map_err(worker_error)?;
    let result_path = canonical_file(result_path, "native frozen policy result")?;
    let result: NativeSuffixBatchResult =
        serde_json::from_slice(&fs::read(&result_path).map_err(worker_error)?)
            .map_err(worker_error)?;
    let execution = result
        .validate_frozen_against(batch, model_bytes, terminal)
        .map_err(worker_error)?;
    let episode_path = canonical_file(
        Path::new(&execution.episode_shard_path),
        "native frozen policy episode shard",
    )?;
    if Path::new(&execution.episode_shard_path) != episode_path {
        return Err(worker_message(
            "native frozen policy result episode shard path is not canonical",
        ));
    }
    let shard = NativeEpisodeShard::read(&episode_path).map_err(worker_error)?;
    if shard.source_frame != batch.source_frame as u64
        || shard.maximum_ticks != batch.maximum_ticks as u32
        || shard.episodes.len() != batch.candidates.len()
        || shard.metadata.checkpoint_identity != execution.restore_identity
        || shard.metadata.source_boundary_fingerprint != batch.source_boundary_fingerprint
        || shard.metadata.objective != terminal.goal
    {
        return Err(worker_message(
            "native frozen policy episode shard differs from its request and result",
        ));
    }
    shard
        .verify_authored_objective(
            &terminal.program_sha256.to_string(),
            &terminal.definition_sha256.to_string(),
        )
        .map_err(worker_error)?;
    let reinference = verify_native_frozen_policy_reinference(
        model_bytes,
        batch.frozen_policy.rollout_exploration.as_ref(),
        &shard,
        terminal.definition_sha256,
        &execution.restore_identity,
        &batch.source_boundary_fingerprint,
    )
    .map_err(worker_error)?;
    if reinference.transition_count != execution.simulated_ticks as usize
        || reinference.episode_count != batch.candidates.len()
    {
        return Err(worker_message(
            "native frozen policy reinference accounting differs from the batch result",
        ));
    }
    Ok(ValidatedNativeFrozenPolicyBatch {
        execution,
        reinference,
    })
}

fn validate_batch_identity(
    batch: &NativeSuffixBatch,
    identity: &NativeSuffixWorkerIdentity,
) -> Result<(), NativeSuffixWorkerError> {
    validate_batch_shape(batch)?;
    let direct_cached_source = batch
        .checkpoint_cache
        .as_ref()
        .is_some_and(|cache| cache.source_identity.is_some());
    if batch.source_frame as u64 != identity.source_frame
        || (!direct_cached_source
            && batch.source_boundary_fingerprint != identity.source_boundary_fingerprint)
        || batch.checkpoint_validation.kind != identity.checkpoint_validation_kind
        || batch.checkpoint_validation.ticks as u64 != identity.checkpoint_validation_ticks
    {
        return Err(worker_message(
            "next suffix batch differs from the authenticated session source",
        ));
    }
    Ok(())
}

fn validate_frozen_batch_identity(
    batch: &NativeFrozenPolicySuffixBatch,
    model_bytes: &[u8],
    identity: &NativeSuffixWorkerIdentity,
    terminal: &NativeTerminalBinding,
) -> Result<(), NativeSuffixWorkerError> {
    batch.validate(model_bytes).map_err(worker_error)?;
    let model = FrozenInferenceModel::from_bytes(model_bytes).map_err(worker_error)?;
    if model.objective_sha256 != terminal.definition_sha256
        || batch.source_frame as u64 != identity.source_frame
        || batch.source_boundary_fingerprint != identity.source_boundary_fingerprint
        || batch.checkpoint_validation.kind != identity.checkpoint_validation_kind
        || batch.checkpoint_validation.ticks as u64 != identity.checkpoint_validation_ticks
        || batch.maximum_ticks as u64 != identity.maximum_ticks
        || terminal != &identity.terminal
    {
        return Err(worker_message(
            "next frozen policy batch differs from the authenticated session source or terminal",
        ));
    }
    Ok(())
}

fn canonical_frozen_model(
    batch: &NativeFrozenPolicySuffixBatch,
) -> Result<PathBuf, NativeSuffixWorkerError> {
    let declared = Path::new(&batch.frozen_policy.model_path);
    let canonical = canonical_file(declared, "frozen policy model")?;
    if declared != canonical {
        return Err(worker_message(
            "frozen policy model path must be canonical and absolute",
        ));
    }
    Ok(canonical)
}

fn validate_batch_shape(batch: &NativeSuffixBatch) -> Result<(), NativeSuffixWorkerError> {
    if !matches!(
        batch.schema.as_str(),
        NATIVE_SUFFIX_BATCH_SCHEMA
            | NATIVE_REACTIVE_SUFFIX_BATCH_SCHEMA
            | NATIVE_CACHED_SUFFIX_BATCH_SCHEMA
    ) || batch.candidates.is_empty()
        || batch.source_boundary_fingerprint.len() != 32
        || !batch
            .source_boundary_fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || batch.maximum_ticks == 0
        || batch.maximum_ticks > MAXIMUM_PERSISTENT_BATCH_TICKS
        || batch.checkpoint_validation.kind != "recorded_replay_window"
        || batch.checkpoint_validation.ticks == 0
        || (batch.schema == NATIVE_CACHED_SUFFIX_BATCH_SCHEMA
            && batch.checkpoint_cache.as_ref().is_none_or(|cache| {
                cache.capacity_bytes == 0
                    || cache.capacity_bytes > 1024 * 1024 * 1024
                    || cache.capacity_entries == 0
                    || cache.capacity_entries > 16
                    || cache.source_identity.is_some() != (cache.source_route_ticks != 0)
                    || cache.source_identity.as_ref().is_some_and(|identity| {
                        identity.len() != 32
                            || !identity
                                .bytes()
                                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                    })
            }))
        || (batch.schema != NATIVE_CACHED_SUFFIX_BATCH_SCHEMA && batch.checkpoint_cache.is_some())
        || (batch.schema == NATIVE_SUFFIX_BATCH_SCHEMA
            && batch
                .candidates
                .iter()
                .any(|candidate| candidate.controller_program_hex.is_some()))
        || (batch.schema == NATIVE_REACTIVE_SUFFIX_BATCH_SCHEMA
            && batch
                .candidates
                .iter()
                .any(|candidate| candidate.controller_program_hex.is_none()))
    {
        return Err(worker_message("native suffix batch shape is invalid"));
    }
    Ok(())
}

fn canonical_file(path: &Path, label: &str) -> Result<PathBuf, NativeSuffixWorkerError> {
    let canonical = path.canonicalize().map_err(|source| {
        worker_message(format!(
            "cannot canonicalize {label} {}: {source}",
            path.display()
        ))
    })?;
    if !canonical.is_file() {
        return Err(worker_message(format!("{label} is not a regular file")));
    }
    Ok(canonical)
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf, NativeSuffixWorkerError> {
    let canonical = path.canonicalize().map_err(|source| {
        worker_message(format!(
            "cannot canonicalize {label} {}: {source}",
            path.display()
        ))
    })?;
    if !canonical.is_dir() {
        return Err(worker_message(format!("{label} is not a directory")));
    }
    Ok(canonical)
}

fn prepare_new_output(path: &Path, label: &str) -> Result<PathBuf, NativeSuffixWorkerError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_err(worker_error)?.join(path)
    };
    if absolute.exists() {
        return Err(worker_message(format!("{label} already exists")));
    }
    let parent = absolute
        .parent()
        .ok_or_else(|| worker_message(format!("{label} has no parent")))?;
    fs::create_dir_all(parent).map_err(|source| {
        worker_message(format!(
            "cannot create {label} parent {}: {source}",
            parent.display()
        ))
    })?;
    let parent = parent.canonicalize().map_err(|source| {
        worker_message(format!(
            "cannot canonicalize {label} parent {}: {source}",
            parent.display()
        ))
    })?;
    let name = absolute
        .file_name()
        .ok_or_else(|| worker_message(format!("{label} has no filename")))?;
    Ok(parent.join(name))
}

fn prepare_new_result_output(path: &Path, label: &str) -> Result<PathBuf, NativeSuffixWorkerError> {
    let output = prepare_new_output(path, label)?;
    let mut episode_name = output.as_os_str().to_os_string();
    episode_name.push(".episodes.dseps");
    if Path::new(&episode_name).exists() {
        return Err(worker_message(format!(
            "{label} episode shard already exists"
        )));
    }
    Ok(output)
}

fn prepare_state_root(path: &Path) -> Result<(), NativeSuffixWorkerError> {
    if path.exists() {
        if !path.is_dir()
            || fs::read_dir(path)
                .map_err(|source| {
                    worker_message(format!(
                        "cannot inspect native suffix state root {}: {source}",
                        path.display()
                    ))
                })?
                .next()
                .is_some()
        {
            return Err(worker_message(
                "native suffix state root must be new or empty",
            ));
        }
    } else {
        fs::create_dir_all(path).map_err(|source| {
            worker_message(format!(
                "cannot create native suffix state root {}: {source}",
                path.display()
            ))
        })?;
    }
    Ok(())
}

fn path_text<'a>(path: &'a Path, label: &str) -> Result<&'a str, NativeSuffixWorkerError> {
    path.to_str()
        .ok_or_else(|| worker_message(format!("{label} path is not UTF-8")))
}

fn sha256_file(path: &Path) -> Result<Digest, NativeSuffixWorkerError> {
    let mut file = File::open(path).map_err(|source| {
        worker_message(format!("cannot hash file {}: {source}", path.display()))
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|source| {
            worker_message(format!("cannot hash file {}: {source}", path.display()))
        })?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(Digest(hasher.finalize().into()))
}

fn sha256(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

#[derive(Debug)]
pub enum NativeSuffixWorkerError {
    Message(String),
    Rejected { code: String, message: String },
}

impl NativeSuffixWorkerError {
    pub fn is_missing_process_local_checkpoint(&self) -> bool {
        matches!(
            self,
            Self::Rejected { code, message }
                if code == "batch_rejected"
                    && message == "requested process-local checkpoint is absent or invalid"
        )
    }
}

fn worker_message(message: impl Into<String>) -> NativeSuffixWorkerError {
    NativeSuffixWorkerError::Message(message.into())
}

fn worker_error(error: impl fmt::Display) -> NativeSuffixWorkerError {
    NativeSuffixWorkerError::Message(error.to_string())
}

fn worker_client_error(error: ClientError) -> NativeSuffixWorkerError {
    match error {
        ClientError::Worker { code, message } => {
            NativeSuffixWorkerError::Rejected { code, message }
        }
        error => worker_error(error),
    }
}

fn elapsed_micros(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX)
}

impl fmt::Display for NativeSuffixWorkerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Message(message) => formatter.write_str(message),
            Self::Rejected { code, message } => {
                write!(formatter, "worker error {code}: {message}")
            }
        }
    }
}

impl Error for NativeSuffixWorkerError {}

#[cfg(test)]
#[path = "native_suffix_worker/tests.rs"]
mod tests;
