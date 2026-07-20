//! Pure, bounded proposal generation for the native checkpoint suffix loop.

use crate::search::{Candidate, MacroAction, SearchError, SegmentProfile};
use crate::tape::{InputFrame, InputTape};
use serde::Serialize;
use std::collections::HashSet;

pub const NATIVE_SUFFIX_BATCH_SCHEMA: &str = "dusklight-suffix-batch/v1";
const MAXIMUM_CANDIDATES: usize = 16_384;
const MAXIMUM_TICKS: usize = 4_096;
const MAXIMUM_EXPANDED_TICKS: usize = 8 * 1_024 * 1_024;
const BUTTON_A: u16 = 0x0100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SuffixProposalMethod {
    Deletion,
    ButtonEdge,
    Heading,
}

#[derive(Debug, Serialize)]
pub struct NativeSuffixBatch {
    pub schema: &'static str,
    pub source_frame: usize,
    pub maximum_ticks: usize,
    pub verify_state_hashes: bool,
    pub candidates: Vec<NativeSuffixCandidate>,
}

#[derive(Debug, Serialize)]
pub struct NativeSuffixCandidate {
    pub id: String,
    pub actions: Vec<MacroAction>,
}

pub fn propose_suffix_batch(
    seed: &Candidate,
    source_frame: usize,
    maximum_ticks: usize,
    candidate_budget: usize,
    method: SuffixProposalMethod,
) -> Result<NativeSuffixBatch, SearchError> {
    seed.validate()?;
    if seed.segment != SegmentProfile::Fsp103ToFsp104
        || maximum_ticks == 0
        || maximum_ticks > MAXIMUM_TICKS
        || candidate_budget == 0
        || candidate_budget > MAXIMUM_CANDIDATES
        || candidate_budget.saturating_mul(maximum_ticks) > MAXIMUM_EXPANDED_TICKS
    {
        return Err(SearchError::InvalidPopulation);
    }
    let source = seed.compile()?;
    if source.frames.len() < maximum_ticks {
        return Err(SearchError::NonCanonicalTape(format!(
            "seed has {} frames but suffix batch requires {maximum_ticks}",
            source.frames.len()
        )));
    }

    let mut output = NativeSuffixBatch {
        schema: NATIVE_SUFFIX_BATCH_SCHEMA,
        source_frame,
        maximum_ticks,
        verify_state_hashes: false,
        candidates: Vec::with_capacity(candidate_budget),
    };
    let mut seen = HashSet::new();
    match method {
        SuffixProposalMethod::Deletion => {
            if source.frames.len() <= maximum_ticks {
                return Err(SearchError::NonCanonicalTape(
                    "deletion proposals require a seed longer than maximum_ticks".into(),
                ));
            }
            for deleted in 0..source.frames.len() {
                let mut frames = source.frames.clone();
                frames.remove(deleted);
                frames.truncate(maximum_ticks);
                push_candidate(
                    &mut output,
                    &mut seen,
                    seed,
                    frames,
                    format!("delete-{deleted}"),
                    candidate_budget,
                )?;
                if output.candidates.len() == candidate_budget {
                    break;
                }
            }
        }
        SuffixProposalMethod::ButtonEdge => {
            let base = source.frames[..maximum_ticks].to_vec();
            let edges = a_button_runs(&base);
            for &(start, end) in &edges {
                for delta in [-4_isize, -3, -2, -1, 1, 2, 3, 4] {
                    let Some(frames) = shift_a_run(&base, start, end, delta) else {
                        continue;
                    };
                    push_candidate(
                        &mut output,
                        &mut seen,
                        seed,
                        frames,
                        format!("button-edge-{start}-{delta:+}"),
                        candidate_budget,
                    )?;
                    if output.candidates.len() == candidate_budget {
                        break;
                    }
                }
                if output.candidates.len() == candidate_budget {
                    break;
                }
            }
            'pairs: for left in 0..edges.len() {
                for right in (left + 1)..edges.len() {
                    for left_delta in [-1_isize, 1] {
                        for right_delta in [-1_isize, 1] {
                            let (left_start, left_end) = edges[left];
                            let Some(frames) = shift_a_run(&base, left_start, left_end, left_delta)
                            else {
                                continue;
                            };
                            let (right_start, right_end) = edges[right];
                            let Some(frames) =
                                shift_a_run(&frames, right_start, right_end, right_delta)
                            else {
                                continue;
                            };
                            push_candidate(
                                &mut output,
                                &mut seen,
                                seed,
                                frames,
                                format!(
                                    "button-pair-{left_start}-{left_delta:+}-{right_start}-{right_delta:+}"
                                ),
                                candidate_budget,
                            )?;
                            if output.candidates.len() == candidate_budget {
                                break 'pairs;
                            }
                        }
                    }
                }
            }
        }
        SuffixProposalMethod::Heading => {
            let base = source.frames[..maximum_ticks].to_vec();
            // A coprime walk spreads an equal proposal budget across the route
            // instead of spending it all on the first corner.
            for ordinal in 0..maximum_ticks {
                let start = ordinal.wrapping_mul(37) % maximum_ticks;
                for window in [1_usize, 2, 4, 8] {
                    for degrees in [-6.0_f64, -3.0, 3.0, 6.0] {
                        let mut frames = base.clone();
                        rotate_heading(&mut frames, start, window, degrees);
                        push_candidate(
                            &mut output,
                            &mut seen,
                            seed,
                            frames,
                            format!("heading-{start}-w{window}-{degrees:+.0}"),
                            candidate_budget,
                        )?;
                        if output.candidates.len() == candidate_budget {
                            break;
                        }
                    }
                    if output.candidates.len() == candidate_budget {
                        break;
                    }
                }
                if output.candidates.len() == candidate_budget {
                    break;
                }
            }
        }
    }
    if output.candidates.is_empty() {
        return Err(SearchError::PopulationStalled);
    }
    Ok(output)
}

