//! Deterministic adjacent-heading refinement of an authenticated scratch route.

use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::InputTape;
use dusklight_learning::scratch_action_catalog::{
    MAX_SCRATCH_REFINEMENT_HEADINGS, SCRATCH_ACTIONS_PER_HEADING, SCRATCH_HEADING_COUNT,
    map_scratch_action_to_finer_catalog, scratch_action_catalog_with_heading_count,
    scratch_action_heading_index, scratch_action_index_with_heading,
};
use dusklight_learning::scratch_q::ScratchQTable;
use dusklight_learning::tactic_asset::TacticAssetCatalog;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use crate::native_residual_campaign::ValidatedNativeResidualExecution;
use crate::native_scratch_learner::{
    NativeScratchRunConfig, load_scratch_refinement_source, run_episode,
};
use crate::native_tactic_route_runner::{
    NativeTacticColdReplayAttempt, NativeTapeColdReplayConfig, exact_cold_replay_attempts,
    run_native_tape_cold_replay_after_execution_validation,
};

const MAX_INCUMBENT_ACTIONS: usize = 100_000;
const REPORT_SCHEMA: &str = "dusklight-native-scratch-heading-report/v1";
const INSPECTION_SCHEMA: &str = "dusklight-native-scratch-heading-inspection/v2";
const CHECKPOINT_SCHEMA: &str = "dusklight-native-scratch-heading-checkpoint/v2";
const CHECKPOINT_MAGIC: &[u8; 8] = b"DSSHDR01";
const CHECKPOINT_VERSION: u16 = 2;
const CHECKPOINT_HEADER_BYTES: usize = 8 + 2 + 2 + 8 + 32;
const CHECKPOINT_COMPRESSION_LEVEL: i32 = 1;

