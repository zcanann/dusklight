//! Boot-specific minimization and timing golf over proved native candidates.

use super::*;

#[derive(Clone)]
pub(super) struct ProvenBootCandidate {
    pub(super) candidate: Candidate,
    pub(super) tape: InputTape,
    pub(super) sim_tick: u64,
    pub(super) tape_frame: u64,
    pub(super) boundary_fingerprint: BoundaryFingerprint,
}

#[derive(Clone)]
pub(super) struct BootReductionTarget {
    pub(super) sim_tick: u64,
    pub(super) tape_frame: u64,
    pub(super) boundary_fingerprint: BoundaryFingerprint,
}

impl BootReductionTarget {
    pub(super) fn accepts(&self, candidate: &ProvenBootCandidate) -> bool {
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
        harness: config.harness.clone(),
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
            let proven = evaluate_boot_batch(
                &evaluation,
                batch.to_vec(),
                &config
                    .output_root
                    .join("rounds")
                    .join(format!("{round:04}"))
                    .join(format!("batch-{batch_index:04}")),
                round,
            )?;
            for candidate in proven.into_iter().filter(|candidate| {
                candidate.boundary_fingerprint == source_fingerprint
                    && candidate.sim_tick <= current.sim_tick
                    && boot_golf_cmp(candidate, &current).is_lt()
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

fn alternate_menu_buttons(buttons: u16) -> Option<u16> {
    match buttons {
        BUTTON_A => Some(BUTTON_START),
        BUTTON_START => Some(BUTTON_A),
        _ => None,
    }
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
mod tests {
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
    fn target_rejects_later_or_different_proof() {
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

    #[test]
    fn shifted_boot_pulse_can_swap_between_a_and_start() {
        let mut candidate = Candidate::baseline(SegmentProfile::BootToFsp103);
        candidate.actions = vec![
            MacroAction::Neutral { frames: 3 },
            MacroAction::Press {
                buttons: vec![dusklight_search::search::ControllerButton::Start],
                hold_frames: 1,
                neutral_frames: 1,
            },
        ];
        let tape = candidate.compile().unwrap();
        let parent = ProvenBootCandidate {
            candidate,
            tape,
            sim_tick: 4,
            tape_frame: 4,
            boundary_fingerprint: BoundaryFingerprint {
                schema: "dusklight.milestone-boundary/v1".into(),
                algorithm: "xxh3-128".into(),
                canonical_encoding: "little-endian-fixed-v1".into(),
                digest: "a".repeat(32),
            },
        };

        let swapped = candidate_with_shifted_pulse(&parent, 0, 1, BUTTON_A, 1)
            .unwrap()
            .compile()
            .unwrap();
        assert_eq!(swapped.frames[1].pads[0].buttons, BUTTON_A);
        assert_eq!(swapped.frames[3].pads[0].buttons, 0);

        let in_place = candidate_with_shifted_pulse(&parent, 0, 3, BUTTON_A, 1)
            .unwrap()
            .compile()
            .unwrap();
        assert_eq!(in_place.frames[3].pads[0].buttons, BUTTON_A);
    }
}
