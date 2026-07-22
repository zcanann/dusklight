//! Bounded post-discovery minimization over authenticated residual successes.

use crate::native_residual_campaign::{NativeResidualAttempt, NativeResidualExecutionBinding};
use crate::native_residual_campaign_runner::{
    NativeResidualCampaignRunConfig, NativeResidualExactReplayCandidate,
    NativeResidualExactReplayPool, validate_exact_replay_attempt_artifacts,
};
use crate::native_suffix_result::NativeTerminalBinding;
use crate::optimization_request::OptimizationRequest;
use crate::residual_campaign::{ResidualCampaignCandidate, ResidualCampaignCheckpoint};
use crate::residual_campaign_runner::{artifact_reference, write_exact_or_new};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::InputTape;
use dusklight_harness_contracts::objective_suite::ArtifactReference;
use dusklight_search::residual_action::{
    CompiledResidualCandidate, ResidualCandidate, ResidualCompilationReport,
    compile_residual_candidate_to_horizon,
};
use dusklight_search::residual_retention::{
    ExactTerminalVerdict, ResidualEvaluationEvidence, ResidualRetentionSnapshot,
};
use dusklight_search::search::tape_input_complexity;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Component, Path};
use std::sync::atomic::AtomicBool;

pub const RESIDUAL_MINIMIZED_CANDIDATE_SCHEMA_V1: &str =
    "dusklight-residual-minimized-candidate/v1";
pub const RESIDUAL_WINNER_MINIMIZATION_SCHEMA_V1: &str =
    "dusklight-residual-winner-minimization/v1";
pub const RESIDUAL_WINNER_MINIMIZATION_REQUEST_SCHEMA_V1: &str =
    "dusklight-residual-winner-minimization-request/v1";

