use super::*;

/// Enumerate coherent local timing edits from the current champion before
/// spending the remaining population on random mutations. These edits are
/// deliberately represented as ordinary candidates: their authenticated
/// outcomes become training data for the learned proposer on the next
/// generation.
pub(super) fn coordinated_imported_timing_edits(
    parent: &Candidate,
    generation: u32,
) -> Result<Vec<Candidate>, SearchError> {
    if !parent
        .actions
        .iter()
        .any(|action| matches!(action, MacroAction::PadRun { .. }))
    {
        return Ok(Vec::new());
    }
    let parent_id = parent.id()?;
    let parent_tape = parent.compile()?;
    let mut edits = Vec::new();
    let mut pulses = Vec::new();
    let mut frame = 0;
    while frame < parent_tape.frames.len() {
        if parent_tape.frames[frame].pads[0].buttons & BUTTON_A == 0 {
            frame += 1;
            continue;
        }
        let start = frame;
        while frame < parent_tape.frames.len()
            && parent_tape.frames[frame].pads[0].buttons & BUTTON_A != 0
        {
            frame += 1;
        }
        pulses.push((start, frame));
    }
    if !pulses.is_empty() {
        let rotation = generation as usize % pulses.len();
        pulses.rotate_left(rotation);
    }
    for (start, end) in pulses {
        for delta in [-1_isize, 1] {
            let shifted_start = start as isize + delta;
            let shifted_end = end as isize + delta;
            if shifted_start < 0 || shifted_end > parent_tape.frames.len() as isize {
                continue;
            }
            let shifted_start = shifted_start as usize;
            let shifted_end = shifted_end as usize;
            let mut tape = parent_tape.clone();
            for input in &mut tape.frames[start..end] {
                input.pads[0].buttons &= !BUTTON_A;
            }
            for input in &mut tape.frames[shifted_start..shifted_end] {
                input.pads[0].buttons |= BUTTON_A;
            }
            let direction = if delta < 0 { "earlier" } else { "later" };
            edits.push(candidate_from_imported_tape_edit(
                parent,
                &parent_id,
                &parent_tape,
                tape,
                generation,
                format!("pad_shift_a_pulse[{start}..{end}]_{direction}"),
            )?);
        }
    }

    let mut deletion_frames = (0..parent_tape.frames.len()).collect::<Vec<_>>();
    if !deletion_frames.is_empty() {
        let rotation = (generation as usize * 17) % deletion_frames.len();
        deletion_frames.rotate_left(rotation);
    }
    deletion_frames.sort_by_key(|index| {
        let redundant = *index > 0 && parent_tape.frames[*index] == parent_tape.frames[*index - 1];
        !redundant
    });
    for frame in deletion_frames {
        if parent_tape.frames.len() <= 1 {
            break;
        }
        let mut tape = parent_tape.clone();
        tape.frames.remove(frame);
        edits.push(candidate_from_imported_tape_edit(
            parent,
            &parent_id,
            &parent_tape,
            tape,
            generation,
            format!("pad_delete_frame[{frame}]"),
        )?);
    }
    Ok(edits)
}

pub(super) fn candidate_from_imported_tape_edit(
    parent: &Candidate,
    parent_id: &str,
    parent_tape: &InputTape,
    tape: InputTape,
    generation: u32,
    description: String,
) -> Result<Candidate, SearchError> {
    let mut child = Candidate::from_absolute_tape(parent.segment, &tape)?;
    let child_tape = child.compile()?;
    child.ancestry = Ancestry {
        generation,
        parent_id: Some(parent_id.to_owned()),
        mutation: Some(description),
        intervention: intervention_range(parent_tape, &child_tape),
    };
    child.validate()?;
    Ok(child)
}

pub(super) fn validate_pair(
    manifest: &PopulationManifest,
    results: &SearchResults,
) -> Result<(), SearchError> {
    validate_population_schema(manifest)?;
    validate_results_schema(results)?;
    if manifest.segment != results.segment {
        return Err(SearchError::SegmentMismatch);
    }
    if manifest.boot != results.boot {
        return Err(SearchError::BootMismatch);
    }
    for result in results.candidates.values() {
        result.validate_for_segment(manifest.segment, results.schema == RESULTS_SCHEMA)?;
    }
    Ok(())
}

