//! Pure, bounded proposal generation for the native checkpoint suffix loop.

use crate::search::{Candidate, MacroAction, SearchError, SegmentProfile};
use crate::tape::{InputFrame, InputTape};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const NATIVE_SUFFIX_BATCH_SCHEMA: &str = "dusklight-suffix-batch/v3";
pub const NATIVE_REACTIVE_SUFFIX_BATCH_SCHEMA: &str = "dusklight-suffix-batch/v8";
const MAXIMUM_CANDIDATES: usize = 16_384;
const MAXIMUM_TICKS: usize = 4_096;
const MAXIMUM_EXPANDED_TICKS: usize = 8 * 1_024 * 1_024;
const MAXIMUM_VALIDATION_TICKS: usize = 256;
const BUTTON_A: u16 = 0x0100;
const ORDON_EXIT_EDGE_X: f64 = -1708.04;
const ORDON_EXIT_EDGE_Z: f64 = -4166.06;
const ORDON_EXIT_LEFT_EDGE_DZ_DX: f64 = 15.9506 / -198.4406;
const ORDON_EXIT_RIGHT_EDGE_DZ_DX: f64 = -3.93 / 67.4398;

pub fn ordon_exit_edge_distance(x: f64, z: f64) -> f64 {
    let slope = if x < ORDON_EXIT_EDGE_X {
        ORDON_EXIT_LEFT_EDGE_DZ_DX
    } else {
        ORDON_EXIT_RIGHT_EDGE_DZ_DX
    };
    z - (ORDON_EXIT_EDGE_Z + slope * (x - ORDON_EXIT_EDGE_X))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SuffixProposalMethod {
    Deletion,
    DeleteHold,
    ButtonEdge,
    Heading,
    Corner,
    CornerWide,
    Collision,
    FineHeading,
    FineTerminal,
    LaneShift,
    FineLaneShift,
    EarlyLaneShift,
    Magnitude,
    AsymmetricLaneShift,
    PostCollision,
    RecoveryBias,
    Timing,
    Path,
    Terminal,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct NativeSuffixBatch {
    pub schema: String,
    pub source_frame: usize,
    pub source_boundary_fingerprint: String,
    pub checkpoint_validation: NativeCheckpointValidation,
    pub maximum_ticks: usize,
    pub verify_state_hashes: bool,
    pub candidates: Vec<NativeSuffixCandidate>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NativeCheckpointValidation {
    pub kind: String,
    pub ticks: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NativeSuffixCandidate {
    pub id: String,
    pub actions: Vec<MacroAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller_program_hex: Option<String>,
}

pub fn propose_suffix_batch(
    seed: &Candidate,
    source_frame: usize,
    source_boundary_fingerprint: &str,
    maximum_ticks: usize,
    candidate_budget: usize,
    method: SuffixProposalMethod,
) -> Result<NativeSuffixBatch, SearchError> {
    seed.validate()?;
    if seed.segment != SegmentProfile::Fsp103ToFsp104
        || !valid_boundary_fingerprint(source_boundary_fingerprint)
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
        schema: NATIVE_SUFFIX_BATCH_SCHEMA.into(),
        source_frame,
        source_boundary_fingerprint: source_boundary_fingerprint.into(),
        checkpoint_validation: NativeCheckpointValidation {
            kind: "recorded_replay_window".into(),
            ticks: maximum_ticks.min(8),
        },
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
        SuffixProposalMethod::DeleteHold => {
            if maximum_ticks < 2 {
                return Err(SearchError::InvalidPopulation);
            }
            let base = source.frames[..maximum_ticks].to_vec();
            let final_pad = base.last().cloned().ok_or(SearchError::InvalidPopulation)?;
            for deleted in 0..maximum_ticks - 1 {
                let mut frames = base.clone();
                frames.remove(deleted);
                frames.push(final_pad.clone());
                push_candidate(
                    &mut output,
                    &mut seen,
                    seed,
                    frames,
                    format!("delete-hold-{deleted}"),
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
            // instead of spending it all on the first corner. Start at the
            // terminal input because late steering is the least disruptive
            // way to turn a one-tick miss into a crossing.
            'variants: for (window, degrees) in [
                (1_usize, [-6.0_f64, 6.0_f64]),
                (2, [-6.0, 6.0]),
                (4, [-3.0, 3.0]),
                (8, [-3.0, 3.0]),
            ] {
                for ordinal in 0..maximum_ticks {
                    let start = maximum_ticks - 1 - ordinal.wrapping_mul(37) % maximum_ticks;
                    for degrees in degrees {
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
                            break 'variants;
                        }
                    }
                }
            }
        }
        SuffixProposalMethod::Corner => {
            let base = source.frames[..maximum_ticks].to_vec();
            // Incumbent diagnostics identify two costly turn episodes. Sweep
            // small signed changes across their entry, apex, and exit ticks.
            let anchors = [36_usize, 38, 40, 66, 70, 72, 74, 76, 78, 80, 82];
            'variants: for (window, degrees) in [
                (1_usize, -3.0_f64),
                (1, 3.0),
                (2, -3.0),
                (2, 3.0),
                (3, -2.0),
                (3, 2.0),
                (4, -2.0),
                (4, 2.0),
                (6, -1.0),
                (6, 1.0),
                (8, -1.0),
                (8, 1.0),
            ] {
                for start in anchors {
                    if start >= maximum_ticks {
                        continue;
                    }
                    let mut frames = base.clone();
                    rotate_heading(&mut frames, start, window, degrees);
                    push_candidate(
                        &mut output,
                        &mut seen,
                        seed,
                        frames,
                        format!("corner-{start}-w{window}-{degrees:+.0}"),
                        candidate_budget,
                    )?;
                    if output.candidates.len() == candidate_budget {
                        break 'variants;
                    }
                }
            }
        }
        SuffixProposalMethod::CornerWide => {
            let base = source.frames[..maximum_ticks].to_vec();
            // Narrow corner edits did not improve the terminal miss enough to
            // recover a tick. Sweep larger rotations over the same measured
            // turn episodes without perturbing unrelated straight-line input.
            let anchors = [32_usize, 36, 40, 64, 68, 72, 76, 80];
            'variants: for (window, degrees) in [
                (3_usize, -12.0_f64),
                (3, 12.0),
                (4, -12.0),
                (4, 12.0),
                (6, -9.0),
                (6, 9.0),
                (8, -9.0),
                (8, 9.0),
                (12, -6.0),
                (12, 6.0),
                (16, -6.0),
                (16, 6.0),
                (20, -4.0),
                (20, 4.0),
            ] {
                for start in anchors {
                    if start >= maximum_ticks {
                        continue;
                    }
                    let mut frames = base.clone();
                    rotate_heading(&mut frames, start, window, degrees);
                    push_candidate(
                        &mut output,
                        &mut seen,
                        seed,
                        frames,
                        format!("corner-wide-{start}-w{window}-{degrees:+.0}"),
                        candidate_budget,
                    )?;
                    if output.candidates.len() == candidate_budget {
                        break 'variants;
                    }
                }
            }
        }
        SuffixProposalMethod::Collision => {
            let base = source.frames[..maximum_ticks].to_vec();
            // Native collision observations put the largest correction losses
            // in this short approach window. Exhaust the local neighborhood
            // around the best measured edit instead of perturbing the route's
            // unrelated turns.
            let anchors = 75_usize..=81;
            'variants: for degrees in [-1.0_f64, -2.0, -3.0, -4.0, 1.0, 2.0] {
                for window in 2_usize..=7 {
                    for start in anchors.clone() {
                        if start >= maximum_ticks {
                            continue;
                        }
                        let mut frames = base.clone();
                        rotate_heading(&mut frames, start, window, degrees);
                        push_candidate(
                            &mut output,
                            &mut seen,
                            seed,
                            frames,
                            format!("collision-{start}-w{window}-{degrees:+.0}"),
                            candidate_budget,
                        )?;
                        if output.candidates.len() == candidate_budget {
                            break 'variants;
                        }
                    }
                }
            }
        }
        SuffixProposalMethod::FineHeading => {
            let base = source.frames[..maximum_ticks].to_vec();
            // Sub-degree rotations become single stick-unit changes after
            // quantization. Walk backward from the exit so a bounded batch
            // covers terminal input first, then earlier path-shaping frames.
            'variants: for degrees in [-0.25_f64, 0.25, -0.5, 0.5, -1.0, 1.0] {
                for ordinal in 0..maximum_ticks {
                    let start = maximum_ticks - 1 - ordinal.wrapping_mul(37) % maximum_ticks;
                    let mut frames = base.clone();
                    rotate_heading(&mut frames, start, 1, degrees);
                    push_candidate(
                        &mut output,
                        &mut seen,
                        seed,
                        frames,
                        format!("fine-heading-{start}-{degrees:+.2}"),
                        candidate_budget,
                    )?;
                    if output.candidates.len() == candidate_budget {
                        break 'variants;
                    }
                }
            }
        }
        SuffixProposalMethod::FineTerminal => {
            let base = source.frames[..maximum_ticks].to_vec();
            let end = maximum_ticks.min(113);
            let start = end.saturating_sub(25);
            'variants: for half_degrees in 1_i32..=10 {
                for sign in [-1.0_f64, 1.0] {
                    let degrees = sign * f64::from(half_degrees) * 0.5;
                    for frame_index in start..end {
                        let mut frames = base.clone();
                        rotate_heading(&mut frames, frame_index, 1, degrees);
                        push_candidate(
                            &mut output,
                            &mut seen,
                            seed,
                            frames,
                            format!("fine-terminal-{frame_index}-{degrees:+.1}"),
                            candidate_budget,
                        )?;
                        if output.candidates.len() == candidate_budget {
                            break 'variants;
                        }
                    }
                }
            }
        }
        SuffixProposalMethod::LaneShift => {
            let base = source.frames[..maximum_ticks].to_vec();
            // Shift the approach laterally before the measured wall contact,
            // then apply the opposite steering bias to recover the exit line.
            'variants: for degrees in [-2.0_f64, 2.0, -4.0, 4.0, -6.0, 6.0] {
                for window in [5_usize, 10, 15, 20] {
                    for start in 50_usize..=75 {
                        if start >= maximum_ticks {
                            continue;
                        }
                        let mut frames = base.clone();
                        rotate_heading(&mut frames, start, window, degrees);
                        rotate_heading(&mut frames, start.saturating_add(window), window, -degrees);
                        push_candidate(
                            &mut output,
                            &mut seen,
                            seed,
                            frames,
                            format!("lane-shift-{start}-w{window}-{degrees:+.0}"),
                            candidate_budget,
                        )?;
                        if output.candidates.len() == candidate_budget {
                            break 'variants;
                        }
                    }
                }
            }
        }
        SuffixProposalMethod::FineLaneShift => {
            let base = source.frames[..maximum_ticks].to_vec();
            'variants: for degrees in [-0.25_f64, 0.25, -0.5, 0.5, -1.0, 1.0] {
                for window in 8_usize..=12 {
                    for start in 45_usize..=60 {
                        if start >= maximum_ticks {
                            continue;
                        }
                        let mut frames = base.clone();
                        rotate_heading(&mut frames, start, window, degrees);
                        rotate_heading(&mut frames, start.saturating_add(window), window, -degrees);
                        push_candidate(
                            &mut output,
                            &mut seen,
                            seed,
                            frames,
                            format!("fine-lane-shift-{start}-w{window}-{degrees:+.2}"),
                            candidate_budget,
                        )?;
                        if output.candidates.len() == candidate_budget {
                            break 'variants;
                        }
                    }
                }
            }
        }
        SuffixProposalMethod::EarlyLaneShift => {
            let base = source.frames[..maximum_ticks].to_vec();
            'variants: for degrees in [-1.0_f64, 1.0, -2.0, 2.0] {
                for window in [5_usize, 10, 15, 20] {
                    for start in 20_usize..=50 {
                        if start >= maximum_ticks {
                            continue;
                        }
                        let mut frames = base.clone();
                        rotate_heading(&mut frames, start, window, degrees);
                        rotate_heading(&mut frames, start.saturating_add(window), window, -degrees);
                        push_candidate(
                            &mut output,
                            &mut seen,
                            seed,
                            frames,
                            format!("early-lane-shift-{start}-w{window}-{degrees:+.0}"),
                            candidate_budget,
                        )?;
                        if output.candidates.len() == candidate_budget {
                            break 'variants;
                        }
                    }
                }
            }
        }
        SuffixProposalMethod::Magnitude => {
            let base = source.frames[..maximum_ticks].to_vec();
            for percent in [80_i32, 90, 95, 105, 110, 120, 130, 140, 150] {
                let mut frames = base.clone();
                scale_stick_magnitude(&mut frames, 0, maximum_ticks, percent);
                push_candidate(
                    &mut output,
                    &mut seen,
                    seed,
                    frames,
                    format!("magnitude-global-{percent}"),
                    candidate_budget,
                )?;
            }
            'variants: for percent in [80_i32, 90, 95, 105, 110, 120, 140] {
                for window in [5_usize, 10, 20] {
                    for start in (20_usize..=110).step_by(5) {
                        if start >= maximum_ticks {
                            continue;
                        }
                        let mut frames = base.clone();
                        scale_stick_magnitude(&mut frames, start, window, percent);
                        push_candidate(
                            &mut output,
                            &mut seen,
                            seed,
                            frames,
                            format!("magnitude-{start}-w{window}-{percent}"),
                            candidate_budget,
                        )?;
                        if output.candidates.len() == candidate_budget {
                            break 'variants;
                        }
                    }
                }
            }
        }
        SuffixProposalMethod::AsymmetricLaneShift => {
            let base = source.frames[..maximum_ticks].to_vec();
            'variants: for degrees in [-2.0_f64, 2.0] {
                for outbound_window in [5_usize, 10] {
                    for gap in [0_usize, 5, 10] {
                        for recovery_window in [5_usize, 10, 15] {
                            for start in 40_usize..=55 {
                                if start >= maximum_ticks {
                                    continue;
                                }
                                let mut frames = base.clone();
                                rotate_heading(&mut frames, start, outbound_window, degrees);
                                let recovery_start =
                                    start.saturating_add(outbound_window).saturating_add(gap);
                                rotate_heading(
                                    &mut frames,
                                    recovery_start,
                                    recovery_window,
                                    -degrees,
                                );
                                push_candidate(
                                    &mut output,
                                    &mut seen,
                                    seed,
                                    frames,
                                    format!(
                                        "asymmetric-lane-{start}-o{outbound_window}-g{gap}-r{recovery_window}-{degrees:+.0}"
                                    ),
                                    candidate_budget,
                                )?;
                                if output.candidates.len() == candidate_budget {
                                    break 'variants;
                                }
                            }
                        }
                    }
                }
            }
        }
        SuffixProposalMethod::PostCollision => {
            let base = source.frames[..maximum_ticks].to_vec();
            'variants: for degrees in [-2.0_f64, 2.0, -4.0, 4.0, -6.0, 6.0, -8.0, 8.0, -10.0, 10.0]
            {
                for window in [10_usize, 20, 30, 40] {
                    for start in [80_usize, 85, 90, 95, 100, 105, 110] {
                        if start >= maximum_ticks {
                            continue;
                        }
                        let mut frames = base.clone();
                        rotate_heading(&mut frames, start, window, degrees);
                        push_candidate(
                            &mut output,
                            &mut seen,
                            seed,
                            frames,
                            format!("post-collision-{start}-w{window}-{degrees:+.0}"),
                            candidate_budget,
                        )?;
                        if output.candidates.len() == candidate_budget {
                            break 'variants;
                        }
                    }
                }
            }
        }
        SuffixProposalMethod::RecoveryBias => {
            let base = source.frames[..maximum_ticks].to_vec();
            'variants: for recovery_degrees in [-0.5_f64, -1.0, -1.5, -2.0, -2.5, -3.0, -3.5, -4.0]
            {
                for recovery_window in [5_usize, 10, 15, 20] {
                    for recovery_start in 55_usize..=75 {
                        let mut frames = base.clone();
                        rotate_heading(&mut frames, 50, 10, 2.0);
                        rotate_heading(
                            &mut frames,
                            recovery_start,
                            recovery_window,
                            recovery_degrees,
                        );
                        push_candidate(
                            &mut output,
                            &mut seen,
                            seed,
                            frames,
                            format!(
                                "recovery-bias-{recovery_start}-w{recovery_window}-{recovery_degrees:+.1}"
                            ),
                            candidate_budget,
                        )?;
                        if output.candidates.len() == candidate_budget {
                            break 'variants;
                        }
                    }
                }
            }
        }
        SuffixProposalMethod::Timing => {
            let base = source.frames[..maximum_ticks].to_vec();
            let starts = (28_usize..=45).chain(60..=90).collect::<Vec<_>>();
            'variants: for (window, delta) in [
                (1_usize, -1_isize),
                (1, 1),
                (2, -1),
                (2, 1),
                (4, -2),
                (4, 2),
                (6, -3),
                (6, 3),
            ] {
                for &start in &starts {
                    if start >= maximum_ticks {
                        continue;
                    }
                    let mut frames = base.clone();
                    shift_heading_timing(&mut frames, &base, start, window, delta);
                    push_candidate(
                        &mut output,
                        &mut seen,
                        seed,
                        frames,
                        format!("timing-{start}-w{window}-{delta:+}"),
                        candidate_budget,
                    )?;
                    if output.candidates.len() == candidate_budget {
                        break 'variants;
                    }
                }
            }
        }
        SuffixProposalMethod::Path => {
            let base = source.frames[..maximum_ticks].to_vec();
            let anchors = [0_usize, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110];
            'variants: for (window, degrees) in [
                (10_usize, -1.0_f64),
                (10, 1.0),
                (20, -1.0),
                (20, 1.0),
                (30, -1.0),
                (30, 1.0),
                (10, -2.0),
                (10, 2.0),
                (20, -2.0),
                (20, 2.0),
            ] {
                for start in anchors {
                    if start >= maximum_ticks {
                        continue;
                    }
                    let mut frames = base.clone();
                    rotate_heading(&mut frames, start, window, degrees);
                    push_candidate(
                        &mut output,
                        &mut seen,
                        seed,
                        frames,
                        format!("path-{start}-w{window}-{degrees:+.0}"),
                        candidate_budget,
                    )?;
                    if output.candidates.len() == candidate_budget {
                        break 'variants;
                    }
                }
            }
        }
        SuffixProposalMethod::Terminal => {
            let base = source.frames[..maximum_ticks].to_vec();
            let anchors = [90_usize, 95, 100, 105, 110, 115, 120];
            'variants: for (window, degrees) in [
                (5_usize, -2.0_f64),
                (5, 2.0),
                (10, -2.0),
                (10, 2.0),
                (15, -2.0),
                (15, 2.0),
                (20, -2.0),
                (20, 2.0),
                (10, -4.0),
                (10, 4.0),
                (15, -4.0),
                (15, 4.0),
                (20, -4.0),
                (20, 4.0),
                (10, -6.0),
                (10, 6.0),
                (20, -6.0),
                (20, 6.0),
            ] {
                for start in anchors {
                    if start >= maximum_ticks {
                        continue;
                    }
                    let mut frames = base.clone();
                    rotate_heading(&mut frames, start, window, degrees);
                    push_candidate(
                        &mut output,
                        &mut seen,
                        seed,
                        frames,
                        format!("terminal-{start}-w{window}-{degrees:+.0}"),
                        candidate_budget,
                    )?;
                    if output.candidates.len() == candidate_budget {
                        break 'variants;
                    }
                }
            }
        }
    }
    if output.candidates.is_empty() {
        return Err(SearchError::PopulationStalled);
    }
    Ok(output)
}