pub struct ResidualWinnerMinimizationConfig<'a> {
    pub repository_root: &'a Path,
    pub optimization: &'a OptimizationRequest,
    pub execution: &'a NativeResidualExecutionBinding,
    pub checkpoint: &'a ResidualCampaignCheckpoint,
    pub source_request: ArtifactReference,
    pub source_execution: ArtifactReference,
    pub source_checkpoint: ArtifactReference,
    pub source_candidate: ArtifactReference,
    pub candidate: &'a ResidualCampaignCandidate,
    pub output_root: &'a Path,
    pub candidate_budget: u64,
    pub resume: bool,
    pub cancellation: Option<&'a AtomicBool>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualWinnerMinimizationRequest {
    pub schema: String,
    pub content_sha256: Digest,
    pub source_request: ArtifactReference,
    pub source_execution: ArtifactReference,
    pub source_checkpoint: ArtifactReference,
    pub source_candidate: ArtifactReference,
    pub candidate_budget: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualMinimizedCandidate {
    pub schema: String,
    pub content_sha256: Digest,
    pub source_candidate: ArtifactReference,
    pub discovered_tape_sha256: Digest,
    pub candidate: ResidualCandidate,
    pub compilation: ResidualCompilationReport,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResidualWinnerMinimizationStatus {
    Minimized,
    NoStrictReduction,
    CandidateBudgetExhausted,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualWinnerMinimizationSummary {
    pub schema: String,
    pub content_sha256: Digest,
    pub status: ResidualWinnerMinimizationStatus,
    pub optimization_request_sha256: Digest,
    pub execution_binding_sha256: Digest,
    pub source_request: ArtifactReference,
    pub source_execution: ArtifactReference,
    pub source_checkpoint: ArtifactReference,
    pub source_candidate: ArtifactReference,
    pub discovered_candidate_sha256: Digest,
    pub discovered_tape_sha256: Digest,
    pub discovered_first_hit_tick: u64,
    pub discovered_input_complexity: u64,
    pub minimized_candidate_sha256: Digest,
    pub minimized_tape_sha256: Digest,
    pub minimized_first_hit_tick: u64,
    pub minimized_input_complexity: u64,
    pub evaluated_candidates: u64,
    pub candidate_budget: u64,
    pub accepted_reduction_count: u64,
    pub charged_simulated_ticks: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimized_candidate: Option<ArtifactReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimized_tape: Option<ArtifactReference>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evaluations: Vec<ResidualReductionEvaluation>,
    pub retention: ResidualRetentionSnapshot,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualReductionEvaluation {
    pub round: u64,
    pub candidate: ResidualCandidate,
    pub compilation: ResidualCompilationReport,
    pub input_complexity: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_hit_tick: Option<u64>,
    pub accepted: bool,
    pub exact_replays: Vec<NativeResidualAttempt>,
}

struct ReductionProposal {
    id: String,
    candidate: ResidualCandidate,
    compiled: CompiledResidualCandidate,
    input_complexity: u64,
}

pub fn run_residual_winner_minimization(
    config: &ResidualWinnerMinimizationConfig<'_>,
) -> Result<ResidualWinnerMinimizationSummary, ResidualWinnerMinimizationError> {
    if config.candidate_budget == 0 || config.candidate_budget > 100_000 {
        return Err(minimization_message(
            "residual winner minimization candidate budget must be in 1..=100000",
        ));
    }
    let root = config
        .repository_root
        .canonicalize()
        .map_err(minimization_error)?;
    validate_bound_value(&root, &config.source_request, config.optimization)?;
    validate_bound_value(&root, &config.source_execution, config.execution)?;
    validate_bound_value(&root, &config.source_checkpoint, config.checkpoint)?;
    validate_bound_value(&root, &config.source_candidate, config.candidate)?;
    config
        .execution
        .validate_files(&root, config.optimization)
        .map_err(minimization_error)?;
    config
        .checkpoint
        .validate(config.optimization, config.execution.content_sha256)
        .map_err(minimization_error)?;
    config.candidate.validate().map_err(minimization_error)?;
    for reference in [
        &config.source_request,
        &config.source_execution,
        &config.source_checkpoint,
        &config.source_candidate,
    ] {
        validate_reference(reference)?;
    }
    let output = config.output_root;
    validate_build_output_path(&root, output)?;
    let incumbent = config.optimization.incumbent.as_ref().ok_or_else(|| {
        minimization_message("residual winner minimization requires an incumbent")
    })?;
    let parent_bytes = fs::read(root.join(&incumbent.tape.path)).map_err(minimization_error)?;
    let parent = InputTape::decode(&parent_bytes)
        .map_err(minimization_error)?
        .tape;
    let source_compiled = compile_residual_candidate_to_horizon(
        &parent,
        &parent_bytes,
        &config.candidate.candidate,
        config.optimization.budgets.exploration_horizon_ticks,
    )
    .map_err(minimization_error)?;
    if source_compiled.report != config.candidate.compilation {
        return Err(minimization_message(
            "discovered residual candidate recompiles differently from its sealed envelope",
        ));
    }
    let mut archive = config
        .checkpoint
        .restore_archive()
        .map_err(minimization_error)?;
    let discovered = archive
        .successes()
        .iter()
        .find(|success| {
            success.candidate_sha256 == config.candidate.candidate.content_sha256
                && success.realized_tape_sha256 == source_compiled.report.realized_tape_sha256
                && success.realized_tape == source_compiled.bytes
                && success.minimized_from.is_none()
        })
        .cloned()
        .ok_or_else(|| {
            minimization_message(
                "winner minimization requires an original retained exact discovery",
            )
        })?;
    let request = ResidualWinnerMinimizationRequest::seal(config)?;
    let output_exists = output.exists();
    if output_exists {
        let metadata = fs::symlink_metadata(output).map_err(minimization_error)?;
        if !config.resume || !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(minimization_message(
                "residual winner minimization output exists without a resumable directory",
            ));
        }
        let existing: ResidualWinnerMinimizationRequest = serde_json::from_slice(
            &fs::read(output.join("request.json")).map_err(minimization_error)?,
        )
        .map_err(minimization_error)?;
        existing.validate()?;
        if existing != request {
            return Err(minimization_message(
                "residual winner minimization resume request differs",
            ));
        }
        let summary_path = output.join("summary.json");
        if summary_path.is_file() {
            let summary: ResidualWinnerMinimizationSummary =
                serde_json::from_slice(&fs::read(summary_path).map_err(minimization_error)?)
                    .map_err(minimization_error)?;
            summary.validate_files(&root)?;
            if summary.source_request != request.source_request
                || summary.source_execution != request.source_execution
                || summary.source_checkpoint != request.source_checkpoint
                || summary.source_candidate != request.source_candidate
                || summary.candidate_budget != request.candidate_budget
            {
                return Err(minimization_message(
                    "completed residual minimization differs from its resume request",
                ));
            }
            return Ok(summary);
        }
    }
    if !output_exists {
        fs::create_dir_all(output).map_err(minimization_error)?;
        write_exact_or_new(&output.join("request.json"), &pretty_json(&request)?)
            .map_err(minimization_error)?;
    }
    let native_config = NativeResidualCampaignRunConfig {
        repository_root: &root,
        optimization: config.optimization,
        execution: config.execution,
        cancellation: config.cancellation,
    };
    let mut replay_pool = NativeResidualExactReplayPool::new(&root, output, &native_config)
        .map_err(minimization_error)?;
    let mut current_candidate = config.candidate.candidate.clone();
    let mut current_compiled = source_compiled;
    let mut current_complexity = tape_input_complexity(&current_compiled.tape);
    if current_complexity != discovered.input_complexity {
        return Err(minimization_message(
            "retained winner complexity differs from its exact recompilation",
        ));
    }
    let mut current_first_hit = discovered.first_hit_tick;
    let mut evaluated = 0_u64;
    let mut charged_ticks = 0_u64;
    let mut accepted = 0_u64;
    let mut evaluation_records = Vec::new();
    let mut evaluation_round = 0_u64;
    let mut seen = BTreeSet::from([current_compiled.report.realized_tape_sha256]);
    let mut granularity = 2_usize;
    let mut exhausted = false;
    loop {
        let remaining = config.candidate_budget - evaluated;
        if remaining == 0 {
            exhausted = true;
            break;
        }
        let component_count = current_candidate
            .analog
            .len()
            .checked_add(current_candidate.buttons.len())
            .ok_or_else(|| minimization_message("residual component count overflowed"))?;
        if component_count <= 1 {
            break;
        }
        let partitions = granularity.min(component_count);
        let mut proposals = reduction_proposals(
            &parent,
            &parent_bytes,
            config.optimization.budgets.exploration_horizon_ticks,
            &current_candidate,
            current_complexity,
            partitions,
            &mut seen,
        )?;
        proposals.truncate(usize::try_from(remaining).unwrap_or(usize::MAX));
        if proposals.is_empty() {
            if partitions == component_count {
                break;
            }
            granularity = (partitions * 2).min(component_count);
            continue;
        }
        let replay_candidates = proposals
            .iter()
            .map(|proposal| NativeResidualExactReplayCandidate {
                id: proposal.id.clone(),
                tape: proposal.compiled.tape.clone(),
            })
            .collect::<Vec<_>>();
        let mut attempts = replay_pool
            .replay(&replay_candidates)
            .map_err(minimization_error)?;
        evaluated = evaluated
            .checked_add(proposals.len() as u64)
            .ok_or_else(|| minimization_message("minimization evaluation count overflowed"))?;
        let mut outcomes = Vec::with_capacity(proposals.len());
        for proposal in proposals {
            let rows = attempts
                .remove(&proposal.id)
                .ok_or_else(|| minimization_message("exact replay result disappeared"))?;
            let proposal_ticks = rows
                .iter()
                .try_fold(0_u64, |total, row| total.checked_add(row.simulated_ticks))
                .ok_or_else(|| minimization_message("minimization tick charge overflowed"))?;
            charged_ticks = charged_ticks
                .checked_add(proposal_ticks)
                .ok_or_else(|| minimization_message("minimization tick charge overflowed"))?;
            let evidence = exact_replay_evidence(config.optimization, &proposal, &rows)?;
            outcomes.push((proposal, rows, evidence));
        }
        let accepted_index = outcomes.iter().position(|(_, _, evidence)| {
            matches!(
                evidence.verdict,
                ExactTerminalVerdict::Reached { first_hit_tick }
                    if first_hit_tick <= discovered.first_hit_tick
            )
        });
        for (index, (proposal, rows, evidence)) in outcomes.iter().enumerate() {
            evaluation_records.push(ResidualReductionEvaluation {
                round: evaluation_round,
                candidate: proposal.candidate.clone(),
                compilation: proposal.compiled.report.clone(),
                input_complexity: proposal.input_complexity,
                first_hit_tick: match evidence.verdict {
                    ExactTerminalVerdict::Reached { first_hit_tick } => Some(first_hit_tick),
                    ExactTerminalVerdict::Miss => None,
                },
                accepted: Some(index) == accepted_index,
                exact_replays: rows.clone(),
            });
        }
        let accepted_proposal = accepted_index
            .and_then(|index| outcomes.into_iter().nth(index))
            .map(|(proposal, _rows, evidence)| {
                let ExactTerminalVerdict::Reached { first_hit_tick } = evidence.verdict else {
                    unreachable!("accepted index requires an exact reached verdict")
                };
                (proposal, evidence, first_hit_tick)
            });
        if let Some((proposal, evidence, first_hit_tick)) = accepted_proposal {
            archive
                .accept_minimized(
                    discovered.realized_tape_sha256,
                    &proposal.compiled,
                    evidence,
                )
                .map_err(minimization_error)?;
            current_candidate = proposal.candidate;
            current_compiled = proposal.compiled;
            current_complexity = proposal.input_complexity;
            current_first_hit = first_hit_tick;
            accepted = accepted
                .checked_add(1)
                .ok_or_else(|| minimization_message("accepted reduction count overflowed"))?;
            granularity = 2;
        } else if partitions == component_count {
            break;
        } else {
            granularity = (partitions * 2).min(component_count);
        }
        evaluation_round = evaluation_round
            .checked_add(1)
            .ok_or_else(|| minimization_message("minimization round overflowed"))?;
    }
    let status = if accepted > 0 {
        ResidualWinnerMinimizationStatus::Minimized
    } else if exhausted {
        ResidualWinnerMinimizationStatus::CandidateBudgetExhausted
    } else {
        ResidualWinnerMinimizationStatus::NoStrictReduction
    };
    let (minimized_candidate, minimized_tape) = if accepted > 0 {
        let artifact = ResidualMinimizedCandidate::seal(
            config.source_candidate.clone(),
            discovered.realized_tape_sha256,
            current_candidate.clone(),
            current_compiled.report.clone(),
        )?;
        artifact.validate_against(
            &parent,
            &parent_bytes,
            config.optimization.budgets.exploration_horizon_ticks,
        )?;
        let candidate_path = output.join("minimized.candidate.json");
        let tape_path = output.join("minimized.tape");
        write_exact_or_new(&candidate_path, &artifact.to_pretty_json()?)
            .map_err(minimization_error)?;
        write_exact_or_new(&tape_path, &current_compiled.bytes).map_err(minimization_error)?;
        (
            Some(artifact_reference(&root, &candidate_path).map_err(minimization_error)?),
            Some(artifact_reference(&root, &tape_path).map_err(minimization_error)?),
        )
    } else {
        (None, None)
    };
    let mut summary = ResidualWinnerMinimizationSummary {
        schema: RESIDUAL_WINNER_MINIMIZATION_SCHEMA_V1.into(),
        content_sha256: Digest::ZERO,
        status,
        optimization_request_sha256: config.optimization.content_sha256,
        execution_binding_sha256: config.execution.content_sha256,
        source_request: config.source_request.clone(),
        source_execution: config.source_execution.clone(),
        source_checkpoint: config.source_checkpoint.clone(),
        source_candidate: config.source_candidate.clone(),
        discovered_candidate_sha256: config.candidate.candidate.content_sha256,
        discovered_tape_sha256: discovered.realized_tape_sha256,
        discovered_first_hit_tick: discovered.first_hit_tick,
        discovered_input_complexity: discovered.input_complexity,
        minimized_candidate_sha256: current_candidate.content_sha256,
        minimized_tape_sha256: current_compiled.report.realized_tape_sha256,
        minimized_first_hit_tick: current_first_hit,
        minimized_input_complexity: current_complexity,
        evaluated_candidates: evaluated,
        candidate_budget: config.candidate_budget,
        accepted_reduction_count: accepted,
        charged_simulated_ticks: charged_ticks,
        minimized_candidate,
        minimized_tape,
        evaluations: evaluation_records,
        retention: archive.snapshot().map_err(minimization_error)?,
    };
    summary.content_sha256 = summary.identity()?;
    summary.validate()?;
    summary.validate_files(&root)?;
    write_exact_or_new(&output.join("summary.json"), &summary.to_pretty_json()?)
        .map_err(minimization_error)?;
    Ok(summary)
}

impl ResidualWinnerMinimizationRequest {
    fn seal(
        config: &ResidualWinnerMinimizationConfig<'_>,
    ) -> Result<Self, ResidualWinnerMinimizationError> {
        let mut value = Self {
            schema: RESIDUAL_WINNER_MINIMIZATION_REQUEST_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            source_request: config.source_request.clone(),
            source_execution: config.source_execution.clone(),
            source_checkpoint: config.source_checkpoint.clone(),
            source_candidate: config.source_candidate.clone(),
            candidate_budget: config.candidate_budget,
        };
        value.content_sha256 = value.identity()?;
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), ResidualWinnerMinimizationError> {
        for reference in [
            &self.source_request,
            &self.source_execution,
            &self.source_checkpoint,
            &self.source_candidate,
        ] {
            validate_reference(reference)?;
        }
        if self.schema != RESIDUAL_WINNER_MINIMIZATION_REQUEST_SCHEMA_V1
            || self.candidate_budget == 0
            || self.candidate_budget > 100_000
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.identity()?
        {
            return Err(minimization_message(
                "residual winner minimization request is invalid or detached",
            ));
        }
        Ok(())
    }

    fn identity(&self) -> Result<Digest, ResidualWinnerMinimizationError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        canonical_digest(
            b"dusklight.residual-winner-minimization-request/v1\0",
            &canonical,
        )
    }
}

impl ResidualMinimizedCandidate {
    fn seal(
        source_candidate: ArtifactReference,
        discovered_tape_sha256: Digest,
        candidate: ResidualCandidate,
        compilation: ResidualCompilationReport,
    ) -> Result<Self, ResidualWinnerMinimizationError> {
        let mut value = Self {
            schema: RESIDUAL_MINIMIZED_CANDIDATE_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            source_candidate,
            discovered_tape_sha256,
            candidate,
            compilation,
        };
        value.content_sha256 = value.identity()?;
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), ResidualWinnerMinimizationError> {
        validate_reference(&self.source_candidate)?;
        self.candidate.validate().map_err(minimization_error)?;
        if self.schema != RESIDUAL_MINIMIZED_CANDIDATE_SCHEMA_V1
            || self.discovered_tape_sha256 == Digest::ZERO
            || self.compilation.candidate_sha256 != self.candidate.content_sha256
            || self.compilation.realized_tape_sha256 == Digest::ZERO
            || self.compilation.realized_tape_sha256 == self.discovered_tape_sha256
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.identity()?
        {
            return Err(minimization_message(
                "residual minimized candidate is invalid or detached",
            ));
        }
        Ok(())
    }

    pub fn validate_against(
        &self,
        parent: &InputTape,
        parent_bytes: &[u8],
        horizon: u64,
    ) -> Result<(), ResidualWinnerMinimizationError> {
        self.validate()?;
        let compiled =
            compile_residual_candidate_to_horizon(parent, parent_bytes, &self.candidate, horizon)
                .map_err(minimization_error)?;
        if compiled.report != self.compilation {
            return Err(minimization_message(
                "residual minimized candidate recompiles differently from its sealed report",
            ));
        }
        Ok(())
    }
}

impl ResidualWinnerMinimizationSummary {
    pub fn validate_files(
        &self,
        repository_root: &Path,
    ) -> Result<(), ResidualWinnerMinimizationError> {
        self.validate()?;
        let root = repository_root.canonicalize().map_err(minimization_error)?;
        let optimization: OptimizationRequest = read_bound_value(&root, &self.source_request)?;
        optimization
            .validate_files(&root)
            .map_err(minimization_error)?;
        let execution: NativeResidualExecutionBinding =
            read_bound_value(&root, &self.source_execution)?;
        execution
            .validate_files(&root, &optimization)
            .map_err(minimization_error)?;
        let checkpoint: ResidualCampaignCheckpoint =
            read_bound_value(&root, &self.source_checkpoint)?;
        checkpoint
            .validate(&optimization, execution.content_sha256)
            .map_err(minimization_error)?;
        let source_candidate: ResidualCampaignCandidate =
            read_bound_value(&root, &self.source_candidate)?;
        source_candidate.validate().map_err(minimization_error)?;
        if self.optimization_request_sha256 != optimization.content_sha256
            || self.execution_binding_sha256 != execution.content_sha256
            || self.discovered_candidate_sha256 != source_candidate.candidate.content_sha256
        {
            return Err(minimization_message(
                "residual minimization summary differs from its source lineage",
            ));
        }
        let incumbent = optimization.incumbent.as_ref().ok_or_else(|| {
            minimization_message("residual winner minimization requires an incumbent")
        })?;
        let parent_bytes = fs::read(root.join(&incumbent.tape.path)).map_err(minimization_error)?;
        let parent = InputTape::decode(&parent_bytes)
            .map_err(minimization_error)?
            .tape;
        let source_compiled = compile_residual_candidate_to_horizon(
            &parent,
            &parent_bytes,
            &source_candidate.candidate,
            optimization.budgets.exploration_horizon_ticks,
        )
        .map_err(minimization_error)?;
        if source_compiled.report != source_candidate.compilation
            || source_compiled.report.realized_tape_sha256 != self.discovered_tape_sha256
            || tape_input_complexity(&source_compiled.tape) != self.discovered_input_complexity
        {
            return Err(minimization_message(
                "residual minimization discovery recompiles differently",
            ));
        }
        let mut archive = checkpoint.restore_archive().map_err(minimization_error)?;
        let discovered = archive
            .successes()
            .iter()
            .find(|success| {
                success.candidate_sha256 == self.discovered_candidate_sha256
                    && success.realized_tape_sha256 == self.discovered_tape_sha256
                    && success.realized_tape == source_compiled.bytes
                    && success.first_hit_tick == self.discovered_first_hit_tick
                    && success.minimized_from.is_none()
            })
            .ok_or_else(|| {
                minimization_message(
                    "residual minimization source is not an original retained discovery",
                )
            })?;
        if discovered.input_complexity != self.discovered_input_complexity {
            return Err(minimization_message(
                "residual minimization source complexity differs",
            ));
        }
        let terminal = NativeTerminalBinding {
            goal: optimization.terminal_predicate.goal.clone(),
            program_sha256: optimization.terminal_predicate.program_sha256,
            definition_sha256: optimization.terminal_predicate.definition_sha256,
        };
        for evaluation in &self.evaluations {
            let compiled = compile_residual_candidate_to_horizon(
                &parent,
                &parent_bytes,
                &evaluation.candidate,
                optimization.budgets.exploration_horizon_ticks,
            )
            .map_err(minimization_error)?;
            if compiled.report != evaluation.compilation
                || tape_input_complexity(&compiled.tape) != evaluation.input_complexity
            {
                return Err(minimization_message(
                    "residual minimization reduction recompiles differently",
                ));
            }
            for attempt in &evaluation.exact_replays {
                validate_exact_replay_attempt_artifacts(
                    &root,
                    &optimization,
                    &execution,
                    &terminal,
                    &compiled.tape,
                    attempt,
                )
                .map_err(minimization_error)?;
            }
            let proposal = ReductionProposal {
                id: "validated-reduction".into(),
                candidate: evaluation.candidate.clone(),
                compiled,
                input_complexity: evaluation.input_complexity,
            };
            let evidence =
                exact_replay_evidence(&optimization, &proposal, &evaluation.exact_replays)?;
            let first_hit_tick = match evidence.verdict {
                ExactTerminalVerdict::Reached { first_hit_tick } => Some(first_hit_tick),
                ExactTerminalVerdict::Miss => None,
            };
            if first_hit_tick != evaluation.first_hit_tick {
                return Err(minimization_message(
                    "residual minimization replay verdict differs",
                ));
            }
            if evaluation.accepted {
                archive
                    .accept_minimized(self.discovered_tape_sha256, &proposal.compiled, evidence)
                    .map_err(minimization_error)?;
            }
        }
        if archive.snapshot().map_err(minimization_error)? != self.retention {
            return Err(minimization_message(
                "residual minimization retention cannot be reproduced",
            ));
        }
        match (&self.minimized_candidate, &self.minimized_tape) {
            (Some(candidate_reference), Some(tape_reference)) => {
                let candidate: ResidualMinimizedCandidate =
                    read_bound_value(&root, candidate_reference)?;
                candidate.validate_against(
                    &parent,
                    &parent_bytes,
                    optimization.budgets.exploration_horizon_ticks,
                )?;
                let tape_bytes = read_bound_bytes(&root, tape_reference)?;
                let final_compiled = compile_residual_candidate_to_horizon(
                    &parent,
                    &parent_bytes,
                    &candidate.candidate,
                    optimization.budgets.exploration_horizon_ticks,
                )
                .map_err(minimization_error)?;
                if candidate.source_candidate != self.source_candidate
                    || candidate.discovered_tape_sha256 != self.discovered_tape_sha256
                    || candidate.candidate.content_sha256 != self.minimized_candidate_sha256
                    || candidate.compilation.realized_tape_sha256 != self.minimized_tape_sha256
                    || tape_bytes != final_compiled.bytes
                {
                    return Err(minimization_message(
                        "residual minimization final artifacts differ",
                    ));
                }
            }
            (None, None) => {}
            _ => {
                return Err(minimization_message(
                    "residual minimization final artifact set is incomplete",
                ));
            }
        }
        Ok(())
    }
}

impl ResidualMinimizedCandidate {
    pub fn to_pretty_json(&self) -> Result<Vec<u8>, ResidualWinnerMinimizationError> {
        pretty_json(self)
    }

    fn identity(&self) -> Result<Digest, ResidualWinnerMinimizationError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        canonical_digest(b"dusklight.residual-minimized-candidate/v1\0", &canonical)
    }
}

