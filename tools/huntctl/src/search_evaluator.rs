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
    pub goal_boundary_fingerprint: String,
    pub candidate: PathBuf,
    pub tape: PathBuf,
    pub proof: PathBuf,
    pub output_root: PathBuf,
}

#[derive(Clone, Debug)]
pub struct BootGolfConfig {
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
pub struct BootGolfSummary {
    pub schema: &'static str,
    pub source_candidate_id: String,
    pub golfed_candidate_id: String,
    pub source_goal_sim_tick: u64,
    pub goal_sim_tick: u64,
    pub goal_tape_frame: u64,
    pub goal_boundary_fingerprint: String,
    pub source_pulse_timestamps: Vec<u64>,
    pub golfed_pulse_timestamps: Vec<u64>,
    pub accepted_moves: u32,
    pub evaluated_candidates: usize,
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

#[derive(Clone)]
struct ProvenBootCandidate {
    candidate: Candidate,
    tape: InputTape,
    sim_tick: u64,
    tape_frame: u64,
    boundary_fingerprint: BoundaryFingerprint,
}

#[derive(Clone)]
struct BootReductionTarget {
    sim_tick: u64,
    tape_frame: u64,
    boundary_fingerprint: BoundaryFingerprint,
}

impl BootReductionTarget {
    fn accepts(&self, candidate: &ProvenBootCandidate) -> bool {
        candidate.sim_tick == self.sim_tick
            && candidate.tape_frame == self.tape_frame
            && candidate.boundary_fingerprint == self.boundary_fingerprint
    }
}

pub fn minimize_boot(config: &BootMinimizeConfig) -> Result<BootMinimizeSummary, EvaluateError> {
    if config.candidate.segment != SegmentProfile::BootToFsp103 {
        return Err(EvaluateError::InvalidConfig(
            "boot minimization requires a boot_to_fsp103 candidate".into(),
        ));
    }
    config.candidate.validate()?;
    if !config.game.is_file()
        || !config.dvd.is_file()
        || !config.working_directory.is_dir()
        || config.workers == 0
        || config.repetitions < 2
        || config.timeout.is_zero()
        || directory_is_nonempty(&config.output_root)?
    {
        return Err(EvaluateError::InvalidConfig(
            "game, DVD, working directory, at least two repetitions, positive execution limits, and a new/empty output root are required"
                .into(),
        ));
    }
    fs::create_dir_all(&config.output_root)?;
    let source_id = config.candidate.id()?;
    let source_tape = config.candidate.compile()?;
    let source_frames = config.candidate.frame_count();
    let source_pulses = pulse_frame_count(&source_tape);
    let mut round = 0_u32;
    let initial = evaluate_boot_batch(
        config,
        vec![config.candidate.clone()],
        &config
            .output_root
            .join("rounds")
            .join(format!("{round:04}")),
        round,
    )?
    .into_iter()
    .next()
    .ok_or_else(|| {
        EvaluateError::InvalidResult(
            "the starting candidate did not reach gameplay-ready-f-sp103".into(),
        )
    })?;
    let mut current = initial;
    let target = BootReductionTarget {
        sim_tick: current.sim_tick,
        tape_frame: current.tape_frame,
        boundary_fingerprint: current.boundary_fingerprint.clone(),
    };
    round += 1;

    // First partition the ordered active frames into contiguous chunks. This
    // splits even one dense 800-frame A/Start mash into removable regions. The
    // frames become neutral rather than disappearing, so surviving pulses keep
    // their exact absolute timestamps throughout ddmin.
    let mut granularity = 2_usize;
    loop {
        let pulse_frames: Vec<_> = current
            .tape
            .frames
            .iter()
            .enumerate()
            .filter_map(|(index, frame)| (frame.pads[0].buttons != 0).then_some(index))
            .collect();
        if pulse_frames.is_empty() {
            break;
        }
        let partitions = granularity.min(pulse_frames.len());
        let mut candidates = Vec::with_capacity(partitions);
        for partition in 0..partitions {
            let start = pulse_frames.len() * partition / partitions;
            let end = pulse_frames.len() * (partition + 1) / partitions;
            let ranges = coalesce_pulse_frames(&pulse_frames[start..end]);
            candidates.push(candidate_with_neutralized_ranges(
                &current,
                &ranges,
                round,
                "ddmin pulse chunk",
            )?);
        }
        let mut proven = evaluate_boot_batch(
            config,
            candidates,
            &config
                .output_root
                .join("rounds")
                .join(format!("{round:04}")),
            round,
        )?;
        proven.retain(|candidate| target.accepts(candidate));
        round += 1;
        if let Some(best) = best_boot_candidate(proven) {
            current = best;
            granularity = 2;
        } else if partitions == pulse_frames.len() {
            break;
        } else {
            granularity = (partitions * 2).min(pulse_frames.len());
        }
    }

    // A run can contain several held or mashed button frames. Finish at frame
    // granularity, repeatedly taking the deletion with the fewest remaining
    // pulse frames and then the earliest exact goal tick.
    loop {
        let pulse_frames: Vec<_> = current
            .tape
            .frames
            .iter()
            .enumerate()
            .filter_map(|(index, frame)| (frame.pads[0].buttons != 0).then_some(index))
            .collect();
        if pulse_frames.is_empty() {
            break;
        }
        let candidates = pulse_frames
            .iter()
            .map(|index| {
                candidate_with_neutralized_ranges(
                    &current,
                    &[(*index, *index + 1)],
                    round,
                    "minimize individual pulse",
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut proven = evaluate_boot_batch(
            config,
            candidates,
            &config
                .output_root
                .join("rounds")
                .join(format!("{round:04}")),
            round,
        )?;
        proven.retain(|candidate| target.accepts(candidate));
        round += 1;
        if let Some(best) = best_boot_candidate(proven) {
            current = best;
        } else {
            break;
        }
    }

    let required_frames = usize::try_from(current.tape_frame)
        .map_err(|_| EvaluateError::InvalidResult("goal tape frame is too large".into()))?
        .checked_add(1)
        .ok_or_else(|| EvaluateError::InvalidResult("goal tape frame overflowed".into()))?;
    if required_frames > current.tape.frames.len() {
        return Err(EvaluateError::InvalidResult(
            "goal tape frame lies outside the candidate tape".into(),
        ));
    }
    let mut trimmed_tape = current.tape.clone();
    trimmed_tape.frames.truncate(required_frames);
    let mut trimmed = Candidate::from_absolute_tape(SegmentProfile::BootToFsp103, &trimmed_tape)?;
    trimmed.ancestry = crate::search::Ancestry {
        generation: round,
        parent_id: Some(current.candidate.id()?),
        mutation: Some("trim after exact goal tape frame".into()),
    };
    let proof_root = config.output_root.join("proof");
    let (mut proof_candidates, proof_report) =
        evaluate_boot_batch_with_report(config, vec![trimmed], &proof_root, round)?;
    proof_candidates.retain(|candidate| target.accepts(candidate));
    let minimized = proof_candidates.into_iter().next().ok_or_else(|| {
        EvaluateError::InvalidResult(
            "the tape trimmed to goal tape_frame + 1 did not reproduce the exact goal".into(),
        )
    })?;

    let candidate_path = config.output_root.join("minimized.candidate.json");
    let tape_path = config.output_root.join("minimized.tape");
    let proof_path = config.output_root.join("proof.json");
    fs::write(
        &candidate_path,
        serde_json::to_vec_pretty(&minimized.candidate)?,
    )?;
    fs::write(&tape_path, minimized.tape.encode()?)?;
    write_json(&proof_path, &proof_report)?;
    let summary = BootMinimizeSummary {
        schema: "dusklight-boot-minimization/v1",
        source_candidate_id: source_id,
        minimized_candidate_id: minimized.candidate.id()?,
        source_frames,
        minimized_frames: minimized.candidate.frame_count(),
        source_pulse_frames: source_pulses,
        minimized_pulse_frames: pulse_frame_count(&minimized.tape),
        goal_sim_tick: minimized.sim_tick,
        goal_tape_frame: minimized.tape_frame,
        goal_boundary_fingerprint: minimized.boundary_fingerprint.digest.clone(),
        candidate: candidate_path,
        tape: tape_path,
        proof: proof_path,
        output_root: config.output_root.clone(),
    };
    write_json(&config.output_root.join("minimize.summary.json"), &summary)?;
    Ok(summary)
}

/// Systematically moves the existing boot pulse sequence to earlier absolute
/// frames. This is coordinate descent over every legal earlier timestamp, not
/// a stochastic search: a move may be retained without improving the goal tick
/// when its earlier timestamp can expose a coordinated improvement on a later
/// pass.
pub fn golf_boot(config: &BootGolfConfig) -> Result<BootGolfSummary, EvaluateError> {
    if config.candidate.segment != SegmentProfile::BootToFsp103 {
        return Err(EvaluateError::InvalidConfig(
            "boot timing golf requires a boot_to_fsp103 candidate".into(),
        ));
    }
    config.candidate.validate()?;
    if !config.game.is_file()
        || !config.dvd.is_file()
        || !config.working_directory.is_dir()
        || config.workers == 0
        || config.repetitions < 2
        || config.timeout.is_zero()
        || directory_is_nonempty(&config.output_root)?
    {
        return Err(EvaluateError::InvalidConfig(
            "game, DVD, working directory, at least two repetitions, positive execution limits, and a new/empty output root are required"
                .into(),
        ));
    }
    fs::create_dir_all(&config.output_root)?;
    let evaluation = BootMinimizeConfig {
        candidate: config.candidate.clone(),
        game: config.game.clone(),
        dvd: config.dvd.clone(),
        output_root: config.output_root.clone(),
        working_directory: config.working_directory.clone(),
        game_args_prefix: config.game_args_prefix.clone(),
        workers: config.workers,
        repetitions: config.repetitions,
        timeout: config.timeout,
    };
    let source_id = config.candidate.id()?;
    let mut round = 0_u32;
    let mut evaluated_candidates = 1_usize;
    let initial = evaluate_boot_batch(
        &evaluation,
        vec![config.candidate.clone()],
        &config
            .output_root
            .join("rounds")
            .join(format!("{round:04}")),
        round,
    )?
    .into_iter()
    .next()
    .ok_or_else(|| {
        EvaluateError::InvalidResult(
            "the starting candidate did not reach gameplay-ready-f-sp103".into(),
        )
    })?;
    let source_goal_sim_tick = initial.sim_tick;
    let source_fingerprint = initial.boundary_fingerprint.clone();
    let source_pulse_timestamps = pulse_timestamps(&initial.tape)?;
    if source_pulse_timestamps.is_empty() {
        return Err(EvaluateError::InvalidConfig(
            "boot timing golf requires at least one pulse frame".into(),
        ));
    }
    let mut current = initial;
    let mut accepted_moves = 0_u32;
    round += 1;

    loop {
        let timestamps = pulse_timestamps(&current.tape)?;
        let mut candidates = Vec::new();
        // Last-to-first ordering makes the menu/cutscene pulses most likely to
        // occupy the first worker slots while retaining deterministic results.
        for pulse_index in (0..timestamps.len()).rev() {
            let earliest = if pulse_index == 0 {
                0
            } else {
                timestamps[pulse_index - 1]
                    .checked_add(1)
                    .ok_or_else(|| EvaluateError::InvalidResult("pulse frame overflowed".into()))?
            };
            for timestamp in (earliest..timestamps[pulse_index]).rev() {
                candidates.push(candidate_with_shifted_pulse(
                    &current,
                    pulse_index,
                    timestamp,
                    round,
                )?);
            }
        }
        if candidates.is_empty() {
            break;
        }
        evaluated_candidates = evaluated_candidates
            .checked_add(candidates.len())
            .ok_or_else(|| EvaluateError::InvalidResult("candidate count overflowed".into()))?;
        let mut proven = evaluate_boot_batch(
            &evaluation,
            candidates,
            &config
                .output_root
                .join("rounds")
                .join(format!("{round:04}")),
            round,
        )?;
        proven.retain(|candidate| {
            candidate.boundary_fingerprint == source_fingerprint
                && candidate.sim_tick <= current.sim_tick
                && boot_golf_cmp(candidate, &current).is_lt()
        });
        let Some(best) = proven.into_iter().min_by(boot_golf_cmp) else {
            break;
        };
        current = best;
        accepted_moves = accepted_moves
            .checked_add(1)
            .ok_or_else(|| EvaluateError::InvalidResult("accepted move count overflowed".into()))?;
        round += 1;
    }

    let exact_target = BootReductionTarget {
        sim_tick: current.sim_tick,
        tape_frame: current.tape_frame,
        boundary_fingerprint: source_fingerprint.clone(),
    };
    let required_frames = usize::try_from(current.tape_frame)
        .map_err(|_| EvaluateError::InvalidResult("goal tape frame is too large".into()))?
        .checked_add(1)
        .ok_or_else(|| EvaluateError::InvalidResult("goal tape frame overflowed".into()))?;
    if required_frames > current.tape.frames.len() {
        return Err(EvaluateError::InvalidResult(
            "goal tape frame lies outside the candidate tape".into(),
        ));
    }
    let mut trimmed_tape = current.tape.clone();
    trimmed_tape.frames.truncate(required_frames);
    let mut trimmed = Candidate::from_absolute_tape(SegmentProfile::BootToFsp103, &trimmed_tape)?;
    trimmed.ancestry = crate::search::Ancestry {
        generation: round,
        parent_id: Some(current.candidate.id()?),
        mutation: Some("trim after exact goal tape frame".into()),
    };
    let proof_root = config.output_root.join("proof");
    let (mut proof_candidates, proof_report) =
        evaluate_boot_batch_with_report(&evaluation, vec![trimmed], &proof_root, round)?;
    evaluated_candidates = evaluated_candidates
        .checked_add(1)
        .ok_or_else(|| EvaluateError::InvalidResult("candidate count overflowed".into()))?;
    proof_candidates.retain(|candidate| exact_target.accepts(candidate));
    let golfed = proof_candidates.into_iter().next().ok_or_else(|| {
        EvaluateError::InvalidResult(
            "the final boot timing candidate did not reproduce its exact proof".into(),
        )
    })?;

    let candidate_path = config.output_root.join("golfed.candidate.json");
    let tape_path = config.output_root.join("golfed.tape");
    let proof_path = config.output_root.join("proof.json");
    fs::write(
        &candidate_path,
        serde_json::to_vec_pretty(&golfed.candidate)?,
    )?;
    fs::write(&tape_path, golfed.tape.encode()?)?;
    write_json(&proof_path, &proof_report)?;
    let summary = BootGolfSummary {
        schema: "dusklight-boot-timing-golf/v1",
        source_candidate_id: source_id,
        golfed_candidate_id: golfed.candidate.id()?,
        source_goal_sim_tick,
        goal_sim_tick: golfed.sim_tick,
        goal_tape_frame: golfed.tape_frame,
        goal_boundary_fingerprint: source_fingerprint.digest,
        source_pulse_timestamps,
        golfed_pulse_timestamps: pulse_timestamps(&golfed.tape)?,
        accepted_moves,
        evaluated_candidates,
        candidate: candidate_path,
        tape: tape_path,
        proof: proof_path,
        output_root: config.output_root.clone(),
    };
    write_json(&config.output_root.join("golf.summary.json"), &summary)?;
    Ok(summary)
}

fn pulse_timestamps(tape: &InputTape) -> Result<Vec<u64>, EvaluateError> {
    tape.frames
        .iter()
        .enumerate()
        .filter(|(_, frame)| frame.pads[0].buttons != 0)
        .map(|(index, _)| {
            u64::try_from(index).map_err(|_| {
                EvaluateError::InvalidResult("pulse timestamp does not fit in u64".into())
            })
        })
        .collect()
}

fn pulse_timestamp_sum(tape: &InputTape) -> Result<u64, EvaluateError> {
    pulse_timestamps(tape)?
        .into_iter()
        .try_fold(0_u64, |sum, timestamp| {
            sum.checked_add(timestamp).ok_or_else(|| {
                EvaluateError::InvalidResult("pulse timestamp sum overflowed".into())
            })
        })
}

fn boot_golf_cmp(left: &ProvenBootCandidate, right: &ProvenBootCandidate) -> std::cmp::Ordering {
    let left_timestamps = pulse_timestamps(&left.tape).expect("validated candidate timestamps");
    let right_timestamps = pulse_timestamps(&right.tape).expect("validated candidate timestamps");
    left.sim_tick
        .cmp(&right.sim_tick)
        .then_with(|| {
            pulse_timestamp_sum(&left.tape)
                .expect("validated candidate timestamp sum")
                .cmp(&pulse_timestamp_sum(&right.tape).expect("validated candidate timestamp sum"))
        })
        .then(left_timestamps.cmp(&right_timestamps))
        .then_with(|| {
            left.candidate
                .id()
                .unwrap()
                .cmp(&right.candidate.id().unwrap())
        })
}

fn candidate_with_shifted_pulse(
    parent: &ProvenBootCandidate,
    pulse_index: usize,
    new_timestamp: u64,
    generation: u32,
) -> Result<Candidate, EvaluateError> {
    let timestamps = pulse_timestamps(&parent.tape)?;
    let old_timestamp = *timestamps.get(pulse_index).ok_or_else(|| {
        EvaluateError::InvalidResult(format!("pulse index {pulse_index} is out of range"))
    })?;
    let new_index = usize::try_from(new_timestamp)
        .map_err(|_| EvaluateError::InvalidResult("new pulse timestamp is too large".into()))?;
    let old_index = usize::try_from(old_timestamp)
        .map_err(|_| EvaluateError::InvalidResult("old pulse timestamp is too large".into()))?;
    if new_timestamp >= old_timestamp
        || parent.tape.frames[new_index].pads[0].buttons != 0
        || (pulse_index > 0 && new_timestamp <= timestamps[pulse_index - 1])
    {
        return Err(EvaluateError::InvalidResult(
            "shifted pulse does not preserve strict input order".into(),
        ));
    }
    let mut tape = parent.tape.clone();
    let pad = tape.frames[old_index].pads[0];
    tape.frames[old_index].pads[0] = RawPadState::default();
    tape.frames[new_index].pads[0] = pad;
    let mut candidate = Candidate::from_absolute_tape(SegmentProfile::BootToFsp103, &tape)?;
    candidate.ancestry = crate::search::Ancestry {
        generation,
        parent_id: Some(parent.candidate.id()?),
        mutation: Some(format!(
            "move pulse {pulse_index} from frame {old_timestamp} to {new_timestamp}"
        )),
    };
    Ok(candidate)
}

fn pulse_frame_count(tape: &InputTape) -> usize {
    tape.frames
        .iter()
        .filter(|frame| frame.pads[0].buttons != 0)
        .count()
}

fn coalesce_pulse_frames(frames: &[usize]) -> Vec<(usize, usize)> {
    let mut runs = Vec::new();
    for &frame in frames {
        if let Some((_, end)) = runs.last_mut()
            && *end == frame
        {
            *end += 1;
        } else {
            runs.push((frame, frame + 1));
        }
    }
    runs
}

fn candidate_with_neutralized_ranges(
    parent: &ProvenBootCandidate,
    ranges: &[(usize, usize)],
    generation: u32,
    mutation: &str,
) -> Result<Candidate, EvaluateError> {
    let mut tape = parent.tape.clone();
    for &(start, end) in ranges {
        for frame in &mut tape.frames[start..end] {
            frame.pads[0] = RawPadState::default();
        }
    }
    let mut candidate = Candidate::from_absolute_tape(SegmentProfile::BootToFsp103, &tape)?;
    candidate.ancestry = crate::search::Ancestry {
        generation,
        parent_id: Some(parent.candidate.id()?),
        mutation: Some(mutation.into()),
    };
    Ok(candidate)
}

fn best_boot_candidate(candidates: Vec<ProvenBootCandidate>) -> Option<ProvenBootCandidate> {
    candidates.into_iter().min_by(|left, right| {
        left.sim_tick
            .cmp(&right.sim_tick)
            .then(left.tape_frame.cmp(&right.tape_frame))
            .then(pulse_frame_count(&left.tape).cmp(&pulse_frame_count(&right.tape)))
            .then_with(|| {
                left.candidate
                    .id()
                    .unwrap()
                    .cmp(&right.candidate.id().unwrap())
            })
    })
}

fn evaluate_boot_batch(
    config: &BootMinimizeConfig,
    candidates: Vec<Candidate>,
    root: &Path,
    generation: u32,
) -> Result<Vec<ProvenBootCandidate>, EvaluateError> {
    Ok(evaluate_boot_batch_with_report(config, candidates, root, generation)?.0)
}

fn evaluate_boot_batch_with_report(
    config: &BootMinimizeConfig,
    candidates: Vec<Candidate>,
    root: &Path,
    generation: u32,
) -> Result<(Vec<ProvenBootCandidate>, EvaluationReport), EvaluateError> {
    let population_root = root.join("population");
    let manifest = write_explicit_population(
        &population_root,
        SegmentProfile::BootToFsp103,
        generation,
        candidates.clone(),
    )?;
    let report = evaluate_population(&EvaluateConfig {
        population_path: population_root.join("manifest.json"),
        game: config.game.clone(),
        dvd: config.dvd.clone(),
        output_root: root.join("evidence"),
        results_path: root.join("results.json"),
        working_directory: config.working_directory.clone(),
        game_args_prefix: config.game_args_prefix.clone(),
        workers: config.workers,
        repetitions: config.repetitions,
        timeout: config.timeout,
    })?;
    let mut proven = Vec::new();
    for candidate in candidates {
        let id = candidate.id()?;
        let attempts: Vec<_> = report
            .attempts
            .iter()
            .filter(|attempt| attempt.candidate_id == id)
            .collect();
        if attempts.len() != config.repetitions as usize
            || !attempts.iter().all(|attempt| attempt.goal_reached)
        {
            continue;
        }
        let observation = attempts[0]
            .milestone_observations
            .get("gameplay-ready-f-sp103")
            .ok_or_else(|| {
                EvaluateError::InvalidResult(format!(
                    "successful boot candidate {id} has no goal observation"
                ))
            })?;
        let boundary_fingerprint = attempts[0]
            .boundary_fingerprints
            .get("gameplay-ready-f-sp103")
            .ok_or_else(|| {
                EvaluateError::InvalidResult(format!(
                    "successful boot candidate {id} has no goal boundary fingerprint"
                ))
            })?
            .clone();
        proven.push(ProvenBootCandidate {
            tape: candidate.compile()?,
            candidate,
            sim_tick: observation.sim_tick,
            tape_frame: observation.tape_frame,
            boundary_fingerprint,
        });
    }
    // Keep manifest live in this scope as a sanity assertion that every exact
    // caller-supplied candidate was represented once.
    debug_assert_eq!(
        manifest.members.len(),
        report.planned_attempts / config.repetitions as usize
    );
    Ok((proven, report))
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

#[cfg(test)]
mod minimize_tests {
    use super::*;

    fn proven(sim_tick: u64, tape_frame: u64, digest: &str) -> ProvenBootCandidate {
        let candidate = Candidate::baseline(SegmentProfile::BootToFsp103);
        ProvenBootCandidate {
            tape: candidate.compile().unwrap(),
            candidate,
            sim_tick,
            tape_frame,
            boundary_fingerprint: BoundaryFingerprint {
                schema: "dusklight.milestone-boundary/v1".into(),
                algorithm: "xxh3-128".into(),
                canonical_encoding: "little-endian-fixed-v1".into(),
                digest: digest.into(),
            },
        }
    }

    #[test]
    fn boot_reduction_target_rejects_later_or_different_proof() {
        let source = proven(439, 439, &"a".repeat(32));
        let target = BootReductionTarget {
            sim_tick: source.sim_tick,
            tape_frame: source.tape_frame,
            boundary_fingerprint: source.boundary_fingerprint.clone(),
        };
        assert!(target.accepts(&source));
        assert!(!target.accepts(&proven(440, 439, &"a".repeat(32))));
        assert!(!target.accepts(&proven(439, 440, &"a".repeat(32))));
        assert!(!target.accepts(&proven(439, 439, &"b".repeat(32))));
    }
}