pub(super) fn write_population(
    output: &Path,
    segment: SegmentProfile,
    generation: u32,
    rng_seed: u64,
    candidates: Vec<Candidate>,
) -> Result<PopulationManifest, SearchError> {
    let boot = candidates
        .first()
        .ok_or(SearchError::InvalidPopulation)?
        .boot
        .clone();
    fs::create_dir_all(output)?;
    let mut members = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if candidate.segment != segment {
            return Err(SearchError::SegmentMismatch);
        }
        if candidate.boot != boot {
            return Err(SearchError::BootMismatch);
        }
        let tape = candidate.compile()?;
        let input_complexity = tape_input_complexity(&tape);
        let id = candidate.id()?;
        let candidate_file = PathBuf::from(format!("{id}.candidate.json"));
        let tape_file = PathBuf::from(format!("{id}.tape"));
        fs::write(
            output.join(&candidate_file),
            serde_json::to_vec_pretty(&candidate)?,
        )?;
        fs::write(output.join(&tape_file), tape.encode()?)?;
        members.push(PopulationMember {
            candidate_id: id,
            candidate_file,
            tape_file,
            frame_count: candidate.frame_count(),
            input_complexity: Some(input_complexity),
            ancestry: candidate.ancestry,
        });
    }
    let manifest = PopulationManifest {
        schema: POPULATION_SCHEMA.into(),
        segment,
        boot,
        generation,
        rng_seed,
        members,
    };
    fs::write(
        output.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(manifest)
}

/// Representation-independent input complexity over the compiled absolute
/// tape. Button bits count independently; every other native PAD field,
/// ownership bit, wait kind, and wait timeout contributes one when it changes.
pub fn tape_input_complexity(tape: &InputTape) -> u64 {
    let mut complexity = 0_u64;
    let mut previous = InputFrame::default();
    for frame in &tape.frames {
        complexity += u64::from((frame.owned_ports ^ previous.owned_ports).count_ones());
        complexity += u64::from(frame.wait_condition != previous.wait_condition);
        complexity += u64::from(frame.wait_timeout_ticks != previous.wait_timeout_ticks);
        for (pad, prior) in frame.pads.iter().zip(&previous.pads) {
            complexity += u64::from((pad.buttons ^ prior.buttons).count_ones());
            complexity += u64::from(pad.stick_x != prior.stick_x);
            complexity += u64::from(pad.stick_y != prior.stick_y);
            complexity += u64::from(pad.substick_x != prior.substick_x);
            complexity += u64::from(pad.substick_y != prior.substick_y);
            complexity += u64::from(pad.trigger_left != prior.trigger_left);
            complexity += u64::from(pad.trigger_right != prior.trigger_right);
            complexity += u64::from(pad.analog_a != prior.analog_a);
            complexity += u64::from(pad.analog_b != prior.analog_b);
            complexity += u64::from(pad.connected != prior.connected);
            complexity += u64::from(pad.error != prior.error);
        }
        previous = frame.clone();
    }
    complexity
}

pub(super) fn validate_population_schema(manifest: &PopulationManifest) -> Result<(), SearchError> {
    if manifest.schema == POPULATION_SCHEMA {
        if manifest
            .members
            .iter()
            .any(|member| member.input_complexity.is_none())
        {
            return Err(SearchError::InvalidPopulation);
        }
        return Ok(());
    }
    if manifest.schema == LEGACY_POPULATION_SCHEMA_V2
        || (manifest.schema == LEGACY_POPULATION_SCHEMA_V1 && manifest.boot == TapeBoot::Process)
    {
        return Ok(());
    }
    Err(SearchError::InvalidSchema(manifest.schema.clone()))
}