pub struct NativeScratchHeadingRunConfig<'a> {
    pub scratch: NativeScratchRunConfig<'a>,
    /// An exhausted heading-refinement checkpoint to refine further. When
    /// absent, the authenticated scratch learner checkpoint is the source.
    pub source_heading_root: Option<&'a Path>,
    pub heading_count: usize,
    pub output_root: &'a Path,
    pub candidate_limit: u64,
    pub maximum_wall_time: Duration,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeScratchHeadingReport {
    pub schema: String,
    pub report_sha256: Digest,
    pub source_checkpoint_sha256: Digest,
    pub optimization_request_sha256: Digest,
    pub execution_binding_sha256: Digest,
    pub action_universe_sha256: Digest,
    pub seed: u64,
    pub stop_reason: String,
    pub attempted_candidates: u64,
    pub terminal_candidates: u64,
    pub strict_winners: u64,
    pub candidates_remaining: u64,
    pub fastest_selected_ticks: u64,
    pub fastest_tape: Option<String>,
    pub fastest_tape_sha256: Option<Digest>,
    pub native_ticks: u64,
    pub native_wall_micros: u64,
    pub wall_micros: u64,
    pub attempts: Vec<NativeScratchHeadingAttempt>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeScratchHeadingAttempt {
    pub attempt_index: u64,
    pub candidate_sha256: Digest,
    pub changed_action_index: u64,
    pub previous_heading_index: u64,
    pub replacement_heading_index: u64,
    pub terminal: bool,
    pub selected_ticks: Option<u64>,
    pub strict_winner: bool,
    pub native_ticks: u64,
    pub native_wall_micros: u64,
    pub tape_sha256: Digest,
    pub cold_replay_attempts: Vec<NativeTacticColdReplayAttempt>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeScratchHeadingInspection {
    pub schema: String,
    pub checkpoint_sha256: Digest,
    pub checkpoint_schema: String,
    pub source_checkpoint_sha256: Digest,
    pub optimization_request_sha256: Digest,
    pub execution_binding_sha256: Digest,
    pub action_universe_sha256: Digest,
    pub heading_count: u64,
    pub stop_reason: String,
    pub candidates_remaining: u64,
    pub incumbent_ticks: u64,
    pub incumbent_action_sequence_sha256: Digest,
    pub incumbent_action_count: u64,
    pub nominal_duration_ticks: u64,
    pub family_counts: BTreeMap<String, u64>,
    pub incumbent_options: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct NativeScratchHeadingCheckpoint {
    schema: String,
    source_checkpoint_sha256: Digest,
    optimization_request_sha256: Digest,
    execution_binding_sha256: Digest,
    action_universe_sha256: Digest,
    seed: u64,
    search: ScratchHeadingSearch,
    report: NativeScratchHeadingReport,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScratchHeadingSearch {
    incumbent_action_sequence: Vec<usize>,
    incumbent_sha256: Digest,
    attempted_candidate_sha256s: BTreeSet<Digest>,
    #[serde(default = "default_heading_count")]
    heading_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ScratchHeadingCandidate {
    pub changed_action_index: usize,
    pub previous_heading_index: usize,
    pub replacement_heading_index: usize,
    pub action_sequence: Vec<usize>,
    pub action_sequence_sha256: Digest,
}

impl ScratchHeadingSearch {
    pub fn with_heading_count(
        incumbent_action_sequence: Vec<usize>,
        heading_count: usize,
    ) -> Result<Self, String> {
        if incumbent_action_sequence.is_empty()
            || incumbent_action_sequence.len() > MAX_INCUMBENT_ACTIONS
            || !(SCRATCH_HEADING_COUNT..=MAX_SCRATCH_REFINEMENT_HEADINGS).contains(&heading_count)
            || !heading_count.is_power_of_two()
            || incumbent_action_sequence
                .iter()
                .any(|action| *action >= heading_count * SCRATCH_ACTIONS_PER_HEADING)
        {
            return Err("scratch heading incumbent is invalid".into());
        }
        let incumbent_sha256 = action_sequence_sha256(&incumbent_action_sequence)?;
        Ok(Self {
            incumbent_action_sequence,
            incumbent_sha256,
            attempted_candidate_sha256s: BTreeSet::new(),
            heading_count,
        })
    }

    pub fn validate(&self, seed: u64, catalog: &TacticAssetCatalog) -> Result<(), String> {
        let rebuilt =
            Self::with_heading_count(self.incumbent_action_sequence.clone(), self.heading_count)?;
        if rebuilt.incumbent_sha256 != self.incumbent_sha256
            || catalog.entries().len() != self.heading_count * SCRATCH_ACTIONS_PER_HEADING
        {
            return Err("scratch heading incumbent identity is invalid".into());
        }
        let candidate_ids = self
            .candidates(seed, catalog)?
            .into_iter()
            .map(|candidate| candidate.action_sequence_sha256)
            .collect::<BTreeSet<_>>();
        if !self.attempted_candidate_sha256s.is_subset(&candidate_ids) {
            return Err("scratch heading attempts are detached from the incumbent".into());
        }
        Ok(())
    }

    pub fn next_candidate(
        &self,
        seed: u64,
        catalog: &TacticAssetCatalog,
    ) -> Result<Option<ScratchHeadingCandidate>, String> {
        Ok(self
            .candidates(seed, catalog)?
            .into_iter()
            .find(|candidate| {
                !self
                    .attempted_candidate_sha256s
                    .contains(&candidate.action_sequence_sha256)
            }))
    }

    pub fn finish_attempt(
        &mut self,
        seed: u64,
        catalog: &TacticAssetCatalog,
        candidate: &ScratchHeadingCandidate,
        accepted_incumbent: Option<Vec<usize>>,
    ) -> Result<(), String> {
        let expected = self
            .candidates(seed, catalog)?
            .into_iter()
            .find(|expected| expected.action_sequence_sha256 == candidate.action_sequence_sha256)
            .ok_or_else(|| "scratch heading candidate is detached from the incumbent".to_owned())?;
        if expected != *candidate {
            return Err("scratch heading candidate identity is inconsistent".into());
        }
        if let Some(incumbent) = accepted_incumbent {
            *self = Self::with_heading_count(incumbent, self.heading_count)?;
        } else if !self
            .attempted_candidate_sha256s
            .insert(candidate.action_sequence_sha256)
        {
            return Err("scratch heading candidate was attempted twice".into());
        }
        Ok(())
    }

    pub fn remaining_candidates(
        &self,
        seed: u64,
        catalog: &TacticAssetCatalog,
    ) -> Result<usize, String> {
        Ok(self
            .candidates(seed, catalog)?
            .into_iter()
            .filter(|candidate| {
                !self
                    .attempted_candidate_sha256s
                    .contains(&candidate.action_sequence_sha256)
            })
            .count())
    }

    fn candidates(
        &self,
        seed: u64,
        catalog: &TacticAssetCatalog,
    ) -> Result<Vec<ScratchHeadingCandidate>, String> {
        let mut unique_sequences = BTreeSet::new();
        let mut ranked = Vec::new();
        for (changed_action_index, action) in
            self.incumbent_action_sequence.iter().copied().enumerate()
        {
            let previous_heading_index =
                scratch_action_heading_index(catalog, action).map_err(|error| error.to_string())?;
            for replacement_heading_index in [
                (previous_heading_index + self.heading_count - 1) % self.heading_count,
                (previous_heading_index + 1) % self.heading_count,
            ] {
                let replacement_action =
                    scratch_action_index_with_heading(catalog, action, replacement_heading_index)
                        .map_err(|error| error.to_string())?;
                let mut action_sequence = self.incumbent_action_sequence.clone();
                action_sequence[changed_action_index] = replacement_action;
                if !unique_sequences.insert(action_sequence.clone()) {
                    continue;
                }
                let action_sequence_sha256 = action_sequence_sha256(&action_sequence)?;
                let rank = candidate_rank(
                    seed,
                    self.incumbent_sha256,
                    action_sequence_sha256,
                    changed_action_index,
                    replacement_heading_index,
                );
                ranked.push((
                    rank,
                    ScratchHeadingCandidate {
                        changed_action_index,
                        previous_heading_index,
                        replacement_heading_index,
                        action_sequence,
                        action_sequence_sha256,
                    },
                ));
            }
        }
        ranked.sort_by_key(|(rank, candidate)| {
            (
                *rank,
                candidate.action_sequence_sha256,
                candidate.changed_action_index,
                candidate.replacement_heading_index,
            )
        });
        Ok(ranked.into_iter().map(|(_, candidate)| candidate).collect())
    }
}

fn default_heading_count() -> usize {
    SCRATCH_HEADING_COUNT
}

fn load_refinement_source(
    config: &NativeScratchHeadingRunConfig<'_>,
) -> Result<crate::native_scratch_learner::ScratchRefinementSource, NativeScratchHeadingError> {
    let scratch_source =
        load_scratch_refinement_source(&config.scratch).map_err(heading_display)?;
    let Some(source_root) = config.source_heading_root else {
        if config.heading_count != SCRATCH_HEADING_COUNT {
            return Err(heading_error(
                "finer headings require an exhausted heading-refinement source",
            ));
        }
        return Ok(scratch_source);
    };
    let checkpoint_path = source_root.join("checkpoint.dssh");
    let checkpoint_bytes = fs::read(&checkpoint_path).map_err(heading_display)?;
    let checkpoint = decode_heading_checkpoint(&checkpoint_bytes)?;
    let source_catalog = scratch_action_catalog_with_heading_count(checkpoint.search.heading_count)
        .map_err(heading_display)?;
    validate_heading_checkpoint(&checkpoint, config, &scratch_source, &source_catalog, false)?;
    if checkpoint.report.stop_reason != "candidate_exhaustion"
        || checkpoint.report.candidates_remaining != 0
    {
        return Err(heading_error(
            "finer-heading source has not exhausted its current frontier",
        ));
    }
    let incumbent_action_sequence = promote_heading_resolution(
        &checkpoint.search.incumbent_action_sequence,
        &source_catalog,
        &scratch_action_catalog_with_heading_count(config.heading_count)
            .map_err(heading_display)?,
    )
    .map_err(heading_error)?;
    Ok(crate::native_scratch_learner::ScratchRefinementSource {
        checkpoint_sha256: sha256(&checkpoint_bytes),
        seed: checkpoint.seed,
        incumbent_action_sequence,
        incumbent_ticks: checkpoint.report.fastest_selected_ticks,
    })
}

fn promote_heading_resolution(
    action_sequence: &[usize],
    source_catalog: &TacticAssetCatalog,
    target_catalog: &TacticAssetCatalog,
) -> Result<Vec<usize>, String> {
    if target_catalog.entries().len() != source_catalog.entries().len().saturating_mul(2) {
        return Err("finer-heading resolution must exactly double its valid source".into());
    }
    action_sequence
        .iter()
        .map(|action| {
            map_scratch_action_to_finer_catalog(source_catalog, target_catalog, *action)
                .map_err(|error| error.to_string())
        })
        .collect()
}

pub fn run_native_scratch_heading_refinement(
    config: &NativeScratchHeadingRunConfig<'_>,
) -> Result<NativeScratchHeadingReport, NativeScratchHeadingError> {
    if config.candidate_limit == 0
        || config.maximum_wall_time.is_zero()
        || !(SCRATCH_HEADING_COUNT..=MAX_SCRATCH_REFINEMENT_HEADINGS)
            .contains(&config.heading_count)
        || !config.heading_count.is_power_of_two()
    {
        return Err(heading_error("scratch heading configuration is invalid"));
    }
    let started = Instant::now();
    let source = load_refinement_source(config)?;
    let root = config
        .scratch
        .repository_root
        .canonicalize()
        .map_err(heading_display)?;
    let authority = ValidatedNativeResidualExecution::authenticate(
        &root,
        config.scratch.optimization,
        config.scratch.execution,
    )
    .map_err(heading_display)?;
    let process_tape = InputTape::decode(
        &fs::read(root.join(&config.scratch.execution.process_boot_tape.path))
            .map_err(heading_display)?,
    )
    .map_err(heading_display)?
    .tape;
    let source_frame = usize::try_from(config.scratch.optimization.route.source_boundary_index)
        .map_err(heading_display)?;
    let prefix_frames = process_tape
        .frames
        .get(..source_frame)
        .ok_or_else(|| heading_error("scratch source frame is beyond the process tape"))?
        .to_vec();
    let catalog =
        scratch_action_catalog_with_heading_count(config.heading_count).map_err(heading_display)?;
    let action_universe_sha256 = catalog.action_schema_sha256();
    let q = ScratchQTable::new(catalog.entries().len()).map_err(heading_display)?;
    let checkpoint_path = config.output_root.join("checkpoint.dssh");
    let mut checkpoint = if checkpoint_path.exists() {
        decode_heading_checkpoint(&fs::read(&checkpoint_path).map_err(heading_display)?)?
    } else {
        if config.output_root.exists()
            && fs::read_dir(config.output_root)
                .map_err(heading_display)?
                .next()
                .is_some()
        {
            return Err(heading_error(format!(
                "scratch heading output exists without a checkpoint: {}",
                config.output_root.display()
            )));
        }
        fs::create_dir_all(config.output_root).map_err(heading_display)?;
        let search = ScratchHeadingSearch::with_heading_count(
            source.incumbent_action_sequence.clone(),
            config.heading_count,
        )
        .map_err(heading_error)?;
        let mut report = NativeScratchHeadingReport {
            schema: REPORT_SCHEMA.into(),
            report_sha256: Digest::ZERO,
            source_checkpoint_sha256: source.checkpoint_sha256,
            optimization_request_sha256: config.scratch.optimization.content_sha256,
            execution_binding_sha256: config.scratch.execution.content_sha256,
            action_universe_sha256,
            seed: source.seed,
            stop_reason: "not_started".into(),
            attempted_candidates: 0,
            terminal_candidates: 0,
            strict_winners: 0,
            candidates_remaining: search
                .remaining_candidates(source.seed, &catalog)
                .map_err(heading_error)? as u64,
            fastest_selected_ticks: source.incumbent_ticks,
            fastest_tape: None,
            fastest_tape_sha256: None,
            native_ticks: 0,
            native_wall_micros: 0,
            wall_micros: 0,
            attempts: Vec::new(),
        };
        seal_heading_report(&mut report)?;
        NativeScratchHeadingCheckpoint {
            schema: CHECKPOINT_SCHEMA.into(),
            source_checkpoint_sha256: source.checkpoint_sha256,
            optimization_request_sha256: config.scratch.optimization.content_sha256,
            execution_binding_sha256: config.scratch.execution.content_sha256,
            action_universe_sha256,
            seed: source.seed,
            search,
            report,
        }
    };
    validate_heading_checkpoint(&checkpoint, config, &source, &catalog, true)?;
    let prior_wall_micros = checkpoint.report.wall_micros;

    while checkpoint.report.attempted_candidates < config.candidate_limit
        && started.elapsed() < config.maximum_wall_time
    {
        let Some(candidate) = checkpoint
            .search
            .next_candidate(checkpoint.seed, &catalog)
            .map_err(heading_error)?
        else {
            break;
        };
        let attempt_index = checkpoint.report.attempted_candidates;
        let attempt_root = config
            .output_root
            .join("attempts")
            .join(format!("attempt-{attempt_index:06}"));
        let outcome = run_episode(
            &root,
            &config.scratch,
            &process_tape,
            &prefix_frames,
            &catalog,
            &q,
            &[],
            Some(&candidate.action_sequence),
            0,
            &attempt_root,
        )
        .map_err(heading_display)?;
        let tape_bytes = outcome.tape.encode().map_err(heading_display)?;
        let tape_sha256 = sha256(&tape_bytes);
        let selected_ticks = outcome.terminal.then(|| {
            outcome
                .tape
                .frames
                .len()
                .saturating_sub(source_frame)
                .saturating_sub(1) as u64
        });
        let strict_winner =
            selected_ticks.is_some_and(|ticks| ticks < checkpoint.report.fastest_selected_ticks);
        let mut cold_replay_attempts = Vec::new();
        if strict_winner {
            let winner_root = config
                .output_root
                .join("winners")
                .join(format!("winner-{:04}", checkpoint.report.strict_winners));
            let (controller_tape, attempts) =
                run_native_tape_cold_replay_after_execution_validation(
                    &NativeTapeColdReplayConfig {
                        repository_root: &root,
                        optimization: config.scratch.optimization,
                        execution: config.scratch.execution,
                        tape: &outcome.tape,
                        tape_bytes: &tape_bytes,
                        first_hit_tick: selected_ticks.unwrap(),
                        repetitions: 2,
                        timeout: config.scratch.cold_replay_timeout,
                        output_root: &winner_root.join("cold-replay"),
                    },
                    &authority,
                )
                .map_err(heading_display)?;
            if !exact_cold_replay_attempts(
                &attempts,
                &controller_tape,
                config.scratch.optimization.route.source_boundary_index,
                selected_ticks.unwrap(),
                outcome.tape.frames.len() as u64,
            ) {
                return Err(heading_error(
                    "scratch heading winner did not cold-replay exactly",
                ));
            }
            fs::create_dir_all(&winner_root).map_err(heading_display)?;
            write_new(&winner_root.join("selected.tape"), &tape_bytes)?;
            checkpoint.report.fastest_selected_ticks = selected_ticks.unwrap();
            checkpoint.report.fastest_tape = Some(path_text(&winner_root.join("selected.tape")));
            checkpoint.report.fastest_tape_sha256 = Some(tape_sha256);
            checkpoint.report.strict_winners += 1;
            cold_replay_attempts = attempts;
        }
        checkpoint
            .search
            .finish_attempt(
                checkpoint.seed,
                &catalog,
                &candidate,
                strict_winner.then(|| candidate.action_sequence.clone()),
            )
            .map_err(heading_error)?;
        checkpoint.report.attempted_candidates += 1;
        checkpoint.report.terminal_candidates += u64::from(outcome.terminal);
        checkpoint.report.native_ticks += outcome.native_ticks;
        checkpoint.report.native_wall_micros += outcome.native_wall_micros;
        checkpoint.report.candidates_remaining = checkpoint
            .search
            .remaining_candidates(checkpoint.seed, &catalog)
            .map_err(heading_error)? as u64;
        checkpoint
            .report
            .attempts
            .push(NativeScratchHeadingAttempt {
                attempt_index,
                candidate_sha256: candidate.action_sequence_sha256,
                changed_action_index: candidate.changed_action_index as u64,
                previous_heading_index: candidate.previous_heading_index as u64,
                replacement_heading_index: candidate.replacement_heading_index as u64,
                terminal: outcome.terminal,
                selected_ticks,
                strict_winner,
                native_ticks: outcome.native_ticks,
                native_wall_micros: outcome.native_wall_micros,
                tape_sha256,
                cold_replay_attempts,
            });
        checkpoint.report.wall_micros =
            prior_wall_micros.saturating_add(elapsed_micros(started.elapsed()));
        seal_heading_report(&mut checkpoint.report)?;
        write_heading_checkpoint(&checkpoint_path, &checkpoint)?;
        write_heading_report(&config.output_root.join("report.json"), &checkpoint.report)?;
    }
    checkpoint.report.stop_reason = if checkpoint.report.candidates_remaining == 0 {
        "candidate_exhaustion"
    } else if checkpoint.report.attempted_candidates >= config.candidate_limit {
        "candidate_limit"
    } else {
        "wall_time_limit"
    }
    .into();
    checkpoint.report.wall_micros =
        prior_wall_micros.saturating_add(elapsed_micros(started.elapsed()));
    seal_heading_report(&mut checkpoint.report)?;
    write_heading_checkpoint(&checkpoint_path, &checkpoint)?;
    write_heading_report(&config.output_root.join("report.json"), &checkpoint.report)?;
    Ok(checkpoint.report)
}

fn validate_heading_checkpoint(
    checkpoint: &NativeScratchHeadingCheckpoint,
    config: &NativeScratchHeadingRunConfig<'_>,
    source: &crate::native_scratch_learner::ScratchRefinementSource,
    catalog: &TacticAssetCatalog,
    enforce_candidate_limit: bool,
) -> Result<(), NativeScratchHeadingError> {
    checkpoint
        .search
        .validate(checkpoint.seed, catalog)
        .map_err(heading_error)?;
    let remaining = checkpoint
        .search
        .remaining_candidates(checkpoint.seed, catalog)
        .map_err(heading_error)? as u64;
    if checkpoint.schema != CHECKPOINT_SCHEMA
        || checkpoint.report.schema != REPORT_SCHEMA
        || (enforce_candidate_limit && checkpoint.search.heading_count != config.heading_count)
        || checkpoint.source_checkpoint_sha256 != source.checkpoint_sha256
        || checkpoint.optimization_request_sha256 != config.scratch.optimization.content_sha256
        || checkpoint.execution_binding_sha256 != config.scratch.execution.content_sha256
        || checkpoint.action_universe_sha256 != catalog.action_schema_sha256()
        || checkpoint.seed != source.seed
        || checkpoint.report.source_checkpoint_sha256 != checkpoint.source_checkpoint_sha256
        || checkpoint.report.optimization_request_sha256 != checkpoint.optimization_request_sha256
        || checkpoint.report.execution_binding_sha256 != checkpoint.execution_binding_sha256
        || checkpoint.report.optimization_request_sha256 != checkpoint.optimization_request_sha256
        || checkpoint.report.execution_binding_sha256 != checkpoint.execution_binding_sha256
        || checkpoint.report.action_universe_sha256 != checkpoint.action_universe_sha256
        || checkpoint.report.seed != checkpoint.seed
        || checkpoint.report.attempted_candidates as usize != checkpoint.report.attempts.len()
        || checkpoint.report.terminal_candidates
            != checkpoint
                .report
                .attempts
                .iter()
                .filter(|attempt| attempt.terminal)
                .count() as u64
        || checkpoint.report.strict_winners
            != checkpoint
                .report
                .attempts
                .iter()
                .filter(|attempt| attempt.strict_winner)
                .count() as u64
        || checkpoint
            .report
            .attempts
            .iter()
            .enumerate()
            .any(|(index, attempt)| {
                attempt.attempt_index != index as u64
                    || (attempt.strict_winner
                        && (!attempt.terminal
                            || attempt.selected_ticks.is_none()
                            || attempt.cold_replay_attempts.len() != 2))
                    || (!attempt.strict_winner && !attempt.cold_replay_attempts.is_empty())
            })
        || checkpoint.report.candidates_remaining != remaining
        || checkpoint.report.fastest_selected_ticks > source.incumbent_ticks
        || checkpoint.report.fastest_tape.is_some() != (checkpoint.report.strict_winners > 0)
        || checkpoint.report.fastest_tape_sha256.is_some() != (checkpoint.report.strict_winners > 0)
        || checkpoint.report.report_sha256 != heading_report_identity(&checkpoint.report)?
        || (enforce_candidate_limit
            && checkpoint.report.attempted_candidates > config.candidate_limit)
    {
        return Err(heading_error(
            "scratch heading checkpoint is detached or inconsistent",
        ));
    }
    Ok(())
}

fn action_sequence_sha256(sequence: &[usize]) -> Result<Digest, String> {
    let bytes = serde_cbor::to_vec(&sequence).map_err(|error| error.to_string())?;
    Ok(Digest(Sha256::digest(bytes).into()))
}

fn candidate_rank(
    seed: u64,
    incumbent_sha256: Digest,
    candidate_sha256: Digest,
    changed_action_index: usize,
    replacement_heading_index: usize,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight-scratch-heading/v1");
    hasher.update(seed.to_le_bytes());
    hasher.update(incumbent_sha256.0);
    hasher.update(candidate_sha256.0);
    hasher.update((changed_action_index as u64).to_le_bytes());
    hasher.update((replacement_heading_index as u64).to_le_bytes());
    hasher.finalize().into()
}

fn write_heading_checkpoint(
    path: &Path,
    checkpoint: &NativeScratchHeadingCheckpoint,
) -> Result<(), NativeScratchHeadingError> {
    write_atomic(path, &encode_heading_checkpoint(checkpoint)?)
}

fn encode_heading_checkpoint(
    checkpoint: &NativeScratchHeadingCheckpoint,
) -> Result<Vec<u8>, NativeScratchHeadingError> {
    let raw = serde_cbor::to_vec(checkpoint).map_err(heading_display)?;
    let compressed = zstd::stream::encode_all(Cursor::new(&raw), CHECKPOINT_COMPRESSION_LEVEL)
        .map_err(heading_display)?;
    let mut bytes = Vec::with_capacity(CHECKPOINT_HEADER_BYTES + compressed.len());
    bytes.extend_from_slice(CHECKPOINT_MAGIC);
    bytes.extend_from_slice(&CHECKPOINT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes.extend_from_slice(&(raw.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&Sha256::digest(&raw));
    bytes.extend_from_slice(&compressed);
    Ok(bytes)
}

fn decode_heading_checkpoint(
    bytes: &[u8],
) -> Result<NativeScratchHeadingCheckpoint, NativeScratchHeadingError> {
    let version = bytes
        .get(8..10)
        .and_then(|value| value.try_into().ok())
        .map(u16::from_le_bytes);
    if bytes.len() <= CHECKPOINT_HEADER_BYTES
        || &bytes[..8] != CHECKPOINT_MAGIC
        || !matches!(version, Some(1 | CHECKPOINT_VERSION))
        || bytes[10..12] != [0, 0]
    {
        return Err(heading_error(
            "scratch heading checkpoint header is invalid",
        ));
    }
    let expected_len = u64::from_le_bytes(bytes[12..20].try_into().unwrap());
    let raw = zstd::stream::decode_all(Cursor::new(&bytes[CHECKPOINT_HEADER_BYTES..]))
        .map_err(heading_display)?;
    if raw.len() as u64 != expected_len || Sha256::digest(&raw)[..] != bytes[20..52] {
        return Err(heading_error(
            "scratch heading checkpoint checksum is invalid",
        ));
    }
    serde_cbor::from_slice(&raw).map_err(heading_display)
}

pub fn inspect_native_scratch_heading_checkpoint(
    input: &Path,
) -> Result<NativeScratchHeadingInspection, NativeScratchHeadingError> {
    let checkpoint_path = if input.is_dir() {
        input.join("checkpoint.dssh")
    } else {
        input.to_path_buf()
    };
    let bytes = fs::read(&checkpoint_path).map_err(heading_display)?;
    let checkpoint = decode_heading_checkpoint(&bytes)?;
    let catalog = scratch_action_catalog_with_heading_count(checkpoint.search.heading_count)
        .map_err(heading_display)?;
    let rebuilt = ScratchHeadingSearch::with_heading_count(
        checkpoint.search.incumbent_action_sequence.clone(),
        checkpoint.search.heading_count,
    )
    .map_err(heading_error)?;
    if !matches!(
        checkpoint.schema.as_str(),
        "dusklight-native-scratch-heading-checkpoint/v1" | CHECKPOINT_SCHEMA
    ) || checkpoint.report.schema != REPORT_SCHEMA
        || checkpoint.search.incumbent_sha256 != rebuilt.incumbent_sha256
        || checkpoint.action_universe_sha256 != catalog.action_schema_sha256()
        || checkpoint.report.source_checkpoint_sha256 != checkpoint.source_checkpoint_sha256
        || checkpoint.report.action_universe_sha256 != checkpoint.action_universe_sha256
        || checkpoint.report.seed != checkpoint.seed
        || checkpoint.report.attempted_candidates as usize != checkpoint.report.attempts.len()
        || checkpoint.report.report_sha256 != heading_report_identity(&checkpoint.report)?
    {
        return Err(heading_error(
            "scratch heading checkpoint cannot be inspected safely",
        ));
    }
    let incumbent_options = checkpoint
        .search
        .incumbent_action_sequence
        .iter()
        .map(|action| {
            catalog
                .entries()
                .get(*action)
                .map(|entry| entry.option_id().to_owned())
                .ok_or_else(|| heading_error("scratch incumbent action is invalid"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut family_counts = BTreeMap::new();
    for option in &incumbent_options {
        let family = option
            .strip_prefix("scratch.")
            .and_then(|value| value.split(".h").next())
            .ok_or_else(|| heading_error("scratch option family is malformed"))?;
        *family_counts.entry(family.to_owned()).or_default() += 1;
    }
    let nominal_duration_ticks = checkpoint
        .search
        .incumbent_action_sequence
        .iter()
        .map(|action| {
            u64::from(
                catalog.entries()[*action]
                    .description()
                    .duration
                    .maximum_ticks,
            )
        })
        .sum();
    Ok(NativeScratchHeadingInspection {
        schema: INSPECTION_SCHEMA.into(),
        checkpoint_sha256: sha256(&bytes),
        checkpoint_schema: checkpoint.schema,
        source_checkpoint_sha256: checkpoint.source_checkpoint_sha256,
        optimization_request_sha256: checkpoint.optimization_request_sha256,
        execution_binding_sha256: checkpoint.execution_binding_sha256,
        action_universe_sha256: checkpoint.action_universe_sha256,
        heading_count: checkpoint.search.heading_count as u64,
        stop_reason: checkpoint.report.stop_reason,
        candidates_remaining: checkpoint.report.candidates_remaining,
        incumbent_ticks: checkpoint.report.fastest_selected_ticks,
        incumbent_action_sequence_sha256: checkpoint.search.incumbent_sha256,
        incumbent_action_count: incumbent_options.len() as u64,
        nominal_duration_ticks,
        family_counts,
        incumbent_options,
    })
}

fn write_heading_report(
    path: &Path,
    report: &NativeScratchHeadingReport,
) -> Result<(), NativeScratchHeadingError> {
    let mut bytes = serde_json::to_vec_pretty(report).map_err(heading_display)?;
    bytes.push(b'\n');
    write_atomic(path, &bytes)
}

fn seal_heading_report(
    report: &mut NativeScratchHeadingReport,
) -> Result<(), NativeScratchHeadingError> {
    report.report_sha256 = Digest::ZERO;
    report.report_sha256 = heading_report_identity(report)?;
    Ok(())
}

fn heading_report_identity(
    report: &NativeScratchHeadingReport,
) -> Result<Digest, NativeScratchHeadingError> {
    let mut canonical = report.clone();
    canonical.report_sha256 = Digest::ZERO;
    Ok(sha256(
        &serde_json::to_vec(&canonical).map_err(heading_display)?,
    ))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), NativeScratchHeadingError> {
    let parent = path
        .parent()
        .ok_or_else(|| heading_error("scratch heading output has no parent"))?;
    fs::create_dir_all(parent).map_err(heading_display)?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(heading_display)?;
    file.write_all(bytes).map_err(heading_display)?;
    file.sync_all().map_err(heading_display)?;
    drop(file);
    if path.exists() {
        fs::remove_file(path).map_err(heading_display)?;
    }
    fs::rename(&temporary, path).map_err(heading_display)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), NativeScratchHeadingError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(heading_display)?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(heading_display)?;
    file.write_all(bytes).map_err(heading_display)?;
    file.sync_all().map_err(heading_display)
}

fn sha256(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

fn elapsed_micros(duration: Duration) -> u64 {
    duration.as_micros().min(u128::from(u64::MAX)) as u64
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeScratchHeadingError(String);

impl fmt::Display for NativeScratchHeadingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NativeScratchHeadingError {}

fn heading_error(message: impl Into<String>) -> NativeScratchHeadingError {
    NativeScratchHeadingError(message.into())
}

fn heading_display(error: impl fmt::Display) -> NativeScratchHeadingError {
    heading_error(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(catalog: &TacticAssetCatalog, option_id: &str) -> usize {
        catalog
            .entries()
            .binary_search_by_key(&option_id, |entry| entry.option_id())
            .unwrap()
    }

    #[test]
    fn candidates_replace_only_heading_and_cover_both_neighbors() {
        let catalog = scratch_action_catalog_with_heading_count(16).unwrap();
        let action = action(&catalog, "scratch.move.h03.t08");
        let search =
            ScratchHeadingSearch::with_heading_count(vec![action], SCRATCH_HEADING_COUNT).unwrap();
        let candidates = search.candidates(17, &catalog).unwrap();
        assert_eq!(candidates.len(), 2);
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.replacement_heading_index)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([2, 4])
        );
        assert!(candidates.iter().all(|candidate| {
            ["scratch.move.h02.t08", "scratch.move.h04.t08"]
                .contains(&catalog.entries()[candidate.action_sequence[0]].option_id())
                && candidate.previous_heading_index == 3
        }));
    }

    #[test]
    fn headings_wrap_and_candidates_are_unique() {
        let catalog = scratch_action_catalog_with_heading_count(16).unwrap();
        let raw = action(&catalog, "scratch.raw.h00.t01");
        let search =
            ScratchHeadingSearch::with_heading_count(vec![raw, raw], SCRATCH_HEADING_COUNT)
                .unwrap();
        let candidates = search.candidates(19, &catalog).unwrap();
        assert_eq!(candidates.len(), 4);
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.replacement_heading_index)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([1, 15])
        );
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.action_sequence.clone())
                .collect::<BTreeSet<_>>()
                .len(),
            4
        );
    }

    #[test]
    fn finer_search_inserts_midpoints_without_changing_action_variants() {
        let coarse_catalog = scratch_action_catalog_with_heading_count(16).unwrap();
        let fine_catalog = scratch_action_catalog_with_heading_count(32).unwrap();
        let coarse = vec![action(&coarse_catalog, "scratch.camera_roll.h03.t08.s1")];
        let promoted = promote_heading_resolution(&coarse, &coarse_catalog, &fine_catalog).unwrap();
        assert_eq!(
            fine_catalog.entries()[promoted[0]].option_id(),
            "scratch.camera_roll.h06.t08.s1"
        );
        let search = ScratchHeadingSearch::with_heading_count(promoted, 32).unwrap();
        let candidates = search.candidates(17, &fine_catalog).unwrap();
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.replacement_heading_index)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([5, 7])
        );
        assert!(candidates.iter().all(|candidate| {
            [
                "scratch.camera_roll.h05.t08.s1",
                "scratch.camera_roll.h07.t08.s1",
            ]
            .contains(&fine_catalog.entries()[candidate.action_sequence[0]].option_id())
                && candidate.previous_heading_index == 6
        }));
        assert!(promote_heading_resolution(&coarse, &coarse_catalog, &coarse_catalog).is_err());
    }

    #[test]
    fn progress_resumes_and_acceptance_resets_to_the_new_incumbent() {
        let catalog = scratch_action_catalog_with_heading_count(16).unwrap();
        let sequence = vec![
            action(&catalog, "scratch.raw.h00.t01"),
            action(&catalog, "scratch.move.h01.t04"),
        ];
        let mut search =
            ScratchHeadingSearch::with_heading_count(sequence, SCRATCH_HEADING_COUNT).unwrap();
        let first = search.next_candidate(23, &catalog).unwrap().unwrap();
        search.finish_attempt(23, &catalog, &first, None).unwrap();
        let encoded = serde_cbor::to_vec(&search).unwrap();
        let mut resumed: ScratchHeadingSearch = serde_cbor::from_slice(&encoded).unwrap();
        let second = resumed.next_candidate(23, &catalog).unwrap().unwrap();
        assert_ne!(first.action_sequence_sha256, second.action_sequence_sha256);
        resumed
            .finish_attempt(23, &catalog, &second, Some(second.action_sequence.clone()))
            .unwrap();
        assert_eq!(resumed.incumbent_action_sequence, second.action_sequence);
        assert!(resumed.attempted_candidate_sha256s.is_empty());
        assert_eq!(resumed.remaining_candidates(23, &catalog).unwrap(), 4);
    }

    #[test]
    fn binary_checkpoint_round_trips_and_rejects_corruption() {
        let catalog = scratch_action_catalog_with_heading_count(16).unwrap();
        let search = ScratchHeadingSearch::with_heading_count(
            vec![action(&catalog, "scratch.raw.h00.t01")],
            SCRATCH_HEADING_COUNT,
        )
        .unwrap();
        let mut report = NativeScratchHeadingReport {
            schema: REPORT_SCHEMA.into(),
            report_sha256: Digest::ZERO,
            source_checkpoint_sha256: Digest([1; 32]),
            optimization_request_sha256: Digest([2; 32]),
            execution_binding_sha256: Digest([3; 32]),
            action_universe_sha256: Digest([4; 32]),
            seed: 5,
            stop_reason: "not_started".into(),
            attempted_candidates: 0,
            terminal_candidates: 0,
            strict_winners: 0,
            candidates_remaining: search.remaining_candidates(5, &catalog).unwrap() as u64,
            fastest_selected_ticks: 360,
            fastest_tape: None,
            fastest_tape_sha256: None,
            native_ticks: 0,
            native_wall_micros: 0,
            wall_micros: 0,
            attempts: Vec::new(),
        };
        seal_heading_report(&mut report).unwrap();
        let checkpoint = NativeScratchHeadingCheckpoint {
            schema: CHECKPOINT_SCHEMA.into(),
            source_checkpoint_sha256: report.source_checkpoint_sha256,
            optimization_request_sha256: report.optimization_request_sha256,
            execution_binding_sha256: report.execution_binding_sha256,
            action_universe_sha256: report.action_universe_sha256,
            seed: report.seed,
            search,
            report,
        };
        let encoded = encode_heading_checkpoint(&checkpoint).unwrap();
        assert_eq!(decode_heading_checkpoint(&encoded).unwrap(), checkpoint);
        let mut legacy_header = encoded.clone();
        legacy_header[8..10].copy_from_slice(&1_u16.to_le_bytes());
        assert_eq!(
            decode_heading_checkpoint(&legacy_header).unwrap(),
            checkpoint
        );
        let mut corrupted = encoded;
        let last = corrupted.len() - 1;
        corrupted[last] ^= 0x80;
        assert!(decode_heading_checkpoint(&corrupted).is_err());
    }
}
