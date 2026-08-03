//! Deterministic single-option refinement of an authenticated scratch-route incumbent.

use crate::native_residual_campaign::ValidatedNativeResidualExecution;
use crate::native_scratch_incumbent::{
    AuthenticatedScratchIncumbent, ScratchIncumbentSource, load_authenticated_scratch_incumbent,
};
use crate::native_scratch_learner::{NativeScratchRunConfig, run_episode};
use crate::native_tactic_route_runner::{
    NativeTacticColdReplayAttempt, NativeTapeColdReplayConfig, exact_cold_replay_attempts,
    run_native_tape_cold_replay_after_execution_validation,
};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::InputTape;
use dusklight_learning::scratch_action_catalog::{
    SCRATCH_HEADING_COUNT, scratch_action_catalog, scratch_action_heading_index,
};
use dusklight_learning::scratch_q::ScratchQTable;
use dusklight_learning::tactic_asset::TacticAssetCatalog;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Write};
use std::path::Path;
use std::time::{Duration, Instant};

const REPORT_SCHEMA: &str = "dusklight-native-scratch-option-refinement-report/v1";
const INSPECTION_SCHEMA: &str = "dusklight-native-scratch-option-refinement-inspection/v1";
const CHECKPOINT_SCHEMA: &str = "dusklight-native-scratch-option-refinement-checkpoint/v1";
const CHECKPOINT_MAGIC: &[u8; 8] = b"DSSOPT01";
const CHECKPOINT_VERSION: u16 = 1;
const CHECKPOINT_HEADER_BYTES: usize = 8 + 2 + 2 + 8 + 32;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScratchOptionEditKind {
    ShortenDuration,
    PromoteRoll,
}