pub(super) fn validate_results_schema(results: &SearchResults) -> Result<(), SearchError> {
    if results.schema == RESULTS_SCHEMA
        || results.schema == LEGACY_RESULTS_SCHEMA_V2
        || (results.schema == LEGACY_RESULTS_SCHEMA_V1 && results.boot == TapeBoot::Process)
    {
        Ok(())
    } else {
        Err(SearchError::InvalidSchema(results.schema.clone()))
    }
}

pub(super) fn mutate(
    parent: &Candidate,
    generation: u32,
    rng: &mut SplitMix64,
) -> Result<Candidate, SearchError> {
    let mut child = parent.clone();
    let parent_id = parent.id()?;
    let parent_tape = parent.compile()?;
    let route = matches!(
        child.segment,
        SegmentProfile::Fsp103ToFsp104 | SegmentProfile::LinkControlToTunnelCrawlStart
    );
    let imported = child
        .actions
        .iter()
        .any(|action| matches!(action, MacroAction::PadRun { .. }));
    let mutation_kind = if imported {
        [0, 4, 6, 7, 8, 9, 10, 11, 12, 13][rng.usize(10)]
    } else if route {
        rng.usize(7)
    } else {
        rng.usize(3)
    };
    let description;
    match mutation_kind {
        0 => {
            let index = rng.usize(child.actions.len());
            let delta_limit = (24_i32 - i32::try_from(generation.min(18)).unwrap()).max(3);
            let delta = rng.signed(delta_limit);
            change_duration(&mut child.actions[index], delta);
            description = format!("duration[{index}]{delta:+}");
        }
        1 if !route => {
            let presses: Vec<_> = child
                .actions
                .iter()
                .enumerate()
                .filter(|(_, action)| matches!(action, MacroAction::Press { .. }))
                .map(|(index, _)| index)
                .collect();
            if presses.is_empty() {
                return Err(SearchError::PopulationStalled);
            }
            let index = presses[rng.usize(presses.len())];
            let delta_limit = (32_i32 - i32::try_from(generation.min(24)).unwrap()).max(8);
            let delta = rng.signed(delta_limit);
            change_duration(&mut child.actions[index], delta);
            description = format!("boot_gap[{index}]{delta:+}");
        }
        1 if route => {
            let movable: Vec<_> = child
                .actions
                .iter()
                .enumerate()
                .filter(|(_, action)| {
                    matches!(action, MacroAction::Move { .. } | MacroAction::Roll { .. })
                })
                .map(|(index, _)| index)
                .collect();
            if movable.is_empty() {
                return Err(SearchError::PopulationStalled);
            }
            let index = movable[rng.usize(movable.len())];
            let delta_limit = (12_i32 - i32::try_from(generation.min(9)).unwrap()).max(2);
            let delta = rng.signed(delta_limit);
            change_angle(&mut child.actions[index], delta);
            description = format!("angle[{index}]{delta:+}");
        }
        2 if !route => {
            let shrinkable: Vec<_> = child
                .actions
                .iter()
                .enumerate()
                .filter_map(|(index, action)| match action {
                    MacroAction::Press { neutral_frames, .. } if *neutral_frames > 0 => Some(index),
                    _ => None,
                })
                .collect();
            if shrinkable.is_empty() {
                return Err(SearchError::PopulationStalled);
            }
            let index = shrinkable[rng.usize(shrinkable.len())];
            let delta = -i32::try_from(1 + rng.usize(48)).unwrap();
            change_duration(&mut child.actions[index], delta);
            description = format!("boot_shrink[{index}]{delta:+}");
        }
        2 if route => {
            let movable: Vec<_> = child
                .actions
                .iter()
                .enumerate()
                .filter(|(_, action)| {
                    matches!(action, MacroAction::Move { .. } | MacroAction::Roll { .. })
                })
                .map(|(index, _)| index)
                .collect();
            let index = movable[rng.usize(movable.len())];
            let delta = rng.signed(12);
            change_magnitude(&mut child.actions[index], delta);
            description = format!("magnitude[{index}]{delta:+}");
        }
        3 if route => {
            let moves: Vec<_> = child
                .actions
                .iter()
                .enumerate()
                .filter_map(|(index, action)| match action {
                    MacroAction::Move {
                        angle_degrees,
                        magnitude,
                        ..
                    } => Some((index, *angle_degrees, *magnitude)),
                    _ => None,
                })
                .collect();
            let (index, angle_degrees, magnitude) = moves[rng.usize(moves.len())];
            child.actions.insert(
                index,
                MacroAction::Roll {
                    angle_degrees,
                    magnitude,
                    button_frame: 0,
                    recovery_frames: 8 + rng.usize(10) as u32,
                    spacing: RollSpacing::default(),
                },
            );
            description = format!("insert_roll[{index}]");
        }
        4 if route && child.actions.len() > 1 => {
            let index = rng.usize(child.actions.len());
            child.actions.remove(index);
            description = format!("delete[{index}]");
        }
        5 if route => {
            let moves: Vec<_> = child
                .actions
                .iter()
                .enumerate()
                .filter_map(|(index, action)| match action {
                    MacroAction::Move {
                        angle_degrees,
                        magnitude,
                        frames,
                    } if *frames >= 4 => Some((index, *angle_degrees, *magnitude, *frames)),
                    _ => None,
                })
                .collect();
            if moves.is_empty() {
                return mutate(parent, generation, rng);
            }
            let (index, angle, magnitude, frames) = moves[rng.usize(moves.len())];
            let first = frames / 2;
            child.actions[index] = MacroAction::Move {
                angle_degrees: angle,
                magnitude,
                frames: first,
            };
            child.actions.insert(
                index + 1,
                MacroAction::Move {
                    angle_degrees: normalize_angle(i32::from(angle) + rng.signed(8)),
                    magnitude,
                    frames: frames - first,
                },
            );
            description = format!("split_move[{index}]");
        }
        7 if imported => {
            let pads: Vec<_> = child
                .actions
                .iter()
                .enumerate()
                .filter_map(|(index, action)| match action {
                    MacroAction::PadRun { pad, .. } if pad.connected && pad.error == 0 => {
                        Some(index)
                    }
                    _ => None,
                })
                .collect();
            let index = pads[rng.usize(pads.len())];
            let mut delta_x = rng.signed(9);
            let delta_y = rng.signed(9);
            if delta_x == 0 && delta_y == 0 {
                delta_x = 1;
            }
            let MacroAction::PadRun { pad, .. } = &mut child.actions[index] else {
                unreachable!("pad-run index was selected above")
            };
            pad.stick_x = (i32::from(pad.stick_x) + delta_x).clamp(-127, 127) as i8;
            pad.stick_y = (i32::from(pad.stick_y) + delta_y).clamp(-127, 127) as i8;
            description = format!("pad_stick[{index}]({delta_x:+},{delta_y:+})");
        }
        8 | 9 if imported => {
            let pads: Vec<_> = child
                .actions
                .iter()
                .enumerate()
                .filter_map(|(index, action)| match action {
                    MacroAction::PadRun { pad, .. } if pad.connected && pad.error == 0 => {
                        Some(index)
                    }
                    _ => None,
                })
                .collect();
            let index = pads[rng.usize(pads.len())];
            let MacroAction::PadRun { pad, .. } = &mut child.actions[index] else {
                unreachable!("pad-run index was selected above")
            };
            let (button, name) = if mutation_kind == 8 {
                (BUTTON_A, "a")
            } else {
                (BUTTON_B, "b")
            };
            pad.buttons ^= button;
            description = format!("pad_toggle_{name}[{index}]");
        }
        10 if imported => {
            let mut tape = parent_tape.clone();
            let widths = [4_usize, 8, 16, 32, 64];
            let width = widths[rng.usize(widths.len())].min(tape.frames.len());
            let start = rng.usize(tape.frames.len() - width + 1);
            let end = start + width;
            let (sum_x, sum_y) =
                tape.frames[start..end]
                    .iter()
                    .fold((0_i64, 0_i64), |(x, y), frame| {
                        (
                            x + i64::from(frame.pads[0].stick_x),
                            y + i64::from(frame.pads[0].stick_y),
                        )
                    });
            let length = ((sum_x * sum_x + sum_y * sum_y) as f64).sqrt();
            let (stick_x, stick_y) = if length < f64::EPSILON {
                (0, 0)
            } else {
                (
                    (sum_x as f64 * 127.0 / length).round().clamp(-127.0, 127.0) as i8,
                    (sum_y as f64 * 127.0 / length).round().clamp(-127.0, 127.0) as i8,
                )
            };
            for frame in &mut tape.frames[start..end] {
                frame.pads[0].stick_x = stick_x;
                frame.pads[0].stick_y = stick_y;
            }
            child = Candidate::from_absolute_tape(parent.segment, &tape)?;
            description = format!("pad_smooth[{start}..{end}]=({stick_x:+},{stick_y:+})");
        }
        11 if imported => {
            let mut tape = parent_tape.clone();
            let period = 16 + rng.usize(9);
            let phase = rng.usize(period);
            let hold = 1 + rng.usize(4);
            for (frame, input) in tape.frames.iter_mut().enumerate() {
                input.pads[0].buttons &= !BUTTON_A;
                if (frame + period - phase) % period < hold {
                    input.pads[0].buttons |= BUTTON_A;
                }
            }
            child = Candidate::from_absolute_tape(parent.segment, &tape)?;
            description = format!("pad_roll_cadence[period={period},phase={phase},hold={hold}]");
        }
        12 if imported && parent_tape.frames.len() > 1 => {
            let mut tape = parent_tape.clone();
            let frame = rng.usize(tape.frames.len());
            tape.frames.remove(frame);
            child = Candidate::from_absolute_tape(parent.segment, &tape)?;
            description = format!("pad_delete_frame[{frame}]");
        }
        13 if imported => {
            let mut tape = parent_tape.clone();
            let rising_edges = (0..tape.frames.len())
                .filter(|frame| {
                    tape.frames[*frame].pads[0].buttons & BUTTON_A != 0
                        && (*frame == 0 || tape.frames[*frame - 1].pads[0].buttons & BUTTON_A == 0)
                })
                .collect::<Vec<_>>();
            if rising_edges.is_empty() {
                let frame = rng.usize(tape.frames.len());
                tape.frames[frame].pads[0].buttons ^= BUTTON_A;
                child = Candidate::from_absolute_tape(parent.segment, &tape)?;
                description = format!("pad_toggle_a[{frame}]");
            } else {
                let source = rising_edges[rng.usize(rising_edges.len())];
                let end = (source..tape.frames.len())
                    .find(|frame| tape.frames[*frame].pads[0].buttons & BUTTON_A == 0)
                    .unwrap_or(tape.frames.len());
                if source == 0 && end == tape.frames.len() {
                    let frame = rng.usize(tape.frames.len());
                    tape.frames[frame].pads[0].buttons &= !BUTTON_A;
                    child = Candidate::from_absolute_tape(parent.segment, &tape)?;
                    description = format!("pad_toggle_a[{frame}]");
                } else {
                    let earlier = if source == 0 {
                        false
                    } else if end == tape.frames.len() {
                        true
                    } else {
                        rng.usize(2) == 0
                    };
                    let (target, target_end) = if earlier {
                        (source - 1, end - 1)
                    } else {
                        (source + 1, end + 1)
                    };
                    for input in &mut tape.frames[source..end] {
                        input.pads[0].buttons &= !BUTTON_A;
                    }
                    for input in &mut tape.frames[target..target_end] {
                        input.pads[0].buttons |= BUTTON_A;
                    }
                    child = Candidate::from_absolute_tape(parent.segment, &tape)?;
                    description =
                        format!("pad_shift_a_pulse[{source}..{end}->{target}..{target_end}]");
                }
            }
        }
        _ => {
            let neutral: Vec<_> = child
                .actions
                .iter()
                .enumerate()
                .filter(|(_, action)| matches!(action, MacroAction::Neutral { .. }))
                .map(|(index, _)| index)
                .collect();
            let index = if neutral.is_empty() {
                rng.usize(child.actions.len())
            } else {
                neutral[rng.usize(neutral.len())]
            };
            let delta = rng.signed(48);
            change_duration(&mut child.actions[index], delta);
            description = format!("timing[{index}]{delta:+}");
        }
    }
    let child_tape = child.compile()?;
    child.ancestry = Ancestry {
        generation,
        parent_id: Some(parent_id),
        mutation: Some(description),
        intervention: intervention_range(&parent_tape, &child_tape),
    };
    child.validate()?;
    Ok(child)
}