impl ResidualWinnerMinimizationSummary {
    pub fn validate(&self) -> Result<(), ResidualWinnerMinimizationError> {
        for reference in [
            &self.source_request,
            &self.source_execution,
            &self.source_checkpoint,
            &self.source_candidate,
        ] {
            validate_reference(reference)?;
        }
        self.retention.validate().map_err(minimization_error)?;
        let minimized = self.status == ResidualWinnerMinimizationStatus::Minimized;
        let status_consistent = match self.status {
            ResidualWinnerMinimizationStatus::Minimized => self.accepted_reduction_count > 0,
            ResidualWinnerMinimizationStatus::NoStrictReduction => {
                self.accepted_reduction_count == 0
                    && self.evaluated_candidates < self.candidate_budget
            }
            ResidualWinnerMinimizationStatus::CandidateBudgetExhausted => {
                self.accepted_reduction_count == 0
                    && self.evaluated_candidates == self.candidate_budget
            }
        };
        let accepted_count = self
            .evaluations
            .iter()
            .filter(|evaluation| evaluation.accepted)
            .count() as u64;
        let replay_ticks = self
            .evaluations
            .iter()
            .try_fold(0_u64, |total, evaluation| {
                evaluation
                    .exact_replays
                    .iter()
                    .try_fold(total, |total, attempt| {
                        total.checked_add(attempt.simulated_ticks)
                    })
            });
        if self.schema != RESIDUAL_WINNER_MINIMIZATION_SCHEMA_V1
            || self.optimization_request_sha256 == Digest::ZERO
            || self.execution_binding_sha256 == Digest::ZERO
            || self.discovered_candidate_sha256 == Digest::ZERO
            || self.discovered_tape_sha256 == Digest::ZERO
            || self.discovered_first_hit_tick == 0
            || self.minimized_candidate_sha256 == Digest::ZERO
            || self.minimized_tape_sha256 == Digest::ZERO
            || self.minimized_first_hit_tick == 0
            || self.minimized_first_hit_tick > self.discovered_first_hit_tick
            || !status_consistent
            || self.candidate_budget == 0
            || self.evaluated_candidates > self.candidate_budget
            || self.evaluated_candidates != self.evaluations.len() as u64
            || self.accepted_reduction_count > self.evaluated_candidates
            || self.accepted_reduction_count != accepted_count
            || replay_ticks != Some(self.charged_simulated_ticks)
            || (self.evaluated_candidates == 0) != (self.charged_simulated_ticks == 0)
            || minimized != (self.accepted_reduction_count > 0)
            || minimized != self.minimized_candidate.is_some()
            || minimized != self.minimized_tape.is_some()
            || minimized == (accepted_count == 0)
            || (minimized
                && (self.minimized_input_complexity >= self.discovered_input_complexity
                    || self.minimized_tape_sha256 == self.discovered_tape_sha256))
            || (!minimized
                && (self.minimized_candidate_sha256 != self.discovered_candidate_sha256
                    || self.minimized_tape_sha256 != self.discovered_tape_sha256
                    || self.minimized_input_complexity != self.discovered_input_complexity))
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.identity()?
        {
            return Err(minimization_message(
                "residual winner minimization summary is invalid or detached",
            ));
        }
        let mut prior_complexity = self.discovered_input_complexity;
        let mut active_round = None;
        let mut round_base_complexity = prior_complexity;
        let mut accepted_in_round = None;
        let mut qualifying_seen = false;
        for evaluation in &self.evaluations {
            if active_round != Some(evaluation.round) {
                if active_round.is_some_and(|round| evaluation.round != round + 1)
                    || active_round.is_none() && evaluation.round != 0
                {
                    return Err(minimization_message(
                        "residual winner minimization rounds are not contiguous",
                    ));
                }
                if let Some(complexity) = accepted_in_round.take() {
                    prior_complexity = complexity;
                }
                active_round = Some(evaluation.round);
                round_base_complexity = prior_complexity;
                qualifying_seen = false;
            }
            evaluation
                .candidate
                .validate()
                .map_err(minimization_error)?;
            let retained = evaluation.first_hit_tick.is_some_and(|first_hit_tick| {
                self.retention.successes.iter().any(|success| {
                    success.candidate_sha256 == evaluation.candidate.content_sha256
                        && success.realized_tape_sha256
                            == evaluation.compilation.realized_tape_sha256
                        && success.first_hit_tick == first_hit_tick
                        && success.input_complexity == evaluation.input_complexity
                        && success.minimized_from == Some(self.discovered_tape_sha256)
                })
            });
            let qualifies = evaluation
                .first_hit_tick
                .is_some_and(|tick| tick <= self.discovered_first_hit_tick);
            let must_accept = qualifies && !qualifying_seen;
            if evaluation.compilation.candidate_sha256 != evaluation.candidate.content_sha256
                || evaluation.compilation.realized_tape_sha256 == Digest::ZERO
                || evaluation.input_complexity >= round_base_complexity
                || !valid_exact_replay_consensus(
                    &evaluation.exact_replays,
                    evaluation.first_hit_tick,
                )
                || evaluation.accepted != must_accept
                || (evaluation.accepted
                    && (evaluation
                        .first_hit_tick
                        .is_none_or(|tick| tick > self.discovered_first_hit_tick)
                        || !retained
                        || accepted_in_round.is_some()))
            {
                return Err(minimization_message(
                    "residual winner minimization evaluation is invalid or detached",
                ));
            }
            qualifying_seen |= qualifies;
            if evaluation.accepted {
                accepted_in_round = Some(evaluation.input_complexity);
            }
        }
        if let Some(final_reduction) = self.evaluations.iter().rfind(|row| row.accepted)
            && (final_reduction.candidate.content_sha256 != self.minimized_candidate_sha256
                || final_reduction.compilation.realized_tape_sha256 != self.minimized_tape_sha256
                || final_reduction.first_hit_tick != Some(self.minimized_first_hit_tick)
                || final_reduction.input_complexity != self.minimized_input_complexity)
        {
            return Err(minimization_message(
                "residual winner minimization final reduction differs from its summary",
            ));
        }
        if let Some(reference) = &self.minimized_candidate {
            validate_reference(reference)?;
        }
        if let Some(reference) = &self.minimized_tape {
            validate_reference(reference)?;
        }
        Ok(())
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, ResidualWinnerMinimizationError> {
        pretty_json(self)
    }

    fn identity(&self) -> Result<Digest, ResidualWinnerMinimizationError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        canonical_digest(b"dusklight.residual-winner-minimization/v1\0", &canonical)
    }
}

