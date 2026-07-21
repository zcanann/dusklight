//! Resumable all-stage observation survey commands.

use crate::{option, repeated_option, required_path, u32_option, u64_option, usize_option};
use huntctl::Digest;
use huntctl::stage_actor_coverage::StageActorCoverageReport;
use huntctl::stage_boot_catalog::{StageBootCandidate, StageBootCatalog};
use huntctl::stage_observation_coverage::{
    ObservationCoverageCaseStatus, StageObservationCoverageReport, StageObservationCoverageSource,
};
use huntctl::stage_survey::{
    STAGE_SURVEY_FIDELITY, StageSurveyExecutionConfig, StageSurveyLedger, StageSurveyPolicy,
    StageSurveyProbeKind, compact_stage_survey_artifacts, execute_stage_survey_attempt,
    stage_survey_identity,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;

const MAX_SURVEY_WORKERS: usize = 64;
const SURVEY_JOURNAL_SCHEMA: &str = "dusklight-stage-survey-journal-record/v1";
const SURVEY_JOURNAL_DIGEST_DOMAIN: &[u8] = b"dusklight.stage-survey-journal-record/v1\0";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SurveyJournalRecord {
    schema: String,
    base_ledger_sha256: Digest,
    sequence: u64,
    previous_record_sha256: Digest,
    candidate_id: String,
    attempt: huntctl::stage_survey::StageSurveyAttempt,
    record_sha256: Digest,
}

#[derive(Clone, Copy, Debug)]
struct SurveyJournalHead {
    base_ledger_sha256: Digest,
    sequence: u64,
    previous_record_sha256: Digest,
    valid_bytes: u64,
}

pub(crate) fn command_survey(args: &[String]) -> Result<(), Box<dyn Error>> {
    match args.first().map(String::as_str) {
        Some("init") => command_init(&args[1..]),
        Some("status") => command_status(&args[1..]),
        Some("run") => command_run(&args[1..]),
        Some("actor-coverage") => command_actor_coverage(&args[1..]),
        Some("observation-coverage") => command_observation_coverage(&args[1..]),
        Some("compact-artifacts") => command_compact_artifacts(&args[1..]),
        _ => Err(
            "survey commands: init, status, run, actor-coverage, observation-coverage, compact-artifacts"
                .into(),
        ),
    }
}

fn command_compact_artifacts(args: &[String]) -> Result<(), Box<dyn Error>> {
    let catalog_path = required_path(args, "--catalog")?;
    let ledger_path = required_path(args, "--ledger")?;
    let state_root = required_path(args, "--state-root")?;
    let catalog = load_catalog(&catalog_path)?;
    let ledger = load_ledger(&ledger_path, &catalog)?;
    let summary = compact_stage_survey_artifacts(&catalog, &ledger, &state_root)?;
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

fn command_observation_coverage(args: &[String]) -> Result<(), Box<dyn Error>> {
    let catalog_path = required_path(args, "--catalog")?;
    let output = required_path(args, "--output")?;
    let ledger_paths = repeated_option(args, "--ledger")
        .into_iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let state_roots = repeated_option(args, "--state-root")
        .into_iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    if ledger_paths.is_empty() || ledger_paths.len() != state_roots.len() {
        return Err(
            "survey observation-coverage requires one --state-root for every --ledger".into(),
        );
    }
    let catalog = load_catalog(&catalog_path)?;
    let ledgers = ledger_paths
        .iter()
        .map(|path| load_ledger(path, &catalog))
        .collect::<Result<Vec<_>, _>>()?;
    let sources = ledgers
        .iter()
        .zip(&state_roots)
        .map(|(ledger, state_root)| StageObservationCoverageSource { ledger, state_root })
        .collect::<Vec<_>>();
    let report = StageObservationCoverageReport::build(&catalog, &sources)?;
    write_ledger(&output, &report.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": report.schema,
            "output": output,
            "report_sha256": report.report_sha256,
            "sources": report.sources.len(),
            "cases": report.cases.len(),
            "verified_cases": report.cases.iter().filter(|case| case.status == ObservationCoverageCaseStatus::VerifiedTrace).count(),
            "cells": report.cells.len(),
        }))?
    );
    Ok(())
}

