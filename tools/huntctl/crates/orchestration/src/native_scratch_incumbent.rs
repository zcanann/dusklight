//! Common authenticated incumbent contract for composable scratch refinements.

use crate::native_residual_campaign::{
    NativeResidualExecutionBinding, ValidatedNativeResidualExecution,
};
use crate::native_scratch_heading::inspect_native_scratch_heading_checkpoint;
use crate::native_scratch_learner::{
    NativeScratchRunConfig, ScratchRefinementSource, load_scratch_refinement_source, run_episode,
};
use crate::native_scratch_option_refinement::{
    NativeScratchOptionInspection, inspect_native_scratch_option_checkpoint,
};
use crate::native_tactic_route_runner::{
    NativeTacticColdReplayArtifact, NativeTacticColdReplayAttempt, NativeTapeColdReplayConfig,
    exact_cold_replay_attempts, run_native_tape_cold_replay_after_execution_validation,
};
use crate::optimization_request::OptimizationRequest;
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::InputTape;
use dusklight_learning::scratch_action_catalog::{
    SCRATCH_HEADING_COUNT, map_scratch_action_to_finer_catalog,
    scratch_action_catalog_with_heading_count,
};
use dusklight_learning::scratch_q::ScratchQTable;
use dusklight_learning::tactic_asset::TacticAssetCatalog;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Write};
use std::path::Path;
use std::time::Duration;

const CHECKPOINT_SCHEMA: &str = "dusklight-native-scratch-incumbent-checkpoint/v1";
const CHECKPOINT_MAGIC: &[u8; 8] = b"DSSINC01";
const CHECKPOINT_VERSION: u16 = 1;
const CHECKPOINT_HEADER_BYTES: usize = 8 + 2 + 2 + 8 + 32;

