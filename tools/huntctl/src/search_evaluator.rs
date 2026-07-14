//! Native, cross-platform population evaluation and multi-generation search.

use crate::search::{
    Candidate, CandidateResult, EvolutionConfig, POPULATION_SCHEMA, PopulationManifest,
    RESULTS_SCHEMA, SearchResults, SegmentProfile, evolve_population, rank_population,
    write_explicit_population, write_seed_population,
};
use crate::tape::{InputTape, RawPadState};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fmt;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub const EVALUATION_SCHEMA: &str = "dusklight-search-evaluation/v2";
pub const ATTEMPT_SCHEMA: &str = "dusklight-search-attempt/v2";
pub const SEARCH_RUN_SCHEMA: &str = "dusklight-search-run/v2";

#[derive(Clone, Debug)]
pub struct EvaluateConfig {
    pub population_path: PathBuf,
    pub game: PathBuf,
    pub dvd: PathBuf,
    pub output_root: PathBuf,
    pub results_path: PathBuf,
    pub working_directory: PathBuf,
    pub game_args_prefix: Vec<String>,
    pub workers: usize,
    pub repetitions: u32,
    pub timeout: Duration,
}

#[derive(Clone, Debug)]
pub struct SearchRunConfig {
    pub segment: SegmentProfile,
    pub seed_candidate: Option<Candidate>,
    pub game: PathBuf,
    pub dvd: PathBuf,
    pub output_root: PathBuf,
    pub working_directory: PathBuf,
    pub game_args_prefix: Vec<String>,
    pub generations: u32,
    pub population_size: usize,
    pub elite_count: usize,
    pub workers: usize,
    pub repetitions: u32,
    pub timeout: Duration,
    pub rng_seed: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct EvaluationReport {
    pub schema: &'static str,
    pub population: PathBuf,
    pub results: PathBuf,
    pub segment: SegmentProfile,
    pub workers: usize,
    pub repetitions: u32,
    pub planned_attempts: usize,
    pub completed_attempts: usize,
    pub infrastructure_faults: usize,
    pub attempts: Vec<AttemptEvidence>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AttemptEvidence {
    pub schema: &'static str,
    pub candidate_id: String,
    pub attempt: u32,
    pub segment: SegmentProfile,
    pub tape: PathBuf,
    pub artifact_root: PathBuf,
    pub state_root: PathBuf,
    pub milestone_result: PathBuf,
    pub stdout: PathBuf,
    pub stderr: PathBuf,
    pub elapsed_millis: u128,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub cancelled: bool,
    pub infrastructure_error: Option<String>,
    pub milestone_depth: u16,
    pub deepest_milestone: String,
    pub first_hit_tick: Option<u64>,
    pub goal_reached: bool,
    pub milestone_observations: BTreeMap<String, MilestoneObservation>,
    pub boundary_fingerprints: BTreeMap<String, BoundaryFingerprint>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MilestoneObservation {
    pub sim_tick: u64,
    pub tape_frame: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BoundaryFingerprint {
    pub schema: String,
    pub algorithm: String,
    pub canonical_encoding: String,
    pub digest: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct SearchRunSummary {
    pub schema: &'static str,
    pub segment: SegmentProfile,
    pub generations: u32,
    pub population_size: usize,
    pub repetitions: u32,
    pub rng_seed: u64,
    pub champion_id: String,
    pub champion_candidate: PathBuf,
    pub champion_tape: PathBuf,
    pub score: crate::search::LexicographicScore,
    pub output_root: PathBuf,
}

#[derive(Clone, Debug)]
pub struct BootMinimizeConfig {
    pub candidate: Candidate,
    pub game: PathBuf,
    pub dvd: PathBuf,
    pub output_root: PathBuf,
    pub working_directory: PathBuf,
    pub game_args_prefix: Vec<String>,
    pub workers: usize,
    pub repetitions: u32,
    pub timeout: Duration,
}

#[derive(Clone, Debug, Serialize)]
pub struct BootMinimizeSummary {
    pub schema: &'static str,
    pub source_candidate_id: String,
    pub minimized_candidate_id: String,
    pub source_frames: u64,
    pub minimized_frames: u64,
    pub source_pulse_frames: usize,
    pub minimized_pulse_frames: usize,
    pub goal_sim_tick: u64,
    pub goal_tape_frame: u64,
    pub candidate: PathBuf,
    pub tape: PathBuf,
    pub proof: PathBuf,
    pub output_root: PathBuf,
}

pub fn evaluate_population(config: &EvaluateConfig) -> Result<EvaluationReport, EvaluateError> {
    let config = normalize_evaluate_config(config)?;
    validate_evaluate_config(&config)?;
    let population_bytes = fs::read(&config.population_path)?;
    let manifest: PopulationManifest = serde_json::from_slice(&population_bytes)?;
    validate_manifest(&manifest, &config.population_path)?;
    let population_root = canonical_parent(&config.population_path)?;
    let trials = build_trials(
        &manifest,
        &population_root,
        &config.output_root,
        config.repetitions,
    )?;
    fs::create_dir_all(&config.output_root)?;
    write_json(
        &config.output_root.join("plan.json"),
        &serde_json::json!({
            "schema": "dusklight-search-evaluation-plan/v2",
            "segment": manifest.segment,
            "population": config.population_path,
            "game": config.game,
            "dvd": config.dvd,
            "workers": config.workers,
            "repetitions": config.repetitions,
            "timeout_millis": config.timeout.as_millis(),
            "attempts": trials.len(),
        }),
    )?;

    let trials = Arc::new(trials);
    let next = Arc::new(AtomicUsize::new(0));
    let cancelled = Arc::new(AtomicBool::new(false));
    let outcomes = Arc::new(Mutex::new(Vec::with_capacity(trials.len())));
    let worker_count = config.workers.min(trials.len()).max(1);

    thread::scope(|scope| {
        let config = &config;
        let segment = manifest.segment;
        for _ in 0..worker_count {
            let trials = Arc::clone(&trials);
            let next = Arc::clone(&next);
            let cancelled = Arc::clone(&cancelled);
            let outcomes = Arc::clone(&outcomes);
            scope.spawn(move || {
                loop {
                    if cancelled.load(Ordering::Acquire) {
                        break;
                    }
                    let index = next.fetch_add(1, Ordering::AcqRel);
                    let Some(trial) = trials.get(index) else {
                        break;
                    };
                    let mut evidence = run_trial(config, segment, trial, &cancelled);
                    if let Err(error) = write_json(&trial.root.join("attempt.json"), &evidence) {
                        evidence.infrastructure_error =
                            Some(format!("could not persist attempt evidence: {error}"));
                    }
                    if evidence.infrastructure_error.is_some() {
                        cancelled.store(true, Ordering::Release);
                    }
                    outcomes.lock().unwrap().push(evidence);
                }
            });
        }
    });

    let mut attempts = Arc::try_unwrap(outcomes)
        .expect("evaluation workers still own outcomes")
        .into_inner()
        .unwrap();
    attempts.sort_by(|left, right| {
        left.candidate_id
            .cmp(&right.candidate_id)
            .then(left.attempt.cmp(&right.attempt))
    });
    let faults = attempts
        .iter()
        .filter(|attempt| attempt.infrastructure_error.is_some())
        .count();
    let report = EvaluationReport {
        schema: EVALUATION_SCHEMA,
        population: config.population_path.clone(),
        results: config.results_path.clone(),
        segment: manifest.segment,
        workers: config.workers,
        repetitions: config.repetitions,
        planned_attempts: trials.len(),
        completed_attempts: attempts.len(),
        infrastructure_faults: faults,
        attempts,
    };
    write_json(&config.output_root.join("evaluation.json"), &report)?;
    if faults != 0 || report.completed_attempts != report.planned_attempts {
        return Err(EvaluateError::Infrastructure {
            faults,
            completed: report.completed_attempts,
            planned: report.planned_attempts,
            evidence: config.output_root.join("evaluation.json"),
        });
    }
    let results = aggregate_results(&manifest, &report.attempts)?;
    if let Some(parent) = config.results_path.parent() {
        fs::create_dir_all(parent)?;
    }
    write_json(&config.results_path, &results)?;
    // Ranking also validates the population/result pairing and all counts.
    rank_population(&manifest, &results)?;
    Ok(report)
}

pub fn run_search(config: &SearchRunConfig) -> Result<SearchRunSummary, EvaluateError> {
    if config.generations == 0
        || config.population_size == 0
        || config.elite_count == 0
        || config.elite_count > config.population_size
    {
        return Err(EvaluateError::InvalidConfig(
            "generations, population size, and elites must be valid and nonzero".into(),
        ));
    }
    if !config.game.is_file()
        || !config.dvd.is_file()
        || !config.working_directory.is_dir()
        || directory_is_nonempty(&config.output_root)?
    {
        return Err(EvaluateError::InvalidConfig(
            "game, DVD, working directory, and a new/empty output root are required".into(),
        ));
    }
    fs::create_dir_all(&config.output_root)?;
    let seed_candidate = config
        .seed_candidate
        .clone()
        .unwrap_or_else(|| Candidate::baseline(config.segment));
    if seed_candidate.segment != config.segment {
        return Err(EvaluateError::InvalidConfig(
            "seed candidate segment does not match the search segment".into(),
        ));
    }
    seed_candidate.validate()?;
    let mut population_root = config.output_root.join("g000");
    let mut manifest = write_seed_population(
        &population_root,
        seed_candidate,
        config.population_size,
        config.rng_seed,
    )?;
    let mut final_results = None;
    for generation in 0..config.generations {
        let manifest_path = population_root.join("manifest.json");
        let results_path = population_root.join("results.json");
        evaluate_population(&EvaluateConfig {
            population_path: manifest_path.clone(),
            game: config.game.clone(),
            dvd: config.dvd.clone(),
            output_root: population_root.join("evaluations"),
            results_path: results_path.clone(),
            working_directory: config.working_directory.clone(),
            game_args_prefix: config.game_args_prefix.clone(),
            workers: config.workers,
            repetitions: config.repetitions,
            timeout: config.timeout,
        })?;
        let results: SearchResults = serde_json::from_slice(&fs::read(&results_path)?)?;
        let leaderboard = rank_population(&manifest, &results)?;
        write_json(&population_root.join("leaderboard.json"), &leaderboard)?;
        final_results = Some(results);
        if generation + 1 < config.generations {
            let next_root = config.output_root.join(format!("g{:03}", generation + 1));
            manifest = evolve_population(
                &manifest_path,
                final_results.as_ref().unwrap(),
                &next_root,
                EvolutionConfig {
                    population_size: config.population_size,
                    elite_count: config.elite_count,
                    rng_seed: config.rng_seed + u64::from(generation) + 1,
                },
            )?;
            population_root = next_root;
        }
    }
    let results = final_results.expect("nonzero generations");
    let leaderboard = rank_population(&manifest, &results)?;
    let champion = leaderboard.first().ok_or(EvaluateError::EmptyLeaderboard)?;
    let member = manifest
        .members
        .iter()
        .find(|member| member.candidate_id == champion.candidate_id)
        .ok_or(EvaluateError::EmptyLeaderboard)?;
    let source = population_root.join(&member.tape_file);
    let champion_tape = config.output_root.join("champion.tape");
    fs::copy(source, &champion_tape)?;
    let champion_candidate = config.output_root.join("champion.candidate.json");
    fs::copy(
        population_root.join(&member.candidate_file),
        &champion_candidate,
    )?;
    let summary = SearchRunSummary {
        schema: SEARCH_RUN_SCHEMA,
        segment: config.segment,
        generations: config.generations,
        population_size: config.population_size,
        repetitions: config.repetitions,
        rng_seed: config.rng_seed,
        champion_id: champion.candidate_id.clone(),
        champion_candidate,
        champion_tape,
        score: champion.score,
        output_root: config.output_root.clone(),
    };
    write_json(&config.output_root.join("run.summary.json"), &summary)?;
    Ok(summary)
}

#[derive(Clone, Debug)]
struct Trial {
    candidate_id: String,
    attempt: u32,
    tape: PathBuf,
    root: PathBuf,
    state: PathBuf,
    milestones: PathBuf,
    stdout: PathBuf,
    stderr: PathBuf,
}

fn build_trials(
    manifest: &PopulationManifest,
    population_root: &Path,
    output_root: &Path,
    repetitions: u32,
) -> Result<Vec<Trial>, EvaluateError> {
    let mut trials = Vec::with_capacity(manifest.members.len() * repetitions as usize);
    for member in &manifest.members {
        if member.candidate_id.is_empty()
            || !member
                .candidate_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(EvaluateError::InvalidManifest(format!(
                "candidate ID {:?} is unsafe",
                member.candidate_id
            )));
        }
        let tape = fs::canonicalize(population_root.join(&member.tape_file))?;
        if !tape.starts_with(population_root) {
            return Err(EvaluateError::InvalidManifest(format!(
                "candidate {} tape escapes the population directory",
                member.candidate_id
            )));
        }
        InputTape::decode(&fs::read(&tape)?)?;
        for attempt in 1..=repetitions {
            let root = output_root
                .join("candidates")
                .join(&member.candidate_id)
                .join(format!("attempt-{attempt:03}"));
            trials.push(Trial {
                candidate_id: member.candidate_id.clone(),
                attempt,
                tape: tape.clone(),
                state: root.join("state"),
                milestones: root.join("milestones.json"),
                stdout: root.join("stdout.txt"),
                stderr: root.join("stderr.txt"),
                root,
            });
        }
    }
    Ok(trials)
}

fn run_trial(
    config: &EvaluateConfig,
    segment: SegmentProfile,
    trial: &Trial,
    global_cancel: &AtomicBool,
) -> AttemptEvidence {
    let started = Instant::now();
    let mut evidence = AttemptEvidence {
        schema: ATTEMPT_SCHEMA,
        candidate_id: trial.candidate_id.clone(),
        attempt: trial.attempt,
        segment,
        tape: trial.tape.clone(),
        artifact_root: trial.root.clone(),
        state_root: trial.state.clone(),
        milestone_result: trial.milestones.clone(),
        stdout: trial.stdout.clone(),
        stderr: trial.stderr.clone(),
        elapsed_millis: 0,
        exit_code: None,
        timed_out: false,
        cancelled: false,
        infrastructure_error: None,
        milestone_depth: 0,
        deepest_milestone: "none".into(),
        first_hit_tick: None,
        goal_reached: false,
        milestone_observations: BTreeMap::new(),
        boundary_fingerprints: BTreeMap::new(),
    };
    let mut run = || -> Result<TrialScore, EvaluateError> {
        fs::create_dir_all(&trial.state)?;
        let stdout = File::create(&trial.stdout)?;
        let stderr = File::create(&trial.stderr)?;
        let mut command = Command::new(&config.game);
        command
            .current_dir(&config.working_directory)
            .args(&config.game_args_prefix)
            .arg("--dvd")
            .arg(&config.dvd);
        if segment == SegmentProfile::Fsp103ToFsp104 {
            command.arg("--stage").arg("F_SP103,1,1,3");
        }
        let milestone_list = match segment {
            SegmentProfile::BootToFsp103 => "gameplay-ready-f-sp103",
            SegmentProfile::Fsp103ToFsp104 => {
                "gameplay-ready-f-sp103,exit-f-sp103-to-f-sp104,entered-f-sp104"
            }
        };
        let goal = match segment {
            SegmentProfile::BootToFsp103 => "gameplay-ready-f-sp103",
            SegmentProfile::Fsp103ToFsp104 => "entered-f-sp104",
        };
        command
            .arg("--input-tape")
            .arg(&trial.tape)
            .arg("--input-tape-end")
            .arg("hold")
            .arg("--automation-data-root")
            .arg(&trial.state)
            .arg("--milestones")
            .arg(milestone_list)
            .arg("--milestone-goal")
            .arg(goal)
            .arg("--milestone-result")
            .arg(&trial.milestones)
            .arg("--cvar")
            .arg("game.instantSaves=true")
            .arg("--cvar")
            .arg("backend.cardFileType=1")
            .arg("--cvar")
            .arg("backend.wasPresetChosen=true")
            .arg("--cvar")
            .arg("game.enableMenuPointer=false")
            .arg("--headless")
            .arg("--fixed-step")
            .arg("--exit-after-tape")
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        hide_window(&mut command);
        let mut child = command.spawn().map_err(EvaluateError::Launch)?;
        loop {
            if global_cancel.load(Ordering::Acquire) {
                evidence.cancelled = true;
                let _ = child.kill();
                let _ = child.wait();
                return Err(EvaluateError::Cancelled);
            }
            if started.elapsed() >= config.timeout {
                evidence.timed_out = true;
                let _ = child.kill();
                let _ = child.wait();
                return Err(EvaluateError::Timeout(config.timeout));
            }
            match child.try_wait()? {
                Some(status) => {
                    evidence.exit_code = status.code();
                    break;
                }
                None => thread::sleep(Duration::from_millis(10)),
            }
        }
        parse_native_milestones(&trial.milestones, segment)
    };
    match run() {
        Ok(score) => {
            evidence.milestone_depth = score.depth;
            evidence.deepest_milestone = score.deepest;
            evidence.first_hit_tick = score.score_tick;
            evidence.goal_reached = score.goal_reached;
            evidence.milestone_observations = score.milestone_observations;
            evidence.boundary_fingerprints = score.boundary_fingerprints;
        }
        Err(error) => evidence.infrastructure_error = Some(error.to_string()),
    }
    evidence.elapsed_millis = started.elapsed().as_millis();
    evidence
}

#[derive(Debug)]
struct TrialScore {
    depth: u16,
    deepest: String,
    score_tick: Option<u64>,
    goal_reached: bool,
    milestone_observations: BTreeMap<String, MilestoneObservation>,
    boundary_fingerprints: BTreeMap<String, BoundaryFingerprint>,
}

#[derive(Deserialize)]
struct NativeMilestoneResult {
    schema: NativeSchema,
    goal: Option<String>,
    goal_reached: bool,
    milestones: Vec<NativeMilestone>,
}

#[derive(Deserialize)]
struct NativeSchema {
    name: String,
    version: u32,
}

#[derive(Deserialize)]
struct NativeMilestone {
    id: String,
    hit: bool,
    sim_tick: Option<u64>,
    tape_frame: Option<u64>,
    evidence: Option<NativeEvidence>,
}

#[derive(Deserialize)]
struct NativeEvidence {
    boundary_fingerprint: BoundaryFingerprint,
}

fn parse_native_milestones(
    path: &Path,
    segment: SegmentProfile,
) -> Result<TrialScore, EvaluateError> {
    let native: NativeMilestoneResult =
        serde_json::from_slice(&fs::read(path).map_err(|error| {
            EvaluateError::NativeResult(format!(
                "worker produced no readable milestone result at {}: {error}",
                path.display()
            ))
        })?)?;
    if native.schema.name != "dusklight.automation.milestones" || native.schema.version != 1 {
        return Err(EvaluateError::NativeResult(
            "unsupported native milestone schema".into(),
        ));
    }
    let expected_goal = match segment {
        SegmentProfile::BootToFsp103 => "gameplay-ready-f-sp103",
        SegmentProfile::Fsp103ToFsp104 => "entered-f-sp104",
    };
    if native.goal.as_deref() != Some(expected_goal) {
        return Err(EvaluateError::NativeResult(format!(
            "native result goal {:?} does not match {expected_goal}",
            native.goal
        )));
    }
    let mut milestones = BTreeMap::new();
    for milestone in native.milestones {
        let id = milestone.id.clone();
        if milestones.insert(id.clone(), milestone).is_some() {
            return Err(EvaluateError::NativeResult(format!(
                "duplicate native milestone {id}"
            )));
        }
    }
    let requested: &[&str] = match segment {
        SegmentProfile::BootToFsp103 => &["gameplay-ready-f-sp103"],
        SegmentProfile::Fsp103ToFsp104 => &[
            "gameplay-ready-f-sp103",
            "exit-f-sp103-to-f-sp104",
            "entered-f-sp104",
        ],
    };
    if milestones.len() != requested.len()
        || requested.iter().any(|id| !milestones.contains_key(*id))
    {
        return Err(EvaluateError::NativeResult(
            "native result does not contain the exact requested milestone set".into(),
        ));
    }
    let mut fingerprints = BTreeMap::new();
    let mut observations = BTreeMap::new();
    for (id, milestone) in &milestones {
        match (
            milestone.hit,
            milestone.sim_tick,
            milestone.tape_frame,
            &milestone.evidence,
        ) {
            (true, Some(sim_tick), Some(tape_frame), Some(evidence)) => {
                validate_fingerprint(&evidence.boundary_fingerprint)?;
                observations.insert(
                    id.clone(),
                    MilestoneObservation {
                        sim_tick,
                        tape_frame,
                    },
                );
                fingerprints.insert(id.clone(), evidence.boundary_fingerprint.clone());
            }
            (false, None, None, None) => {}
            _ => {
                return Err(EvaluateError::NativeResult(format!(
                    "milestone {id} has inconsistent hit evidence"
                )));
            }
        }
    }
    let hit = |id: &str| milestones[id].hit;
    let tick = |id: &str| milestones[id].sim_tick;
    if native.goal_reached != hit(expected_goal) {
        return Err(EvaluateError::NativeResult(
            "goal_reached disagrees with the goal milestone".into(),
        ));
    }
    let (depth, deepest, score_tick) = match segment {
        SegmentProfile::BootToFsp103 if hit("gameplay-ready-f-sp103") => {
            (2, "gameplay-ready-f-sp103", tick("gameplay-ready-f-sp103"))
        }
        SegmentProfile::BootToFsp103 => (0, "none", None),
        SegmentProfile::Fsp103ToFsp104 if hit("entered-f-sp104") => {
            if !hit("exit-f-sp103-to-f-sp104") {
                return Err(EvaluateError::NativeResult(
                    "entered F_SP104 without the required source-exit milestone".into(),
                ));
            }
            (4, "entered-f-sp104", tick("exit-f-sp103-to-f-sp104"))
        }
        SegmentProfile::Fsp103ToFsp104 if hit("exit-f-sp103-to-f-sp104") => (
            3,
            "exit-f-sp103-to-f-sp104",
            tick("exit-f-sp103-to-f-sp104"),
        ),
        SegmentProfile::Fsp103ToFsp104 if hit("gameplay-ready-f-sp103") => {
            (2, "gameplay-ready-f-sp103", tick("gameplay-ready-f-sp103"))
        }
        SegmentProfile::Fsp103ToFsp104 => (0, "none", None),
    };
    if segment == SegmentProfile::Fsp103ToFsp104
        && hit("exit-f-sp103-to-f-sp104")
        && !hit("gameplay-ready-f-sp103")
    {
        return Err(EvaluateError::NativeResult(
            "source exit was hit without the gameplay-ready prerequisite".into(),
        ));
    }
    Ok(TrialScore {
        depth,
        deepest: deepest.into(),
        score_tick,
        goal_reached: native.goal_reached,
        milestone_observations: observations,
        boundary_fingerprints: fingerprints,
    })
}

fn validate_fingerprint(fingerprint: &BoundaryFingerprint) -> Result<(), EvaluateError> {
    if fingerprint.schema != "dusklight.milestone-boundary/v1"
        || fingerprint.algorithm != "xxh3-128"
        || fingerprint.canonical_encoding != "little-endian-fixed-v1"
        || fingerprint.digest.len() != 32
        || !fingerprint
            .digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(EvaluateError::NativeResult(
            "invalid native boundary fingerprint".into(),
        ));
    }
    Ok(())
}

fn aggregate_results(
    manifest: &PopulationManifest,
    attempts: &[AttemptEvidence],
) -> Result<SearchResults, EvaluateError> {
    let mut candidates = BTreeMap::new();
    for member in &manifest.members {
        let samples: Vec<_> = attempts
            .iter()
            .filter(|attempt| attempt.candidate_id == member.candidate_id)
            .collect();
        if samples.is_empty()
            || samples
                .iter()
                .any(|sample| sample.infrastructure_error.is_some())
        {
            return Err(EvaluateError::InvalidResult(format!(
                "candidate {} does not have a complete valid sample set",
                member.candidate_id
            )));
        }
        let reference = samples[0];
        if samples.iter().skip(1).any(|sample| {
            sample.milestone_depth != reference.milestone_depth
                || sample.deepest_milestone != reference.deepest_milestone
                || sample.first_hit_tick != reference.first_hit_tick
                || sample.goal_reached != reference.goal_reached
                || sample.milestone_observations != reference.milestone_observations
                || sample.boundary_fingerprints != reference.boundary_fingerprints
        }) {
            return Err(EvaluateError::InvalidResult(format!(
                "candidate {} produced nondeterministic milestone evidence across identical trials",
                member.candidate_id
            )));
        }
        let depth = reference.milestone_depth;
        let ticks = if depth == 0 {
            Vec::new()
        } else {
            vec![
                reference.first_hit_tick.ok_or_else(|| {
                    EvaluateError::InvalidResult(format!(
                        "candidate {} reached depth {depth} without a score tick",
                        member.candidate_id
                    ))
                })?;
                samples.len()
            ]
        };
        candidates.insert(
            member.candidate_id.clone(),
            CandidateResult {
                milestone_depth: depth,
                attempts: samples.len() as u32,
                successes: if depth == 0 { 0 } else { samples.len() as u32 },
                first_hit_ticks: ticks,
            },
        );
    }
    Ok(SearchResults {
        schema: RESULTS_SCHEMA.into(),
        segment: manifest.segment,
        candidates,
    })
}

fn validate_evaluate_config(config: &EvaluateConfig) -> Result<(), EvaluateError> {
    if config.workers == 0 || config.repetitions == 0 || config.timeout.is_zero() {
        return Err(EvaluateError::InvalidConfig(
            "workers, repetitions, and timeout must be greater than zero".into(),
        ));
    }
    if !config.game.is_file() {
        return Err(EvaluateError::InvalidConfig(format!(
            "--game is not a file: {}",
            config.game.display()
        )));
    }
    if !config.dvd.is_file() {
        return Err(EvaluateError::InvalidConfig(format!(
            "--dvd is not a file: {}",
            config.dvd.display()
        )));
    }
    if !config.working_directory.is_dir() {
        return Err(EvaluateError::InvalidConfig(format!(
            "working directory does not exist: {}",
            config.working_directory.display()
        )));
    }
    if directory_is_nonempty(&config.output_root)? {
        return Err(EvaluateError::InvalidConfig(format!(
            "output root must be new or empty: {}",
            config.output_root.display()
        )));
    }
    Ok(())
}

fn normalize_evaluate_config(config: &EvaluateConfig) -> Result<EvaluateConfig, EvaluateError> {
    let absolute = |path: &Path| -> Result<PathBuf, EvaluateError> {
        if path.is_absolute() {
            Ok(path.to_path_buf())
        } else {
            Ok(std::env::current_dir()?.join(path))
        }
    };
    Ok(EvaluateConfig {
        population_path: fs::canonicalize(&config.population_path)?,
        game: fs::canonicalize(&config.game)?,
        dvd: fs::canonicalize(&config.dvd)?,
        output_root: absolute(&config.output_root)?,
        results_path: absolute(&config.results_path)?,
        working_directory: fs::canonicalize(&config.working_directory)?,
        game_args_prefix: config.game_args_prefix.clone(),
        workers: config.workers,
        repetitions: config.repetitions,
        timeout: config.timeout,
    })
}

fn directory_is_nonempty(path: &Path) -> Result<bool, EvaluateError> {
    Ok(path.exists() && fs::read_dir(path)?.next().is_some())
}

fn validate_manifest(manifest: &PopulationManifest, path: &Path) -> Result<(), EvaluateError> {
    if manifest.schema != POPULATION_SCHEMA || manifest.members.is_empty() {
        return Err(EvaluateError::InvalidManifest(format!(
            "invalid population manifest {}",
            path.display()
        )));
    }
    let mut ids = HashSet::new();
    if manifest
        .members
        .iter()
        .any(|member| !ids.insert(&member.candidate_id))
    {
        return Err(EvaluateError::InvalidManifest(
            "population contains duplicate candidate IDs".into(),
        ));
    }
    Ok(())
}

fn canonical_parent(path: &Path) -> Result<PathBuf, EvaluateError> {
    let parent = path
        .parent()
        .ok_or_else(|| EvaluateError::InvalidManifest("manifest has no parent".into()))?;
    Ok(fs::canonicalize(parent)?)
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), EvaluateError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

#[cfg(windows)]
fn hide_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_window(_: &mut Command) {}

#[derive(Debug)]
pub enum EvaluateError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Search(crate::search::SearchError),
    Tape(crate::tape::TapeError),
    InvalidConfig(String),
    InvalidManifest(String),
    InvalidResult(String),
    NativeResult(String),
    Launch(std::io::Error),
    Timeout(Duration),
    Cancelled,
    Infrastructure {
        faults: usize,
        completed: usize,
        planned: usize,
        evidence: PathBuf,
    },
    EmptyLeaderboard,
}

impl fmt::Display for EvaluateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "search evaluator I/O failed: {error}"),
            Self::Json(error) => write!(formatter, "invalid search evaluator JSON: {error}"),
            Self::Search(error) => error.fmt(formatter),
            Self::Tape(error) => error.fmt(formatter),
            Self::InvalidConfig(message) => {
                write!(formatter, "invalid evaluator config: {message}")
            }
            Self::InvalidManifest(message) => write!(formatter, "invalid population: {message}"),
            Self::InvalidResult(message) => write!(formatter, "invalid search result: {message}"),
            Self::NativeResult(message) => {
                write!(formatter, "invalid native milestone result: {message}")
            }
            Self::Launch(error) => write!(formatter, "could not launch Dusklight: {error}"),
            Self::Timeout(duration) => write!(
                formatter,
                "Dusklight timed out after {} ms",
                duration.as_millis()
            ),
            Self::Cancelled => {
                formatter.write_str("trial cancelled after another infrastructure fault")
            }
            Self::Infrastructure {
                faults,
                completed,
                planned,
                evidence,
            } => write!(
                formatter,
                "population evaluation failed: {faults} infrastructure fault(s), {completed}/{planned} attempts completed; evidence: {}",
                evidence.display()
            ),
            Self::EmptyLeaderboard => formatter.write_str("search produced an empty leaderboard"),
        }
    }
}

impl Error for EvaluateError {}

impl From<std::io::Error> for EvaluateError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<serde_json::Error> for EvaluateError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}
impl From<crate::search::SearchError> for EvaluateError {
    fn from(value: crate::search::SearchError) -> Self {
        Self::Search(value)
    }
}
impl From<crate::tape::TapeError> for EvaluateError {
    fn from(value: crate::tape::TapeError) -> Self {
        Self::Tape(value)
    }
}