pub(super) fn intervention_range(
    parent: &InputTape,
    child: &InputTape,
) -> Option<InterventionRange> {
    let shared_limit = parent.frames.len().min(child.frames.len());
    let start = parent
        .frames
        .iter()
        .zip(&child.frames)
        .position(|(left, right)| left != right)
        .unwrap_or(shared_limit);
    let maximum_suffix = shared_limit.saturating_sub(start);
    let suffix = parent
        .frames
        .iter()
        .rev()
        .zip(child.frames.iter().rev())
        .take(maximum_suffix)
        .take_while(|(left, right)| left == right)
        .count();
    let parent_end = parent.frames.len().saturating_sub(suffix);
    let child_end = child.frames.len().saturating_sub(suffix);
    (parent_end > start || child_end > start).then_some(InterventionRange {
        start_frame: start as u64,
        end_frame_exclusive: child_end as u64,
        parent_end_frame_exclusive: parent_end as u64,
    })
}

pub(super) fn change_duration(action: &mut MacroAction, delta: i32) {
    pub(super) fn adjusted(value: u32, delta: i32, minimum: u32) -> u32 {
        (i64::from(value) + i64::from(delta)).clamp(i64::from(minimum), 10_000) as u32
    }
    match action {
        MacroAction::Move { frames, .. }
        | MacroAction::Neutral { frames }
        | MacroAction::PadRun { frames, .. } => *frames = adjusted(*frames, delta, 1),
        MacroAction::Roll {
            recovery_frames, ..
        } => *recovery_frames = adjusted(*recovery_frames, delta, 0).min(300),
        MacroAction::Press { neutral_frames, .. } => {
            *neutral_frames = adjusted(*neutral_frames, delta, 0)
        }
        MacroAction::GameTactic { .. }
        | MacroAction::MotionPath { .. }
        | MacroAction::PortOneMotionPath { .. } => {}
    }
}