#[allow(clippy::too_many_arguments)]
fn reduction_proposals(
    parent: &InputTape,
    parent_bytes: &[u8],
    horizon: u64,
    current: &ResidualCandidate,
    current_complexity: u64,
    partitions: usize,
    seen: &mut BTreeSet<Digest>,
) -> Result<Vec<ReductionProposal>, ResidualWinnerMinimizationError> {
    let component_count = current.analog.len() + current.buttons.len();
    if component_count <= 1 || partitions == 0 || partitions > component_count {
        return Ok(Vec::new());
    }
    let mut proposals = Vec::new();
    for partition in 0..partitions {
        let start = component_count * partition / partitions;
        let end = component_count * (partition + 1) / partitions;
        let analog = current
            .analog
            .iter()
            .enumerate()
            .filter(|(index, _)| *index < start || *index >= end)
            .map(|(_, value)| value.clone())
            .collect::<Vec<_>>();
        let button_offset = current.analog.len();
        let buttons = current
            .buttons
            .iter()
            .enumerate()
            .filter(|(index, _)| {
                let combined = button_offset + *index;
                combined < start || combined >= end
            })
            .map(|(_, value)| value.clone())
            .collect::<Vec<_>>();
        if analog.is_empty() && buttons.is_empty() {
            continue;
        }
        let candidate =
            ResidualCandidate::seal(parent_bytes, analog, buttons).map_err(minimization_error)?;
        let compiled =
            compile_residual_candidate_to_horizon(parent, parent_bytes, &candidate, horizon)
                .map_err(minimization_error)?;
        let complexity = tape_input_complexity(&compiled.tape);
        if complexity >= current_complexity || !seen.insert(compiled.report.realized_tape_sha256) {
            continue;
        }
        let digest = candidate.content_sha256.to_string();
        proposals.push(ReductionProposal {
            id: format!("min-p{partition:05}-{}", &digest[..12]),
            candidate,
            compiled,
            input_complexity: complexity,
        });
    }
    Ok(proposals)
}