pub struct NativeScratchOptionRunConfig<'a> {
    pub scratch: NativeScratchRunConfig<'a>,
    pub source: ScratchIncumbentSource<'a>,
    pub edit_kind: ScratchOptionEditKind,
    pub output_root: &'a Path,
    pub candidate_limit: u64,
    pub maximum_wall_time: Duration,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeScratchOptionReport {
    pub schema: String,
    pub report_sha256: Digest,
    pub source_checkpoint_sha256: Digest,
    pub optimization_request_sha256: Digest,
    pub execution_binding_sha256: Digest,
    pub action_universe_sha256: Digest,
    pub seed: u64,
    pub edit_kind: ScratchOptionEditKind,
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
    pub attempts: Vec<NativeScratchOptionAttempt>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeScratchOptionAttempt {
    pub attempt_index: u64,
    pub candidate_sha256: Digest,
    pub changed_action_index: u64,
    pub previous_option_id: String,
    pub replacement_option_id: String,
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
pub struct NativeScratchOptionInspection {
    pub schema: String,
    pub checkpoint_sha256: Digest,
    pub checkpoint_schema: String,
    pub source_checkpoint_sha256: Digest,
    pub optimization_request_sha256: Digest,
    pub execution_binding_sha256: Digest,
    pub action_universe_sha256: Digest,
    pub seed: u64,
    pub edit_kind: ScratchOptionEditKind,
    pub stop_reason: String,
    pub candidates_remaining: u64,
    pub incumbent_ticks: u64,
    pub incumbent_action_sequence_sha256: Digest,
    pub incumbent_options: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct NativeScratchOptionCheckpoint {
    schema: String,
    source_checkpoint_sha256: Digest,
    optimization_request_sha256: Digest,
    execution_binding_sha256: Digest,
    action_universe_sha256: Digest,
    seed: u64,
    edit_kind: ScratchOptionEditKind,
    search: ScratchOptionSearch,
    report: NativeScratchOptionReport,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ScratchOptionSearch {
    incumbent_action_sequence: Vec<usize>,
    incumbent_sha256: Digest,
    attempted_candidate_sha256s: BTreeSet<Digest>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ScratchOptionCandidate {
    changed_action_index: usize,
    previous_option_id: String,
    replacement_option_id: String,
    action_sequence: Vec<usize>,
    action_sequence_sha256: Digest,
}

impl ScratchOptionSearch {
    fn new(
        incumbent_action_sequence: Vec<usize>,
        catalog: &TacticAssetCatalog,
    ) -> Result<Self, String> {
        if incumbent_action_sequence.is_empty()
            || incumbent_action_sequence
                .iter()
                .any(|action| *action >= catalog.entries().len())
        {
            return Err("scratch option incumbent is invalid".into());
        }
        Ok(Self {
            incumbent_sha256: action_sequence_sha256(&incumbent_action_sequence)?,
            incumbent_action_sequence,
            attempted_candidate_sha256s: BTreeSet::new(),
        })
    }

    fn validate(
        &self,
        seed: u64,
        catalog: &TacticAssetCatalog,
        edit_kind: ScratchOptionEditKind,
    ) -> Result<(), String> {
        let rebuilt = Self::new(self.incumbent_action_sequence.clone(), catalog)?;
        if rebuilt.incumbent_sha256 != self.incumbent_sha256 {
            return Err("scratch option incumbent identity is invalid".into());
        }
        let candidates = self
            .candidates(seed, catalog, edit_kind)?
            .into_iter()
            .map(|candidate| candidate.action_sequence_sha256)
            .collect::<BTreeSet<_>>();
        if !self.attempted_candidate_sha256s.is_subset(&candidates) {
            return Err("scratch option attempts are detached from the incumbent".into());
        }
        Ok(())
    }

    fn next_candidate(
        &self,
        seed: u64,
        catalog: &TacticAssetCatalog,
        edit_kind: ScratchOptionEditKind,
    ) -> Result<Option<ScratchOptionCandidate>, String> {
        Ok(self
            .candidates(seed, catalog, edit_kind)?
            .into_iter()
            .find(|candidate| {
                !self
                    .attempted_candidate_sha256s
                    .contains(&candidate.action_sequence_sha256)
            }))
    }

    fn finish_attempt(
        &mut self,
        seed: u64,
        catalog: &TacticAssetCatalog,
        edit_kind: ScratchOptionEditKind,
        candidate: &ScratchOptionCandidate,
        accepted: Option<Vec<usize>>,
    ) -> Result<(), String> {
        let expected = self
            .candidates(seed, catalog, edit_kind)?
            .into_iter()
            .find(|value| value.action_sequence_sha256 == candidate.action_sequence_sha256)
            .ok_or_else(|| "scratch option candidate is detached".to_owned())?;
        if expected != *candidate {
            return Err("scratch option candidate identity is inconsistent".into());
        }
        if let Some(incumbent) = accepted {
            *self = Self::new(incumbent, catalog)?;
        } else if !self
            .attempted_candidate_sha256s
            .insert(candidate.action_sequence_sha256)
        {
            return Err("scratch option candidate was attempted twice".into());
        }
        Ok(())
    }

    fn remaining(
        &self,
        seed: u64,
        catalog: &TacticAssetCatalog,
        edit_kind: ScratchOptionEditKind,
    ) -> Result<usize, String> {
        Ok(self
            .candidates(seed, catalog, edit_kind)?
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
        edit_kind: ScratchOptionEditKind,
    ) -> Result<Vec<ScratchOptionCandidate>, String> {
        let mut unique = BTreeSet::new();
        let mut ranked = Vec::new();
        for (changed_action_index, action) in
            self.incumbent_action_sequence.iter().copied().enumerate()
        {
            let previous = catalog.entries()[action].option_id();
            let Some(replacement_option_id) = replacement_option(action, catalog, edit_kind) else {
                continue;
            };
            let replacement_action = catalog
                .entries()
                .binary_search_by_key(&replacement_option_id.as_str(), |entry| entry.option_id())
                .map_err(|_| "shorter scratch option is missing".to_owned())?;
            let mut action_sequence = self.incumbent_action_sequence.clone();
            action_sequence[changed_action_index] = replacement_action;
            if !unique.insert(action_sequence.clone()) {
                continue;
            }
            let action_sequence_sha256 = action_sequence_sha256(&action_sequence)?;
            ranked.push((
                candidate_rank(
                    seed,
                    edit_kind,
                    self.incumbent_sha256,
                    action_sequence_sha256,
                    changed_action_index,
                ),
                ScratchOptionCandidate {
                    changed_action_index,
                    previous_option_id: previous.into(),
                    replacement_option_id,
                    action_sequence,
                    action_sequence_sha256,
                },
            ));
        }
        ranked.sort_by_key(|(rank, candidate)| {
            (
                *rank,
                candidate.action_sequence_sha256,
                candidate.changed_action_index,
            )
        });
        Ok(ranked.into_iter().map(|(_, candidate)| candidate).collect())
    }
}

fn replacement_option(
    action: usize,
    catalog: &TacticAssetCatalog,
    edit_kind: ScratchOptionEditKind,
) -> Option<String> {
    let option_id = catalog.entries().get(action)?.option_id();
    if edit_kind == ScratchOptionEditKind::PromoteRoll {
        if option_id.starts_with("scratch.roll.") {
            return None;
        }
        let heading = scratch_action_heading_index(catalog, action).ok()?;
        let replacement = format!("scratch.roll.h{heading:02}.r03");
        return catalog.entry(&replacement).map(|_| replacement);
    }
    let mut components = option_id.split('.').map(str::to_owned).collect::<Vec<_>>();
    let component = components
        .iter_mut()
        .find(|component| matches!(component.as_str(), "t16" | "t08" | "r07"))?;
    *component = match component.as_str() {
        "t16" => "t08".into(),
        "t08" => "t04".into(),
        "r07" => "r03".into(),
        _ => return None,
    };
    let replacement = components.join(".");
    catalog.entry(&replacement).map(|_| replacement)
}

pub fn run_native_scratch_option_refinement(
    config: &NativeScratchOptionRunConfig<'_>,
) -> Result<NativeScratchOptionReport, NativeScratchOptionError> {
    if config.candidate_limit == 0 || config.maximum_wall_time.is_zero() {
        return Err(option_error("scratch option configuration is invalid"));
    }
    let started = Instant::now();
    let source =
        load_authenticated_scratch_incumbent(&config.scratch, config.source, SCRATCH_HEADING_COUNT)
            .map_err(option_display)?;
    let root = config
        .scratch
        .repository_root
        .canonicalize()
        .map_err(option_display)?;
    let authority = ValidatedNativeResidualExecution::authenticate(
        &root,
        config.scratch.optimization,
        config.scratch.execution,
    )
    .map_err(option_display)?;
    let process_tape = InputTape::decode(
        &fs::read(root.join(&config.scratch.execution.process_boot_tape.path))
            .map_err(option_display)?,
    )
    .map_err(option_display)?
    .tape;
    let source_frame = usize::try_from(config.scratch.optimization.route.source_boundary_index)
        .map_err(option_display)?;
    let prefix_frames = process_tape
        .frames
        .get(..source_frame)
        .ok_or_else(|| option_error("scratch source frame is beyond the process tape"))?
        .to_vec();
    let catalog = scratch_action_catalog().map_err(option_display)?;
    let action_universe_sha256 = catalog.action_schema_sha256();
    if source.heading_count != SCRATCH_HEADING_COUNT {
        return Err(option_error(
            "scratch option source heading count is unsupported",
        ));
    }
    let q = ScratchQTable::new(catalog.entries().len()).map_err(option_display)?;
    let checkpoint_path = config.output_root.join("checkpoint.dsso");
    let mut checkpoint = if checkpoint_path.exists() {
        decode_checkpoint(&fs::read(&checkpoint_path).map_err(option_display)?)?
    } else {
        if config.output_root.exists()
            && fs::read_dir(config.output_root)
                .map_err(option_display)?
                .next()
                .is_some()
        {
            return Err(option_error(
                "scratch option output exists without a checkpoint",
            ));
        }
        fs::create_dir_all(config.output_root).map_err(option_display)?;
        let search = ScratchOptionSearch::new(source.incumbent_action_sequence.clone(), &catalog)
            .map_err(option_error)?;
        let mut report = NativeScratchOptionReport {
            schema: REPORT_SCHEMA.into(),
            report_sha256: Digest::ZERO,
            source_checkpoint_sha256: source.checkpoint_sha256,
            optimization_request_sha256: config.scratch.optimization.content_sha256,
            execution_binding_sha256: config.scratch.execution.content_sha256,
            action_universe_sha256,
            seed: source.seed,
            edit_kind: config.edit_kind,
            stop_reason: "not_started".into(),
            attempted_candidates: 0,
            terminal_candidates: 0,
            strict_winners: 0,
            candidates_remaining: search
                .remaining(source.seed, &catalog, config.edit_kind)
                .map_err(option_error)? as u64,
            fastest_selected_ticks: source.incumbent_ticks,
            fastest_tape: None,
            fastest_tape_sha256: None,
            native_ticks: 0,
            native_wall_micros: 0,
            wall_micros: 0,
            attempts: Vec::new(),
        };
        seal_report(&mut report)?;
        NativeScratchOptionCheckpoint {
            schema: CHECKPOINT_SCHEMA.into(),
            source_checkpoint_sha256: source.checkpoint_sha256,
            optimization_request_sha256: config.scratch.optimization.content_sha256,
            execution_binding_sha256: config.scratch.execution.content_sha256,
            action_universe_sha256,
            seed: source.seed,
            edit_kind: config.edit_kind,
            search,
            report,
        }
    };
    validate_checkpoint(&checkpoint, config, &source, &catalog)?;
    let prior_wall_micros = checkpoint.report.wall_micros;
    while checkpoint.report.attempted_candidates < config.candidate_limit
        && started.elapsed() < config.maximum_wall_time
    {
        let Some(candidate) = checkpoint
            .search
            .next_candidate(checkpoint.seed, &catalog, checkpoint.edit_kind)
            .map_err(option_error)?
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
        .map_err(option_display)?;
        let tape_bytes = outcome.tape.encode().map_err(option_display)?;
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
                .map_err(option_display)?;
            if !exact_cold_replay_attempts(
                &attempts,
                &controller_tape,
                config.scratch.optimization.route.source_boundary_index,
                selected_ticks.unwrap(),
                outcome.tape.frames.len() as u64,
            ) {
                return Err(option_error("scratch option winner did not replay exactly"));
            }
            fs::create_dir_all(&winner_root).map_err(option_display)?;
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
                checkpoint.edit_kind,
                &candidate,
                strict_winner.then(|| candidate.action_sequence.clone()),
            )
            .map_err(option_error)?;
        checkpoint.report.attempted_candidates += 1;
        checkpoint.report.terminal_candidates += u64::from(outcome.terminal);
        checkpoint.report.native_ticks += outcome.native_ticks;
        checkpoint.report.native_wall_micros += outcome.native_wall_micros;
        checkpoint.report.candidates_remaining = checkpoint
            .search
            .remaining(checkpoint.seed, &catalog, checkpoint.edit_kind)
            .map_err(option_error)? as u64;
        checkpoint.report.attempts.push(NativeScratchOptionAttempt {
            attempt_index,
            candidate_sha256: candidate.action_sequence_sha256,
            changed_action_index: candidate.changed_action_index as u64,
            previous_option_id: candidate.previous_option_id,
            replacement_option_id: candidate.replacement_option_id,
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
        seal_report(&mut checkpoint.report)?;
        write_checkpoint(&checkpoint_path, &checkpoint)?;
        write_report(&config.output_root.join("report.json"), &checkpoint.report)?;
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
    seal_report(&mut checkpoint.report)?;
    write_checkpoint(&checkpoint_path, &checkpoint)?;
    write_report(&config.output_root.join("report.json"), &checkpoint.report)?;
    Ok(checkpoint.report)
}

fn validate_checkpoint(
    checkpoint: &NativeScratchOptionCheckpoint,
    config: &NativeScratchOptionRunConfig<'_>,
    source: &AuthenticatedScratchIncumbent,
    catalog: &TacticAssetCatalog,
) -> Result<(), NativeScratchOptionError> {
    checkpoint
        .search
        .validate(checkpoint.seed, catalog, checkpoint.edit_kind)
        .map_err(option_error)?;
    let remaining = checkpoint
        .search
        .remaining(checkpoint.seed, catalog, checkpoint.edit_kind)
        .map_err(option_error)? as u64;
    if checkpoint.schema != CHECKPOINT_SCHEMA
        || checkpoint.report.schema != REPORT_SCHEMA
        || checkpoint.edit_kind != config.edit_kind
        || checkpoint.source_checkpoint_sha256 != source.checkpoint_sha256
        || checkpoint.optimization_request_sha256 != config.scratch.optimization.content_sha256
        || checkpoint.execution_binding_sha256 != config.scratch.execution.content_sha256
        || checkpoint.action_universe_sha256 != catalog.action_schema_sha256()
        || checkpoint.seed != config.scratch.seed
        || checkpoint.report.source_checkpoint_sha256 != checkpoint.source_checkpoint_sha256
        || checkpoint.report.optimization_request_sha256 != checkpoint.optimization_request_sha256
        || checkpoint.report.execution_binding_sha256 != checkpoint.execution_binding_sha256
        || checkpoint.report.action_universe_sha256 != checkpoint.action_universe_sha256
        || checkpoint.report.seed != checkpoint.seed
        || checkpoint.report.edit_kind != checkpoint.edit_kind
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
        || checkpoint.report.report_sha256 != report_identity(&checkpoint.report)?
        || checkpoint.report.attempted_candidates > config.candidate_limit
    {
        return Err(option_error("scratch option checkpoint is detached"));
    }
    Ok(())
}

fn action_sequence_sha256(sequence: &[usize]) -> Result<Digest, String> {
    serde_cbor::to_vec(&sequence)
        .map(|bytes| sha256(&bytes))
        .map_err(|error| error.to_string())
}

fn candidate_rank(
    seed: u64,
    edit_kind: ScratchOptionEditKind,
    incumbent: Digest,
    candidate: Digest,
    index: usize,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight-scratch-option/v1");
    hasher.update(seed.to_le_bytes());
    hasher.update([edit_kind as u8]);
    hasher.update(incumbent.0);
    hasher.update(candidate.0);
    hasher.update((index as u64).to_le_bytes());
    hasher.finalize().into()
}

fn write_checkpoint(
    path: &Path,
    checkpoint: &NativeScratchOptionCheckpoint,
) -> Result<(), NativeScratchOptionError> {
    write_atomic(path, &encode_checkpoint(checkpoint)?)
}

fn encode_checkpoint(
    checkpoint: &NativeScratchOptionCheckpoint,
) -> Result<Vec<u8>, NativeScratchOptionError> {
    let raw = serde_cbor::to_vec(checkpoint).map_err(option_display)?;
    let compressed = zstd::stream::encode_all(Cursor::new(&raw), 1).map_err(option_display)?;
    let mut bytes = Vec::with_capacity(CHECKPOINT_HEADER_BYTES + compressed.len());
    bytes.extend_from_slice(CHECKPOINT_MAGIC);
    bytes.extend_from_slice(&CHECKPOINT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes.extend_from_slice(&(raw.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&Sha256::digest(&raw));
    bytes.extend_from_slice(&compressed);
    Ok(bytes)
}

fn decode_checkpoint(
    bytes: &[u8],
) -> Result<NativeScratchOptionCheckpoint, NativeScratchOptionError> {
    if bytes.len() <= CHECKPOINT_HEADER_BYTES
        || &bytes[..8] != CHECKPOINT_MAGIC
        || u16::from_le_bytes(bytes[8..10].try_into().unwrap()) != CHECKPOINT_VERSION
        || bytes[10..12] != [0, 0]
    {
        return Err(option_error("scratch option checkpoint header is invalid"));
    }
    let expected_len = u64::from_le_bytes(bytes[12..20].try_into().unwrap());
    let raw = zstd::stream::decode_all(Cursor::new(&bytes[CHECKPOINT_HEADER_BYTES..]))
        .map_err(option_display)?;
    if raw.len() as u64 != expected_len || Sha256::digest(&raw)[..] != bytes[20..52] {
        return Err(option_error(
            "scratch option checkpoint checksum is invalid",
        ));
    }
    serde_cbor::from_slice(&raw).map_err(option_display)
}

pub fn inspect_native_scratch_option_checkpoint(
    input: &Path,
) -> Result<NativeScratchOptionInspection, NativeScratchOptionError> {
    let checkpoint_path = if input.is_dir() {
        input.join("checkpoint.dsso")
    } else {
        input.to_path_buf()
    };
    let bytes = fs::read(&checkpoint_path).map_err(option_display)?;
    let checkpoint = decode_checkpoint(&bytes)?;
    let catalog = scratch_action_catalog().map_err(option_display)?;
    checkpoint
        .search
        .validate(checkpoint.seed, &catalog, checkpoint.edit_kind)
        .map_err(option_error)?;
    let remaining = checkpoint
        .search
        .remaining(checkpoint.seed, &catalog, checkpoint.edit_kind)
        .map_err(option_error)? as u64;
    if checkpoint.schema != CHECKPOINT_SCHEMA
        || checkpoint.report.schema != REPORT_SCHEMA
        || checkpoint.action_universe_sha256 != catalog.action_schema_sha256()
        || checkpoint.report.source_checkpoint_sha256 != checkpoint.source_checkpoint_sha256
        || checkpoint.report.optimization_request_sha256 != checkpoint.optimization_request_sha256
        || checkpoint.report.execution_binding_sha256 != checkpoint.execution_binding_sha256
        || checkpoint.report.action_universe_sha256 != checkpoint.action_universe_sha256
        || checkpoint.report.seed != checkpoint.seed
        || checkpoint.report.edit_kind != checkpoint.edit_kind
        || checkpoint.report.attempted_candidates as usize != checkpoint.report.attempts.len()
        || checkpoint.report.candidates_remaining != remaining
        || checkpoint.report.report_sha256 != report_identity(&checkpoint.report)?
    {
        return Err(option_error(
            "scratch option checkpoint cannot be inspected safely",
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
                .ok_or_else(|| option_error("scratch option incumbent action is invalid"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(NativeScratchOptionInspection {
        schema: INSPECTION_SCHEMA.into(),
        checkpoint_sha256: sha256(&bytes),
        checkpoint_schema: checkpoint.schema,
        source_checkpoint_sha256: checkpoint.source_checkpoint_sha256,
        optimization_request_sha256: checkpoint.optimization_request_sha256,
        execution_binding_sha256: checkpoint.execution_binding_sha256,
        action_universe_sha256: checkpoint.action_universe_sha256,
        seed: checkpoint.seed,
        edit_kind: checkpoint.edit_kind,
        stop_reason: checkpoint.report.stop_reason,
        candidates_remaining: checkpoint.report.candidates_remaining,
        incumbent_ticks: checkpoint.report.fastest_selected_ticks,
        incumbent_action_sequence_sha256: checkpoint.search.incumbent_sha256,
        incumbent_options,
    })
}

fn write_report(
    path: &Path,
    report: &NativeScratchOptionReport,
) -> Result<(), NativeScratchOptionError> {
    let mut bytes = serde_json::to_vec_pretty(report).map_err(option_display)?;
    bytes.push(b'\n');
    write_atomic(path, &bytes)
}

fn seal_report(report: &mut NativeScratchOptionReport) -> Result<(), NativeScratchOptionError> {
    report.report_sha256 = Digest::ZERO;
    report.report_sha256 = report_identity(report)?;
    Ok(())
}

fn report_identity(report: &NativeScratchOptionReport) -> Result<Digest, NativeScratchOptionError> {
    let mut canonical = report.clone();
    canonical.report_sha256 = Digest::ZERO;
    Ok(sha256(
        &serde_json::to_vec(&canonical).map_err(option_display)?,
    ))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), NativeScratchOptionError> {
    let parent = path
        .parent()
        .ok_or_else(|| option_error("scratch option output has no parent"))?;
    fs::create_dir_all(parent).map_err(option_display)?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(option_display)?;
    file.write_all(bytes).map_err(option_display)?;
    file.sync_all().map_err(option_display)?;
    drop(file);
    if path.exists() {
        fs::remove_file(path).map_err(option_display)?;
    }
    fs::rename(temporary, path).map_err(option_display)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), NativeScratchOptionError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(option_display)?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(option_display)?;
    file.write_all(bytes).map_err(option_display)?;
    file.sync_all().map_err(option_display)
}

fn sha256(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}
fn elapsed_micros(value: Duration) -> u64 {
    value.as_micros().min(u128::from(u64::MAX)) as u64
}
fn path_text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeScratchOptionError(String);
impl fmt::Display for NativeScratchOptionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}
impl Error for NativeScratchOptionError {}
fn option_error(message: impl Into<String>) -> NativeScratchOptionError {
    NativeScratchOptionError(message.into())
}
fn option_display(error: impl fmt::Display) -> NativeScratchOptionError {
    option_error(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(catalog: &TacticAssetCatalog, option: &str) -> usize {
        catalog
            .entries()
            .binary_search_by_key(&option, |entry| entry.option_id())
            .unwrap()
    }

    fn replacement(
        catalog: &TacticAssetCatalog,
        option: &str,
        edit_kind: ScratchOptionEditKind,
    ) -> Option<String> {
        replacement_option(action(catalog, option), catalog, edit_kind)
    }

    #[test]
    fn shortening_preserves_family_heading_and_schedule() {
        let catalog = scratch_action_catalog().unwrap();
        assert_eq!(
            replacement(
                &catalog,
                "scratch.camera_move.h03.t16.l1",
                ScratchOptionEditKind::ShortenDuration,
            )
            .as_deref(),
            Some("scratch.camera_move.h03.t08.l1")
        );
        assert_eq!(
            replacement(
                &catalog,
                "scratch.camera_roll.h14.t16.s2",
                ScratchOptionEditKind::ShortenDuration,
            )
            .as_deref(),
            Some("scratch.camera_roll.h14.t08.s2")
        );
        assert_eq!(
            replacement(
                &catalog,
                "scratch.move.h09.t08",
                ScratchOptionEditKind::ShortenDuration,
            )
            .as_deref(),
            Some("scratch.move.h09.t04")
        );
        assert_eq!(
            replacement(
                &catalog,
                "scratch.roll.h02.r07",
                ScratchOptionEditKind::ShortenDuration,
            )
            .as_deref(),
            Some("scratch.roll.h02.r03")
        );
        assert_eq!(
            replacement(
                &catalog,
                "scratch.camera_move.h03.t08.l1",
                ScratchOptionEditKind::ShortenDuration,
            ),
            None
        );
    }

    #[test]
    fn roll_promotion_preserves_semantic_heading_and_uses_short_recovery() {
        let catalog = scratch_action_catalog().unwrap();
        assert_eq!(
            replacement(
                &catalog,
                "scratch.camera_move.h03.t08.l1",
                ScratchOptionEditKind::PromoteRoll,
            )
            .as_deref(),
            Some("scratch.roll.h03.r03")
        );
        assert_eq!(
            replacement(
                &catalog,
                "scratch.roll.h03.r07",
                ScratchOptionEditKind::PromoteRoll,
            ),
            None
        );
    }

    #[test]
    fn candidates_are_bounded_unique_and_resume() {
        let catalog = scratch_action_catalog().unwrap();
        let sequence = vec![
            action(&catalog, "scratch.camera_move.h03.t16.l1"),
            action(&catalog, "scratch.move.h09.t08"),
            action(&catalog, "scratch.raw.h00.t01"),
        ];
        let mut search = ScratchOptionSearch::new(sequence, &catalog).unwrap();
        let kind = ScratchOptionEditKind::ShortenDuration;
        assert_eq!(search.remaining(7, &catalog, kind).unwrap(), 2);
        let first = search.next_candidate(7, &catalog, kind).unwrap().unwrap();
        search
            .finish_attempt(7, &catalog, kind, &first, None)
            .unwrap();
        let encoded = serde_cbor::to_vec(&search).unwrap();
        let resumed: ScratchOptionSearch = serde_cbor::from_slice(&encoded).unwrap();
        assert_eq!(resumed.remaining(7, &catalog, kind).unwrap(), 1);
    }

    #[test]
    fn binary_checkpoint_round_trips_and_rejects_corruption() {
        let catalog = scratch_action_catalog().unwrap();
        let search =
            ScratchOptionSearch::new(vec![action(&catalog, "scratch.move.h09.t08")], &catalog)
                .unwrap();
        let mut report = NativeScratchOptionReport {
            schema: REPORT_SCHEMA.into(),
            report_sha256: Digest::ZERO,
            source_checkpoint_sha256: Digest([1; 32]),
            optimization_request_sha256: Digest([2; 32]),
            execution_binding_sha256: Digest([3; 32]),
            action_universe_sha256: catalog.action_schema_sha256(),
            seed: 5,
            edit_kind: ScratchOptionEditKind::ShortenDuration,
            stop_reason: "not_started".into(),
            attempted_candidates: 0,
            terminal_candidates: 0,
            strict_winners: 0,
            candidates_remaining: search
                .remaining(5, &catalog, ScratchOptionEditKind::ShortenDuration)
                .unwrap() as u64,
            fastest_selected_ticks: 333,
            fastest_tape: None,
            fastest_tape_sha256: None,
            native_ticks: 0,
            native_wall_micros: 0,
            wall_micros: 0,
            attempts: Vec::new(),
        };
        seal_report(&mut report).unwrap();
        let checkpoint = NativeScratchOptionCheckpoint {
            schema: CHECKPOINT_SCHEMA.into(),
            source_checkpoint_sha256: report.source_checkpoint_sha256,
            optimization_request_sha256: report.optimization_request_sha256,
            execution_binding_sha256: report.execution_binding_sha256,
            action_universe_sha256: report.action_universe_sha256,
            seed: report.seed,
            edit_kind: report.edit_kind,
            search,
            report,
        };
        let encoded = encode_checkpoint(&checkpoint).unwrap();
        assert_eq!(decode_checkpoint(&encoded).unwrap(), checkpoint);
        let mut corrupted = encoded;
        let last = corrupted.len() - 1;
        corrupted[last] ^= 0x80;
        assert!(decode_checkpoint(&corrupted).is_err());
    }
}