#[derive(Clone, Copy, Debug)]
pub enum ScratchIncumbentSource<'a> {
    Scratch,
    Heading(&'a Path),
    Option(&'a Path),
    Incumbent(&'a Path),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthenticatedScratchIncumbent {
    pub checkpoint_sha256: Digest,
    pub seed: u64,
    pub heading_count: usize,
    pub incumbent_action_sequence: Vec<usize>,
    pub incumbent_ticks: u64,
}

pub struct NativeScratchIncumbentMigrationConfig<'a> {
    pub repository_root: &'a Path,
    pub optimization: &'a OptimizationRequest,
    pub execution: &'a NativeResidualExecutionBinding,
    pub source_option_root: &'a Path,
    pub output_root: &'a Path,
    pub seed: u64,
    pub maximum_episode_ticks: u32,
    pub cold_replay_timeout: Duration,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct NativeScratchIncumbentCheckpoint {
    schema: String,
    source_checkpoint_sha256: Digest,
    source_optimization_request_sha256: Digest,
    source_execution_binding_sha256: Digest,
    optimization_request_sha256: Digest,
    execution_binding_sha256: Digest,
    action_universe_sha256: Digest,
    seed: u64,
    heading_count: usize,
    incumbent_action_sequence: Vec<usize>,
    incumbent_action_sequence_sha256: Digest,
    incumbent_ticks: u64,
    tape_sha256: Digest,
    tape_bytes: Vec<u8>,
    controller_tape: NativeTacticColdReplayArtifact,
    cold_replay_attempts: Vec<NativeTacticColdReplayAttempt>,
}

pub fn load_authenticated_scratch_incumbent(
    config: &NativeScratchRunConfig<'_>,
    source: ScratchIncumbentSource<'_>,
    target_heading_count: usize,
) -> Result<AuthenticatedScratchIncumbent, ScratchIncumbentError> {
    match source {
        ScratchIncumbentSource::Scratch => {
            if target_heading_count != SCRATCH_HEADING_COUNT {
                return Err(incumbent_error(
                    "scratch learner source can only use its original heading catalog",
                ));
            }
            let source = load_scratch_refinement_source(config).map_err(incumbent_display)?;
            Ok(from_scratch_source(source))
        }
        ScratchIncumbentSource::Heading(path) => {
            let source =
                inspect_native_scratch_heading_checkpoint(path).map_err(incumbent_display)?;
            if source.schema != "dusklight-native-scratch-heading-inspection/v2"
                || source.checkpoint_schema != "dusklight-native-scratch-heading-checkpoint/v2"
                || source.stop_reason != "candidate_exhaustion"
                || source.candidates_remaining != 0
                || source.optimization_request_sha256 != config.optimization.content_sha256
                || source.execution_binding_sha256 != config.execution.content_sha256
                || source.seed != config.seed
            {
                return Err(incumbent_error(
                    "heading source is not an exhausted authenticated incumbent",
                ));
            }
            load_option_ids(
                source.checkpoint_sha256,
                source.seed,
                source.heading_count as usize,
                source.incumbent_ticks,
                source.incumbent_action_sequence_sha256,
                &source.incumbent_options,
                source.action_universe_sha256,
                target_heading_count,
            )
        }
        ScratchIncumbentSource::Option(path) => {
            let source =
                inspect_native_scratch_option_checkpoint(path).map_err(incumbent_display)?;
            if source.schema != "dusklight-native-scratch-option-refinement-inspection/v1"
                || source.checkpoint_schema
                    != "dusklight-native-scratch-option-refinement-checkpoint/v1"
                || source.stop_reason != "candidate_exhaustion"
                || source.candidates_remaining != 0
                || source.optimization_request_sha256 != config.optimization.content_sha256
                || source.execution_binding_sha256 != config.execution.content_sha256
                || source.seed != config.seed
            {
                return Err(incumbent_error(
                    "option source is not an exhausted authenticated incumbent",
                ));
            }
            load_option_ids(
                source.checkpoint_sha256,
                source.seed,
                SCRATCH_HEADING_COUNT,
                source.incumbent_ticks,
                source.incumbent_action_sequence_sha256,
                &source.incumbent_options,
                source.action_universe_sha256,
                target_heading_count,
            )
        }
        ScratchIncumbentSource::Incumbent(path) => {
            load_replayed_incumbent(config, path, target_heading_count)
        }
    }
}

pub fn migrate_native_scratch_option_incumbent(
    config: &NativeScratchIncumbentMigrationConfig<'_>,
) -> Result<AuthenticatedScratchIncumbent, ScratchIncumbentError> {
    if config.maximum_episode_ticks == 0 || config.cold_replay_timeout.is_zero() {
        return Err(incumbent_error("scratch incumbent migration is invalid"));
    }
    let checkpoint_path = config.output_root.join("checkpoint.dssi");
    let scratch = NativeScratchRunConfig {
        repository_root: config.repository_root,
        optimization: config.optimization,
        execution: config.execution,
        output_root: config.output_root,
        seed: config.seed,
        episodes: 1,
        maximum_episode_ticks: config.maximum_episode_ticks,
        epsilon_per_million: 0,
        maximum_wall_time: Duration::from_secs(1),
        cold_replay_timeout: config.cold_replay_timeout,
    };
    if checkpoint_path.exists() {
        return load_replayed_incumbent(&scratch, config.output_root, SCRATCH_HEADING_COUNT);
    }
    if config.output_root.exists()
        && fs::read_dir(config.output_root)
            .map_err(incumbent_display)?
            .next()
            .is_some()
    {
        return Err(incumbent_error(
            "scratch incumbent migration output exists without a checkpoint",
        ));
    }
    let source = inspect_native_scratch_option_checkpoint(config.source_option_root)
        .map_err(incumbent_display)?;
    validate_migration_source(&source, config.seed)?;
    let source_incumbent = load_option_ids(
        source.checkpoint_sha256,
        source.seed,
        SCRATCH_HEADING_COUNT,
        source.incumbent_ticks,
        source.incumbent_action_sequence_sha256,
        &source.incumbent_options,
        source.action_universe_sha256,
        SCRATCH_HEADING_COUNT,
    )?;
    let root = config
        .repository_root
        .canonicalize()
        .map_err(incumbent_display)?;
    let authority = ValidatedNativeResidualExecution::authenticate(
        &root,
        config.optimization,
        config.execution,
    )
    .map_err(incumbent_display)?;
    let process_tape = InputTape::decode(
        &fs::read(root.join(&config.execution.process_boot_tape.path))
            .map_err(incumbent_display)?,
    )
    .map_err(incumbent_display)?
    .tape;
    let source_frame = usize::try_from(config.optimization.route.source_boundary_index)
        .map_err(incumbent_display)?;
    let prefix_frames = process_tape
        .frames
        .get(..source_frame)
        .ok_or_else(|| incumbent_error("scratch migration source frame exceeds the tape"))?
        .to_vec();
    let catalog = scratch_action_catalog_with_heading_count(SCRATCH_HEADING_COUNT)
        .map_err(incumbent_display)?;
    let q = ScratchQTable::new(catalog.entries().len()).map_err(incumbent_display)?;
    let attempt_root = config.output_root.join("migration-attempt");
    let outcome = run_episode(
        &root,
        &scratch,
        &process_tape,
        &prefix_frames,
        &catalog,
        &q,
        &[],
        Some(&source_incumbent.incumbent_action_sequence),
        0,
        &attempt_root,
    )
    .map_err(incumbent_display)?;
    let incumbent_ticks = outcome
        .tape
        .frames
        .len()
        .checked_sub(source_frame)
        .and_then(|ticks| ticks.checked_sub(1))
        .ok_or_else(|| incumbent_error("scratch migrated tape is shorter than its source"))?
        as u64;
    if !outcome.terminal || incumbent_ticks != source.incumbent_ticks {
        return Err(incumbent_error(
            "scratch incumbent did not reproduce its exact terminal tick during migration",
        ));
    }
    let tape_bytes = outcome.tape.encode().map_err(incumbent_display)?;
    let replay_root = config.output_root.join("cold-replay");
    let (controller_tape, cold_replay_attempts) =
        run_native_tape_cold_replay_after_execution_validation(
            &NativeTapeColdReplayConfig {
                repository_root: &root,
                optimization: config.optimization,
                execution: config.execution,
                tape: &outcome.tape,
                tape_bytes: &tape_bytes,
                first_hit_tick: incumbent_ticks,
                repetitions: 2,
                timeout: config.cold_replay_timeout,
                output_root: &replay_root,
            },
            &authority,
        )
        .map_err(incumbent_display)?;
    if !exact_cold_replay_attempts(
        &cold_replay_attempts,
        &controller_tape,
        config.optimization.route.source_boundary_index,
        incumbent_ticks,
        outcome.tape.frames.len() as u64,
    ) {
        return Err(incumbent_error(
            "scratch migrated incumbent did not cold replay exactly",
        ));
    }
    let checkpoint = NativeScratchIncumbentCheckpoint {
        schema: CHECKPOINT_SCHEMA.into(),
        source_checkpoint_sha256: source.checkpoint_sha256,
        source_optimization_request_sha256: source.optimization_request_sha256,
        source_execution_binding_sha256: source.execution_binding_sha256,
        optimization_request_sha256: config.optimization.content_sha256,
        execution_binding_sha256: config.execution.content_sha256,
        action_universe_sha256: catalog.action_schema_sha256(),
        seed: config.seed,
        heading_count: SCRATCH_HEADING_COUNT,
        incumbent_action_sequence: source_incumbent.incumbent_action_sequence,
        incumbent_action_sequence_sha256: source.incumbent_action_sequence_sha256,
        incumbent_ticks,
        tape_sha256: sha256(&tape_bytes),
        tape_bytes,
        controller_tape,
        cold_replay_attempts,
    };
    fs::create_dir_all(config.output_root).map_err(incumbent_display)?;
    write_atomic(&checkpoint_path, &encode_checkpoint(&checkpoint)?)?;
    load_replayed_incumbent(&scratch, config.output_root, SCRATCH_HEADING_COUNT)
}

fn validate_migration_source(
    source: &NativeScratchOptionInspection,
    seed: u64,
) -> Result<(), ScratchIncumbentError> {
    if source.schema != "dusklight-native-scratch-option-refinement-inspection/v1"
        || source.checkpoint_schema != "dusklight-native-scratch-option-refinement-checkpoint/v1"
        || source.stop_reason != "candidate_exhaustion"
        || source.candidates_remaining != 0
        || source.seed != seed
    {
        return Err(incumbent_error(
            "scratch migration source is not an exhausted option incumbent",
        ));
    }
    Ok(())
}

fn load_replayed_incumbent(
    config: &NativeScratchRunConfig<'_>,
    input: &Path,
    target_heading_count: usize,
) -> Result<AuthenticatedScratchIncumbent, ScratchIncumbentError> {
    let checkpoint_path = if input.is_dir() {
        input.join("checkpoint.dssi")
    } else {
        input.to_path_buf()
    };
    let bytes = fs::read(&checkpoint_path).map_err(incumbent_display)?;
    let checkpoint = decode_checkpoint(&bytes)?;
    let catalog = scratch_action_catalog_with_heading_count(checkpoint.heading_count)
        .map_err(incumbent_display)?;
    if checkpoint.schema != CHECKPOINT_SCHEMA
        || checkpoint.optimization_request_sha256 != config.optimization.content_sha256
        || checkpoint.execution_binding_sha256 != config.execution.content_sha256
        || checkpoint.seed != config.seed
        || checkpoint.heading_count != SCRATCH_HEADING_COUNT
        || checkpoint.action_universe_sha256 != catalog.action_schema_sha256()
        || checkpoint.incumbent_action_sequence.is_empty()
        || checkpoint
            .incumbent_action_sequence
            .iter()
            .any(|action| *action >= catalog.entries().len())
        || action_sequence_sha256(&checkpoint.incumbent_action_sequence)?
            != checkpoint.incumbent_action_sequence_sha256
        || sha256(&checkpoint.tape_bytes) != checkpoint.tape_sha256
        || checkpoint.cold_replay_attempts.len() != 2
    {
        return Err(incumbent_error(
            "replayed scratch incumbent checkpoint is detached",
        ));
    }
    let tape = InputTape::decode(&checkpoint.tape_bytes)
        .map_err(incumbent_display)?
        .tape;
    if !exact_cold_replay_attempts(
        &checkpoint.cold_replay_attempts,
        &checkpoint.controller_tape,
        config.optimization.route.source_boundary_index,
        checkpoint.incumbent_ticks,
        tape.frames.len() as u64,
    ) {
        return Err(incumbent_error(
            "replayed scratch incumbent evidence is not exact",
        ));
    }
    load_action_sequence(
        sha256(&bytes),
        checkpoint.seed,
        checkpoint.heading_count,
        checkpoint.incumbent_ticks,
        checkpoint.incumbent_action_sequence,
        target_heading_count,
    )
}

fn load_action_sequence(
    checkpoint_sha256: Digest,
    seed: u64,
    source_heading_count: usize,
    ticks: u64,
    source_actions: Vec<usize>,
    target_heading_count: usize,
) -> Result<AuthenticatedScratchIncumbent, ScratchIncumbentError> {
    let incumbent_action_sequence = if target_heading_count == source_heading_count {
        source_actions
    } else if target_heading_count == source_heading_count.saturating_mul(2) {
        let source_catalog = scratch_action_catalog_with_heading_count(source_heading_count)
            .map_err(incumbent_display)?;
        let target_catalog = scratch_action_catalog_with_heading_count(target_heading_count)
            .map_err(incumbent_display)?;
        source_actions
            .iter()
            .map(|action| {
                map_scratch_action_to_finer_catalog(&source_catalog, &target_catalog, *action)
                    .map_err(incumbent_display)
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        return Err(incumbent_error(
            "incumbent target headings must match or exactly double the source",
        ));
    };
    Ok(AuthenticatedScratchIncumbent {
        checkpoint_sha256,
        seed,
        heading_count: target_heading_count,
        incumbent_action_sequence,
        incumbent_ticks: ticks,
    })
}

fn from_scratch_source(source: ScratchRefinementSource) -> AuthenticatedScratchIncumbent {
    AuthenticatedScratchIncumbent {
        checkpoint_sha256: source.checkpoint_sha256,
        seed: source.seed,
        heading_count: SCRATCH_HEADING_COUNT,
        incumbent_action_sequence: source.incumbent_action_sequence,
        incumbent_ticks: source.incumbent_ticks,
    }
}

#[allow(clippy::too_many_arguments)]
fn load_option_ids(
    checkpoint_sha256: Digest,
    seed: u64,
    source_heading_count: usize,
    ticks: u64,
    expected_sequence_sha256: Digest,
    options: &[String],
    expected_action_universe_sha256: Digest,
    target_heading_count: usize,
) -> Result<AuthenticatedScratchIncumbent, ScratchIncumbentError> {
    let source_catalog = scratch_action_catalog_with_heading_count(source_heading_count)
        .map_err(incumbent_display)?;
    if source_catalog.action_schema_sha256() != expected_action_universe_sha256 {
        return Err(incumbent_error("incumbent action universe is detached"));
    }
    let source_actions = option_ids_to_actions(options, &source_catalog)?;
    if action_sequence_sha256(&source_actions)? != expected_sequence_sha256 {
        return Err(incumbent_error("incumbent action sequence is detached"));
    }
    load_action_sequence(
        checkpoint_sha256,
        seed,
        source_heading_count,
        ticks,
        source_actions,
        target_heading_count,
    )
}

fn option_ids_to_actions(
    options: &[String],
    catalog: &TacticAssetCatalog,
) -> Result<Vec<usize>, ScratchIncumbentError> {
    options
        .iter()
        .map(|option| {
            catalog
                .entries()
                .binary_search_by_key(&option.as_str(), |entry| entry.option_id())
                .map_err(|_| incumbent_error("incumbent option is absent from its catalog"))
        })
        .collect()
}

fn action_sequence_sha256(sequence: &[usize]) -> Result<Digest, ScratchIncumbentError> {
    let bytes = serde_cbor::to_vec(&sequence).map_err(incumbent_display)?;
    Ok(Digest(Sha256::digest(bytes).into()))
}

fn encode_checkpoint(
    checkpoint: &NativeScratchIncumbentCheckpoint,
) -> Result<Vec<u8>, ScratchIncumbentError> {
    let raw = serde_cbor::to_vec(checkpoint).map_err(incumbent_display)?;
    let compressed = zstd::stream::encode_all(Cursor::new(&raw), 1).map_err(incumbent_display)?;
    let mut bytes = Vec::with_capacity(CHECKPOINT_HEADER_BYTES + compressed.len());
    bytes.extend_from_slice(CHECKPOINT_MAGIC);
    bytes.extend_from_slice(&CHECKPOINT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes.extend_from_slice(&(raw.len() as u64).to_le_bytes());
    bytes.extend_from_slice(Sha256::digest(&raw).as_slice());
    bytes.extend_from_slice(&compressed);
    Ok(bytes)
}

fn decode_checkpoint(
    bytes: &[u8],
) -> Result<NativeScratchIncumbentCheckpoint, ScratchIncumbentError> {
    if bytes.len() < CHECKPOINT_HEADER_BYTES
        || &bytes[..8] != CHECKPOINT_MAGIC
        || u16::from_le_bytes(bytes[8..10].try_into().unwrap()) != CHECKPOINT_VERSION
        || u16::from_le_bytes(bytes[10..12].try_into().unwrap()) != 0
    {
        return Err(incumbent_error(
            "scratch incumbent checkpoint header is invalid",
        ));
    }
    let expected_len = u64::from_le_bytes(bytes[12..20].try_into().unwrap());
    let raw = zstd::stream::decode_all(Cursor::new(&bytes[CHECKPOINT_HEADER_BYTES..]))
        .map_err(incumbent_display)?;
    if raw.len() as u64 != expected_len || Sha256::digest(&raw)[..] != bytes[20..52] {
        return Err(incumbent_error(
            "scratch incumbent checkpoint checksum is invalid",
        ));
    }
    serde_cbor::from_slice(&raw).map_err(incumbent_display)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ScratchIncumbentError> {
    let parent = path
        .parent()
        .ok_or_else(|| incumbent_error("scratch incumbent checkpoint has no parent"))?;
    fs::create_dir_all(parent).map_err(incumbent_display)?;
    let temporary = parent.join("checkpoint.dssi.tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(incumbent_display)?;
    file.write_all(bytes).map_err(incumbent_display)?;
    file.sync_all().map_err(incumbent_display)?;
    drop(file);
    fs::rename(&temporary, path).map_err(incumbent_display)
}

fn sha256(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ScratchIncumbentError(String);

impl fmt::Display for ScratchIncumbentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ScratchIncumbentError {}

fn incumbent_error(message: impl Into<String>) -> ScratchIncumbentError {
    ScratchIncumbentError(message.into())
}

fn incumbent_display(error: impl fmt::Display) -> ScratchIncumbentError {
    incumbent_error(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binary_fixture() -> NativeScratchIncumbentCheckpoint {
        NativeScratchIncumbentCheckpoint {
            schema: CHECKPOINT_SCHEMA.into(),
            source_checkpoint_sha256: Digest([1; 32]),
            source_optimization_request_sha256: Digest([2; 32]),
            source_execution_binding_sha256: Digest([3; 32]),
            optimization_request_sha256: Digest([4; 32]),
            execution_binding_sha256: Digest([5; 32]),
            action_universe_sha256: Digest([6; 32]),
            seed: 7,
            heading_count: 16,
            incumbent_action_sequence: vec![8, 9],
            incumbent_action_sequence_sha256: Digest([10; 32]),
            incumbent_ticks: 242,
            tape_sha256: Digest([11; 32]),
            tape_bytes: vec![12, 13],
            controller_tape: NativeTacticColdReplayArtifact {
                path: "selected.tape".into(),
                sha256: Digest([14; 32]),
            },
            cold_replay_attempts: Vec::new(),
        }
    }

    #[test]
    fn option_ids_round_trip_and_promote_without_index_geometry() {
        let coarse = scratch_action_catalog_with_heading_count(16).unwrap();
        let options = vec![
            "scratch.camera_move.h03.t08.l1".to_owned(),
            "scratch.roll.h15.r03".to_owned(),
        ];
        let actions = option_ids_to_actions(&options, &coarse).unwrap();
        let loaded = load_option_ids(
            Digest([1; 32]),
            7,
            16,
            242,
            action_sequence_sha256(&actions).unwrap(),
            &options,
            coarse.action_schema_sha256(),
            32,
        )
        .unwrap();
        let fine = scratch_action_catalog_with_heading_count(32).unwrap();
        assert_eq!(
            fine.entries()[loaded.incumbent_action_sequence[0]].option_id(),
            "scratch.camera_move.h06.t08.l1"
        );
        assert_eq!(
            fine.entries()[loaded.incumbent_action_sequence[1]].option_id(),
            "scratch.roll.h30.r03"
        );
    }

    #[test]
    fn binary_checkpoint_round_trips_and_rejects_corruption() {
        let checkpoint = binary_fixture();
        let bytes = encode_checkpoint(&checkpoint).unwrap();
        assert_eq!(decode_checkpoint(&bytes).unwrap(), checkpoint);
        let mut corrupted = bytes;
        *corrupted.last_mut().unwrap() ^= 0x80;
        assert!(decode_checkpoint(&corrupted).is_err());
    }
}