fn exact_replay_evidence(
    optimization: &OptimizationRequest,
    proposal: &ReductionProposal,
    attempts: &[NativeResidualAttempt],
) -> Result<ResidualEvaluationEvidence, ResidualWinnerMinimizationError> {
    if attempts.len() != usize::from(optimization.execution.repetitions) {
        return Err(minimization_message(
            "minimization exact replay omits a required repetition",
        ));
    }
    let first_hit_tick = attempts.first().and_then(|attempt| attempt.first_hit_tick);
    let boundary = attempts
        .first()
        .map(|attempt| attempt.terminal_boundary_fingerprint.as_str())
        .unwrap_or_default();
    if attempts.iter().enumerate().any(|(index, attempt)| {
        attempt.repetition as usize != index + 1
            || attempt.first_hit_tick != first_hit_tick
            || attempt.terminal_boundary_fingerprint != boundary
            || attempt.simulated_ticks == 0
            || attempt.behavior_sha256 == Digest::ZERO
    }) {
        return Err(minimization_message(
            "minimization repetitions disagree on exact terminal replay",
        ));
    }
    Ok(ResidualEvaluationEvidence {
        candidate_sha256: proposal.candidate.content_sha256,
        realized_tape_sha256: proposal.compiled.report.realized_tape_sha256,
        terminal_program_sha256: optimization.terminal_predicate.program_sha256,
        terminal_definition_sha256: optimization.terminal_predicate.definition_sha256,
        evaluation_sha256: canonical_digest(
            b"dusklight.residual-minimization-attempts/v1\0",
            &attempts,
        )?,
        episode_sha256: canonical_digest(
            b"dusklight.residual-minimization-episodes/v1\0",
            &attempts
                .iter()
                .map(|attempt| attempt.episode_shard.sha256)
                .collect::<Vec<_>>(),
        )?,
        behavior_sha256: canonical_digest(
            b"dusklight.residual-minimization-behavior/v1\0",
            &attempts
                .iter()
                .map(|attempt| attempt.behavior_sha256)
                .collect::<Vec<_>>(),
        )?,
        verdict: first_hit_tick.map_or(ExactTerminalVerdict::Miss, |first_hit_tick| {
            ExactTerminalVerdict::Reached { first_hit_tick }
        }),
        shaped_progress_millionths: None,
        native_risk_events: None,
    })
}