fn command_actor_coverage(args: &[String]) -> Result<(), Box<dyn Error>> {
    let catalog_path = required_path(args, "--catalog")?;
    let ledger_path = required_path(args, "--ledger")?;
    let state_root = required_path(args, "--state-root")?;
    let output = required_path(args, "--output")?;
    let catalog = load_catalog(&catalog_path)?;
    let ledger = load_ledger(&ledger_path, &catalog)?;
    let report = StageActorCoverageReport::build(&catalog, &ledger, &state_root)?;
    write_ledger(&output, &report.canonical_bytes()?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": report.schema,
            "output": output,
            "report_sha256": report.report_sha256,
            "ready_cases": report.ready_case_count,
            "verified_cases": report.verified_case_count,
            "rejected_cases": report.rejected_case_count,
            "stages": report.stages.len(),
            "profiles": report.profiles.len(),
        }))?
    );
    Ok(())
}

fn command_init(args: &[String]) -> Result<(), Box<dyn Error>> {
    let catalog_path = required_path(args, "--catalog")?;
    let ledger_path = required_path(args, "--ledger")?;
    let game = required_path(args, "--game")?;
    let dvd = required_path(args, "--dvd")?;
    if ledger_path.exists() {
        return Err(format!("survey ledger already exists: {}", ledger_path.display()).into());
    }
    let catalog = load_catalog(&catalog_path)?;
    let policy = policy_from_args(args)?;
    let game_args = repeated_option(args, "--game-arg");
    let identity = stage_survey_identity(&catalog, &game, &dvd, &policy, &game_args)?;
    let ledger = StageSurveyLedger::new(&catalog, identity, policy)?;
    write_ledger(&ledger_path, &ledger.canonical_bytes(&catalog)?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": ledger.schema,
            "catalog": catalog_path,
            "ledger": ledger_path,
            "identity": ledger.identity,
            "policy": ledger.policy,
            "progress": ledger.progress(&catalog)?,
        }))?
    );
    Ok(())
}

fn command_status(args: &[String]) -> Result<(), Box<dyn Error>> {
    let catalog_path = required_path(args, "--catalog")?;
    let ledger_path = required_path(args, "--ledger")?;
    let catalog = load_catalog(&catalog_path)?;
    let ledger = load_ledger(&ledger_path, &catalog)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": ledger.schema,
            "catalog": catalog_path,
            "ledger": ledger_path,
            "ledger_sha256": ledger.digest(&catalog)?,
            "identity": ledger.identity,
            "policy": ledger.policy,
            "progress": ledger.progress(&catalog)?,
        }))?
    );
    Ok(())
}