/// Combines complementary disjoint mutations from an evaluated batch. The
/// endpoint model ranks proposals only; native terminal success remains the
/// authority.
pub fn propose_ranked_suffix_refinement(
    seed: &Candidate,
    parent: &NativeSuffixBatch,
    terminal_observations: &[(String, f64, f64)],
    candidate_budget: usize,
) -> Result<NativeSuffixBatch, SearchError> {
    if parent.schema != NATIVE_SUFFIX_BATCH_SCHEMA
        || !valid_boundary_fingerprint(&parent.source_boundary_fingerprint)
        || parent.checkpoint_validation.kind != "recorded_replay_window"
        || parent.checkpoint_validation.ticks == 0
        || parent.checkpoint_validation.ticks > MAXIMUM_VALIDATION_TICKS
        || candidate_budget == 0
        || candidate_budget > MAXIMUM_CANDIDATES
        || candidate_budget.saturating_mul(parent.maximum_ticks) > MAXIMUM_EXPANDED_TICKS
    {
        return Err(SearchError::InvalidPopulation);
    }
    let source = seed.compile()?;
    if source.frames.len() < parent.maximum_ticks {
        return Err(SearchError::NonCanonicalTape(
            "ranked refinement seed is shorter than its parent batch".into(),
        ));
    }
    let base = source.frames[..parent.maximum_ticks].to_vec();
    let by_id = parent
        .candidates
        .iter()
        .map(|candidate| (candidate.id.as_str(), candidate))
        .collect::<std::collections::HashMap<_, _>>();

    // Exact repeated endpoints reveal mutations that had no gameplay effect,
    // giving a robust baseline without a separate native run.
    let mut endpoint_counts = std::collections::BTreeMap::new();
    for (_, x, z) in terminal_observations {
        *endpoint_counts
            .entry((x.to_bits(), z.to_bits()))
            .or_insert(0_usize) += 1;
    }
    let ((baseline_x_bits, baseline_z_bits), _) = endpoint_counts
        .into_iter()
        .max_by(|left, right| left.1.cmp(&right.1).then_with(|| right.0.cmp(&left.0)))
        .ok_or(SearchError::InvalidResult)?;
    let baseline_x = f64::from_bits(baseline_x_bits);
    let baseline_z = f64::from_bits(baseline_z_bits);

    // The promoted world inventory reconstructs the near edge of the F_SP104
    // exit triangle from (-1708.04, -4166.06) to (-1906.4806, -4150.1094).
    // Rank by signed XZ distance to that edge, while success remains native.
    let mut ranked = terminal_observations.to_vec();
    ranked.sort_by(|left, right| {
        ordon_exit_edge_distance(left.1, left.2)
            .total_cmp(&ordon_exit_edge_distance(right.1, right.2))
            .then_with(|| left.0.cmp(&right.0))
    });
    // Retain enough individually measured variables for a cumulative fine-edit
    // lane. Pair/triple enumeration below remains capped at sixteen because
    // coarse heading changes were measurably non-additive in Link's dynamics.
    let selected_ids = ranked
        .into_iter()
        .take(64)
        .map(|sample| sample.0)
        .collect::<Vec<_>>();

    let mut mutations = Vec::new();
    for id in selected_ids {
        let candidate = by_id.get(id.as_str()).ok_or(SearchError::InvalidResult)?;
        let (_, terminal_x, terminal_z) = terminal_observations
            .iter()
            .find(|sample| sample.0 == id)
            .ok_or(SearchError::InvalidResult)?;
        let tape = Candidate {
            schema: crate::search::CANDIDATE_SCHEMA.into(),
            segment: seed.segment,
            boot: seed.boot.clone(),
            actions: candidate.actions.clone(),
            ancestry: crate::search::Ancestry::default(),
        }
        .compile()?;
        if tape.frames.len() != base.len() {
            return Err(SearchError::InvalidResult);
        }
        let changes = base
            .iter()
            .zip(&tape.frames)
            .enumerate()
            .filter_map(|(index, (original, mutated))| {
                (original != mutated).then_some((index, mutated.clone()))
            })
            .collect::<Vec<_>>();
        if !changes.is_empty() {
            mutations.push((
                id,
                changes,
                terminal_x - baseline_x,
                terminal_z - baseline_z,
            ));
        }
    }
    if mutations.len() < 2 {
        return Err(SearchError::PopulationStalled);
    }

    let mut output = NativeSuffixBatch {
        schema: NATIVE_SUFFIX_BATCH_SCHEMA.into(),
        source_frame: parent.source_frame,
        source_boundary_fingerprint: parent.source_boundary_fingerprint.clone(),
        checkpoint_validation: parent.checkpoint_validation.clone(),
        maximum_ticks: parent.maximum_ticks,
        verify_state_hashes: false,
        candidates: Vec::with_capacity(candidate_budget),
    };
    let mut seen = HashSet::new();
    let baseline_distance = ordon_exit_edge_distance(baseline_x, baseline_z);
    let mut greedy_frames = base.clone();
    let mut greedy_touched = HashSet::new();
    let mut greedy_count = 0_usize;
    for mutation in &mutations {
        if ordon_exit_edge_distance(baseline_x + mutation.2, baseline_z + mutation.3)
            >= baseline_distance
            || mutation
                .1
                .iter()
                .any(|(frame_index, _)| greedy_touched.contains(frame_index))
        {
            continue;
        }
        for (frame_index, frame) in &mutation.1 {
            greedy_touched.insert(*frame_index);
            greedy_frames[*frame_index] = frame.clone();
        }
        greedy_count += 1;
        if greedy_count >= 2 {
            push_candidate(
                &mut output,
                &mut seen,
                seed,
                greedy_frames.clone(),
                format!("greedy-{greedy_count}"),
                candidate_budget,
            )?;
        }
    }
    let mut ranked_subsets = Vec::new();
    let combination_count = mutations.len().min(16);
    for mask in 1_u64..(1_u64 << combination_count) {
        if !(2..=3).contains(&mask.count_ones()) {
            continue;
        }
        let mut predicted_x = baseline_x;
        let mut predicted_z = baseline_z;
        for (index, mutation) in mutations.iter().take(combination_count).enumerate() {
            if mask & (1_u64 << index) != 0 {
                predicted_x += mutation.2;
                predicted_z += mutation.3;
            }
        }
        let lane_penalty = if !(-1906.4806..=-1640.6002).contains(&predicted_x) {
            10_000.0
        } else {
            0.0
        };
        let score = ordon_exit_edge_distance(predicted_x, predicted_z) + lane_penalty;
        ranked_subsets.push((score, mask));
    }
    ranked_subsets.sort_by(|left, right| {
        left.0
            .total_cmp(&right.0)
            .then_with(|| left.1.cmp(&right.1))
    });
    for (_, mask) in ranked_subsets {
        let mut frames = base.clone();
        let mut touched = HashSet::new();
        let mut compatible = true;
        for (mutation_index, mutation) in mutations.iter().enumerate() {
            if mask & (1_u64 << mutation_index) == 0 {
                continue;
            }
            for (frame_index, frame) in &mutation.1 {
                if !touched.insert(*frame_index) && frames[*frame_index] != *frame {
                    compatible = false;
                    break;
                }
                frames[*frame_index] = frame.clone();
            }
            if !compatible {
                break;
            }
        }
        if !compatible {
            continue;
        }
        push_candidate(
            &mut output,
            &mut seen,
            seed,
            frames,
            format!("ranked-{mask:x}"),
            candidate_budget,
        )?;
        if output.candidates.len() == candidate_budget {
            return Ok(output);
        }
    }
    if output.candidates.is_empty() {
        return Err(SearchError::PopulationStalled);
    }
    Ok(output)
}