fn valid_exact_replay_consensus(
    attempts: &[NativeResidualAttempt],
    first_hit_tick: Option<u64>,
) -> bool {
    let boundary = attempts
        .first()
        .map(|attempt| attempt.terminal_boundary_fingerprint.as_str())
        .unwrap_or_default();
    !attempts.is_empty()
        && !boundary.is_empty()
        && attempts.iter().enumerate().all(|(index, attempt)| {
            attempt.repetition as usize == index + 1
                && attempt.first_hit_tick == first_hit_tick
                && attempt.terminal_boundary_fingerprint == boundary
                && attempt.simulated_ticks > 0
                && attempt.behavior_sha256 != Digest::ZERO
        })
}

fn validate_reference(
    reference: &ArtifactReference,
) -> Result<(), ResidualWinnerMinimizationError> {
    let path = Path::new(&reference.path);
    if reference.sha256 == Digest::ZERO
        || path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(minimization_message(
            "residual minimization artifact reference is invalid",
        ));
    }
    Ok(())
}

fn validate_build_output_path(
    root: &Path,
    output: &Path,
) -> Result<(), ResidualWinnerMinimizationError> {
    let relative = output.strip_prefix(root).map_err(|_| {
        minimization_message("residual minimization output is outside the repository")
    })?;
    let mut components = relative.components();
    if !matches!(components.next(), Some(Component::Normal(value)) if value == "build")
        || components.any(|component| !matches!(component, Component::Normal(_)))
        || relative.components().count() < 2
    {
        return Err(minimization_message(
            "residual minimization output must be below repository build/",
        ));
    }
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(value) = component else {
            unreachable!("validated output has only normal components")
        };
        current.push(value);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(minimization_message(
                    "residual minimization output traverses a symlink",
                ));
            }
            Ok(metadata) if current != output && !metadata.is_dir() => {
                return Err(minimization_message(
                    "residual minimization output parent is not a directory",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(minimization_error(error)),
        }
    }
    Ok(())
}

fn validate_bound_value<T: DeserializeOwned + PartialEq>(
    root: &Path,
    reference: &ArtifactReference,
    expected: &T,
) -> Result<(), ResidualWinnerMinimizationError> {
    let actual: T = read_bound_value(root, reference)?;
    if &actual != expected {
        return Err(minimization_message(
            "residual minimization source value differs from its bound file",
        ));
    }
    Ok(())
}

fn read_bound_value<T: DeserializeOwned>(
    root: &Path,
    reference: &ArtifactReference,
) -> Result<T, ResidualWinnerMinimizationError> {
    let bytes = read_bound_bytes(root, reference)?;
    serde_json::from_slice(&bytes).map_err(minimization_error)
}

fn read_bound_bytes(
    root: &Path,
    reference: &ArtifactReference,
) -> Result<Vec<u8>, ResidualWinnerMinimizationError> {
    validate_reference(reference)?;
    let path = root.join(&reference.path);
    let metadata = fs::symlink_metadata(&path).map_err(minimization_error)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 64 * 1024 * 1024
    {
        return Err(minimization_message(
            "residual minimization source is not a bounded physical file",
        ));
    }
    let canonical = path.canonicalize().map_err(minimization_error)?;
    if !canonical.starts_with(root) {
        return Err(minimization_message(
            "residual minimization source escapes the repository",
        ));
    }
    let bytes = fs::read(canonical).map_err(minimization_error)?;
    if Digest(Sha256::digest(&bytes).into()) != reference.sha256 {
        return Err(minimization_message(
            "residual minimization source digest differs",
        ));
    }
    Ok(bytes)
}