fn command_run(args: &[String]) -> Result<(), Box<dyn Error>> {
    let catalog_path = required_path(args, "--catalog")?;
    let ledger_path = required_path(args, "--ledger")?;
    let game = required_path(args, "--game")?;
    let dvd = required_path(args, "--dvd")?;
    let state_root = required_path(args, "--state-root")?;
    let candidate_ids = repeated_option(args, "--candidate");
    if !candidate_ids.is_empty() && option(args, "--limit").is_some() {
        return Err("survey run accepts repeated --candidate values or --limit, not both".into());
    }
    let limit = usize_option(args, "--limit", 1)?;
    let workers = usize_option(args, "--workers", 1)?;
    if limit == 0 {
        return Err("survey run --limit must be greater than zero".into());
    }
    if workers == 0 || workers > MAX_SURVEY_WORKERS {
        return Err(format!("survey run --workers must be in 1..={MAX_SURVEY_WORKERS}").into());
    }
    let game_args = repeated_option(args, "--game-arg");
    let catalog = load_catalog(&catalog_path)?;
    let (mut ledger, mut journal) = load_ledger_for_run(&ledger_path, &catalog)?;
    let candidates = select_run_candidates(&catalog, &ledger, &candidate_ids, limit)?;
    let observed_identity =
        stage_survey_identity(&catalog, &game, &dvd, &ledger.policy, &game_args)?;
    if observed_identity != ledger.identity {
        return Err(
            "survey executable, disc, arguments, policy, or observation identity changed".into(),
        );
    }
    fs::create_dir_all(&state_root)?;
    let execution = StageSurveyExecutionConfig {
        executable: game,
        game_data: dvd,
        state_root,
        game_args,
    };
    let jobs = candidates
        .into_iter()
        .map(|candidate| {
            let attempt_number = ledger
                .cases
                .iter()
                .find(|case| case.candidate_id == candidate.id)
                .map_or(1, |case| case.attempts.len() + 1);
            Ok((candidate, u8::try_from(attempt_number)?))
        })
        .collect::<Result<Vec<_>, std::num::TryFromIntError>>()?;
    let next_job = AtomicUsize::new(0);
    let (sender, receiver) = mpsc::channel();
    let worker_count = workers.min(jobs.len().max(1));
    let policy = ledger.policy.clone();
    let mut completed = BTreeMap::new();
    let mut recorder = ledger.recorder(&catalog)?;
    thread::scope(|scope| -> Result<(), Box<dyn Error>> {
        for _ in 0..worker_count {
            let sender = sender.clone();
            let jobs = &jobs;
            let execution = &execution;
            let policy = &policy;
            let next_job = &next_job;
            scope.spawn(move || {
                loop {
                    let index = next_job.fetch_add(1, Ordering::Relaxed);
                    let Some((candidate, attempt_number)) = jobs.get(index) else {
                        break;
                    };
                    let result =
                        execute_stage_survey_attempt(candidate, policy, *attempt_number, execution);
                    if sender.send((index, candidate.clone(), result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);
        for _ in 0..jobs.len() {
            let (index, candidate, result) = receiver.recv()?;
            let attempt = result?;
            recorder.record_attempt(&candidate.id, attempt.clone())?;
            let attempt = recorder
                .ledger()
                .cases
                .iter()
                .find(|case| case.candidate_id == candidate.id)
                .and_then(|case| case.attempts.last())
                .expect("recorded survey attempt exists");
            // Persist one small authenticated append per completed process.
            // Rewriting the growing canonical ledger here made an all-catalog
            // pass quadratic; the journal preserves the same interruption
            // boundary and is folded into the ledger after a clean batch.
            journal = append_journal_record(&ledger_path, journal, &candidate.id, attempt)?;
            completed.insert(
                index,
                json!({
                    "candidate_id": candidate.id,
                    "attempt": attempt,
                    "classification": recorder.ledger().cases.iter()
                        .find(|case| case.candidate_id == candidate.id)
                        .and_then(|case| case.classification),
                }),
            );
        }
        Ok(())
    })?;
    drop(recorder);
    compact_ledger_journal(&ledger_path, &catalog, &ledger)?;
    let completed = completed.into_values().collect::<Vec<_>>();
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema": "dusklight-stage-survey-run/v1",
            "catalog": catalog_path,
            "ledger": ledger_path,
            "completed": completed,
            "progress": ledger.progress(&catalog)?,
        }))?
    );
    Ok(())
}

fn select_run_candidates(
    catalog: &StageBootCatalog,
    ledger: &StageSurveyLedger,
    candidate_ids: &[String],
    limit: usize,
) -> Result<Vec<StageBootCandidate>, Box<dyn Error>> {
    if candidate_ids.is_empty() {
        return Ok(ledger
            .next_candidates(catalog, limit)?
            .into_iter()
            .cloned()
            .collect());
    }
    let mut unique = BTreeSet::new();
    for candidate_id in candidate_ids {
        if !unique.insert(candidate_id.as_str()) {
            return Err(format!("duplicate survey candidate: {candidate_id}").into());
        }
    }
    let candidates = catalog
        .candidates
        .iter()
        .map(|candidate| (candidate.id.as_str(), candidate))
        .collect::<BTreeMap<_, _>>();
    let pending = ledger
        .next_candidates(catalog, catalog.candidates.len())?
        .into_iter()
        .map(|candidate| candidate.id.as_str())
        .collect::<BTreeSet<_>>();
    candidate_ids
        .iter()
        .map(|candidate_id| {
            let candidate = candidates
                .get(candidate_id.as_str())
                .ok_or_else(|| format!("unknown survey candidate: {candidate_id}"))?;
            if !pending.contains(candidate_id.as_str()) {
                return Err(format!(
                    "survey candidate is already finalized: {candidate_id}"
                ));
            }
            Ok((*candidate).clone())
        })
        .collect::<Result<Vec<_>, String>>()
        .map_err(Into::into)
}

fn policy_from_args(args: &[String]) -> Result<StageSurveyPolicy, Box<dyn Error>> {
    let attempts: u8 = option(args, "--attempts")
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(2);
    Ok(StageSurveyPolicy {
        probe_ticks: u32_option(args, "--probe-ticks", 30)?,
        probe: match option(args, "--probe").as_deref().unwrap_or("neutral") {
            "neutral" => StageSurveyProbeKind::Neutral,
            "movement" => StageSurveyProbeKind::Movement,
            "camera" => StageSurveyProbeKind::Camera,
            "targeting" => StageSurveyProbeKind::Targeting,
            "basic-actions" => StageSurveyProbeKind::BasicActions,
            "contact-sweep" => StageSurveyProbeKind::ContactSweep,
            "actor-activation" => StageSurveyProbeKind::ActorActivation,
            "loading-sweep" => StageSurveyProbeKind::LoadingSweep,
            value => {
                return Err(format!(
                    "unknown survey probe {value:?}; expected neutral, movement, camera, targeting, basic-actions, contact-sweep, actor-activation, or loading-sweep"
                )
                .into());
            }
        },
        native_stage_readiness_ticks: u32_option(args, "--readiness-ticks", 150)?,
        host_timeout_millis: u64_option(args, "--timeout-ms", 120_000)?,
        maximum_attempts_per_case: attempts,
        fidelity_profile: STAGE_SURVEY_FIDELITY.into(),
    })
}

fn load_catalog(path: &Path) -> Result<StageBootCatalog, Box<dyn Error>> {
    Ok(StageBootCatalog::decode_canonical(&fs::read(path)?)?)
}

fn load_ledger(
    path: &Path,
    catalog: &StageBootCatalog,
) -> Result<StageSurveyLedger, Box<dyn Error>> {
    Ok(load_ledger_with_journal(path, catalog)?.0)
}

fn load_ledger_for_run(
    path: &Path,
    catalog: &StageBootCatalog,
) -> Result<(StageSurveyLedger, SurveyJournalHead), Box<dyn Error>> {
    let (ledger, mut head) = load_ledger_with_journal(path, catalog)?;
    let journal_path = journal_path(path);
    if journal_path.exists() {
        let length = journal_path.metadata()?.len();
        if length != head.valid_bytes {
            let journal = OpenOptions::new().write(true).open(&journal_path)?;
            journal.set_len(head.valid_bytes)?;
            journal.sync_all()?;
        }
        let on_disk = StageSurveyLedger::decode_canonical(&fs::read(path)?, catalog)?;
        let on_disk_digest = on_disk.digest(catalog)?;
        if on_disk_digest != head.base_ledger_sha256 {
            // A prior clean fold reached disk before journal removal. All
            // records were proven already present during replay, so reset the
            // append chain against the new canonical base.
            fs::remove_file(&journal_path)?;
            head = SurveyJournalHead {
                base_ledger_sha256: on_disk_digest,
                sequence: 0,
                previous_record_sha256: Digest::ZERO,
                valid_bytes: 0,
            };
        }
    }
    Ok((ledger, head))
}

fn load_ledger_with_journal(
    path: &Path,
    catalog: &StageBootCatalog,
) -> Result<(StageSurveyLedger, SurveyJournalHead), Box<dyn Error>> {
    let mut ledger = StageSurveyLedger::decode_canonical(&fs::read(path)?, catalog)?;
    let base_ledger_sha256 = ledger.digest(catalog)?;
    let journal_path = journal_path(path);
    let mut head = SurveyJournalHead {
        base_ledger_sha256,
        sequence: 0,
        previous_record_sha256: Digest::ZERO,
        valid_bytes: 0,
    };
    let bytes = match fs::read(&journal_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((ledger, head));
        }
        Err(error) => return Err(error.into()),
    };
    let mut recorder = ledger.recorder(catalog)?;
    let complete_len = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
    let mut offset = 0_usize;
    let mut base_matches = true;
    while offset < complete_len {
        let relative_end = bytes[offset..complete_len]
            .iter()
            .position(|byte| *byte == b'\n')
            .expect("complete journal slice ends with newline");
        let end = offset + relative_end + 1;
        let encoded = &bytes[offset..end];
        let record: SurveyJournalRecord = serde_json::from_slice(encoded)?;
        let expected_sequence = head
            .sequence
            .checked_add(1)
            .ok_or("survey ledger journal sequence overflowed")?;
        if journal_record_bytes(&record)? != encoded
            || record.schema != SURVEY_JOURNAL_SCHEMA
            || record.sequence != expected_sequence
            || record.previous_record_sha256 != head.previous_record_sha256
            || record.record_sha256 != journal_record_digest(&record)?
            || record.attempt.number == 0
        {
            return Err("survey ledger journal is corrupt or noncanonical".into());
        }
        if head.sequence == 0 {
            head.base_ledger_sha256 = record.base_ledger_sha256;
            base_matches = record.base_ledger_sha256 == base_ledger_sha256;
        } else if record.base_ledger_sha256 != head.base_ledger_sha256 {
            return Err("survey ledger journal base identity changed".into());
        }

        let existing = recorder
            .ledger()
            .cases
            .iter()
            .find(|case| case.candidate_id == record.candidate_id)
            .and_then(|case| case.attempts.get(usize::from(record.attempt.number) - 1));
        match existing {
            Some(attempt) if attempt == &record.attempt => {}
            Some(_) => return Err("survey ledger journal conflicts with canonical ledger".into()),
            None if base_matches => {
                let expected_attempt_number = recorder
                    .ledger()
                    .cases
                    .iter()
                    .find(|case| case.candidate_id == record.candidate_id)
                    .map_or(1, |case| case.attempts.len() + 1);
                if usize::from(record.attempt.number) != expected_attempt_number {
                    return Err("survey ledger journal attempt sequence is not contiguous".into());
                }
                recorder.record_attempt(&record.candidate_id, record.attempt.clone())?
            }
            None => {
                return Err(
                    "survey ledger journal does not descend from the canonical ledger".into(),
                );
            }
        }
        head.sequence = record.sequence;
        head.previous_record_sha256 = record.record_sha256;
        head.valid_bytes = u64::try_from(end)?;
        offset = end;
    }
    drop(recorder);
    ledger.validate(catalog)?;
    Ok((ledger, head))
}