pub(super) fn change_angle(action: &mut MacroAction, delta: i32) {
    match action {
        MacroAction::Move { angle_degrees, .. } | MacroAction::Roll { angle_degrees, .. } => {
            *angle_degrees = normalize_angle(i32::from(*angle_degrees) + delta)
        }
        _ => {}
    }
}

pub(super) fn change_magnitude(action: &mut MacroAction, delta: i32) {
    match action {
        MacroAction::Move { magnitude, .. } | MacroAction::Roll { magnitude, .. } => {
            *magnitude = (i32::from(*magnitude) + delta).clamp(1, 127) as u8
        }
        _ => {}
    }
}

pub(super) fn normalize_angle(angle: i32) -> i16 {
    (angle + 180).rem_euclid(360) as i16 - 180
}

pub(super) fn boot_baseline() -> Vec<MacroAction> {
    let neutral = |frames| MacroAction::Neutral { frames };
    let press = |button, neutral_frames| MacroAction::Press {
        buttons: vec![button],
        hold_frames: 1,
        neutral_frames,
    };
    vec![
        neutral(115),
        press(ControllerButton::Start, 31),
        press(ControllerButton::Start, 252),
        press(ControllerButton::A, 7),
        press(ControllerButton::Start, 29),
        press(ControllerButton::Start, 1),
        press(ControllerButton::Start, 67),
        press(ControllerButton::Start, 1),
        press(ControllerButton::Start, 65),
        press(ControllerButton::Start, 1),
        press(ControllerButton::Start, 115),
    ]
}