fn push_candidate(
    output: &mut NativeSuffixBatch,
    seen: &mut HashSet<Vec<u8>>,
    seed: &Candidate,
    frames: Vec<InputFrame>,
    id: String,
    budget: usize,
) -> Result<(), SearchError> {
    if output.candidates.len() >= budget || frames.len() != output.maximum_ticks {
        return Ok(());
    }
    let tape = InputTape {
        boot: seed.boot.clone(),
        tick_rate_numerator: 30,
        tick_rate_denominator: 1,
        frames,
    };
    let actions = Candidate::from_absolute_tape(seed.segment, &tape)?.actions;
    let identity = serde_json::to_vec(&actions)?;
    if seen.insert(identity) {
        output
            .candidates
            .push(NativeSuffixCandidate { id, actions });
    }
    Ok(())
}

fn a_button_runs(frames: &[InputFrame]) -> Vec<(usize, usize)> {
    let mut runs = Vec::new();
    let mut index = 0;
    while index < frames.len() {
        if frames[index].pads[0].buttons & BUTTON_A == 0 {
            index += 1;
            continue;
        }
        let start = index;
        while index < frames.len() && frames[index].pads[0].buttons & BUTTON_A != 0 {
            index += 1;
        }
        runs.push((start, index));
    }
    runs
}

fn shift_a_run(
    base: &[InputFrame],
    start: usize,
    end: usize,
    delta: isize,
) -> Option<Vec<InputFrame>> {
    let shifted_start = start.checked_add_signed(delta)?;
    let shifted_end = end.checked_add_signed(delta)?;
    if shifted_end > base.len() {
        return None;
    }
    let mut frames = base.to_vec();
    for frame in &mut frames[start..end] {
        frame.pads[0].buttons &= !BUTTON_A;
    }
    for frame in &mut frames[shifted_start..shifted_end] {
        frame.pads[0].buttons |= BUTTON_A;
    }
    Some(frames)
}

fn rotate_heading(frames: &mut [InputFrame], start: usize, window: usize, degrees: f64) {
    let radians = degrees.to_radians();
    let (sin, cos) = radians.sin_cos();
    let end = start.saturating_add(window).min(frames.len());
    for frame in &mut frames[start..end] {
        let x = f64::from(frame.pads[0].stick_x);
        let y = f64::from(frame.pads[0].stick_y);
        if x == 0.0 && y == 0.0 {
            continue;
        }
        frame.pads[0].stick_x = (x * cos + y * sin).round().clamp(-127.0, 127.0) as i8;
        frame.pads[0].stick_y = (y * cos - x * sin).round().clamp(-127.0, 127.0) as i8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::CANDIDATE_SCHEMA;
    use crate::search::{Ancestry, SearchPadState};
    use crate::tape::TapeBoot;

    fn seed() -> Candidate {
        let pad = SearchPadState {
            buttons: 0,
            stick_x: 100,
            stick_y: 50,
            substick_x: 0,
            substick_y: 0,
            trigger_left: 0,
            trigger_right: 0,
            analog_a: 0,
            analog_b: 0,
            connected: true,
            error: 0,
        };
        Candidate {
            schema: CANDIDATE_SCHEMA.into(),
            segment: SegmentProfile::Fsp103ToFsp104,
            boot: TapeBoot::Process,
            actions: vec![MacroAction::PadRun { pad, frames: 5 }],
            ancestry: Ancestry::default(),
        }
    }

    #[test]
    fn deletion_batch_is_exactly_bounded_and_lossless() {
        let batch =
            propose_suffix_batch(&seed(), 440, 4, 5, SuffixProposalMethod::Deletion).unwrap();
        assert_eq!(batch.candidates.len(), 1);
        let candidate = Candidate {
            schema: CANDIDATE_SCHEMA.into(),
            segment: SegmentProfile::Fsp103ToFsp104,
            boot: TapeBoot::Process,
            actions: batch.candidates[0].actions.clone(),
            ancestry: Ancestry::default(),
        };
        assert_eq!(candidate.compile().unwrap().frames.len(), 4);
    }

    #[test]
    fn heading_budget_is_filled_with_unique_exact_candidates() {
        let batch =
            propose_suffix_batch(&seed(), 440, 5, 8, SuffixProposalMethod::Heading).unwrap();
        assert_eq!(batch.candidates.len(), 8);
        assert!(batch.candidates.iter().all(|candidate| {
            Candidate {
                schema: CANDIDATE_SCHEMA.into(),
                segment: SegmentProfile::Fsp103ToFsp104,
                boot: TapeBoot::Process,
                actions: candidate.actions.clone(),
                ancestry: Ancestry::default(),
            }
            .compile()
            .is_ok_and(|tape| tape.frames.len() == 5)
        }));
    }
}