fn append_journal_record(
    ledger_path: &Path,
    head: SurveyJournalHead,
    candidate_id: &str,
    attempt: &huntctl::stage_survey::StageSurveyAttempt,
) -> Result<SurveyJournalHead, Box<dyn Error>> {
    let sequence = head
        .sequence
        .checked_add(1)
        .ok_or("survey journal overflow")?;
    let mut record = SurveyJournalRecord {
        schema: SURVEY_JOURNAL_SCHEMA.into(),
        base_ledger_sha256: head.base_ledger_sha256,
        sequence,
        previous_record_sha256: head.previous_record_sha256,
        candidate_id: candidate_id.into(),
        attempt: attempt.clone(),
        record_sha256: Digest::ZERO,
    };
    record.record_sha256 = journal_record_digest(&record)?;
    let bytes = journal_record_bytes(&record)?;
    let path = journal_path(ledger_path);
    let mut output = OpenOptions::new().create(true).append(true).open(path)?;
    output.write_all(&bytes)?;
    output.sync_all()?;
    Ok(SurveyJournalHead {
        base_ledger_sha256: head.base_ledger_sha256,
        sequence,
        previous_record_sha256: record.record_sha256,
        valid_bytes: head.valid_bytes + u64::try_from(bytes.len())?,
    })
}