pub(super) fn route_baseline() -> Vec<MacroAction> {
    vec![
        // Direct-stage processes still need time to construct the room and
        // player. This is deliberately part of the candidate so evolution can
        // safely drive it downward and the resulting tape remains standalone.
        MacroAction::Neutral { frames: 180 },
        MacroAction::Move {
            angle_degrees: 0,
            magnitude: 127,
            frames: 45,
        },
        MacroAction::Move {
            angle_degrees: 90,
            magnitude: 127,
            frames: 14,
        },
        MacroAction::Move {
            angle_degrees: 0,
            magnitude: 127,
            frames: 20,
        },
        MacroAction::Move {
            angle_degrees: 90,
            magnitude: 127,
            frames: 10,
        },
        MacroAction::Move {
            angle_degrees: 0,
            magnitude: 127,
            frames: 316,
        },
        MacroAction::Neutral { frames: 80 },
    ]
}

/// Suffix-only seed for the authored link-control -> tunnel-crawl objective.
/// Unlike the legacy direct-stage profile, construction time is supplied by
/// the immutable clean-boot prefix and must not be hidden in the candidate.
pub(super) fn tunnel_crawl_baseline() -> Vec<MacroAction> {
    // This profile is anchored to an observed suffix and intentionally has no
    // synthetic baseline. Import the promoted absolute tape losslessly with
    // Candidate::from_absolute_tape; validation rejects this empty placeholder
    // if a generic caller attempts to seed without doing so.
    Vec::new()
}