fn valid_boundary_fingerprint(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
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
        output.candidates.push(NativeSuffixCandidate {
            id,
            actions,
            controller_program_hex: None,
        });
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

fn scale_stick_magnitude(frames: &mut [InputFrame], start: usize, window: usize, percent: i32) {
    let end = start.saturating_add(window).min(frames.len());
    for frame in &mut frames[start..end] {
        let scale = f64::from(percent) / 100.0;
        frame.pads[0].stick_x = (f64::from(frame.pads[0].stick_x) * scale)
            .round()
            .clamp(-127.0, 127.0) as i8;
        frame.pads[0].stick_y = (f64::from(frame.pads[0].stick_y) * scale)
            .round()
            .clamp(-127.0, 127.0) as i8;
    }
}

fn shift_heading_timing(
    frames: &mut [InputFrame],
    source: &[InputFrame],
    start: usize,
    window: usize,
    delta: isize,
) {
    let end = start.saturating_add(window).min(frames.len());
    for (index, frame) in frames.iter_mut().enumerate().take(end).skip(start) {
        let Some(source_index) = index.checked_add_signed(delta) else {
            continue;
        };
        if source_index >= source.len() {
            continue;
        }
        frame.pads[0].stick_x = source[source_index].pads[0].stick_x;
        frame.pads[0].stick_y = source[source_index].pads[0].stick_y;
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
        let batch = propose_suffix_batch(
            &seed(),
            440,
            "ac7c32788fc3b5c59046386d95b9b5b4",
            4,
            5,
            SuffixProposalMethod::Deletion,
        )
        .unwrap();
        assert_eq!(batch.schema, NATIVE_SUFFIX_BATCH_SCHEMA);
        assert_eq!(
            batch.checkpoint_validation,
            NativeCheckpointValidation {
                kind: "recorded_replay_window".into(),
                ticks: 4,
            }
        );
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
    fn delete_hold_shifts_input_without_changing_candidate_length() {
        let mut distinct = seed();
        distinct.actions = (0_i8..5)
            .map(|offset| MacroAction::PadRun {
                pad: SearchPadState {
                    buttons: 0,
                    stick_x: 100 + offset,
                    stick_y: 50,
                    substick_x: 0,
                    substick_y: 0,
                    trigger_left: 0,
                    trigger_right: 0,
                    analog_a: 0,
                    analog_b: 0,
                    connected: true,
                    error: 0,
                },
                frames: 1,
            })
            .collect();
        let batch = propose_suffix_batch(
            &distinct,
            440,
            "ac7c32788fc3b5c59046386d95b9b5b4",
            5,
            5,
            SuffixProposalMethod::DeleteHold,
        )
        .unwrap();
        assert_eq!(batch.candidates.len(), 4);
        let first = Candidate {
            schema: CANDIDATE_SCHEMA.into(),
            segment: SegmentProfile::Fsp103ToFsp104,
            boot: TapeBoot::Process,
            actions: batch.candidates[0].actions.clone(),
            ancestry: Ancestry::default(),
        }
        .compile()
        .unwrap();
        assert_eq!(first.frames.len(), 5);
        assert_eq!(first.frames[0].pads[0].stick_x, 101);
        assert_eq!(first.frames[3].pads[0].stick_x, 104);
        assert_eq!(first.frames[4].pads[0].stick_x, 104);
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

    #[test]
    fn suffix_batch_rejects_unpinned_or_noncanonical_source_fingerprints() {
        for fingerprint in [
            "",
            "ac7c32788fc3b5c59046386d95b9b5b",
            "AC7C32788FC3B5C59046386D95B9B5B4",
            "gc7c32788fc3b5c59046386d95b9b5b4",
        ] {
            assert!(
                propose_suffix_batch(
                    &seed(),
                    440,
                    fingerprint,
                    4,
                    5,
                    SuffixProposalMethod::Deletion,
                )
                .is_err()
            );
        }
    }

    #[test]
    fn heading_budget_is_filled_with_unique_exact_candidates() {
        let batch = propose_suffix_batch(
            &seed(),
            440,
            "ac7c32788fc3b5c59046386d95b9b5b4",
            5,
            8,
            SuffixProposalMethod::Heading,
        )
        .unwrap();
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

    #[test]
    fn ranked_refinement_combines_distinct_high_progress_mutations() {
        let parent = propose_suffix_batch(
            &seed(),
            440,
            "ac7c32788fc3b5c59046386d95b9b5b4",
            5,
            8,
            SuffixProposalMethod::Heading,
        )
        .unwrap();
        let scores = parent
            .candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| {
                let offset = index as f64;
                (candidate.id.clone(), 10.0 - offset, 20.0 - offset)
            })
            .collect::<Vec<_>>();
        let refined = propose_ranked_suffix_refinement(&seed(), &parent, &scores, 6).unwrap();
        assert_eq!(
            refined.source_boundary_fingerprint,
            parent.source_boundary_fingerprint
        );
        assert_eq!(refined.checkpoint_validation, parent.checkpoint_validation);
        assert!(!refined.candidates.is_empty());
        assert!(refined.candidates.iter().all(|candidate| {
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

    #[test]
    fn ranked_refinement_rejects_unvalidated_checkpoint_contracts() {
        let mut parent = propose_suffix_batch(
            &seed(),
            440,
            "ac7c32788fc3b5c59046386d95b9b5b4",
            5,
            8,
            SuffixProposalMethod::Heading,
        )
        .unwrap();
        parent.checkpoint_validation.ticks = 0;
        let scores = parent
            .candidates
            .iter()
            .map(|candidate| (candidate.id.clone(), 0.0, 0.0))
            .collect::<Vec<_>>();
        assert!(propose_ranked_suffix_refinement(&seed(), &parent, &scores, 6).is_err());
    }
}
