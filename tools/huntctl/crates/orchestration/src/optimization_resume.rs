//! Crash-safe append journal for route optimization campaigns.

use crate::optimization_request::OptimizationRequest;
use dusklight_automation_contracts::artifact::Digest;
use dusklight_harness_contracts::objective_suite::ArtifactReference;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const OPTIMIZATION_RESUME_RECORD_SCHEMA_V1: &str = "dusklight-optimization-resume-record/v1";
pub const OPTIMIZATION_RESUME_STATE_SCHEMA_V1: &str = "dusklight-optimization-resume-state/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptimizationResumeRecord {
    pub schema: String,
    pub request_sha256: Digest,
    pub sequence: u64,
    pub previous_record_sha256: Digest,
    pub event: OptimizationResumeEvent,
    pub record_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OptimizationResumeEvent {
    CandidateSealed {
        candidate_id: String,
        candidate: ArtifactReference,
        compiled_tape: ArtifactReference,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_tape_sha256: Option<Digest>,
        generation: u64,
        proposer_seed: u64,
    },
    EvaluationCompleted {
        candidate_id: String,
        candidate_sha256: Digest,
        result: ArtifactReference,
        simulated_ticks: u64,
    },
    OptimizerCheckpoint {
        generation: u64,
        completed_candidates: u64,
        state: ArtifactReference,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptimizationResumeState {
    pub schema: String,
    pub request_sha256: Digest,
    pub journal_sha256: Digest,
    pub valid_journal_bytes: u64,
    pub record_count: u64,
    pub last_record_sha256: Digest,
    pub next_sequence: u64,
    pub candidates: Vec<OptimizationResumeCandidate>,
    pub completed_candidates: u64,
    pub charged_simulated_ticks: u64,
    pub pending_candidate_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_optimizer_checkpoint: Option<OptimizationCheckpointState>,
    pub uncheckpointed_completions: u64,
    pub state_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptimizationResumeCandidate {
    pub id: String,
    pub candidate: ArtifactReference,
    pub candidate_sha256: Digest,
    pub compiled_tape: ArtifactReference,
    pub compiled_tape_sha256: Digest,
    pub generation: u64,
    pub proposer_seed: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<ArtifactReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_sha256: Option<Digest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub simulated_ticks: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptimizationCheckpointState {
    pub generation: u64,
    pub completed_candidates: u64,
    pub artifact: ArtifactReference,
    pub artifact_sha256: Digest,
}

impl OptimizationResumeState {
    pub fn validate(&self) -> Result<(), OptimizationResumeError> {
        if self.schema != OPTIMIZATION_RESUME_STATE_SCHEMA_V1
            || self.request_sha256 == Digest::ZERO
            || self.journal_sha256 == Digest::ZERO
            || self.next_sequence != self.record_count.saturating_add(1)
            || self.completed_candidates
                != self
                    .candidates
                    .iter()
                    .filter(|candidate| candidate.result_sha256.is_some())
                    .count() as u64
            || self.pending_candidate_ids
                != self
                    .candidates
                    .iter()
                    .filter(|candidate| candidate.result_sha256.is_none())
                    .map(|candidate| candidate.id.clone())
                    .collect::<Vec<_>>()
            || !self
                .candidates
                .windows(2)
                .all(|pair| pair[0].id < pair[1].id)
            || self.state_sha256 != self.compute_identity()?
        {
            return Err(resume_error("optimization resume state or seal is invalid"));
        }
        if self.candidates.iter().any(|candidate| {
            candidate.candidate.sha256 != candidate.candidate_sha256
                || candidate.compiled_tape.sha256 != candidate.compiled_tape_sha256
                || candidate.result.as_ref().map(|result| result.sha256) != candidate.result_sha256
        }) || self
            .latest_optimizer_checkpoint
            .as_ref()
            .is_some_and(|checkpoint| checkpoint.artifact.sha256 != checkpoint.artifact_sha256)
        {
            return Err(resume_error(
                "optimization resume artifact references differ from their folded identities",
            ));
        }
        let checkpointed = self
            .latest_optimizer_checkpoint
            .as_ref()
            .map_or(0, |checkpoint| checkpoint.completed_candidates);
        if checkpointed > self.completed_candidates
            || self.uncheckpointed_completions != self.completed_candidates - checkpointed
        {
            return Err(resume_error(
                "optimization checkpoint progress differs from completed evaluations",
            ));
        }
        Ok(())
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, OptimizationResumeError> {
        let mut bytes =
            serde_json::to_vec_pretty(self).map_err(|source| resume_error(source.to_string()))?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    fn compute_identity(&self) -> Result<Digest, OptimizationResumeError> {
        let mut canonical = self.clone();
        canonical.state_sha256 = Digest::ZERO;
        canonical_digest(b"dusklight.optimization-resume-state/v1\0", &canonical)
    }
}

pub fn initialize_optimization_resume(
    request: &OptimizationRequest,
    repository_root: &Path,
) -> Result<OptimizationResumeState, OptimizationResumeError> {
    request
        .validate_files(repository_root)
        .map_err(|source| resume_error(source.to_string()))?;
    let root = canonical_root(repository_root)?;
    let journal_path = output_path(&root, &request.resume.journal_path)?;
    let state_path = output_path(&root, &request.resume.state_path)?;
    if journal_path.exists() || state_path.exists() {
        return Err(resume_error(
            "optimization resume journal or state already exists",
        ));
    }
    create_parent(&journal_path)?;
    let journal = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&journal_path)
        .map_err(OptimizationResumeError::io)?;
    journal.sync_all().map_err(OptimizationResumeError::io)?;
    sync_parent(&journal_path)?;
    let state = fold_journal(request, &root, &journal_path, false)?;
    write_state_atomically(&state_path, &state)?;
    Ok(state)
}

pub fn load_optimization_resume(
    request: &OptimizationRequest,
    repository_root: &Path,
) -> Result<OptimizationResumeState, OptimizationResumeError> {
    request
        .validate_files(repository_root)
        .map_err(|source| resume_error(source.to_string()))?;
    let root = canonical_root(repository_root)?;
    let journal_path = output_path(&root, &request.resume.journal_path)?;
    let state_path = output_path(&root, &request.resume.state_path)?;
    let state = fold_journal(request, &root, &journal_path, true)?;
    write_state_atomically(&state_path, &state)?;
    Ok(state)
}

pub fn append_optimization_resume_event(
    request: &OptimizationRequest,
    repository_root: &Path,
    event: OptimizationResumeEvent,
) -> Result<OptimizationResumeState, OptimizationResumeError> {
    let current = load_optimization_resume(request, repository_root)?;
    let root = canonical_root(repository_root)?;
    validate_event_artifacts(request, &root, &event)?;
    validate_next_event(request, &current, &event)?;
    let mut record = OptimizationResumeRecord {
        schema: OPTIMIZATION_RESUME_RECORD_SCHEMA_V1.into(),
        request_sha256: request.content_sha256,
        sequence: current.next_sequence,
        previous_record_sha256: current.last_record_sha256,
        event,
        record_sha256: Digest::ZERO,
    };
    record.record_sha256 = record_identity(&record)?;
    let bytes = record_bytes(&record)?;
    let journal_path = output_path(&root, &request.resume.journal_path)?;
    let mut journal = OpenOptions::new()
        .append(true)
        .open(&journal_path)
        .map_err(OptimizationResumeError::io)?;
    journal
        .write_all(&bytes)
        .map_err(OptimizationResumeError::io)?;
    journal.sync_all().map_err(OptimizationResumeError::io)?;
    drop(journal);
    load_optimization_resume(request, repository_root)
}

fn fold_journal(
    request: &OptimizationRequest,
    root: &Path,
    journal_path: &Path,
    recover_tail: bool,
) -> Result<OptimizationResumeState, OptimizationResumeError> {
    let bytes = fs::read(journal_path).map_err(OptimizationResumeError::io)?;
    let valid_bytes = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
    if valid_bytes != bytes.len() && recover_tail {
        let journal = OpenOptions::new()
            .write(true)
            .open(journal_path)
            .map_err(OptimizationResumeError::io)?;
        journal
            .set_len(valid_bytes as u64)
            .map_err(OptimizationResumeError::io)?;
        journal.sync_all().map_err(OptimizationResumeError::io)?;
    } else if valid_bytes != bytes.len() {
        return Err(resume_error(
            "optimization journal has an incomplete final record",
        ));
    }
    let committed = &bytes[..valid_bytes];
    let mut records = Vec::new();
    let mut previous = Digest::ZERO;
    for (index, line) in committed.split_inclusive(|byte| *byte == b'\n').enumerate() {
        let record: OptimizationResumeRecord = serde_json::from_slice(&line[..line.len() - 1])
            .map_err(|source| {
                resume_error(format!(
                    "optimization journal record {} is invalid: {source}",
                    index + 1
                ))
            })?;
        if record_bytes(&record)? != line
            || record.schema != OPTIMIZATION_RESUME_RECORD_SCHEMA_V1
            || record.request_sha256 != request.content_sha256
            || record.sequence != index as u64 + 1
            || record.previous_record_sha256 != previous
            || record.record_sha256 != record_identity(&record)?
        {
            return Err(resume_error(
                "optimization journal is noncanonical, detached, or corrupt",
            ));
        }
        validate_event_artifacts(request, root, &record.event)?;
        previous = record.record_sha256;
        records.push(record);
    }

    let mut candidates = BTreeMap::<String, OptimizationResumeCandidate>::new();
    let mut compiled_tapes = BTreeSet::new();
    let mut charged_ticks = 0_u64;
    let mut latest_checkpoint = None;
    for record in &records {
        match &record.event {
            OptimizationResumeEvent::CandidateSealed {
                candidate_id,
                candidate,
                compiled_tape,
                generation,
                proposer_seed,
                ..
            } => {
                if candidates.contains_key(candidate_id)
                    || !compiled_tapes.insert(compiled_tape.sha256)
                {
                    return Err(resume_error(
                        "optimization journal repeats a sealed candidate ID or compiled tape",
                    ));
                }
                candidates.insert(
                    candidate_id.clone(),
                    OptimizationResumeCandidate {
                        id: candidate_id.clone(),
                        candidate: candidate.clone(),
                        candidate_sha256: candidate.sha256,
                        compiled_tape: compiled_tape.clone(),
                        compiled_tape_sha256: compiled_tape.sha256,
                        generation: *generation,
                        proposer_seed: *proposer_seed,
                        result: None,
                        result_sha256: None,
                        simulated_ticks: None,
                    },
                );
            }
            OptimizationResumeEvent::EvaluationCompleted {
                candidate_id,
                candidate_sha256,
                result,
                simulated_ticks,
            } => {
                let candidate = candidates.get_mut(candidate_id).ok_or_else(|| {
                    resume_error("evaluation precedes its sealed optimization candidate")
                })?;
                if candidate.candidate_sha256 != *candidate_sha256
                    || candidate.result_sha256.is_some()
                    || *simulated_ticks == 0
                {
                    return Err(resume_error(
                        "optimization evaluation is detached, duplicate, or uncharged",
                    ));
                }
                charged_ticks = charged_ticks
                    .checked_add(*simulated_ticks)
                    .ok_or_else(|| resume_error("optimization simulated-tick charge overflowed"))?;
                if charged_ticks > request.budgets.simulated_tick_budget {
                    return Err(resume_error(
                        "optimization journal exceeds the sealed simulated-tick budget",
                    ));
                }
                candidate.result = Some(result.clone());
                candidate.result_sha256 = Some(result.sha256);
                candidate.simulated_ticks = Some(*simulated_ticks);
            }
            OptimizationResumeEvent::OptimizerCheckpoint {
                generation,
                completed_candidates,
                state,
            } => {
                let completed = candidates
                    .values()
                    .filter(|candidate| candidate.result_sha256.is_some())
                    .count() as u64;
                if *completed_candidates != completed
                    || latest_checkpoint.as_ref().is_some_and(
                        |prior: &OptimizationCheckpointState| {
                            *generation < prior.generation
                                || *completed_candidates < prior.completed_candidates
                        },
                    )
                {
                    return Err(resume_error(
                        "optimizer checkpoint does not match monotonic completed evaluation progress",
                    ));
                }
                latest_checkpoint = Some(OptimizationCheckpointState {
                    generation: *generation,
                    completed_candidates: *completed_candidates,
                    artifact: state.clone(),
                    artifact_sha256: state.sha256,
                });
            }
        }
    }
    if candidates.len() as u64 > request.budgets.candidate_budget {
        return Err(resume_error(
            "optimization journal exceeds the sealed candidate budget",
        ));
    }
    let candidate_rows = candidates.into_values().collect::<Vec<_>>();
    let completed_candidates = candidate_rows
        .iter()
        .filter(|candidate| candidate.result_sha256.is_some())
        .count() as u64;
    let pending_candidate_ids = candidate_rows
        .iter()
        .filter(|candidate| candidate.result_sha256.is_none())
        .map(|candidate| candidate.id.clone())
        .collect::<Vec<_>>();
    let checkpointed = latest_checkpoint
        .as_ref()
        .map_or(0, |checkpoint| checkpoint.completed_candidates);
    let mut state = OptimizationResumeState {
        schema: OPTIMIZATION_RESUME_STATE_SCHEMA_V1.into(),
        request_sha256: request.content_sha256,
        journal_sha256: sha256(committed),
        valid_journal_bytes: valid_bytes as u64,
        record_count: records.len() as u64,
        last_record_sha256: previous,
        next_sequence: records.len() as u64 + 1,
        candidates: candidate_rows,
        completed_candidates,
        charged_simulated_ticks: charged_ticks,
        pending_candidate_ids,
        latest_optimizer_checkpoint: latest_checkpoint,
        uncheckpointed_completions: completed_candidates - checkpointed,
        state_sha256: Digest::ZERO,
    };
    state.state_sha256 = state.compute_identity()?;
    state.validate()?;
    Ok(state)
}

fn validate_next_event(
    request: &OptimizationRequest,
    state: &OptimizationResumeState,
    event: &OptimizationResumeEvent,
) -> Result<(), OptimizationResumeError> {
    match event {
        OptimizationResumeEvent::CandidateSealed {
            candidate_id,
            compiled_tape,
            parent_tape_sha256,
            ..
        } => {
            validate_id(candidate_id)?;
            if state.uncheckpointed_completions >= request.resume.checkpoint_every_candidates {
                return Err(resume_error(
                    "optimizer checkpoint is required before sealing another candidate",
                ));
            }
            if state.candidates.len() as u64 >= request.budgets.candidate_budget
                || state.candidates.iter().any(|candidate| {
                    candidate.id == *candidate_id
                        || candidate.compiled_tape_sha256 == compiled_tape.sha256
                })
                || *parent_tape_sha256
                    != request
                        .incumbent
                        .as_ref()
                        .map(|incumbent| incumbent.tape.sha256)
            {
                return Err(resume_error(
                    "candidate is duplicate, over budget, or detached from the incumbent",
                ));
            }
        }
        OptimizationResumeEvent::EvaluationCompleted {
            candidate_id,
            candidate_sha256,
            simulated_ticks,
            ..
        } => {
            let maximum_candidate_ticks = request
                .budgets
                .exploration_horizon_ticks
                .checked_mul(u64::from(request.execution.repetitions))
                .ok_or_else(|| resume_error("candidate tick bound overflowed"))?;
            if *simulated_ticks > maximum_candidate_ticks {
                return Err(resume_error(
                    "evaluation exceeds its per-candidate exploration tick bound",
                ));
            }
            let candidate = state
                .candidates
                .iter()
                .find(|candidate| candidate.id == *candidate_id)
                .ok_or_else(|| resume_error("evaluation candidate is not sealed"))?;
            if candidate.candidate_sha256 != *candidate_sha256
                || candidate.result_sha256.is_some()
                || *simulated_ticks == 0
                || state
                    .charged_simulated_ticks
                    .checked_add(*simulated_ticks)
                    .is_none_or(|ticks| ticks > request.budgets.simulated_tick_budget)
            {
                return Err(resume_error(
                    "evaluation is duplicate, detached, or exceeds the tick budget",
                ));
            }
        }
        OptimizationResumeEvent::OptimizerCheckpoint {
            generation,
            completed_candidates,
            ..
        } => {
            if *completed_candidates != state.completed_candidates
                || state
                    .latest_optimizer_checkpoint
                    .as_ref()
                    .is_some_and(|prior| {
                        *generation < prior.generation
                            || *completed_candidates < prior.completed_candidates
                    })
            {
                return Err(resume_error(
                    "optimizer checkpoint does not match completed progress",
                ));
            }
        }
    }
    Ok(())
}

fn validate_event_artifacts(
    request: &OptimizationRequest,
    root: &Path,
    event: &OptimizationResumeEvent,
) -> Result<(), OptimizationResumeError> {
    match event {
        OptimizationResumeEvent::CandidateSealed {
            candidate_id,
            candidate,
            compiled_tape,
            parent_tape_sha256,
            ..
        } => {
            validate_id(candidate_id)?;
            validate_artifact(root, "candidate", candidate)?;
            validate_artifact(root, "compiled tape", compiled_tape)?;
            if *parent_tape_sha256
                != request
                    .incumbent
                    .as_ref()
                    .map(|incumbent| incumbent.tape.sha256)
            {
                return Err(resume_error(
                    "sealed candidate parent differs from the optimization incumbent",
                ));
            }
        }
        OptimizationResumeEvent::EvaluationCompleted {
            candidate_id,
            candidate_sha256,
            result,
            simulated_ticks,
        } => {
            validate_id(candidate_id)?;
            if *candidate_sha256 == Digest::ZERO || *simulated_ticks == 0 {
                return Err(resume_error(
                    "evaluation identity or tick charge is invalid",
                ));
            }
            validate_artifact(root, "evaluation result", result)?;
        }
        OptimizationResumeEvent::OptimizerCheckpoint {
            completed_candidates,
            state,
            ..
        } => {
            if *completed_candidates > request.budgets.candidate_budget {
                return Err(resume_error(
                    "optimizer checkpoint exceeds candidate budget",
                ));
            }
            validate_artifact(root, "optimizer checkpoint", state)?;
        }
    }
    Ok(())
}

fn validate_artifact(
    root: &Path,
    label: &str,
    artifact: &ArtifactReference,
) -> Result<(), OptimizationResumeError> {
    let relative = Path::new(&artifact.path);
    if artifact.sha256 == Digest::ZERO
        || artifact.path.is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(resume_error(format!("{label} reference is invalid")));
    }
    let path = root.join(relative).canonicalize().map_err(|source| {
        resume_error(format!(
            "cannot resolve {label} {}: {source}",
            artifact.path
        ))
    })?;
    if !path.starts_with(root) || !path.is_file() {
        return Err(resume_error(format!("{label} must be a repository file")));
    }
    if sha256(&fs::read(path).map_err(OptimizationResumeError::io)?) != artifact.sha256 {
        return Err(resume_error(format!("{label} content digest differs")));
    }
    Ok(())
}

fn validate_id(value: &str) -> Result<(), OptimizationResumeError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
    {
        return Err(resume_error("optimization candidate ID is invalid"));
    }
    Ok(())
}

fn record_identity(record: &OptimizationResumeRecord) -> Result<Digest, OptimizationResumeError> {
    let mut canonical = record.clone();
    canonical.record_sha256 = Digest::ZERO;
    canonical_digest(b"dusklight.optimization-resume-record/v1\0", &canonical)
}

fn record_bytes(record: &OptimizationResumeRecord) -> Result<Vec<u8>, OptimizationResumeError> {
    let mut bytes =
        serde_json::to_vec(record).map_err(|source| resume_error(source.to_string()))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn canonical_digest(
    domain: &[u8],
    value: &impl Serialize,
) -> Result<Digest, OptimizationResumeError> {
    let bytes = serde_json::to_vec(value).map_err(|source| resume_error(source.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

fn sha256(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

fn canonical_root(path: &Path) -> Result<PathBuf, OptimizationResumeError> {
    path.canonicalize().map_err(|source| {
        resume_error(format!(
            "cannot resolve repository root {}: {source}",
            path.display()
        ))
    })
}

fn output_path(root: &Path, relative: &str) -> Result<PathBuf, OptimizationResumeError> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || !relative.starts_with("build")
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(resume_error(
            "optimization resume output must be a canonical path beneath build/",
        ));
    }
    Ok(root.join(relative))
}

fn create_parent(path: &Path) -> Result<(), OptimizationResumeError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(OptimizationResumeError::io)?;
    }
    Ok(())
}

fn write_state_atomically(
    path: &Path,
    state: &OptimizationResumeState,
) -> Result<(), OptimizationResumeError> {
    create_parent(path)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|source| resume_error(source.to_string()))?
        .as_nanos();
    let temporary = path.with_extension(format!("tmp-{}-{nonce}", std::process::id()));
    let result = (|| {
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(OptimizationResumeError::io)?;
        output
            .write_all(&state.to_pretty_json()?)
            .map_err(OptimizationResumeError::io)?;
        output.sync_all().map_err(OptimizationResumeError::io)?;
        drop(output);
        fs::rename(&temporary, path).map_err(OptimizationResumeError::io)?;
        sync_parent(path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn sync_parent(path: &Path) -> Result<(), OptimizationResumeError> {
    let parent = path
        .parent()
        .ok_or_else(|| resume_error("optimization persistence path has no parent"))?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(OptimizationResumeError::io)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptimizationResumeError(String);

fn resume_error(message: impl Into<String>) -> OptimizationResumeError {
    OptimizationResumeError(message.into())
}

impl OptimizationResumeError {
    fn io(source: std::io::Error) -> Self {
        resume_error(source.to_string())
    }
}

impl fmt::Display for OptimizationResumeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for OptimizationResumeError {}