pub(super) fn validate_magnitude(magnitude: u8) -> Result<(), SearchError> {
    if magnitude <= 127 {
        Ok(())
    } else {
        Err(SearchError::InvalidMagnitude(magnitude))
    }
}

pub(super) fn validate_duration(frames: u32) -> Result<(), SearchError> {
    if (1..=10_000).contains(&frames) {
        Ok(())
    } else {
        Err(SearchError::InvalidDuration(frames))
    }
}

pub(super) fn owned_frame(pad: RawPadState) -> InputFrame {
    let mut frame = InputFrame {
        owned_ports: 0x0f,
        ..InputFrame::default()
    };
    frame.pads[0] = pad;
    frame
}

pub(super) fn imported_frame(pad: RawPadState) -> InputFrame {
    let disconnected = RawPadState {
        connected: false,
        error: -1,
        ..RawPadState::default()
    };
    let mut frame = InputFrame {
        owned_ports: 0x01,
        pads: [disconnected; 4],
        ..InputFrame::default()
    };
    frame.pads[0] = pad;
    frame
}

pub(super) fn move_frame(angle_degrees: i16, magnitude: u8, buttons: u16) -> InputFrame {
    let radians = f64::from(angle_degrees).to_radians();
    let magnitude = f64::from(magnitude);
    let pad = RawPadState {
        buttons,
        stick_x: (radians.sin() * magnitude).round().clamp(-127.0, 127.0) as i8,
        stick_y: (radians.cos() * magnitude).round().clamp(-127.0, 127.0) as i8,
        ..RawPadState::default()
    };
    owned_frame(pad)
}

pub(super) fn push_frames(output: &mut Vec<InputFrame>, frame: InputFrame, count: u32) {
    output.extend(std::iter::repeat_n(frame, count as usize));
}

#[derive(Clone, Copy, Debug)]
pub(super) struct SplitMix64(u64);

impl SplitMix64 {
    pub(super) fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub(super) fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
        value ^ (value >> 31)
    }

    pub(super) fn usize(&mut self, exclusive_max: usize) -> usize {
        debug_assert!(exclusive_max > 0);
        (self.next() % exclusive_max as u64) as usize
    }

    pub(super) fn signed(&mut self, inclusive_magnitude: i32) -> i32 {
        let width = u64::try_from(inclusive_magnitude * 2 + 1).unwrap();
        (self.next() % width) as i32 - inclusive_magnitude
    }
}
