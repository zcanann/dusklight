//! Sealed authority and crash-safe journal for unattended native goal learning.
//!
//! The runner is deliberately split from this state machine. Native execution
//! may only append a phase after it has independently validated the referenced
//! artifacts; replaying this journal then reproduces the exact generation and
//! parent-corpus lineage without launching the game.

use crate::native_residual_campaign::{
    NativeResidualCampaignError, NativeResidualExecutionBinding,
};
use crate::optimization_request::OptimizationRequest;
use dusklight_automation_contracts::artifact::Digest;
use dusklight_evidence::native_episode_shard::NativeEpisodeShard;
use dusklight_harness_contracts::objective_suite::ArtifactReference;
use dusklight_learning::native_goal_frozen_policy::NativeGoalFrozenPolicyConfig;
use dusklight_learning::native_goal_reachability::NativeGoalReachabilityConfig;
use dusklight_learning::native_goal_trajectory::NativeGoalTrajectoryConfig;
use dusklight_learning::native_replay_corpus::NativeReplayCorpus;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{Seek, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const NATIVE_GOAL_LEARNING_LOOP_REQUEST_SCHEMA_V1: &str =
    "dusklight-native-goal-learning-loop-request/v1";
pub const NATIVE_GOAL_LEARNING_LOOP_RECORD_SCHEMA_V2: &str =
    "dusklight-native-goal-learning-loop-record/v2";
pub const NATIVE_GOAL_LEARNING_LOOP_STATE_SCHEMA_V2: &str =
    "dusklight-native-goal-learning-loop-state/v2";

const MIN_GENERATIONS: u16 = 3;
const MAX_GENERATIONS: u16 = 1_024;
const MAX_ROLLOUTS_PER_GENERATION: u16 = 256;
const MAX_SIMULATED_TICKS: u64 = 1_000_000_000_000;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeGoalLearningLoopRequest {
    pub schema: String,
    pub content_sha256: Digest,
    pub optimization_request_sha256: Digest,
    pub native_execution_sha256: Digest,
    pub initial_replay_corpus: ArtifactReference,
    pub initial_episode_shards: Vec<ArtifactReference>,
    pub generation_limit: u16,
    pub rollouts_per_generation: u16,
    pub simulated_tick_budget: u64,
    pub trajectory: NativeGoalTrajectoryConfig,
    pub reachability: NativeGoalReachabilityConfig,
    pub policy: NativeGoalFrozenPolicyConfig,
    pub resume: NativeGoalLearningLoopResume,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeGoalLearningLoopResume {
    pub journal_path: String,
    pub state_path: String,
    pub artifact_root: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeGoalLearningLoopRecord {
    pub schema: String,
    pub request_sha256: Digest,
    pub sequence: u64,
    pub previous_record_sha256: Digest,
    pub event: NativeGoalLearningLoopEvent,
    pub record_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
// These are durable JSONL records, not a hot in-memory message queue. Keeping
// artifact references inline makes the sealed event schema inspectable and
// avoids allocation-dependent representation solely to equalize enum sizes.
#[allow(clippy::large_enum_variant)]
pub enum NativeGoalLearningLoopEvent {
    GenerationPrepared {
        generation: u16,
        input_corpus_sha256: Digest,
        dataset_sha256: Digest,
        reachability_model_sha256: Digest,
        policy_manifest_sha256: Digest,
        frozen_model_xxh3_128: String,
        dataset: ArtifactReference,
        reachability_model: ArtifactReference,
        policy_manifest: ArtifactReference,
        frozen_model: ArtifactReference,
        native_batches: Vec<ArtifactReference>,
    },
    GenerationExecuted {
        generation: u16,
        prepared_record_sha256: Digest,
        native_results: Vec<ArtifactReference>,
        episode_shards: Vec<ArtifactReference>,
        reinference_reports: Vec<ArtifactReference>,
        realized_tapes: Vec<ArtifactReference>,
        simulated_ticks: u64,
        successes: u16,
    },
    GenerationCommitted {
        generation: u16,
        executed_record_sha256: Digest,
        output_corpus_sha256: Digest,
        output_corpus: ArtifactReference,
        entries: u64,
        transitions: u64,
    },
    LoopStopped {
        next_generation: u16,
        reason: NativeGoalLearningStopReason,
        active_corpus_sha256: Digest,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        evidence: Option<ArtifactReference>,
        proposal_source: NativeGoalLearningProposalSource,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeGoalLearningStopReason {
    GenerationLimitReached,
    SimulatedTickBudgetReached,
    HeldOutReachabilityRejected,
    HeldOutPolicyRejected,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeGoalLearningProposalSource {
    FrozenGoalPolicy,
    RetainedBaseline,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeGoalLearningGenerationState {
    pub generation: u16,
    pub input_corpus_sha256: Digest,
    pub dataset_sha256: Digest,
    pub reachability_model_sha256: Digest,
    pub policy_manifest_sha256: Digest,
    pub frozen_model_xxh3_128: String,
    pub dataset: ArtifactReference,
    pub reachability_model: ArtifactReference,
    pub policy_manifest: ArtifactReference,
    pub frozen_model: ArtifactReference,
    pub native_batches: Vec<ArtifactReference>,
    pub prepared_record_sha256: Digest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_results: Option<Vec<ArtifactReference>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode_shards: Option<Vec<ArtifactReference>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reinference_reports: Option<Vec<ArtifactReference>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realized_tapes: Option<Vec<ArtifactReference>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub simulated_ticks: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub successes: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executed_record_sha256: Option<Digest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_corpus_sha256: Option<Digest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_corpus: Option<ArtifactReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_entries: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_transitions: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub committed_record_sha256: Option<Digest>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeGoalLearningStopState {
    pub next_generation: u16,
    pub reason: NativeGoalLearningStopReason,
    pub active_corpus_sha256: Digest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<ArtifactReference>,
    pub proposal_source: NativeGoalLearningProposalSource,
    pub record_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeGoalLearningLoopState {
    pub schema: String,
    pub request_sha256: Digest,
    pub initial_corpus_sha256: Digest,
    pub journal_sha256: Digest,
    pub valid_journal_bytes: u64,
    pub record_count: u64,
    pub last_record_sha256: Digest,
    pub next_sequence: u64,
    pub generations: Vec<NativeGoalLearningGenerationState>,
    pub committed_generations: u16,
    pub charged_simulated_ticks: u64,
    pub active_corpus_sha256: Digest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped: Option<NativeGoalLearningStopState>,
    pub state_sha256: Digest,
}

impl NativeGoalLearningLoopRequest {
    #[allow(clippy::too_many_arguments)]
    pub fn seal(
        optimization: &OptimizationRequest,
        execution: &NativeResidualExecutionBinding,
        initial_replay_corpus: ArtifactReference,
        mut initial_episode_shards: Vec<ArtifactReference>,
        generation_limit: u16,
        rollouts_per_generation: u16,
        simulated_tick_budget: u64,
        trajectory: NativeGoalTrajectoryConfig,
        reachability: NativeGoalReachabilityConfig,
        policy: NativeGoalFrozenPolicyConfig,
        resume: NativeGoalLearningLoopResume,
    ) -> Result<Self, NativeGoalLearningLoopError> {
        initial_episode_shards.sort_by(|left, right| {
            (left.sha256, left.path.as_str()).cmp(&(right.sha256, right.path.as_str()))
        });
        let mut request = Self {
            schema: NATIVE_GOAL_LEARNING_LOOP_REQUEST_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            optimization_request_sha256: optimization.content_sha256,
            native_execution_sha256: execution.content_sha256,
            initial_replay_corpus,
            initial_episode_shards,
            generation_limit,
            rollouts_per_generation,
            simulated_tick_budget,
            trajectory,
            reachability,
            policy,
            resume,
        };
        request.content_sha256 = request.identity()?;
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<(), NativeGoalLearningLoopError> {
        self.trajectory.validate().map_err(loop_error)?;
        self.reachability.validate().map_err(loop_error)?;
        self.policy.validate().map_err(loop_error)?;
        validate_artifact_shape("initial replay corpus", &self.initial_replay_corpus)?;
        if self.schema != NATIVE_GOAL_LEARNING_LOOP_REQUEST_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.optimization_request_sha256 == Digest::ZERO
            || self.native_execution_sha256 == Digest::ZERO
            || self.generation_limit < MIN_GENERATIONS
            || self.generation_limit > MAX_GENERATIONS
            || self.rollouts_per_generation == 0
            || self.rollouts_per_generation > MAX_ROLLOUTS_PER_GENERATION
            || self.simulated_tick_budget == 0
            || self.simulated_tick_budget > MAX_SIMULATED_TICKS
            || self.initial_episode_shards.is_empty()
            || self.content_sha256 != self.identity()?
        {
            return Err(loop_message(
                "native goal learning-loop request is invalid or detached",
            ));
        }
        let mut prior = None;
        for reference in &self.initial_episode_shards {
            validate_artifact_shape("initial native episode shard", reference)?;
            let key = (reference.sha256, reference.path.as_str());
            if prior.is_some_and(|prior| prior >= key) {
                return Err(loop_message(
                    "initial native episode shards are not uniquely canonical",
                ));
            }
            prior = Some(key);
        }
        let paths = [
            self.resume.journal_path.as_str(),
            self.resume.state_path.as_str(),
            self.resume.artifact_root.as_str(),
        ];
        for (label, path) in [
            ("learning-loop journal", paths[0]),
            ("learning-loop state", paths[1]),
            ("learning-loop artifact root", paths[2]),
        ] {
            validate_relative_path(label, path)?;
        }
        if paths[0] == paths[1]
            || paths[0] == paths[2]
            || paths[1] == paths[2]
            || paths[0].starts_with(&format!("{}/", paths[2]))
            || paths[1].starts_with(&format!("{}/", paths[2]))
        {
            return Err(loop_message(
                "learning-loop journal, state, and artifact paths overlap",
            ));
        }
        Ok(())
    }

    pub fn validate_files(
        &self,
        repository_root: &Path,
        optimization: &OptimizationRequest,
        execution: &NativeResidualExecutionBinding,
    ) -> Result<NativeGoalLearningLoopValidationReport, NativeGoalLearningLoopError> {
        self.validate()?;
        let root = canonical_root(repository_root)?;
        optimization.validate_files(&root).map_err(loop_error)?;
        execution
            .validate_files(&root, optimization)
            .map_err(loop_error)?;
        if self.optimization_request_sha256 != optimization.content_sha256
            || self.native_execution_sha256 != execution.content_sha256
        {
            return Err(loop_message(
                "learning-loop request differs from its optimization or native execution authority",
            ));
        }
        let corpus_bytes = read_reference(&root, &self.initial_replay_corpus)?;
        let corpus: NativeReplayCorpus =
            serde_json::from_slice(&corpus_bytes).map_err(loop_error)?;
        corpus.validate().map_err(loop_error)?;
        let mut shard_identities = BTreeSet::new();
        let mut episode_count = 0_u64;
        for reference in &self.initial_episode_shards {
            let path = referenced_path(&root, reference)?;
            let shard = NativeEpisodeShard::read(path).map_err(loop_error)?;
            if shard.content_sha256 != reference.sha256 {
                return Err(loop_message(
                    "initial episode shard content identity differs from its artifact reference",
                ));
            }
            shard_identities.insert(shard.content_sha256);
            episode_count = episode_count
                .checked_add(shard.episodes.len() as u64)
                .ok_or_else(|| loop_message("initial episode count overflowed"))?;
        }
        if corpus
            .entries
            .iter()
            .any(|entry| !shard_identities.contains(&entry.shard_sha256))
        {
            return Err(loop_message(
                "initial replay corpus references an episode shard outside the sealed input set",
            ));
        }
        let minimum_ticks = u64::from(self.generation_limit)
            .checked_mul(u64::from(self.rollouts_per_generation))
            .and_then(|count| count.checked_mul(optimization.budgets.exploration_horizon_ticks))
            .ok_or_else(|| loop_message("learning-loop minimum tick budget overflowed"))?;
        if self.simulated_tick_budget < minimum_ticks {
            return Err(loop_message(
                "learning-loop simulated tick budget cannot cover its declared generations and rollouts",
            ));
        }
        Ok(NativeGoalLearningLoopValidationReport {
            schema: NATIVE_GOAL_LEARNING_LOOP_REQUEST_SCHEMA_V1,
            request_sha256: self.content_sha256,
            optimization_request_sha256: optimization.content_sha256,
            native_execution_sha256: execution.content_sha256,
            initial_corpus_sha256: corpus.corpus_sha256,
            initial_corpus_generation: corpus.generation,
            initial_entries: corpus.report.entries as u64,
            initial_episode_shards: self.initial_episode_shards.len() as u64,
            initial_episodes: episode_count,
            generation_limit: self.generation_limit,
            rollouts_per_generation: self.rollouts_per_generation,
            simulated_tick_budget: self.simulated_tick_budget,
        })
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, NativeGoalLearningLoopError> {
        pretty_json(self)
    }

    fn identity(&self) -> Result<Digest, NativeGoalLearningLoopError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        canonical_digest(
            b"dusklight.native-goal-learning-loop-request/v1\0",
            &canonical,
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeGoalLearningLoopValidationReport {
    pub schema: &'static str,
    pub request_sha256: Digest,
    pub optimization_request_sha256: Digest,
    pub native_execution_sha256: Digest,
    pub initial_corpus_sha256: Digest,
    pub initial_corpus_generation: u32,
    pub initial_entries: u64,
    pub initial_episode_shards: u64,
    pub initial_episodes: u64,
    pub generation_limit: u16,
    pub rollouts_per_generation: u16,
    pub simulated_tick_budget: u64,
}

impl NativeGoalLearningLoopState {
    pub fn validate(&self) -> Result<(), NativeGoalLearningLoopError> {
        if self.schema != NATIVE_GOAL_LEARNING_LOOP_STATE_SCHEMA_V2
            || self.request_sha256 == Digest::ZERO
            || self.initial_corpus_sha256 == Digest::ZERO
            || self.journal_sha256 == Digest::ZERO
            || self.next_sequence != self.record_count.saturating_add(1)
            || self.committed_generations
                != self
                    .generations
                    .iter()
                    .filter(|generation| generation.committed_record_sha256.is_some())
                    .count() as u16
            || self
                .generations
                .windows(2)
                .any(|pair| pair[0].generation.checked_add(1) != Some(pair[1].generation))
            || self
                .generations
                .first()
                .is_some_and(|generation| generation.generation != 1)
            || self.state_sha256 != self.identity()?
        {
            return Err(loop_message(
                "native goal learning-loop state or seal is invalid",
            ));
        }
        let charged = self
            .generations
            .iter()
            .try_fold(0_u64, |total, generation| {
                total.checked_add(generation.simulated_ticks.unwrap_or(0))
            });
        if charged != Some(self.charged_simulated_ticks) {
            return Err(loop_message(
                "learning-loop state does not charge every executed rollout tick",
            ));
        }
        let active = self
            .generations
            .iter()
            .rev()
            .find_map(|generation| generation.output_corpus_sha256)
            .unwrap_or(self.initial_corpus_sha256);
        if self.active_corpus_sha256 != active
            || self
                .stopped
                .as_ref()
                .is_some_and(|stopped| stopped.active_corpus_sha256 != active)
        {
            return Err(loop_message(
                "learning-loop active corpus differs from committed generation lineage",
            ));
        }
        for generation in &self.generations {
            validate_generation_state(generation)?;
        }
        Ok(())
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, NativeGoalLearningLoopError> {
        pretty_json(self)
    }

    fn identity(&self) -> Result<Digest, NativeGoalLearningLoopError> {
        let mut canonical = self.clone();
        canonical.state_sha256 = Digest::ZERO;
        canonical_digest(
            b"dusklight.native-goal-learning-loop-state/v2\0",
            &canonical,
        )
    }
}

pub fn initialize_native_goal_learning_loop(
    request: &NativeGoalLearningLoopRequest,
    repository_root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
) -> Result<NativeGoalLearningLoopState, NativeGoalLearningLoopError> {
    let report = request.validate_files(repository_root, optimization, execution)?;
    let root = canonical_root(repository_root)?;
    let journal_path = output_path(&root, &request.resume.journal_path)?;
    let state_path = output_path(&root, &request.resume.state_path)?;
    if journal_path.exists() || state_path.exists() {
        return Err(loop_message(
            "native goal learning-loop journal or state already exists",
        ));
    }
    create_parent(&journal_path)?;
    let journal = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&journal_path)
        .map_err(NativeGoalLearningLoopError::io)?;
    journal
        .sync_all()
        .map_err(NativeGoalLearningLoopError::io)?;
    sync_parent(&journal_path)?;
    let state = fold_journal(request, &root, report.initial_corpus_sha256)?;
    write_state_atomically(&state_path, &state)?;
    Ok(state)
}

pub fn load_native_goal_learning_loop(
    request: &NativeGoalLearningLoopRequest,
    repository_root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
) -> Result<NativeGoalLearningLoopState, NativeGoalLearningLoopError> {
    let report = request.validate_files(repository_root, optimization, execution)?;
    let root = canonical_root(repository_root)?;
    let state = fold_journal(request, &root, report.initial_corpus_sha256)?;
    write_state_atomically(&output_path(&root, &request.resume.state_path)?, &state)?;
    Ok(state)
}

pub fn append_native_goal_learning_loop_event(
    request: &NativeGoalLearningLoopRequest,
    repository_root: &Path,
    optimization: &OptimizationRequest,
    execution: &NativeResidualExecutionBinding,
    event: NativeGoalLearningLoopEvent,
) -> Result<NativeGoalLearningLoopState, NativeGoalLearningLoopError> {
    let current =
        load_native_goal_learning_loop(request, repository_root, optimization, execution)?;
    let root = canonical_root(repository_root)?;
    validate_event_artifacts(&root, &event)?;
    let mut preview = current.clone();
    apply_event(request, &mut preview, &event, Digest([1; 32]))?;
    let mut record = NativeGoalLearningLoopRecord {
        schema: NATIVE_GOAL_LEARNING_LOOP_RECORD_SCHEMA_V2.into(),
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
        .write(true)
        .open(&journal_path)
        .map_err(NativeGoalLearningLoopError::io)?;
    journal
        .set_len(current.valid_journal_bytes)
        .map_err(NativeGoalLearningLoopError::io)?;
    journal
        .seek(std::io::SeekFrom::Start(current.valid_journal_bytes))
        .map_err(NativeGoalLearningLoopError::io)?;
    journal
        .write_all(&bytes)
        .map_err(NativeGoalLearningLoopError::io)?;
    journal
        .sync_all()
        .map_err(NativeGoalLearningLoopError::io)?;
    sync_parent(&journal_path)?;
    let report = request.validate_files(&root, optimization, execution)?;
    let state = fold_journal(request, &root, report.initial_corpus_sha256)?;
    write_state_atomically(&output_path(&root, &request.resume.state_path)?, &state)?;
    Ok(state)
}

fn fold_journal(
    request: &NativeGoalLearningLoopRequest,
    root: &Path,
    initial_corpus_sha256: Digest,
) -> Result<NativeGoalLearningLoopState, NativeGoalLearningLoopError> {
    let journal_path = output_path(root, &request.resume.journal_path)?;
    let bytes = fs::read(&journal_path).map_err(NativeGoalLearningLoopError::io)?;
    let valid_len = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |offset| offset + 1);
    let valid = &bytes[..valid_len];
    let mut state = NativeGoalLearningLoopState {
        schema: NATIVE_GOAL_LEARNING_LOOP_STATE_SCHEMA_V2.into(),
        request_sha256: request.content_sha256,
        initial_corpus_sha256,
        journal_sha256: sha256(valid),
        valid_journal_bytes: valid_len as u64,
        record_count: 0,
        last_record_sha256: Digest::ZERO,
        next_sequence: 1,
        generations: Vec::new(),
        committed_generations: 0,
        charged_simulated_ticks: 0,
        active_corpus_sha256: initial_corpus_sha256,
        stopped: None,
        state_sha256: Digest::ZERO,
    };
    for line in valid
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let record: NativeGoalLearningLoopRecord =
            serde_json::from_slice(line).map_err(loop_error)?;
        if record.schema != NATIVE_GOAL_LEARNING_LOOP_RECORD_SCHEMA_V2
            || record.request_sha256 != request.content_sha256
            || record.sequence != state.next_sequence
            || record.previous_record_sha256 != state.last_record_sha256
            || record.record_sha256 == Digest::ZERO
            || record.record_sha256 != record_identity(&record)?
        {
            return Err(loop_message(
                "native goal learning-loop journal chain is invalid",
            ));
        }
        validate_event_artifacts(root, &record.event)?;
        apply_event(request, &mut state, &record.event, record.record_sha256)?;
        state.record_count += 1;
        state.last_record_sha256 = record.record_sha256;
        state.next_sequence += 1;
    }
    state.state_sha256 = state.identity()?;
    state.validate()?;
    Ok(state)
}

fn apply_event(
    request: &NativeGoalLearningLoopRequest,
    state: &mut NativeGoalLearningLoopState,
    event: &NativeGoalLearningLoopEvent,
    record_sha256: Digest,
) -> Result<(), NativeGoalLearningLoopError> {
    if state.stopped.is_some() {
        return Err(loop_message(
            "learning-loop journal continues after a stop event",
        ));
    }
    match event {
        NativeGoalLearningLoopEvent::GenerationPrepared {
            generation,
            input_corpus_sha256,
            dataset_sha256,
            reachability_model_sha256,
            policy_manifest_sha256,
            frozen_model_xxh3_128,
            dataset,
            reachability_model,
            policy_manifest,
            frozen_model,
            native_batches,
        } => {
            let expected = state.generations.len() as u16 + 1;
            if *generation != expected
                || *generation > request.generation_limit
                || *input_corpus_sha256 != state.active_corpus_sha256
                || [
                    *dataset_sha256,
                    *reachability_model_sha256,
                    *policy_manifest_sha256,
                ]
                .contains(&Digest::ZERO)
                || !lower_hex(frozen_model_xxh3_128, 32)
                || native_batches.len() != usize::from(request.rollouts_per_generation)
                || state
                    .generations
                    .last()
                    .is_some_and(|prior| prior.committed_record_sha256.is_none())
            {
                return Err(loop_message(
                    "learning-loop prepared generation is out of order or detached",
                ));
            }
            state.generations.push(NativeGoalLearningGenerationState {
                generation: *generation,
                input_corpus_sha256: *input_corpus_sha256,
                dataset_sha256: *dataset_sha256,
                reachability_model_sha256: *reachability_model_sha256,
                policy_manifest_sha256: *policy_manifest_sha256,
                frozen_model_xxh3_128: frozen_model_xxh3_128.clone(),
                dataset: dataset.clone(),
                reachability_model: reachability_model.clone(),
                policy_manifest: policy_manifest.clone(),
                frozen_model: frozen_model.clone(),
                native_batches: native_batches.clone(),
                prepared_record_sha256: record_sha256,
                native_results: None,
                episode_shards: None,
                reinference_reports: None,
                realized_tapes: None,
                simulated_ticks: None,
                successes: None,
                executed_record_sha256: None,
                output_corpus_sha256: None,
                output_corpus: None,
                output_entries: None,
                output_transitions: None,
                committed_record_sha256: None,
            });
        }
        NativeGoalLearningLoopEvent::GenerationExecuted {
            generation,
            prepared_record_sha256,
            native_results,
            episode_shards,
            reinference_reports,
            realized_tapes,
            simulated_ticks,
            successes,
        } => {
            let current = state.generations.last_mut().ok_or_else(|| {
                loop_message("learning-loop execution has no prepared generation")
            })?;
            if current.generation != *generation
                || current.prepared_record_sha256 != *prepared_record_sha256
                || current.executed_record_sha256.is_some()
                || native_results.len() != usize::from(request.rollouts_per_generation)
                || episode_shards.len() != native_results.len()
                || reinference_reports.len() != native_results.len()
                || realized_tapes.len() != native_results.len()
                || *simulated_ticks == 0
                || *successes > request.rollouts_per_generation
                || state
                    .charged_simulated_ticks
                    .checked_add(*simulated_ticks)
                    .is_none_or(|ticks| ticks > request.simulated_tick_budget)
            {
                return Err(loop_message(
                    "learning-loop executed generation is invalid or detached",
                ));
            }
            current.native_results = Some(native_results.clone());
            current.episode_shards = Some(episode_shards.clone());
            current.reinference_reports = Some(reinference_reports.clone());
            current.realized_tapes = Some(realized_tapes.clone());
            current.simulated_ticks = Some(*simulated_ticks);
            current.successes = Some(*successes);
            current.executed_record_sha256 = Some(record_sha256);
            state.charged_simulated_ticks += *simulated_ticks;
        }
        NativeGoalLearningLoopEvent::GenerationCommitted {
            generation,
            executed_record_sha256,
            output_corpus_sha256,
            output_corpus,
            entries,
            transitions,
        } => {
            let current = state
                .generations
                .last_mut()
                .ok_or_else(|| loop_message("learning-loop commit has no executed generation"))?;
            if current.generation != *generation
                || current.executed_record_sha256 != Some(*executed_record_sha256)
                || current.committed_record_sha256.is_some()
                || *output_corpus_sha256 == Digest::ZERO
                || *output_corpus_sha256 == current.input_corpus_sha256
                || *entries == 0
                || *transitions == 0
            {
                return Err(loop_message(
                    "learning-loop committed generation is invalid or detached",
                ));
            }
            current.output_corpus_sha256 = Some(*output_corpus_sha256);
            current.output_corpus = Some(output_corpus.clone());
            current.output_entries = Some(*entries);
            current.output_transitions = Some(*transitions);
            current.committed_record_sha256 = Some(record_sha256);
            state.committed_generations += 1;
            state.active_corpus_sha256 = *output_corpus_sha256;
        }
        NativeGoalLearningLoopEvent::LoopStopped {
            next_generation,
            reason,
            active_corpus_sha256,
            evidence,
            proposal_source,
        } => {
            let expected_next = state.committed_generations.saturating_add(1);
            let pending = state
                .generations
                .last()
                .is_some_and(|generation| generation.committed_record_sha256.is_none());
            if pending
                || *next_generation != expected_next
                || *active_corpus_sha256 != state.active_corpus_sha256
                || (*reason == NativeGoalLearningStopReason::GenerationLimitReached
                    && state.committed_generations != request.generation_limit)
                || (*reason == NativeGoalLearningStopReason::SimulatedTickBudgetReached
                    && state.charged_simulated_ticks < request.simulated_tick_budget)
                || (matches!(
                    reason,
                    NativeGoalLearningStopReason::HeldOutReachabilityRejected
                        | NativeGoalLearningStopReason::HeldOutPolicyRejected
                ) != evidence.is_some())
                || (matches!(
                    reason,
                    NativeGoalLearningStopReason::HeldOutReachabilityRejected
                        | NativeGoalLearningStopReason::HeldOutPolicyRejected
                ) != (*proposal_source == NativeGoalLearningProposalSource::RetainedBaseline))
            {
                return Err(loop_message(
                    "learning-loop stop event is premature or detached",
                ));
            }
            state.stopped = Some(NativeGoalLearningStopState {
                next_generation: *next_generation,
                reason: *reason,
                active_corpus_sha256: *active_corpus_sha256,
                evidence: evidence.clone(),
                proposal_source: *proposal_source,
                record_sha256,
            });
        }
    }
    Ok(())
}

fn validate_generation_state(
    generation: &NativeGoalLearningGenerationState,
) -> Result<(), NativeGoalLearningLoopError> {
    if generation.generation == 0
        || generation.input_corpus_sha256 == Digest::ZERO
        || generation.dataset_sha256 == Digest::ZERO
        || generation.reachability_model_sha256 == Digest::ZERO
        || generation.policy_manifest_sha256 == Digest::ZERO
        || !lower_hex(&generation.frozen_model_xxh3_128, 32)
        || generation.prepared_record_sha256 == Digest::ZERO
    {
        return Err(loop_message("learning-loop generation state is incomplete"));
    }
    let executed = generation.executed_record_sha256.is_some();
    if [
        generation.native_results.is_some(),
        generation.episode_shards.is_some(),
        generation.reinference_reports.is_some(),
        generation.realized_tapes.is_some(),
        generation.simulated_ticks.is_some(),
        generation.successes.is_some(),
    ]
    .iter()
    .any(|present| *present != executed)
    {
        return Err(loop_message(
            "learning-loop execution phase is only partially materialized",
        ));
    }
    let committed = generation.committed_record_sha256.is_some();
    if [
        generation.output_corpus_sha256.is_some(),
        generation.output_corpus.is_some(),
        generation.output_entries.is_some(),
        generation.output_transitions.is_some(),
    ]
    .iter()
    .any(|present| *present != committed)
        || (committed && !executed)
    {
        return Err(loop_message(
            "learning-loop commit phase is only partially materialized",
        ));
    }
    Ok(())
}

fn validate_event_artifacts(
    root: &Path,
    event: &NativeGoalLearningLoopEvent,
) -> Result<(), NativeGoalLearningLoopError> {
    let references: Vec<&ArtifactReference> = match event {
        NativeGoalLearningLoopEvent::GenerationPrepared {
            dataset,
            reachability_model,
            policy_manifest,
            frozen_model,
            native_batches,
            ..
        } => {
            let mut references = vec![dataset, reachability_model, policy_manifest, frozen_model];
            references.extend(native_batches);
            references
        }
        NativeGoalLearningLoopEvent::GenerationExecuted {
            native_results,
            episode_shards,
            reinference_reports,
            realized_tapes,
            ..
        } => native_results
            .iter()
            .chain(episode_shards)
            .chain(reinference_reports)
            .chain(realized_tapes)
            .collect(),
        NativeGoalLearningLoopEvent::GenerationCommitted { output_corpus, .. } => {
            vec![output_corpus]
        }
        NativeGoalLearningLoopEvent::LoopStopped { evidence, .. } => evidence.iter().collect(),
    };
    for reference in references {
        validate_artifact_shape("learning-loop event artifact", reference)?;
        read_reference(root, reference)?;
    }
    Ok(())
}

fn record_identity(
    record: &NativeGoalLearningLoopRecord,
) -> Result<Digest, NativeGoalLearningLoopError> {
    let mut canonical = record.clone();
    canonical.record_sha256 = Digest::ZERO;
    canonical_digest(
        b"dusklight.native-goal-learning-loop-record/v2\0",
        &canonical,
    )
}

fn record_bytes(
    record: &NativeGoalLearningLoopRecord,
) -> Result<Vec<u8>, NativeGoalLearningLoopError> {
    let mut bytes = serde_json::to_vec(record).map_err(loop_error)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn validate_artifact_shape(
    label: &str,
    reference: &ArtifactReference,
) -> Result<(), NativeGoalLearningLoopError> {
    validate_relative_path(label, &reference.path)?;
    if reference.sha256 == Digest::ZERO {
        return Err(loop_message(format!("{label} has a zero digest")));
    }
    Ok(())
}

fn validate_relative_path(label: &str, value: &str) -> Result<(), NativeGoalLearningLoopError> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(loop_message(format!(
            "{label} path is not repository relative"
        )));
    }
    Ok(())
}

fn read_reference(
    root: &Path,
    reference: &ArtifactReference,
) -> Result<Vec<u8>, NativeGoalLearningLoopError> {
    validate_artifact_shape("artifact", reference)?;
    let bytes = fs::read(root.join(&reference.path)).map_err(NativeGoalLearningLoopError::io)?;
    if sha256(&bytes) != reference.sha256 {
        return Err(loop_message("learning-loop artifact digest differs"));
    }
    Ok(bytes)
}

fn referenced_path(
    root: &Path,
    reference: &ArtifactReference,
) -> Result<PathBuf, NativeGoalLearningLoopError> {
    read_reference(root, reference)?;
    Ok(root.join(&reference.path))
}

fn output_path(root: &Path, relative: &str) -> Result<PathBuf, NativeGoalLearningLoopError> {
    validate_relative_path("learning-loop output", relative)?;
    Ok(root.join(relative))
}

fn canonical_root(root: &Path) -> Result<PathBuf, NativeGoalLearningLoopError> {
    root.canonicalize().map_err(NativeGoalLearningLoopError::io)
}

fn create_parent(path: &Path) -> Result<(), NativeGoalLearningLoopError> {
    let parent = path
        .parent()
        .ok_or_else(|| loop_message("learning-loop output has no parent"))?;
    fs::create_dir_all(parent).map_err(NativeGoalLearningLoopError::io)
}

fn write_state_atomically(
    path: &Path,
    state: &NativeGoalLearningLoopState,
) -> Result<(), NativeGoalLearningLoopError> {
    create_parent(path)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(loop_error)?
        .as_nanos();
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| loop_message("learning-loop state filename is invalid"))?;
    let temporary =
        path.with_file_name(format!(".{file_name}.{}.{}.tmp", std::process::id(), nonce));
    let bytes = state.to_pretty_json()?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(NativeGoalLearningLoopError::io)?;
    output
        .write_all(&bytes)
        .map_err(NativeGoalLearningLoopError::io)?;
    output.sync_all().map_err(NativeGoalLearningLoopError::io)?;
    fs::rename(&temporary, path).map_err(NativeGoalLearningLoopError::io)?;
    sync_parent(path)
}

fn sync_parent(path: &Path) -> Result<(), NativeGoalLearningLoopError> {
    fs::File::open(
        path.parent()
            .ok_or_else(|| loop_message("learning-loop output has no parent"))?,
    )
    .and_then(|directory| directory.sync_all())
    .map_err(NativeGoalLearningLoopError::io)
}

fn pretty_json(value: &impl Serialize) -> Result<Vec<u8>, NativeGoalLearningLoopError> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(loop_error)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn canonical_digest(
    domain: &[u8],
    value: &impl Serialize,
) -> Result<Digest, NativeGoalLearningLoopError> {
    let bytes = serde_json::to_vec(value).map_err(loop_error)?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

fn sha256(bytes: &[u8]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Digest(hasher.finalize().into())
}

fn lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Debug)]
pub struct NativeGoalLearningLoopError {
    message: String,
    source: Option<std::io::Error>,
}

impl NativeGoalLearningLoopError {
    fn io(source: std::io::Error) -> Self {
        Self {
            message: source.to_string(),
            source: Some(source),
        }
    }
}

fn loop_message(message: impl Into<String>) -> NativeGoalLearningLoopError {
    NativeGoalLearningLoopError {
        message: message.into(),
        source: None,
    }
}

fn loop_error(error: impl fmt::Display) -> NativeGoalLearningLoopError {
    loop_message(error.to_string())
}

impl fmt::Display for NativeGoalLearningLoopError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for NativeGoalLearningLoopError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_ref()
            .map(|source| source as &(dyn Error + 'static))
    }
}

impl From<NativeResidualCampaignError> for NativeGoalLearningLoopError {
    fn from(error: NativeResidualCampaignError) -> Self {
        loop_error(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

    fn test_root() -> PathBuf {
        let nonce = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "dusklight-native-goal-loop-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("campaign/artifacts")).unwrap();
        root.canonicalize().unwrap()
    }

    fn artifact(root: &Path, name: &str, payload: &[u8]) -> ArtifactReference {
        let relative = format!("campaign/artifacts/{name}");
        fs::write(root.join(&relative), payload).unwrap();
        ArtifactReference {
            path: relative,
            sha256: sha256(payload),
        }
    }

    fn request(root: &Path) -> NativeGoalLearningLoopRequest {
        let initial_corpus = artifact(root, "initial-corpus.json", b"initial corpus");
        let initial_shard = artifact(root, "initial.dseps", b"initial shard");
        let mut request = NativeGoalLearningLoopRequest {
            schema: NATIVE_GOAL_LEARNING_LOOP_REQUEST_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            optimization_request_sha256: Digest([1; 32]),
            native_execution_sha256: Digest([2; 32]),
            initial_replay_corpus: initial_corpus,
            initial_episode_shards: vec![initial_shard],
            generation_limit: 3,
            rollouts_per_generation: 2,
            simulated_tick_budget: 1_000,
            trajectory: NativeGoalTrajectoryConfig::default(),
            reachability: NativeGoalReachabilityConfig::default(),
            policy: NativeGoalFrozenPolicyConfig::default(),
            resume: NativeGoalLearningLoopResume {
                journal_path: "campaign/resume/events.jsonl".into(),
                state_path: "campaign/resume/state.json".into(),
                artifact_root: "campaign/artifacts".into(),
            },
        };
        request.content_sha256 = request.identity().unwrap();
        request.validate().unwrap();
        request
    }

    fn initialize_journal(root: &Path, request: &NativeGoalLearningLoopRequest) {
        let journal = root.join(&request.resume.journal_path);
        fs::create_dir_all(journal.parent().unwrap()).unwrap();
        fs::write(journal, []).unwrap();
    }

    fn append_test_event(
        root: &Path,
        request: &NativeGoalLearningLoopRequest,
        initial_corpus_sha256: Digest,
        event: NativeGoalLearningLoopEvent,
    ) -> NativeGoalLearningLoopState {
        let state = fold_journal(request, root, initial_corpus_sha256).unwrap();
        let mut record = NativeGoalLearningLoopRecord {
            schema: NATIVE_GOAL_LEARNING_LOOP_RECORD_SCHEMA_V2.into(),
            request_sha256: request.content_sha256,
            sequence: state.next_sequence,
            previous_record_sha256: state.last_record_sha256,
            event,
            record_sha256: Digest::ZERO,
        };
        record.record_sha256 = record_identity(&record).unwrap();
        let mut journal = OpenOptions::new()
            .append(true)
            .open(root.join(&request.resume.journal_path))
            .unwrap();
        journal.write_all(&record_bytes(&record).unwrap()).unwrap();
        journal.sync_all().unwrap();
        fold_journal(request, root, initial_corpus_sha256).unwrap()
    }

    fn generation_artifacts(
        root: &Path,
        generation: u16,
        phase: &str,
        count: usize,
    ) -> Vec<ArtifactReference> {
        (0..count)
            .map(|index| {
                let name = format!("generation-{generation}-{phase}-{index}.bin");
                artifact(root, &name, name.as_bytes())
            })
            .collect()
    }

    #[test]
    fn three_generations_fold_with_exact_phase_and_parent_lineage() {
        let root = test_root();
        let request = request(&root);
        initialize_journal(&root, &request);
        let initial = Digest([3; 32]);
        let mut active = initial;
        let mut state = fold_journal(&request, &root, initial).unwrap();
        assert_eq!(state.next_sequence, 1);

        for generation in 1..=3 {
            let prepared = generation_artifacts(&root, generation, "prepared", 5);
            let batches = generation_artifacts(&root, generation, "batch", 2);
            state = append_test_event(
                &root,
                &request,
                initial,
                NativeGoalLearningLoopEvent::GenerationPrepared {
                    generation,
                    input_corpus_sha256: active,
                    dataset_sha256: Digest([10 + generation as u8; 32]),
                    reachability_model_sha256: Digest([20 + generation as u8; 32]),
                    policy_manifest_sha256: Digest([30 + generation as u8; 32]),
                    frozen_model_xxh3_128: format!("{generation:032x}"),
                    dataset: prepared[0].clone(),
                    reachability_model: prepared[1].clone(),
                    policy_manifest: prepared[2].clone(),
                    frozen_model: prepared[3].clone(),
                    native_batches: batches,
                },
            );
            let prepared_record_sha256 = state.generations.last().unwrap().prepared_record_sha256;
            let results = generation_artifacts(&root, generation, "result", 2);
            let shards = generation_artifacts(&root, generation, "shard", 2);
            let reinference = generation_artifacts(&root, generation, "reinference", 2);
            let realized = generation_artifacts(&root, generation, "realized", 2);
            state = append_test_event(
                &root,
                &request,
                initial,
                NativeGoalLearningLoopEvent::GenerationExecuted {
                    generation,
                    prepared_record_sha256,
                    native_results: results,
                    episode_shards: shards,
                    reinference_reports: reinference,
                    realized_tapes: realized,
                    simulated_ticks: 20,
                    successes: 1,
                },
            );
            let executed_record_sha256 = state
                .generations
                .last()
                .unwrap()
                .executed_record_sha256
                .unwrap();
            active = Digest([40 + generation as u8; 32]);
            let corpus = artifact(
                &root,
                &format!("generation-{generation}-corpus.json"),
                &[generation as u8],
            );
            state = append_test_event(
                &root,
                &request,
                initial,
                NativeGoalLearningLoopEvent::GenerationCommitted {
                    generation,
                    executed_record_sha256,
                    output_corpus_sha256: active,
                    output_corpus: corpus,
                    entries: u64::from(generation) + 4,
                    transitions: u64::from(generation) * 20,
                },
            );
            assert_eq!(state.active_corpus_sha256, active);
        }

        state = append_test_event(
            &root,
            &request,
            initial,
            NativeGoalLearningLoopEvent::LoopStopped {
                next_generation: 4,
                reason: NativeGoalLearningStopReason::GenerationLimitReached,
                active_corpus_sha256: active,
                evidence: None,
                proposal_source: NativeGoalLearningProposalSource::FrozenGoalPolicy,
            },
        );
        assert_eq!(state.committed_generations, 3);
        assert_eq!(state.record_count, 10);
        assert_eq!(state.charged_simulated_ticks, 60);
        assert_eq!(state.active_corpus_sha256, active);
        assert_eq!(
            state.stopped.as_ref().unwrap().reason,
            NativeGoalLearningStopReason::GenerationLimitReached
        );
        assert!(
            state
                .generations
                .windows(2)
                .all(|pair| pair[0].output_corpus_sha256 == Some(pair[1].input_corpus_sha256))
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resume_ignores_a_torn_tail_but_rejects_artifact_tampering() {
        let root = test_root();
        let request = request(&root);
        initialize_journal(&root, &request);
        let initial = Digest([3; 32]);
        let prepared = generation_artifacts(&root, 1, "prepared", 5);
        let batches = generation_artifacts(&root, 1, "batch", 2);
        let state = append_test_event(
            &root,
            &request,
            initial,
            NativeGoalLearningLoopEvent::GenerationPrepared {
                generation: 1,
                input_corpus_sha256: initial,
                dataset_sha256: Digest([11; 32]),
                reachability_model_sha256: Digest([21; 32]),
                policy_manifest_sha256: Digest([31; 32]),
                frozen_model_xxh3_128: "1".repeat(32),
                dataset: prepared[0].clone(),
                reachability_model: prepared[1].clone(),
                policy_manifest: prepared[2].clone(),
                frozen_model: prepared[3].clone(),
                native_batches: batches,
            },
        );
        let mut journal = OpenOptions::new()
            .append(true)
            .open(root.join(&request.resume.journal_path))
            .unwrap();
        journal.write_all(b"{\"torn\"").unwrap();
        journal.sync_all().unwrap();
        let resumed = fold_journal(&request, &root, initial).unwrap();
        assert_eq!(resumed.record_count, state.record_count);
        assert_eq!(resumed.last_record_sha256, state.last_record_sha256);

        fs::write(root.join(&prepared[0].path), b"tampered").unwrap();
        assert!(
            fold_journal(&request, &root, initial)
                .unwrap_err()
                .to_string()
                .contains("artifact digest differs")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reducer_rejects_skips_partial_execution_and_premature_stop() {
        let root = test_root();
        let request = request(&root);
        initialize_journal(&root, &request);
        let initial = Digest([3; 32]);
        let mut state = fold_journal(&request, &root, initial).unwrap();
        let prepared = generation_artifacts(&root, 2, "prepared", 5);
        let batches = generation_artifacts(&root, 2, "batch", 2);
        let skipped = NativeGoalLearningLoopEvent::GenerationPrepared {
            generation: 2,
            input_corpus_sha256: initial,
            dataset_sha256: Digest([12; 32]),
            reachability_model_sha256: Digest([22; 32]),
            policy_manifest_sha256: Digest([32; 32]),
            frozen_model_xxh3_128: "2".repeat(32),
            dataset: prepared[0].clone(),
            reachability_model: prepared[1].clone(),
            policy_manifest: prepared[2].clone(),
            frozen_model: prepared[3].clone(),
            native_batches: batches,
        };
        assert!(apply_event(&request, &mut state, &skipped, Digest([9; 32])).is_err());
        let premature = NativeGoalLearningLoopEvent::LoopStopped {
            next_generation: 1,
            reason: NativeGoalLearningStopReason::GenerationLimitReached,
            active_corpus_sha256: initial,
            evidence: None,
            proposal_source: NativeGoalLearningProposalSource::FrozenGoalPolicy,
        };
        assert!(apply_event(&request, &mut state, &premature, Digest([9; 32])).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
