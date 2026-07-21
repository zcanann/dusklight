//! Resumable all-stage observation survey commands.

use crate::{option, repeated_option, required_path, u32_option, u64_option, usize_option};
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
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;

const MAX_SURVEY_WORKERS: usize = 64;

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
    let mut ledger = load_ledger(&ledger_path, &catalog)?;
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
            ledger.record_attempt(&catalog, &candidate.id, attempt.clone())?;
            // Persist after every completed process, not only after the batch,
            // so interruption loses no classified entry.
            write_ledger(&ledger_path, &ledger.canonical_bytes(&catalog)?)?;
            completed.insert(
                index,
                json!({
                    "candidate_id": candidate.id,
                    "attempt": attempt,
                    "classification": ledger.cases.iter()
                        .find(|case| case.candidate_id == candidate.id)
                        .and_then(|case| case.classification),
                }),
            );
        }
        Ok(())
    })?;
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
                return Err(
                    format!("survey candidate is already finalized: {candidate_id}").into(),
                );
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
            value => {
                return Err(format!(
                    "unknown survey probe {value:?}; expected neutral, movement, camera, targeting, basic-actions, or contact-sweep"
                )
                .into());
            }
        },
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
    Ok(StageSurveyLedger::decode_canonical(
        &fs::read(path)?,
        catalog,
    )?)
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
