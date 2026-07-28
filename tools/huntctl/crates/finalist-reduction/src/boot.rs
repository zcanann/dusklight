//! Boot-specific minimization and timing golf over proved native candidates.

use super::*;
use sha2::{Digest as _, Sha256};
use std::io::Read;

mod types;

use types::{
    BootGolfBatchCache, BootGolfCachedProof, BootGolfRunIdentity, BootReductionTarget,
    ProvenBootCandidate, alternate_menu_buttons, pulse_frame_count,
};

pub fn minimize_boot(config: &BootMinimizeConfig) -> Result<BootMinimizeSummary, EvaluateError> {
    if config.candidate.segment != SegmentProfile::BootToFsp103 {
        return Err(EvaluateError::InvalidConfig(
            "boot minimization requires a boot_to_fsp103 candidate".into(),
        ));
    }
    config.candidate.validate()?;
    validate_boot_harness(config.harness.as_ref())?;
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
    trimmed.ancestry = Ancestry {
        generation: round,
        parent_id: Some(current.candidate.id()?),
        mutation: Some("trim after exact goal tape frame".into()),
        intervention: Some(InterventionRange {
            start_frame: required_frames as u64,
            end_frame_exclusive: required_frames as u64,
            parent_end_frame_exclusive: current.tape.frames.len() as u64,
        }),
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
const BOOT_GOLF_EVALUATION_BATCH_SIZE: usize = 32;
const BUTTON_A: u16 = 0x0100;
const BUTTON_START: u16 = 0x1000;

pub fn golf_boot(config: &BootGolfConfig) -> Result<BootGolfSummary, EvaluateError> {
    if config.candidate.segment != SegmentProfile::BootToFsp103 {
        return Err(EvaluateError::InvalidConfig(
            "boot timing golf requires a boot_to_fsp103 candidate".into(),
        ));
    }
    config.candidate.validate()?;
    validate_boot_harness(config.harness.as_ref())?;
    let output_nonempty = directory_is_nonempty(&config.output_root)?;
    if !config.game.is_file()
        || !config.dvd.is_file()
        || !config.working_directory.is_dir()
        || config.workers == 0
        || config.repetitions < 2
        || config.timeout.is_zero()
        || (!config.resume && output_nonempty)
        || (config.resume && !output_nonempty)
        || (config.resume && config.output_root.join("golf.summary.json").exists())
    {
        return Err(EvaluateError::InvalidConfig(
            "boot timing golf requires valid execution inputs, at least two repetitions, and either a new output root or --resume with an incomplete matching output root"
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
        harness: config.harness.clone(),
    };
    let source_id = config.candidate.id()?;
    let mut round = 0_u32;
    let mut evaluated_candidates = 1_usize;
    let initial = evaluate_boot_batch(
        &evaluation,
        vec![config.candidate.clone()],
        &fresh_boot_evidence_root(
            &config
                .output_root
                .join("rounds")
                .join(format!("{round:04}")),
            config.resume,
        )?,
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
    let run_identity = boot_golf_run_identity(config, &source_id, &initial)?;
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
            let old_index = usize::try_from(timestamps[pulse_index])
                .map_err(|_| EvaluateError::InvalidResult("pulse timestamp is too large".into()))?;
            let authored_buttons = current.tape.frames[old_index].pads[0].buttons;
            if let Some(alternate_buttons) = alternate_menu_buttons(authored_buttons) {
                candidates.push(candidate_with_shifted_pulse(
                    &current,
                    pulse_index,
                    timestamps[pulse_index],
                    alternate_buttons,
                    round,
                )?);
            }
            for timestamp in (earliest..timestamps[pulse_index]).rev() {
                candidates.push(candidate_with_shifted_pulse(
                    &current,
                    pulse_index,
                    timestamp,
                    authored_buttons,
                    round,
                )?);
                if let Some(alternate_buttons) = alternate_menu_buttons(authored_buttons) {
                    candidates.push(candidate_with_shifted_pulse(
                        &current,
                        pulse_index,
                        timestamp,
                        alternate_buttons,
                        round,
                    )?);
                }
            }
        }
        if candidates.is_empty() {
            break;
        }
        evaluated_candidates = evaluated_candidates
            .checked_add(candidates.len())
            .ok_or_else(|| EvaluateError::InvalidResult("candidate count overflowed".into()))?;
        // Keep native evidence sets bounded. A full boot coordinate round can
        // contain hundreds of candidates and thousands of trace artifacts;
        // aggregating that as one population needlessly makes controller
        // memory scale with the whole round.
        let mut best: Option<ProvenBootCandidate> = None;
        for (batch_index, batch) in candidates
            .chunks(BOOT_GOLF_EVALUATION_BATCH_SIZE)
            .enumerate()
        {
            let proven = evaluate_or_load_boot_golf_batch(
                &evaluation,
                &run_identity,
                batch.to_vec(),
                round,
                batch_index,
                config.resume,
            )?;
            for candidate in proven.into_iter().filter(|candidate| {
                candidate.boundary_fingerprint == source_fingerprint
                    && candidate.sim_tick <= current.sim_tick
                    && boot_golf_quality_cmp(candidate, &current).is_lt()
            }) {
                if best
                    .as_ref()
                    .is_none_or(|incumbent| boot_golf_cmp(&candidate, incumbent).is_lt())
                {
                    best = Some(candidate);
                }
            }
        }
        let Some(best) = best else {
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
    trimmed.ancestry = Ancestry {
        generation: round,
        parent_id: Some(current.candidate.id()?),
        mutation: Some("trim after exact goal tape frame".into()),
        intervention: Some(InterventionRange {
            start_frame: required_frames as u64,
            end_frame_exclusive: required_frames as u64,
            parent_end_frame_exclusive: current.tape.frames.len() as u64,
        }),
    };
    let proof_root = fresh_boot_evidence_root(&config.output_root.join("proof"), config.resume)?;
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

fn boot_golf_run_identity(
    config: &BootGolfConfig,
    source_candidate_id: &str,
    source: &ProvenBootCandidate,
) -> Result<BootGolfRunIdentity, EvaluateError> {
    let timeout_millis = u64::try_from(config.timeout.as_millis()).map_err(|_| {
        EvaluateError::InvalidConfig("boot golf timeout does not fit in u64 milliseconds".into())
    })?;
    Ok(BootGolfRunIdentity {
        schema: "dusklight-boot-timing-golf-run/v1".into(),
        strategy: "a-start-coordinate-descent/v3".into(),
        source_candidate_id: source_candidate_id.into(),
        source_goal_sim_tick: source.sim_tick,
        source_goal_tape_frame: source.tape_frame,
        source_boundary_fingerprint: source.boundary_fingerprint.clone(),
        game_sha256: sha256_file(&config.game)?,
        dvd_sha256: sha256_file(&config.dvd)?,
        working_directory: fs::canonicalize(&config.working_directory)?,
        game_args_prefix: config.game_args_prefix.clone(),
        repetitions: config.repetitions,
        timeout_millis,
        harness_request_sha256: config
            .harness
            .as_ref()
            .map(|harness| harness.request_template.content_sha256),
    })
}

fn sha256_file(path: &Path) -> Result<ArtifactDigest, EvaluateError> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(ArtifactDigest(hasher.finalize().into()))
}

fn boot_golf_batch_cache_digest(
    cache: &BootGolfBatchCache,
) -> Result<ArtifactDigest, EvaluateError> {
    let mut unsigned = cache.clone();
    unsigned.content_sha256 = ArtifactDigest::ZERO;
    Ok(ArtifactDigest(
        Sha256::digest(serde_json::to_vec(&unsigned)?).into(),
    ))
}

fn validate_boot_golf_batch_cache(
    cache: &BootGolfBatchCache,
    run: &BootGolfRunIdentity,
    output_root: &Path,
    round: u32,
    batch_index: usize,
    candidate_ids: &[String],
) -> Result<(), EvaluateError> {
    let candidate_set = candidate_ids.iter().collect::<BTreeSet<_>>();
    let proven_ids = cache
        .proven
        .iter()
        .map(|proof| &proof.candidate_id)
        .collect::<BTreeSet<_>>();
    let relative_path_is_safe = |path: &Path| {
        !path.as_os_str().is_empty()
            && path.components().all(|component| {
                matches!(
                    component,
                    std::path::Component::Normal(_) | std::path::Component::CurDir
                )
            })
    };
    let evaluation_match = relative_path_is_safe(&cache.evaluation)
        && output_root.join(&cache.evaluation).is_file()
        && sha256_file(&output_root.join(&cache.evaluation))? == cache.evaluation_sha256;
    let results_match = relative_path_is_safe(&cache.results)
        && output_root.join(&cache.results).is_file()
        && sha256_file(&output_root.join(&cache.results))? == cache.results_sha256;
    if cache.schema != "dusklight-boot-timing-golf-batch/v1"
        || cache.content_sha256 == ArtifactDigest::ZERO
        || cache.content_sha256 != boot_golf_batch_cache_digest(cache)?
        || &cache.run != run
        || cache.round != round
        || cache.batch_index != batch_index
        || cache.candidate_ids != candidate_ids
        || candidate_set.len() != candidate_ids.len()
        || proven_ids.len() != cache.proven.len()
        || !proven_ids.is_subset(&candidate_set)
        || !evaluation_match
        || !results_match
    {
        return Err(EvaluateError::InvalidResult(format!(
            "boot golf batch {round:04}/{batch_index:04} is stale, inconsistent, or tampered"
        )));
    }
    Ok(())
}

fn boot_golf_batch_cache_path(output_root: &Path, round: u32, batch_index: usize) -> PathBuf {
    output_root
        .join("rounds")
        .join(format!("{round:04}"))
        .join(format!("batch-{batch_index:04}.cache.json"))
}

fn write_boot_golf_batch_cache(
    path: &Path,
    mut cache: BootGolfBatchCache,
) -> Result<(), EvaluateError> {
    cache.content_sha256 = boot_golf_batch_cache_digest(&cache)?;
    let bytes = serde_json::to_vec_pretty(&cache)?;
    if path.exists() {
        if fs::read(path)? != bytes {
            return Err(EvaluateError::InvalidResult(format!(
                "boot golf batch cache destination changed: {}",
                path.display()
            )));
        }
        return Ok(());
    }
    let parent = path.parent().ok_or_else(|| {
        EvaluateError::InvalidConfig("boot golf batch cache has no parent directory".into())
    })?;
    fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| EvaluateError::InvalidConfig("invalid batch cache file name".into()))?;
    let temporary = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));
    fs::write(&temporary, bytes)?;
    fs::rename(&temporary, path)?;
    Ok(())
}

fn fresh_boot_evidence_root(base: &Path, resume: bool) -> Result<PathBuf, EvaluateError> {
    if !base.exists() {
        return Ok(base.to_path_buf());
    }
    if !resume {
        return Err(EvaluateError::InvalidConfig(format!(
            "boot golf evidence root already exists: {}",
            base.display()
        )));
    }
    for attempt in 1..=u16::MAX {
        let candidate = base.with_file_name(format!(
            "{}-resume-{attempt:04}",
            base.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("evidence")
        ));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(EvaluateError::InvalidResult(
        "too many resumed boot golf evidence roots".into(),
    ))
}

fn evaluate_or_load_boot_golf_batch(
    config: &BootMinimizeConfig,
    run: &BootGolfRunIdentity,
    candidates: Vec<Candidate>,
    round: u32,
    batch_index: usize,
    resume: bool,
) -> Result<Vec<ProvenBootCandidate>, EvaluateError> {
    let candidate_ids = candidates
        .iter()
        .map(Candidate::id)
        .collect::<Result<Vec<_>, _>>()?;
    let cache_path = boot_golf_batch_cache_path(&config.output_root, round, batch_index);
    if resume && cache_path.is_file() {
        let cache: BootGolfBatchCache = serde_json::from_slice(&fs::read(&cache_path)?)?;
        validate_boot_golf_batch_cache(
            &cache,
            run,
            &config.output_root,
            round,
            batch_index,
            &candidate_ids,
        )?;
        return cache
            .proven
            .into_iter()
            .map(|proof| {
                let candidate = candidates
                    .iter()
                    .find(|candidate| candidate.id().is_ok_and(|id| id == proof.candidate_id))
                    .cloned()
                    .ok_or_else(|| {
                        EvaluateError::InvalidResult(format!(
                            "cached proof references absent candidate {}",
                            proof.candidate_id
                        ))
                    })?;
                Ok(ProvenBootCandidate {
                    tape: candidate.compile()?,
                    candidate,
                    sim_tick: proof.sim_tick,
                    tape_frame: proof.tape_frame,
                    boundary_fingerprint: proof.boundary_fingerprint,
                })
            })
            .collect();
    }

    let base = config
        .output_root
        .join("rounds")
        .join(format!("{round:04}"))
        .join(format!("batch-{batch_index:04}"));
    let evidence_root = fresh_boot_evidence_root(&base, resume)?;
    let (proven, report) =
        evaluate_boot_batch_with_report(config, candidates, &evidence_root, round)?;
    if report.completed_attempts != report.planned_attempts || report.infrastructure_faults != 0 {
        return Err(EvaluateError::InvalidResult(format!(
            "boot golf batch {round:04}/{batch_index:04} did not seal every planned attempt"
        )));
    }
    let canonical_output_root = fs::canonicalize(&config.output_root)?;
    let canonical_results = fs::canonicalize(&report.results)?;
    let results = canonical_results
        .strip_prefix(&canonical_output_root)
        .map_err(|_| {
            EvaluateError::InvalidResult(format!(
                "boot golf results escaped output root: {}",
                canonical_results.display()
            ))
        })?;
    let results = results.to_path_buf();
    let results_sha256 = sha256_file(&report.results)?;
    let evaluation_path = fs::canonicalize(evidence_root.join("evidence/evaluation.json"))?;
    let evaluation = evaluation_path
        .strip_prefix(&canonical_output_root)
        .map_err(|_| {
            EvaluateError::InvalidResult(format!(
                "boot golf evaluation escaped output root: {}",
                evaluation_path.display()
            ))
        })?
        .to_path_buf();
    let evaluation_sha256 = sha256_file(&evaluation_path)?;
    let cached_proofs = proven
        .iter()
        .map(|candidate| {
            Ok(BootGolfCachedProof {
                candidate_id: candidate.candidate.id()?,
                sim_tick: candidate.sim_tick,
                tape_frame: candidate.tape_frame,
                boundary_fingerprint: candidate.boundary_fingerprint.clone(),
            })
        })
        .collect::<Result<Vec<_>, EvaluateError>>()?;
    write_boot_golf_batch_cache(
        &cache_path,
        BootGolfBatchCache {
            schema: "dusklight-boot-timing-golf-batch/v1".into(),
            content_sha256: ArtifactDigest::ZERO,
            run: run.clone(),
            round,
            batch_index,
            candidate_ids,
            proven: cached_proofs,
            evaluation,
            evaluation_sha256,
            results,
            results_sha256,
        },
    )?;
    Ok(proven)
}

fn validate_boot_harness(harness: Option<&HarnessEvaluateConfig>) -> Result<(), EvaluateError> {
    if harness
        .is_some_and(|harness| harness.request_template.objective.goal != "gameplay-ready-f-sp103")
    {
        return Err(EvaluateError::InvalidConfig(
            "boot finalist reduction requires run-request goal gameplay-ready-f-sp103".into(),
        ));
    }
    Ok(())
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
    boot_golf_quality_cmp(left, right).then_with(|| {
        left.candidate
            .id()
            .unwrap()
            .cmp(&right.candidate.id().unwrap())
    })
}

fn boot_golf_quality_cmp(
    left: &ProvenBootCandidate,
    right: &ProvenBootCandidate,
) -> std::cmp::Ordering {
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
}

fn candidate_with_shifted_pulse(
    parent: &ProvenBootCandidate,
    pulse_index: usize,
    new_timestamp: u64,
    new_buttons: u16,
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
    let old_buttons = parent.tape.frames[old_index].pads[0].buttons;
    if new_timestamp > old_timestamp
        || (new_timestamp == old_timestamp && new_buttons == old_buttons)
        || (new_timestamp != old_timestamp && parent.tape.frames[new_index].pads[0].buttons != 0)
        || (pulse_index > 0 && new_timestamp <= timestamps[pulse_index - 1])
    {
        return Err(EvaluateError::InvalidResult(
            "shifted pulse does not preserve strict input order".into(),
        ));
    }
    let mut tape = parent.tape.clone();
    let mut pad = tape.frames[old_index].pads[0];
    pad.buttons = new_buttons;
    tape.frames[old_index].pads[0] = RawPadState::default();
    tape.frames[new_index].pads[0] = pad;
    let mut candidate = Candidate::from_absolute_tape(SegmentProfile::BootToFsp103, &tape)?;
    candidate.ancestry = Ancestry {
        generation,
        parent_id: Some(parent.candidate.id()?),
        mutation: Some(if old_timestamp == new_timestamp {
            format!("swap pulse {pulse_index} at frame {old_timestamp}")
        } else if old_buttons == new_buttons {
            format!("move pulse {pulse_index} from frame {old_timestamp} to {new_timestamp}")
        } else {
            format!(
                "move and swap pulse {pulse_index} from frame {old_timestamp} to {new_timestamp}"
            )
        }),
        intervention: Some(InterventionRange {
            start_frame: old_timestamp.min(new_timestamp),
            end_frame_exclusive: old_timestamp.max(new_timestamp) + 1,
            parent_end_frame_exclusive: old_timestamp.max(new_timestamp) + 1,
        }),
    };
    Ok(candidate)
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
    candidate.ancestry = Ancestry {
        generation,
        parent_id: Some(parent.candidate.id()?),
        mutation: Some(mutation.into()),
        intervention: Some(InterventionRange {
            start_frame: ranges
                .iter()
                .map(|(start, _)| *start as u64)
                .min()
                .unwrap_or(0),
            end_frame_exclusive: ranges.iter().map(|(_, end)| *end as u64).max().unwrap_or(0),
            parent_end_frame_exclusive: ranges
                .iter()
                .map(|(_, end)| *end as u64)
                .max()
                .unwrap_or(0),
        }),
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
        episode_store: None,
        results_path: root.join("results.json"),
        working_directory: config.working_directory.clone(),
        game_args_prefix: config.game_args_prefix.clone(),
        workers: config.workers,
        repetitions: config.repetitions,
        timeout: config.timeout,
        harness: config.harness.clone(),
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

#[cfg(test)]
#[path = "boot/tests.rs"]
mod tests;
