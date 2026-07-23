//! Crash-safe append journal for route optimization campaigns.

use crate::optimization_request::OptimizationRequest;
use dusklight_automation_contracts::artifact::Digest;
use dusklight_harness_contracts::objective_suite::ArtifactReference;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
#[cfg(not(windows))]
use std::fs::File;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const OPTIMIZATION_RESUME_RECORD_SCHEMA_V1: &str = "dusklight-optimization-resume-record/v1";
pub const OPTIMIZATION_RESUME_RECORD_SCHEMA_V2: &str = "dusklight-optimization-resume-record/v2";
pub const OPTIMIZATION_RESUME_STATE_SCHEMA_V2: &str = "dusklight-optimization-resume-state/v2";

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
    DemonstrationSeeded {
        demonstration: ArtifactReference,
        simulated_ticks: u64,
    },
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub demonstration: Option<ArtifactReference>,
    #[serde(default)]
    pub demonstration_simulated_ticks: u64,
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
        if self.schema != OPTIMIZATION_RESUME_STATE_SCHEMA_V2
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
            .demonstration
            .as_ref()
            .is_some_and(|demonstration| demonstration.sha256 == Digest::ZERO)
            || self.demonstration.is_some() != (self.demonstration_simulated_ticks > 0)
            || self
                .latest_optimizer_checkpoint
                .as_ref()
                .is_some_and(|checkpoint| checkpoint.artifact.sha256 != checkpoint.artifact_sha256)
        {
            return Err(resume_error(
                "optimization resume artifact references differ from their folded identities",
            ));
        }
        let charged = self
            .candidates
            .iter()
            .try_fold(self.demonstration_simulated_ticks, |total, candidate| {
                total.checked_add(candidate.simulated_ticks.unwrap_or(0))
            });
        if charged != Some(self.charged_simulated_ticks) {
            return Err(resume_error(
                "optimization resume state does not charge all native simulation",
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
        canonical_digest(b"dusklight.optimization-resume-state/v2\0", &canonical)
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

/// Returns completed candidate IDs in the journal's authoritative evaluation
/// order. The full fold runs first, so callers never consume an unsealed,
/// detached, or torn ordering as campaign evidence.
pub(crate) fn optimization_evaluation_order(
    request: &OptimizationRequest,
    repository_root: &Path,
) -> Result<Vec<String>, OptimizationResumeError> {
    request
        .validate_files(repository_root)
        .map_err(|source| resume_error(source.to_string()))?;
    let root = canonical_root(repository_root)?;
    let journal_path = output_path(&root, &request.resume.journal_path)?;
    let state = fold_journal(request, &root, &journal_path, false)?;
    let bytes = fs::read(&journal_path).map_err(OptimizationResumeError::io)?;
    if bytes.len() as u64 != state.valid_journal_bytes {
        return Err(resume_error(
            "optimization evaluation order differs from the validated journal",
        ));
    }
    let mut order = Vec::new();
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        let record: OptimizationResumeRecord = serde_json::from_slice(&line[..line.len() - 1])
            .map_err(|source| resume_error(source.to_string()))?;
        if let OptimizationResumeEvent::EvaluationCompleted { candidate_id, .. } = record.event {
            order.push(candidate_id);
        }
    }
    if order.len() as u64 != state.completed_candidates
        || order.iter().collect::<BTreeSet<_>>().len() != order.len()
    {
        return Err(resume_error(
            "optimization evaluation order is incomplete or repeats a candidate",
        ));
    }
    Ok(order)
}

pub fn append_optimization_resume_event(
    request: &OptimizationRequest,
    repository_root: &Path,
    event: OptimizationResumeEvent,
) -> Result<OptimizationResumeState, OptimizationResumeError> {
    append_optimization_resume_events(request, repository_root, vec![event])
}

/// Appends one validated batch of journal events with a single durable
/// write and refold. Every event is previewed in order, so batching cannot
/// bypass candidate, tick, checkpoint, or artifact constraints.
pub fn append_optimization_resume_events(
    request: &OptimizationRequest,
    repository_root: &Path,
    events: Vec<OptimizationResumeEvent>,
) -> Result<OptimizationResumeState, OptimizationResumeError> {
    if events.is_empty() {
        return Err(resume_error(
            "optimization resume batch must contain at least one event",
        ));
    }
    let current = load_optimization_resume(request, repository_root)?;
    let root = canonical_root(repository_root)?;
    let mut preview = current.clone();
    let mut sequence = current.next_sequence;
    let mut previous = current.last_record_sha256;
    let mut bytes = Vec::new();
    for event in events {
        validate_event_artifacts(request, &root, &event)?;
        validate_next_event(request, &preview, &event)?;
        apply_preview_event(&mut preview, &event)?;
        let mut record = OptimizationResumeRecord {
            schema: OPTIMIZATION_RESUME_RECORD_SCHEMA_V2.into(),
            request_sha256: request.content_sha256,
            sequence,
            previous_record_sha256: previous,
            event,
            record_sha256: Digest::ZERO,
        };
        record.record_sha256 = record_identity(&record)?;
        bytes.extend(record_bytes(&record)?);
        previous = record.record_sha256;
        sequence = sequence
            .checked_add(1)
            .ok_or_else(|| resume_error("optimization journal sequence overflowed"))?;
    }
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

fn apply_preview_event(
    state: &mut OptimizationResumeState,
    event: &OptimizationResumeEvent,
) -> Result<(), OptimizationResumeError> {
    match event {
        OptimizationResumeEvent::DemonstrationSeeded {
            demonstration,
            simulated_ticks,
        } => {
            state.demonstration = Some(demonstration.clone());
            state.demonstration_simulated_ticks = *simulated_ticks;
            state.charged_simulated_ticks = state
                .charged_simulated_ticks
                .checked_add(*simulated_ticks)
                .ok_or_else(|| resume_error("simulated-tick charge overflowed"))?;
        }
        OptimizationResumeEvent::CandidateSealed {
            candidate_id,
            candidate,
            compiled_tape,
            generation,
            proposer_seed,
            ..
        } => {
            state.candidates.push(OptimizationResumeCandidate {
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
            });
            state
                .candidates
                .sort_by(|left, right| left.id.cmp(&right.id));
            state.pending_candidate_ids = state
                .candidates
                .iter()
                .filter(|candidate| candidate.result.is_none())
                .map(|candidate| candidate.id.clone())
                .collect();
        }
        OptimizationResumeEvent::EvaluationCompleted {
            candidate_id,
            result,
            simulated_ticks,
            ..
        } => {
            let candidate = state
                .candidates
                .iter_mut()
                .find(|candidate| candidate.id == *candidate_id)
                .ok_or_else(|| resume_error("evaluation candidate is not sealed"))?;
            candidate.result = Some(result.clone());
            candidate.result_sha256 = Some(result.sha256);
            candidate.simulated_ticks = Some(*simulated_ticks);
            state.completed_candidates = state
                .completed_candidates
                .checked_add(1)
                .ok_or_else(|| resume_error("completed candidate count overflowed"))?;
            state.charged_simulated_ticks = state
                .charged_simulated_ticks
                .checked_add(*simulated_ticks)
                .ok_or_else(|| resume_error("simulated-tick charge overflowed"))?;
            state.uncheckpointed_completions = state
                .uncheckpointed_completions
                .checked_add(1)
                .ok_or_else(|| resume_error("uncheckpointed completion count overflowed"))?;
            state
                .pending_candidate_ids
                .retain(|pending| pending != candidate_id);
        }
        OptimizationResumeEvent::OptimizerCheckpoint {
            generation,
            completed_candidates,
            state: artifact,
        } => {
            state.latest_optimizer_checkpoint = Some(OptimizationCheckpointState {
                generation: *generation,
                completed_candidates: *completed_candidates,
                artifact: artifact.clone(),
                artifact_sha256: artifact.sha256,
            });
            state.uncheckpointed_completions = 0;
        }
    }
    Ok(())
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
            || !matches!(
                record.schema.as_str(),
                OPTIMIZATION_RESUME_RECORD_SCHEMA_V1 | OPTIMIZATION_RESUME_RECORD_SCHEMA_V2
            )
            || (record.schema == OPTIMIZATION_RESUME_RECORD_SCHEMA_V1
                && matches!(
                    &record.event,
                    OptimizationResumeEvent::DemonstrationSeeded { .. }
                ))
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
    let mut demonstration = None;
    let mut demonstration_simulated_ticks = 0_u64;
    let mut latest_checkpoint = None;
    for record in &records {
        match &record.event {
            OptimizationResumeEvent::DemonstrationSeeded {
                demonstration: artifact,
                simulated_ticks,
            } => {
                if demonstration.is_some()
                    || !candidates.is_empty()
                    || latest_checkpoint.is_some()
                    || *simulated_ticks == 0
                    || *simulated_ticks > request.budgets.exploration_horizon_ticks
                {
                    return Err(resume_error(
                        "optimization demonstration seed is duplicate, late, or overlong",
                    ));
                }
                demonstration = Some(artifact.clone());
                demonstration_simulated_ticks = *simulated_ticks;
                charged_ticks = charged_ticks
                    .checked_add(*simulated_ticks)
                    .ok_or_else(|| resume_error("optimization simulated-tick charge overflowed"))?;
                if charged_ticks > request.budgets.simulated_tick_budget {
                    return Err(resume_error(
                        "optimization demonstration exceeds the sealed simulated-tick budget",
                    ));
                }
            }
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
                    || *simulated_ticks > maximum_candidate_ticks(request)?
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
        schema: OPTIMIZATION_RESUME_STATE_SCHEMA_V2.into(),
        request_sha256: request.content_sha256,
        journal_sha256: sha256(committed),
        valid_journal_bytes: valid_bytes as u64,
        record_count: records.len() as u64,
        last_record_sha256: previous,
        next_sequence: records.len() as u64 + 1,
        demonstration,
        demonstration_simulated_ticks,
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
        OptimizationResumeEvent::DemonstrationSeeded {
            demonstration: _,
            simulated_ticks,
        } => {
            if state.demonstration.is_some()
                || !state.candidates.is_empty()
                || state.latest_optimizer_checkpoint.is_some()
                || *simulated_ticks == 0
                || *simulated_ticks > request.budgets.exploration_horizon_ticks
                || state
                    .charged_simulated_ticks
                    .checked_add(*simulated_ticks)
                    .is_none_or(|ticks| ticks > request.budgets.simulated_tick_budget)
            {
                return Err(resume_error(
                    "optimization demonstration seed is duplicate, late, or exceeds its budget",
                ));
            }
        }
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
            let maximum_candidate_ticks = maximum_candidate_ticks(request)?;
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

fn maximum_candidate_ticks(request: &OptimizationRequest) -> Result<u64, OptimizationResumeError> {
    let terminal_runs = 1_u64
        .checked_add(request.execution.alternate_terminal_goals.len() as u64)
        .ok_or_else(|| resume_error("candidate terminal-run count overflowed"))?;
    request
        .budgets
        .exploration_horizon_ticks
        .checked_mul(u64::from(request.execution.repetitions))
        .and_then(|ticks| ticks.checked_mul(terminal_runs))
        .ok_or_else(|| resume_error("candidate tick bound overflowed"))
}

fn validate_event_artifacts(
    request: &OptimizationRequest,
    root: &Path,
    event: &OptimizationResumeEvent,
) -> Result<(), OptimizationResumeError> {
    match event {
        OptimizationResumeEvent::DemonstrationSeeded {
            demonstration,
            simulated_ticks,
        } => {
            if *simulated_ticks == 0 {
                return Err(resume_error(
                    "optimization demonstration has no simulated-tick charge",
                ));
            }
            validate_artifact(root, "incumbent demonstration", demonstration)?;
        }
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
    let domain = match record.schema.as_str() {
        OPTIMIZATION_RESUME_RECORD_SCHEMA_V1 => {
            b"dusklight.optimization-resume-record/v1\0".as_slice()
        }
        _ => b"dusklight.optimization-resume-record/v2\0".as_slice(),
    };
    canonical_digest(domain, &canonical)
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
        replace_atomically(&temporary, path)?;
        sync_parent(path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(not(windows))]
fn sync_parent(path: &Path) -> Result<(), OptimizationResumeError> {
    let parent = path
        .parent()
        .ok_or_else(|| resume_error("optimization persistence path has no parent"))?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(OptimizationResumeError::io)
}

#[cfg(windows)]
fn sync_parent(path: &Path) -> Result<(), OptimizationResumeError> {
    // Opening a directory with File::open fails with ERROR_ACCESS_DENIED on
    // Windows. The state replacement below uses MOVEFILE_WRITE_THROUGH, which
    // does not return until the move is flushed. Still validate the parent so
    // a malformed persistence path cannot silently skip this boundary.
    let parent = path
        .parent()
        .ok_or_else(|| resume_error("optimization persistence path has no parent"))?;
    if !parent.is_dir() {
        return Err(resume_error(
            "optimization persistence parent is not a directory",
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_atomically(source: &Path, destination: &Path) -> Result<(), OptimizationResumeError> {
    fs::rename(source, destination).map_err(OptimizationResumeError::io)
}

#[cfg(windows)]
fn replace_atomically(source: &Path, destination: &Path) -> Result<(), OptimizationResumeError> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
    }

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let moved = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(OptimizationResumeError::io(std::io::Error::last_os_error()))
    } else {
        Ok(())
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_ROOT: AtomicU64 = AtomicU64::new(0);

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new() -> Self {
            let serial = NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "dusklight-optimization-resume-{}-{serial}",
                std::process::id()
            ));
            if path.exists() {
                fs::remove_dir_all(&path).unwrap();
            }
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn source_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(4)
            .unwrap()
            .to_path_buf()
    }

    fn copy_repository_file(source: &Path, destination_root: &Path, relative: &str) {
        let destination = destination_root.join(relative);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::copy(source.join(relative), destination).unwrap();
    }

    fn copy_tree(source: &Path, destination: &Path) {
        fs::create_dir_all(destination).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let target = destination.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    fn fixture(checkpoint_every_candidates: u64) -> (TestRoot, OptimizationRequest) {
        let root = TestRoot::new();
        let source = source_root();
        let request_path =
            "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json";
        let mut request: OptimizationRequest =
            serde_json::from_slice(&fs::read(source.join(request_path)).unwrap()).unwrap();
        copy_repository_file(&source, &root.0, "routes/Glitch Exhibition/intro.timeline");
        copy_tree(
            &source.join("routes/Glitch Exhibition/intro"),
            &root.0.join("routes/Glitch Exhibition/intro"),
        );
        let suffix = NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed);
        request.resume.state_path = format!("build/test-{suffix}/state.json");
        request.resume.journal_path = format!("build/test-{suffix}/journal.jsonl");
        request.resume.checkpoint_every_candidates = checkpoint_every_candidates;
        request.refresh_content_sha256().unwrap();
        request.validate_files(&root.0).unwrap();
        (root, request)
    }

    fn artifact(root: &Path, relative: &str, bytes: &[u8]) -> ArtifactReference {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
        ArtifactReference {
            path: relative.into(),
            sha256: sha256(bytes),
        }
    }

    fn candidate_event(
        request: &OptimizationRequest,
        id: &str,
        candidate: ArtifactReference,
        compiled_tape: ArtifactReference,
    ) -> OptimizationResumeEvent {
        OptimizationResumeEvent::CandidateSealed {
            candidate_id: id.into(),
            candidate,
            compiled_tape,
            parent_tape_sha256: Some(request.incumbent.as_ref().unwrap().tape.sha256),
            generation: 0,
            proposer_seed: 7,
        }
    }

    #[test]
    fn incumbent_demonstration_is_first_unique_and_charged() {
        let (root, request) = fixture(1);
        initialize_optimization_resume(&request, &root.0).unwrap();
        let demonstration = artifact(
            &root.0,
            "build/artifacts/incumbent-demonstration.json",
            b"demonstration",
        );
        let state = append_optimization_resume_event(
            &request,
            &root.0,
            OptimizationResumeEvent::DemonstrationSeeded {
                demonstration: demonstration.clone(),
                simulated_ticks: request.incumbent.as_ref().unwrap().first_hit_tick,
            },
        )
        .unwrap();

        assert_eq!(state.demonstration, Some(demonstration.clone()));
        assert_eq!(state.demonstration_simulated_ticks, 125);
        assert_eq!(state.charged_simulated_ticks, 125);
        assert_eq!(state.record_count, 1);

        let error = append_optimization_resume_event(
            &request,
            &root.0,
            OptimizationResumeEvent::DemonstrationSeeded {
                demonstration,
                simulated_ticks: 125,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("duplicate"));
    }

    #[test]
    fn alternate_terminal_run_expands_only_the_sealed_candidate_tick_bound() {
        let (root, request) = fixture(2);
        initialize_optimization_resume(&request, &root.0).unwrap();
        let candidate = artifact(&root.0, "build/artifacts/alternate.json", b"candidate");
        let candidate_sha256 = candidate.sha256;
        let tape = artifact(&root.0, "build/artifacts/alternate.tape", b"tape");
        let result = artifact(&root.0, "build/artifacts/alternate-result.json", b"result");
        let state = append_optimization_resume_events(
            &request,
            &root.0,
            vec![
                candidate_event(&request, "g0-alternate", candidate, tape),
                OptimizationResumeEvent::EvaluationCompleted {
                    candidate_id: "g0-alternate".into(),
                    candidate_sha256,
                    result,
                    simulated_ticks: request.budgets.exploration_horizon_ticks * 2,
                },
            ],
        )
        .unwrap();
        assert_eq!(state.charged_simulated_ticks, 320);

        let (root, mut request) = fixture(2);
        request.execution.alternate_terminal_goals.clear();
        request.refresh_content_sha256().unwrap();
        initialize_optimization_resume(&request, &root.0).unwrap();
        let candidate = artifact(&root.0, "build/artifacts/main-only.json", b"candidate");
        let candidate_sha256 = candidate.sha256;
        let tape = artifact(&root.0, "build/artifacts/main-only.tape", b"tape");
        let result = artifact(&root.0, "build/artifacts/main-only-result.json", b"result");
        let error = append_optimization_resume_events(
            &request,
            &root.0,
            vec![
                candidate_event(&request, "g0-main-only", candidate, tape),
                OptimizationResumeEvent::EvaluationCompleted {
                    candidate_id: "g0-main-only".into(),
                    candidate_sha256,
                    result,
                    simulated_ticks: request.budgets.exploration_horizon_ticks * 2,
                },
            ],
        )
        .unwrap_err();
        assert!(error.to_string().contains("per-candidate"));
    }

    #[test]
    fn v1_candidate_journal_resumes_without_a_late_demonstration() {
        let (root, request) = fixture(1);
        initialize_optimization_resume(&request, &root.0).unwrap();
        let candidate = artifact(&root.0, "build/artifacts/legacy.json", b"legacy");
        let tape = artifact(&root.0, "build/artifacts/legacy.tape", b"legacy-tape");
        let mut record = OptimizationResumeRecord {
            schema: OPTIMIZATION_RESUME_RECORD_SCHEMA_V1.into(),
            request_sha256: request.content_sha256,
            sequence: 1,
            previous_record_sha256: Digest::ZERO,
            event: candidate_event(&request, "legacy-g0-c0", candidate, tape),
            record_sha256: Digest::ZERO,
        };
        record.record_sha256 = record_identity(&record).unwrap();
        fs::write(
            root.0.join(&request.resume.journal_path),
            record_bytes(&record).unwrap(),
        )
        .unwrap();

        let state = load_optimization_resume(&request, &root.0).unwrap();
        assert_eq!(state.schema, OPTIMIZATION_RESUME_STATE_SCHEMA_V2);
        assert!(state.demonstration.is_none());
        assert_eq!(state.candidates.len(), 1);

        let demonstration = artifact(&root.0, "build/artifacts/late-demonstration.json", b"late");
        let error = append_optimization_resume_event(
            &request,
            &root.0,
            OptimizationResumeEvent::DemonstrationSeeded {
                demonstration,
                simulated_ticks: 125,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("late"));
    }

    #[test]
    fn batch_previews_candidate_evaluation_and_checkpoint_in_order() {
        let (root, request) = fixture(1);
        let initial = initialize_optimization_resume(&request, &root.0).unwrap();
        let candidate = artifact(&root.0, "build/artifacts/candidate.json", b"candidate");
        let candidate_sha256 = candidate.sha256;
        let tape = artifact(&root.0, "build/artifacts/candidate.tape", b"tape");
        let result = artifact(&root.0, "build/artifacts/result.json", b"result");
        let checkpoint = artifact(&root.0, "build/artifacts/checkpoint.json", b"checkpoint");

        let state = append_optimization_resume_events(
            &request,
            &root.0,
            vec![
                candidate_event(&request, "g0-c0", candidate, tape),
                OptimizationResumeEvent::EvaluationCompleted {
                    candidate_id: "g0-c0".into(),
                    candidate_sha256,
                    result,
                    simulated_ticks: request.budgets.exploration_horizon_ticks,
                },
                OptimizationResumeEvent::OptimizerCheckpoint {
                    generation: 1,
                    completed_candidates: 1,
                    state: checkpoint,
                },
            ],
        )
        .unwrap();

        assert_eq!(state.record_count, 3);
        assert_eq!(state.next_sequence, 4);
        assert_eq!(state.completed_candidates, 1);
        assert_eq!(state.charged_simulated_ticks, 160);
        assert!(state.pending_candidate_ids.is_empty());
        assert_eq!(state.uncheckpointed_completions, 0);
        assert_eq!(
            state
                .latest_optimizer_checkpoint
                .as_ref()
                .unwrap()
                .generation,
            1
        );
        assert_ne!(state.last_record_sha256, initial.last_record_sha256);
        assert_eq!(load_optimization_resume(&request, &root.0).unwrap(), state);
    }

    #[test]
    fn invalid_later_event_rejects_the_whole_batch_before_append() {
        let (root, request) = fixture(2);
        let initial = initialize_optimization_resume(&request, &root.0).unwrap();
        let first_candidate = artifact(&root.0, "build/artifacts/first.json", b"first");
        let second_candidate = artifact(&root.0, "build/artifacts/second.json", b"second");
        let shared_tape = artifact(&root.0, "build/artifacts/shared.tape", b"shared");

        let error = append_optimization_resume_events(
            &request,
            &root.0,
            vec![
                candidate_event(&request, "g0-c0", first_candidate, shared_tape.clone()),
                candidate_event(&request, "g0-c1", second_candidate, shared_tape),
            ],
        )
        .unwrap_err();

        assert!(error.to_string().contains("duplicate"));
        assert_eq!(
            load_optimization_resume(&request, &root.0).unwrap(),
            initial
        );
    }

    #[test]
    fn batch_cannot_cross_the_checkpoint_boundary() {
        let (root, request) = fixture(1);
        let initial = initialize_optimization_resume(&request, &root.0).unwrap();
        let first_candidate = artifact(&root.0, "build/artifacts/first.json", b"first");
        let first_candidate_sha256 = first_candidate.sha256;
        let first_tape = artifact(&root.0, "build/artifacts/first.tape", b"first-tape");
        let result = artifact(&root.0, "build/artifacts/result.json", b"result");
        let second_candidate = artifact(&root.0, "build/artifacts/second.json", b"second");
        let second_tape = artifact(&root.0, "build/artifacts/second.tape", b"second-tape");

        let error = append_optimization_resume_events(
            &request,
            &root.0,
            vec![
                candidate_event(&request, "g0-c0", first_candidate, first_tape),
                OptimizationResumeEvent::EvaluationCompleted {
                    candidate_id: "g0-c0".into(),
                    candidate_sha256: first_candidate_sha256,
                    result,
                    simulated_ticks: 160,
                },
                candidate_event(&request, "g0-c1", second_candidate, second_tape),
            ],
        )
        .unwrap_err();

        assert!(error.to_string().contains("checkpoint is required"));
        assert_eq!(
            load_optimization_resume(&request, &root.0).unwrap(),
            initial
        );
    }

    #[test]
    fn empty_batch_is_rejected_without_changing_the_journal() {
        let (root, request) = fixture(1);
        let initial = initialize_optimization_resume(&request, &root.0).unwrap();
        let error = append_optimization_resume_events(&request, &root.0, Vec::new()).unwrap_err();

        assert!(error.to_string().contains("at least one event"));
        assert_eq!(
            load_optimization_resume(&request, &root.0).unwrap(),
            initial
        );
    }
}
