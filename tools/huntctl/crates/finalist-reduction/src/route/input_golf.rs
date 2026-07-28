use super::*;

pub(super) const BUTTON_A: u16 = 0x0100;
pub(super) const BUTTON_START: u16 = 0x1000;
const MENU_BUTTONS: u16 = BUTTON_A | BUTTON_START;

#[derive(Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct InputGolfQuality {
    pub(super) goal_sim_tick: u64,
    pub(super) pulse_count: usize,
    pub(super) frame_count: u64,
    pub(super) timestamp_sum: u64,
    pub(super) timestamps: Vec<u64>,
    pub(super) candidate_id: String,
}

pub(super) fn input_golf_quality(
    candidate: &ProvenRouteCandidate,
) -> Result<InputGolfQuality, EvaluateError> {
    let timestamps = button_pulse_timestamps(&candidate.tape)?;
    Ok(InputGolfQuality {
        goal_sim_tick: candidate.goal_sim_tick,
        pulse_count: timestamps.len(),
        frame_count: candidate.candidate.frame_count(),
        timestamp_sum: timestamps_sum(&timestamps)?,
        timestamps,
        candidate_id: candidate.candidate.id()?,
    })
}

pub(super) fn timestamps_sum(timestamps: &[u64]) -> Result<u64, EvaluateError> {
    timestamps.iter().try_fold(0_u64, |sum, timestamp| {
        sum.checked_add(*timestamp)
            .ok_or_else(|| EvaluateError::InvalidResult("pulse timestamp sum overflowed".into()))
    })
}

pub(super) fn button_pulse_timestamps(tape: &InputTape) -> Result<Vec<u64>, EvaluateError> {
    tape.frames
        .iter()
        .enumerate()
        .filter(|(_, frame)| is_isolated_button_pulse(&frame.pads[0]))
        .map(|(index, _)| {
            u64::try_from(index).map_err(|_| {
                EvaluateError::InvalidResult("pulse timestamp does not fit in u64".into())
            })
        })
        .collect()
}

fn is_isolated_button_pulse(pad: &RawPadState) -> bool {
    pad.buttons != 0 && pad.buttons & !MENU_BUTTONS == 0
}

pub(super) fn input_golf_proposals(
    parent: &ProvenRouteCandidate,
    generation: u32,
    budget: usize,
) -> Result<Vec<Candidate>, EvaluateError> {
    let timestamps = button_pulse_timestamps(&parent.tape)?;
    let mut proposals = Vec::new();
    let mut ids = BTreeSet::new();

    for (pulse_index, timestamp) in timestamps.iter().copied().enumerate() {
        if proposals.len() == budget {
            return Ok(proposals);
        }
        let mut tape = parent.tape.clone();
        tape.frames[timestamp as usize].pads[0].buttons &= !MENU_BUTTONS;
        push_input_golf_candidate(
            parent,
            tape,
            generation,
            format!("delete button pulse {pulse_index} at frame {timestamp}"),
            timestamp,
            timestamp + 1,
            &mut ids,
            &mut proposals,
        )?;
    }

    for (pulse_index, timestamp) in timestamps.iter().copied().enumerate() {
        if proposals.len() == budget {
            return Ok(proposals);
        }
        let mut tape = parent.tape.clone();
        let Some(alternate) = alternate_menu_pulse(tape.frames[timestamp as usize].pads[0]) else {
            continue;
        };
        tape.frames[timestamp as usize].pads[0] = alternate;
        push_input_golf_candidate(
            parent,
            tape,
            generation,
            format!("swap button pulse {pulse_index} at frame {timestamp}"),
            timestamp,
            timestamp + 1,
            &mut ids,
            &mut proposals,
        )?;
    }

    for pulse_index in (0..timestamps.len()).rev() {
        let old_timestamp = timestamps[pulse_index];
        let earliest = if pulse_index == 0 {
            0
        } else {
            timestamps[pulse_index - 1]
                .checked_add(1)
                .ok_or_else(|| EvaluateError::InvalidResult("pulse frame overflowed".into()))?
        };
        for new_timestamp in (earliest..old_timestamp).rev() {
            if proposals.len() == budget {
                return Ok(proposals);
            }
            let new_index = new_timestamp as usize;
            if parent.tape.frames[new_index].pads[0].buttons != 0 {
                continue;
            }
            let mut tape = parent.tape.clone();
            let pad = tape.frames[old_timestamp as usize].pads[0];
            tape.frames[old_timestamp as usize].pads[0].buttons &= !MENU_BUTTONS;
            tape.frames[new_index].pads[0].buttons = pad.buttons & MENU_BUTTONS;
            push_input_golf_candidate(
                parent,
                tape.clone(),
                generation,
                format!(
                    "move button pulse {pulse_index} from frame {old_timestamp} to {new_timestamp}"
                ),
                new_timestamp,
                old_timestamp + 1,
                &mut ids,
                &mut proposals,
            )?;
            if proposals.len() == budget {
                return Ok(proposals);
            }
            if let Some(alternate) = alternate_menu_pulse(pad) {
                tape.frames[new_index].pads[0].buttons = alternate.buttons;
                push_input_golf_candidate(
                    parent,
                    tape,
                    generation,
                    format!(
                        "move and swap button pulse {pulse_index} from frame {old_timestamp} to {new_timestamp}"
                    ),
                    new_timestamp,
                    old_timestamp + 1,
                    &mut ids,
                    &mut proposals,
                )?;
            }
        }
    }
    Ok(proposals)
}

fn alternate_menu_pulse(mut pad: RawPadState) -> Option<RawPadState> {
    pad.buttons = match pad.buttons {
        BUTTON_A => BUTTON_START,
        BUTTON_START => BUTTON_A,
        _ => return None,
    };
    Some(pad)
}

#[allow(clippy::too_many_arguments)]
fn push_input_golf_candidate(
    parent: &ProvenRouteCandidate,
    tape: InputTape,
    generation: u32,
    mutation: String,
    intervention_start: u64,
    intervention_end: u64,
    ids: &mut BTreeSet<String>,
    proposals: &mut Vec<Candidate>,
) -> Result<(), EvaluateError> {
    let mut candidate = Candidate::from_absolute_tape(parent.candidate.segment, &tape)?;
    candidate.ancestry = Ancestry {
        generation,
        parent_id: Some(parent.candidate.id()?),
        mutation: Some(mutation),
        intervention: Some(InterventionRange {
            start_frame: intervention_start,
            end_frame_exclusive: intervention_end,
            parent_end_frame_exclusive: intervention_end,
        }),
    };
    let id = candidate.id()?;
    if ids.insert(id) {
        proposals.push(candidate);
    }
    Ok(())
}