fn compact_ledger_journal(
    path: &Path,
    catalog: &StageBootCatalog,
    ledger: &StageSurveyLedger,
) -> Result<(), Box<dyn Error>> {
    write_ledger(path, &ledger.canonical_bytes(catalog)?)?;
    let journal = journal_path(path);
    if journal.exists() {
        fs::remove_file(journal)?;
    }
    Ok(())
}

fn journal_path(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_owned();
    value.push(".journal");
    PathBuf::from(value)
}

fn journal_record_digest(record: &SurveyJournalRecord) -> Result<Digest, Box<dyn Error>> {
    let mut payload = record.clone();
    payload.record_sha256 = Digest::ZERO;
    let mut hasher = Sha256::new();
    hasher.update(SURVEY_JOURNAL_DIGEST_DOMAIN);
    hasher.update(serde_json::to_vec(&payload)?);
    Ok(Digest(hasher.finalize().into()))
}

fn journal_record_bytes(record: &SurveyJournalRecord) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut bytes = serde_json::to_vec(record)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn write_ledger(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let temporary = temporary_path(path);
    if temporary.exists() {
        fs::remove_file(&temporary)?;
    }
    let mut output = File::create(&temporary)?;
    output.write_all(bytes)?;
    output.sync_all()?;
    drop(output);
    replace_file(&temporary, path)?;
    Ok(())
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_owned();
    value.push(".next");
    PathBuf::from(value)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> Result<(), Box<dyn Error>> {
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
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let moved = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> Result<(), Box<dyn Error>> {
    fs::rename(source, destination)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use huntctl::stage_boot_catalog::{
        BootLayerSource, BootLayerSourceKind, BootPointSource, BootPointSourceKind,
        STAGE_BOOT_CATALOG_SCHEMA, StageCatalogStatus, StageInventoryStatus,
    };
    use huntctl::stage_survey::{
        STAGE_SURVEY_FIDELITY, StageSurveyAttemptOutcome, StageSurveyIdentity,
    };
    use std::sync::atomic::AtomicU64;
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_TEST_ROOT: AtomicU64 = AtomicU64::new(0);

    fn test_catalog() -> StageBootCatalog {
        StageBootCatalog {
            schema: STAGE_BOOT_CATALOG_SCHEMA.into(),
            known_loader_sha256: None,
            stages: vec![StageCatalogStatus {
                stage: "F_SP103".into(),
                resources_present: true,
                inventory_status: StageInventoryStatus::Complete,
                inventory_sha256: Some(Digest([7; 32])),
                diagnostic: None,
                room_count: 1,
                player_spawn_count: 1,
                candidate_count: 1,
            }],
            candidates: vec![StageBootCandidate {
                id: "F_SP103/room/0/point/0/layer/-1".into(),
                stage: "F_SP103".into(),
                room: 0,
                point: 0,
                layer: -1,
                point_sources: vec![BootPointSource {
                    kind: BootPointSourceKind::RetailPlayerSpawn,
                    stable_id: Some("spawn-0".into()),
                }],
                layer_sources: vec![BootLayerSource {
                    kind: BootLayerSourceKind::ResolvedDefault,
                    chunk_tag: None,
                }],
            }],
        }
    }

    fn test_ledger(catalog: &StageBootCatalog) -> StageSurveyLedger {
        StageSurveyLedger::new(
            catalog,
            StageSurveyIdentity {
                catalog_sha256: catalog.digest().unwrap(),
                executable_sha256: Digest([1; 32]),
                game_data_sha256: Digest([2; 32]),
                card_fixture_sha256: Digest([3; 32]),
                observation_schema_sha256: Digest([4; 32]),
                settings_sha256: Digest([5; 32]),
            },
            StageSurveyPolicy {
                probe_ticks: 1,
                probe: StageSurveyProbeKind::Neutral,
                native_stage_readiness_ticks: 150,
                host_timeout_millis: 1_000,
                maximum_attempts_per_case: 2,
                fidelity_profile: STAGE_SURVEY_FIDELITY.into(),
            },
        )
        .unwrap()
    }

    fn failed_attempt() -> huntctl::stage_survey::StageSurveyAttempt {
        huntctl::stage_survey::StageSurveyAttempt {
            number: 0,
            outcome: StageSurveyAttemptOutcome::ProcessCrash,
            exit_code: Some(1),
            elapsed_millis: 50,
            observation_sha256: None,
            actor_catalog_sha256: None,
            observed_actor_count: None,
            retained_actor_count: None,
            actor_catalog_truncated: None,
            state_sequence_sha256: None,
            observed_origin: None,
            observed_final: None,
            diagnostic_code: Some("test_failure".into()),
        }
    }

    fn temporary_root() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "huntctl-survey-journal-{}-{nonce}-{}",
            std::process::id(),
            NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn survey_journal_replays_truncates_and_folds() {
        let root = temporary_root();
        fs::create_dir_all(&root).unwrap();
        let ledger_path = root.join("ledger.json");
        let catalog = test_catalog();
        let mut ledger = test_ledger(&catalog);
        write_ledger(&ledger_path, &ledger.canonical_bytes(&catalog).unwrap()).unwrap();

        let (_, head) = load_ledger_for_run(&ledger_path, &catalog).unwrap();
        let candidate_id = catalog.candidates[0].id.clone();
        ledger
            .record_attempt(&catalog, &candidate_id, failed_attempt())
            .unwrap();
        let attempt = ledger.cases[0].attempts[0].clone();
        let head = append_journal_record(&ledger_path, head, &candidate_id, &attempt).unwrap();
        assert_eq!(head.sequence, 1);

        let mut journal = OpenOptions::new()
            .append(true)
            .open(journal_path(&ledger_path))
            .unwrap();
        journal.write_all(br#"{"partial":"#).unwrap();
        journal.sync_all().unwrap();
        drop(journal);

        let (replayed, recovered_head) = load_ledger_for_run(&ledger_path, &catalog).unwrap();
        assert_eq!(replayed, ledger);
        assert_eq!(recovered_head.sequence, 1);
        assert_eq!(
            fs::metadata(journal_path(&ledger_path)).unwrap().len(),
            recovered_head.valid_bytes
        );

        compact_ledger_journal(&ledger_path, &catalog, &replayed).unwrap();
        assert!(!journal_path(&ledger_path).exists());
        assert_eq!(
            StageSurveyLedger::decode_canonical(&fs::read(&ledger_path).unwrap(), &catalog)
                .unwrap(),
            ledger
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn survey_journal_accepts_a_completed_fold_before_removal() {
        let root = temporary_root();
        fs::create_dir_all(&root).unwrap();
        let ledger_path = root.join("ledger.json");
        let catalog = test_catalog();
        let mut ledger = test_ledger(&catalog);
        write_ledger(&ledger_path, &ledger.canonical_bytes(&catalog).unwrap()).unwrap();

        let (_, head) = load_ledger_for_run(&ledger_path, &catalog).unwrap();
        let candidate_id = catalog.candidates[0].id.clone();
        ledger
            .record_attempt(&catalog, &candidate_id, failed_attempt())
            .unwrap();
        append_journal_record(
            &ledger_path,
            head,
            &candidate_id,
            &ledger.cases[0].attempts[0],
        )
        .unwrap();
        // Model the crash window after atomic ledger replacement but before
        // stale-journal removal.
        write_ledger(&ledger_path, &ledger.canonical_bytes(&catalog).unwrap()).unwrap();

        let (replayed, head) = load_ledger_for_run(&ledger_path, &catalog).unwrap();
        assert_eq!(replayed, ledger);
        assert_eq!(head.sequence, 0);
        assert!(!journal_path(&ledger_path).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn survey_recorder_uses_catalog_identity_not_lexical_id_order() {
        let mut catalog = test_catalog();
        catalog.candidates[0].point = 2;
        catalog.candidates[0].id = "F_SP103/room/0/point/2/layer/-1".into();
        let mut point_ten = catalog.candidates[0].clone();
        point_ten.point = 10;
        point_ten.id = "F_SP103/room/0/point/10/layer/-1".into();
        point_ten.point_sources[0].stable_id = Some("spawn-1".into());
        catalog.candidates.push(point_ten);
        catalog.stages[0].candidate_count = 2;
        catalog.stages[0].player_spawn_count = 2;
        catalog.validate().unwrap();

        let mut ledger = test_ledger(&catalog);
        let mut recorder = ledger.recorder(&catalog).unwrap();
        recorder
            .record_attempt("F_SP103/room/0/point/10/layer/-1", failed_attempt())
            .unwrap();
        assert_eq!(
            recorder.ledger().cases[0].candidate_id,
            "F_SP103/room/0/point/10/layer/-1"
        );
    }
}