fn canonical_digest(
    domain: &[u8],
    value: &impl Serialize,
) -> Result<Digest, ResidualWinnerMinimizationError> {
    let encoded = serde_json::to_vec(value).map_err(minimization_error)?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((encoded.len() as u64).to_le_bytes());
    hasher.update(encoded);
    Ok(Digest(hasher.finalize().into()))
}

fn pretty_json(value: &impl Serialize) -> Result<Vec<u8>, ResidualWinnerMinimizationError> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(minimization_error)?;
    bytes.push(b'\n');
    Ok(bytes)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResidualWinnerMinimizationError(String);

fn minimization_message(message: impl Into<String>) -> ResidualWinnerMinimizationError {
    ResidualWinnerMinimizationError(message.into())
}

fn minimization_error(error: impl fmt::Display) -> ResidualWinnerMinimizationError {
    minimization_message(error.to_string())
}

impl fmt::Display for ResidualWinnerMinimizationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ResidualWinnerMinimizationError {}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_automation_contracts::tape::{InputFrame, RawPadState, TapeBoot};
    use dusklight_search::residual_action::{AnalogChannel, AnalogResidual, TemporalBasis};
    use dusklight_search::residual_retention::{
        FailureRetentionPolicy, ResidualOutcomeArchive, ResidualRetentionConfig,
    };

    fn parent() -> (InputTape, Vec<u8>) {
        let tape = InputTape {
            boot: TapeBoot::Process,
            tick_rate_numerator: 30,
            tick_rate_denominator: 1,
            frames: vec![
                InputFrame {
                    owned_ports: 1,
                    pads: [
                        RawPadState {
                            connected: true,
                            ..RawPadState::default()
                        },
                        RawPadState::default(),
                        RawPadState::default(),
                        RawPadState::default(),
                    ],
                    ..InputFrame::default()
                };
                8
            ],
        };
        let bytes = tape.encode().unwrap();
        (tape, bytes)
    }

    #[test]
    fn component_reductions_are_strict_deterministic_and_never_empty() {
        let (parent, bytes) = parent();
        let candidate = ResidualCandidate::seal(
            &bytes,
            vec![
                AnalogResidual {
                    port: 0,
                    channel: AnalogChannel::MainX,
                    basis: TemporalBasis::ExactFrame {
                        frame: 1,
                        delta: 10,
                    },
                },
                AnalogResidual {
                    port: 0,
                    channel: AnalogChannel::MainX,
                    basis: TemporalBasis::ExactFrame {
                        frame: 5,
                        delta: -10,
                    },
                },
            ],
            vec![],
        )
        .unwrap();
        let compiled =
            compile_residual_candidate_to_horizon(&parent, &bytes, &candidate, 8).unwrap();
        let complexity = tape_input_complexity(&compiled.tape);
        let mut seen = BTreeSet::new();
        let proposals =
            reduction_proposals(&parent, &bytes, 8, &candidate, complexity, 2, &mut seen).unwrap();
        assert_eq!(proposals.len(), 2);
        assert!(proposals.iter().all(|proposal| {
            proposal.candidate.analog.len() == 1
                && proposal.candidate.buttons.is_empty()
                && proposal.input_complexity < complexity
        }));
        let repeated =
            reduction_proposals(&parent, &bytes, 8, &candidate, complexity, 2, &mut seen).unwrap();
        assert!(repeated.is_empty());
    }

    #[test]
    fn sealed_minimized_candidate_rejects_detachment() {
        let (parent, bytes) = parent();
        let candidate = ResidualCandidate::seal(
            &bytes,
            vec![AnalogResidual {
                port: 0,
                channel: AnalogChannel::MainX,
                basis: TemporalBasis::ExactFrame {
                    frame: 1,
                    delta: 10,
                },
            }],
            vec![],
        )
        .unwrap();
        let compiled =
            compile_residual_candidate_to_horizon(&parent, &bytes, &candidate, 8).unwrap();
        let source = ArtifactReference {
            path: "build/campaigns/source/candidate.json".into(),
            sha256: Digest([7; 32]),
        };
        let mut artifact =
            ResidualMinimizedCandidate::seal(source, Digest([9; 32]), candidate, compiled.report)
                .unwrap();
        artifact.compilation.realized_tape_sha256 = Digest([3; 32]);
        artifact.content_sha256 = artifact.identity().unwrap();
        artifact.validate().unwrap();
        assert!(artifact.validate_against(&parent, &bytes, 8).is_err());
    }

    #[test]
    fn exact_replay_evidence_requires_identical_terminal_repetitions() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../..")
            .canonicalize()
            .unwrap();
        let optimization: OptimizationRequest = serde_json::from_slice(
            &fs::read(root.join(
                "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json",
            ))
            .unwrap(),
        )
        .unwrap();
        let (parent, bytes) = parent();
        let candidate = ResidualCandidate::seal(
            &bytes,
            vec![AnalogResidual {
                port: 0,
                channel: AnalogChannel::MainX,
                basis: TemporalBasis::ExactFrame {
                    frame: 1,
                    delta: 10,
                },
            }],
            vec![],
        )
        .unwrap();
        let compiled =
            compile_residual_candidate_to_horizon(&parent, &bytes, &candidate, 8).unwrap();
        let proposal = ReductionProposal {
            id: "min-test".into(),
            candidate,
            input_complexity: tape_input_complexity(&compiled.tape),
            compiled,
        };
        let attempt = |repetition, boundary: &str| NativeResidualAttempt {
            repetition,
            worker_seed: 1,
            wire_candidate_id: format!("min-test-r{repetition:03}"),
            batch_request: ArtifactReference {
                path: format!("build/request-{repetition}.json"),
                sha256: Digest([1; 32]),
            },
            batch_result: ArtifactReference {
                path: format!("build/result-{repetition}.json"),
                sha256: Digest([2; 32]),
            },
            episode_shard: ArtifactReference {
                path: format!("build/episode-{repetition}.dseps"),
                sha256: Digest([3; 32]),
            },
            restore_identity: "4".repeat(32),
            checkpoint_bytes: 1,
            simulated_ticks: 120,
            first_hit_tick: Some(120),
            terminal_boundary_fingerprint: boundary.into(),
            behavior_sha256: Digest([5; 32]),
        };
        let reached = exact_replay_evidence(&optimization, &proposal, &[attempt(1, "a")]).unwrap();
        assert_eq!(
            reached.verdict,
            ExactTerminalVerdict::Reached {
                first_hit_tick: 120
            }
        );
        let mut repeated = optimization;
        repeated.execution.repetitions = 2;
        assert!(
            exact_replay_evidence(&repeated, &proposal, &[attempt(1, "a"), attempt(2, "b")],)
                .is_err()
        );
    }

    #[test]
    fn summary_reproduces_every_accepted_reduction_and_rejects_resealed_drift() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../..")
            .canonicalize()
            .unwrap();
        let optimization: OptimizationRequest = serde_json::from_slice(
            &fs::read(root.join(
                "routes/Glitch Exhibition/intro/benchmarks/ordon-q125-residual-campaign.request.json",
            ))
            .unwrap(),
        )
        .unwrap();
        let (parent, bytes) = parent();
        let source_candidate = ResidualCandidate::seal(
            &bytes,
            vec![
                AnalogResidual {
                    port: 0,
                    channel: AnalogChannel::MainX,
                    basis: TemporalBasis::ExactFrame {
                        frame: 1,
                        delta: 10,
                    },
                },
                AnalogResidual {
                    port: 0,
                    channel: AnalogChannel::MainX,
                    basis: TemporalBasis::ExactFrame {
                        frame: 5,
                        delta: -10,
                    },
                },
            ],
            vec![],
        )
        .unwrap();
        let source =
            compile_residual_candidate_to_horizon(&parent, &bytes, &source_candidate, 8).unwrap();
        let source_complexity = tape_input_complexity(&source.tape);
        let mut seen = BTreeSet::new();
        let mut reductions = reduction_proposals(
            &parent,
            &bytes,
            8,
            &source_candidate,
            source_complexity,
            2,
            &mut seen,
        )
        .unwrap();
        let minimized = reductions.remove(0);
        let rejected = reductions.remove(0);
        let mut archive = ResidualOutcomeArchive::new(ResidualRetentionConfig {
            parent_tape_sha256: source_candidate.parent_tape_sha256,
            terminal_program_sha256: optimization.terminal_predicate.program_sha256,
            terminal_definition_sha256: optimization.terminal_predicate.definition_sha256,
            exploration_horizon_ticks: 8,
            promotion_before_tick: 5,
            maximum_candidates: 10,
            failures: FailureRetentionPolicy::All,
        })
        .unwrap();
        let evidence =
            |candidate: &CompiledResidualCandidate, byte: u8| ResidualEvaluationEvidence {
                candidate_sha256: candidate.report.candidate_sha256,
                realized_tape_sha256: candidate.report.realized_tape_sha256,
                terminal_program_sha256: optimization.terminal_predicate.program_sha256,
                terminal_definition_sha256: optimization.terminal_predicate.definition_sha256,
                evaluation_sha256: Digest([byte; 32]),
                episode_sha256: Digest([byte.saturating_add(1); 32]),
                behavior_sha256: Digest([byte.saturating_add(2); 32]),
                verdict: ExactTerminalVerdict::Reached { first_hit_tick: 6 },
                shaped_progress_millionths: None,
                native_risk_events: None,
            };
        archive.record(&source, evidence(&source, 10)).unwrap();
        let attempt = NativeResidualAttempt {
            repetition: 1,
            worker_seed: 1,
            wire_candidate_id: format!("{}-r001", minimized.id),
            batch_request: ArtifactReference {
                path: "build/min/request.json".into(),
                sha256: Digest([20; 32]),
            },
            batch_result: ArtifactReference {
                path: "build/min/result.json".into(),
                sha256: Digest([21; 32]),
            },
            episode_shard: ArtifactReference {
                path: "build/min/episode.dseps".into(),
                sha256: Digest([22; 32]),
            },
            restore_identity: "4".repeat(32),
            checkpoint_bytes: 1,
            simulated_ticks: 6,
            first_hit_tick: Some(6),
            terminal_boundary_fingerprint: "5".repeat(32),
            behavior_sha256: Digest([23; 32]),
        };
        let rejected_attempt = NativeResidualAttempt {
            repetition: 1,
            worker_seed: 1,
            wire_candidate_id: format!("{}-r001", rejected.id),
            batch_request: ArtifactReference {
                path: "build/min/rejected-request.json".into(),
                sha256: Digest([24; 32]),
            },
            batch_result: ArtifactReference {
                path: "build/min/rejected-result.json".into(),
                sha256: Digest([25; 32]),
            },
            episode_shard: ArtifactReference {
                path: "build/min/rejected-episode.dseps".into(),
                sha256: Digest([26; 32]),
            },
            restore_identity: "4".repeat(32),
            checkpoint_bytes: 1,
            simulated_ticks: 7,
            first_hit_tick: None,
            terminal_boundary_fingerprint: "6".repeat(32),
            behavior_sha256: Digest([27; 32]),
        };
        let minimized_evidence =
            exact_replay_evidence(&optimization, &minimized, std::slice::from_ref(&attempt))
                .unwrap();
        archive
            .accept_minimized(
                source.report.realized_tape_sha256,
                &minimized.compiled,
                minimized_evidence,
            )
            .unwrap();
        let reference = |path: &str, byte| ArtifactReference {
            path: path.into(),
            sha256: Digest([byte; 32]),
        };
        let mut summary = ResidualWinnerMinimizationSummary {
            schema: RESIDUAL_WINNER_MINIMIZATION_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            status: ResidualWinnerMinimizationStatus::Minimized,
            optimization_request_sha256: optimization.content_sha256,
            execution_binding_sha256: Digest([30; 32]),
            source_request: reference("routes/request.json", 31),
            source_execution: reference("build/source/execution.json", 32),
            source_checkpoint: reference("build/source/checkpoint.json", 33),
            source_candidate: reference("build/source/candidate.json", 34),
            discovered_candidate_sha256: source_candidate.content_sha256,
            discovered_tape_sha256: source.report.realized_tape_sha256,
            discovered_first_hit_tick: 6,
            discovered_input_complexity: source_complexity,
            minimized_candidate_sha256: minimized.candidate.content_sha256,
            minimized_tape_sha256: minimized.compiled.report.realized_tape_sha256,
            minimized_first_hit_tick: 6,
            minimized_input_complexity: minimized.input_complexity,
            evaluated_candidates: 2,
            candidate_budget: 3,
            accepted_reduction_count: 1,
            charged_simulated_ticks: 13,
            minimized_candidate: Some(reference("build/min/minimized.candidate.json", 35)),
            minimized_tape: Some(reference("build/min/minimized.tape", 36)),
            evaluations: vec![
                ResidualReductionEvaluation {
                    round: 0,
                    candidate: minimized.candidate,
                    compilation: minimized.compiled.report,
                    input_complexity: minimized.input_complexity,
                    first_hit_tick: Some(6),
                    accepted: true,
                    exact_replays: vec![attempt],
                },
                ResidualReductionEvaluation {
                    round: 0,
                    candidate: rejected.candidate,
                    compilation: rejected.compiled.report,
                    input_complexity: rejected.input_complexity,
                    first_hit_tick: None,
                    accepted: false,
                    exact_replays: vec![rejected_attempt],
                },
            ],
            retention: archive.snapshot().unwrap(),
        };
        summary.content_sha256 = summary.identity().unwrap();
        summary.validate().unwrap();

        summary.evaluations[0].input_complexity = source_complexity;
        summary.content_sha256 = summary.identity().unwrap();
        assert!(summary.validate().is_err());
    }
}
