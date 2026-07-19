//! Finite-sample search primitives for milestone-backed TAS optimization.

use dusklight_automation_contracts::tape::{InputFrame, InputTape, RawPadState, TapeBoot};
use dusklight_control::game_tactic::GameTacticPlan;
use dusklight_control::motion_path::MotionPathPlan;
use dusklight_control::roll_option::{RollOptionPlan, RollSpacing};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

pub const CANDIDATE_SCHEMA: &str = "dusklight-search-candidate/v2";
const LEGACY_CANDIDATE_SCHEMA: &str = "dusklight-search-candidate/v1";
pub const POPULATION_SCHEMA: &str = "dusklight-search-population/v3";
const LEGACY_POPULATION_SCHEMA_V2: &str = "dusklight-search-population/v2";
const LEGACY_POPULATION_SCHEMA_V1: &str = "dusklight-search-population/v1";
pub const RESULTS_SCHEMA: &str = "dusklight-search-results/v3";
const LEGACY_RESULTS_SCHEMA_V2: &str = "dusklight-search-results/v2";
const LEGACY_RESULTS_SCHEMA_V1: &str = "dusklight-search-results/v1";
const MAX_ACTIONS: usize = 4096;
const MAX_FRAMES: u64 = 1_000_000;
const BUTTON_A: u16 = 0x0100;
const BUTTON_B: u16 = 0x0200;
const BUTTON_START: u16 = 0x1000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentProfile {
    BootToFsp103,
    Fsp103ToFsp104,
    LinkControlToTunnelCrawlStart,
}

impl SegmentProfile {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BootToFsp103 => "boot_to_fsp103",
            Self::Fsp103ToFsp104 => "fsp103_to_fsp104",
            Self::LinkControlToTunnelCrawlStart => "link_control_to_tunnel_crawl_start",
        }
    }

    pub const fn target(self) -> MilestoneId {
        match self {
            Self::BootToFsp103 => MilestoneId::Fsp103RouteControl,
            Self::Fsp103ToFsp104 => MilestoneId::Fsp104Loaded,
            Self::LinkControlToTunnelCrawlStart => MilestoneId::TunnelCrawlStart,
        }
    }

    pub const fn target_depth(self) -> u16 {
        self.target().depth()
    }
}

impl std::str::FromStr for SegmentProfile {
    type Err = SearchError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "boot_to_fsp103" => Ok(Self::BootToFsp103),
            "fsp103_to_fsp104" => Ok(Self::Fsp103ToFsp104),
            "link_control_to_tunnel_crawl_start" => Ok(Self::LinkControlToTunnelCrawlStart),
            _ => Err(SearchError::InvalidSegment(value.into())),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MilestoneId {
    Fsp103Playable,
    Fsp103RouteControl,
    Fsp104LoadingTrigger,
    Fsp104Loaded,
    TunnelCrawlStart,
}

impl MilestoneId {
    pub const fn depth(self) -> u16 {
        match self {
            Self::Fsp103Playable => 1,
            Self::Fsp103RouteControl => 2,
            Self::Fsp104LoadingTrigger => 3,
            Self::Fsp104Loaded => 4,
            Self::TunnelCrawlStart => 5,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControllerButton {
    A,
    B,
    Start,
}

impl ControllerButton {
    const fn mask(self) -> u16 {
        match self {
            Self::A => BUTTON_A,
            Self::B => BUTTON_B,
            Self::Start => BUTTON_START,
        }
    }
}

/// Search-friendly controller macros. Angles use degrees: 0 is forward and
/// +90 is right. A roll presses B on its first frame and holds its stick for
/// the requested recovery frames.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum MacroAction {
    Move {
        angle_degrees: i16,
        magnitude: u8,
        frames: u32,
    },
    Roll {
        angle_degrees: i16,
        magnitude: u8,
        #[serde(default)]
        button_frame: u32,
        recovery_frames: u32,
        #[serde(default)]
        spacing: RollSpacing,
    },
    Neutral {
        frames: u32,
    },
    Press {
        buttons: Vec<ControllerButton>,
        hold_frames: u32,
        neutral_frames: u32,
    },
    GameTactic {
        plan: GameTacticPlan,
    },
    MotionPath {
        plan: MotionPathPlan,
    },
    /// Lossless run-length encoded port-zero state used to import an observed
    /// absolute movement tape without quantizing its analog samples.
    PadRun {
        pad: SearchPadState,
        frames: u32,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SearchPadState {
    pub buttons: u16,
    pub stick_x: i8,
    pub stick_y: i8,
    pub substick_x: i8,
    pub substick_y: i8,
    pub trigger_left: u8,
    pub trigger_right: u8,
    pub analog_a: u8,
    pub analog_b: u8,
    pub connected: bool,
    pub error: i8,
}

impl From<RawPadState> for SearchPadState {
    fn from(pad: RawPadState) -> Self {
        Self {
            buttons: pad.buttons,
            stick_x: pad.stick_x,
            stick_y: pad.stick_y,
            substick_x: pad.substick_x,
            substick_y: pad.substick_y,
            trigger_left: pad.trigger_left,
            trigger_right: pad.trigger_right,
            analog_a: pad.analog_a,
            analog_b: pad.analog_b,
            connected: pad.connected,
            error: pad.error,
        }
    }
}

impl From<SearchPadState> for RawPadState {
    fn from(pad: SearchPadState) -> Self {
        Self {
            buttons: pad.buttons,
            stick_x: pad.stick_x,
            stick_y: pad.stick_y,
            substick_x: pad.substick_x,
            substick_y: pad.substick_y,
            trigger_left: pad.trigger_left,
            trigger_right: pad.trigger_right,
            analog_a: pad.analog_a,
            analog_b: pad.analog_b,
            connected: pad.connected,
            error: pad.error,
        }
    }
}

impl MacroAction {
    fn frame_count(&self) -> u64 {
        match self {
            Self::Move { frames, .. } | Self::Neutral { frames } => u64::from(*frames),
            Self::Roll {
                button_frame,
                recovery_frames,
                ..
            } => u64::from(*button_frame) + 1 + u64::from(*recovery_frames),
            Self::Press {
                hold_frames,
                neutral_frames,
                ..
            } => u64::from(*hold_frames) + u64::from(*neutral_frames),
            Self::GameTactic { plan } => u64::from(
                plan.planned_ticks()
                    .unwrap_or(crate::game_tactic::MAX_TACTIC_TICKS + 1),
            ),
            Self::MotionPath { plan } => u64::from(plan.duration_ticks),
            Self::PadRun { frames, .. } => u64::from(*frames),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub schema: String,
    pub segment: SegmentProfile,
    #[serde(default)]
    pub boot: TapeBoot,
    pub actions: Vec<MacroAction>,
    pub ancestry: Ancestry,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Ancestry {
    pub generation: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mutation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intervention: Option<InterventionRange>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InterventionRange {
    pub start_frame: u64,
    pub end_frame_exclusive: u64,
    pub parent_end_frame_exclusive: u64,
}

impl Candidate {
    pub fn baseline(segment: SegmentProfile) -> Self {
        let actions = match segment {
            SegmentProfile::BootToFsp103 => boot_baseline(),
            SegmentProfile::Fsp103ToFsp104 => route_baseline(),
            SegmentProfile::LinkControlToTunnelCrawlStart => tunnel_crawl_baseline(),
        };
        Self {
            schema: CANDIDATE_SCHEMA.into(),
            segment,
            boot: match segment {
                SegmentProfile::Fsp103ToFsp104 => TapeBoot::Stage {
                    stage: "F_SP103".into(),
                    room: 1,
                    point: 1,
                    layer: 3,
                    save_slot: None,
                    fixture: None,
                },
                _ => TapeBoot::Process,
            },
            actions,
            ancestry: Ancestry::default(),
        }
    }

    pub fn validate(&self) -> Result<(), SearchError> {
        if self.schema != CANDIDATE_SCHEMA && self.schema != LEGACY_CANDIDATE_SCHEMA {
            return Err(SearchError::InvalidSchema(self.schema.clone()));
        }
        if self.schema == LEGACY_CANDIDATE_SCHEMA && self.boot != TapeBoot::Process {
            return Err(SearchError::InvalidSchema(self.schema.clone()));
        }
        if self.actions.is_empty() || self.actions.len() > MAX_ACTIONS {
            return Err(SearchError::InvalidActionCount(self.actions.len()));
        }
        let mut frames = 0_u64;
        for action in &self.actions {
            match action {
                MacroAction::Move {
                    magnitude, frames, ..
                } => {
                    validate_magnitude(*magnitude)?;
                    validate_duration(*frames)?;
                }
                MacroAction::Roll {
                    angle_degrees,
                    magnitude,
                    button_frame,
                    recovery_frames,
                    spacing,
                    ..
                } => {
                    let plan = RollOptionPlan {
                        schema: crate::roll_option::ROLL_OPTION_SCHEMA_V1.into(),
                        direction_degrees: *angle_degrees,
                        magnitude: *magnitude,
                        button_frame: *button_frame,
                        recovery_frames: *recovery_frames,
                        spacing: *spacing,
                        cancellation_conditions: Vec::new(),
                    };
                    plan.validate()
                        .map_err(|error| SearchError::NonCanonicalTape(error.to_string()))?;
                }
                MacroAction::Neutral { frames } => validate_duration(*frames)?,
                MacroAction::Press {
                    buttons,
                    hold_frames,
                    neutral_frames,
                } => {
                    if buttons.is_empty() || *hold_frames == 0 || *hold_frames > 30 {
                        return Err(SearchError::InvalidButtonPress);
                    }
                    if *neutral_frames > 10_000 {
                        return Err(SearchError::InvalidDuration(*neutral_frames));
                    }
                }
                MacroAction::GameTactic { plan } => {
                    if !plan.cancellation_conditions.is_empty() {
                        return Err(SearchError::NonCanonicalTape(
                            "static search tactics cannot declare reactive cancellation conditions"
                                .into(),
                        ));
                    }
                    plan.validate()
                        .map_err(|error| SearchError::NonCanonicalTape(error.to_string()))?;
                }
                MacroAction::MotionPath { plan } => {
                    if !plan.cancellation_conditions.is_empty() {
                        return Err(SearchError::NonCanonicalTape(
                            "static search paths cannot declare reactive cancellation conditions"
                                .into(),
                        ));
                    }
                    plan.validate()
                        .map_err(|error| SearchError::NonCanonicalTape(error.to_string()))?;
                }
                MacroAction::PadRun { frames, .. } => validate_duration(*frames)?,
            }
            frames = frames
                .checked_add(action.frame_count())
                .ok_or(SearchError::TooManyFrames)?;
        }
        if frames > MAX_FRAMES {
            return Err(SearchError::TooManyFrames);
        }
        Ok(())
    }

    /// Stable content ID. Ancestry is deliberately excluded so identical input
    /// programs deduplicate even when rediscovered through different parents.
    pub fn id(&self) -> Result<String, SearchError> {
        self.validate()?;
        let identity = serde_json::to_vec(&(self.segment, &self.boot, &self.actions))?;
        Ok(format!("{:x}", Sha256::digest(identity)))
    }

    pub fn frame_count(&self) -> u64 {
        self.actions.iter().map(MacroAction::frame_count).sum()
    }

    pub fn compile(&self) -> Result<InputTape, SearchError> {
        self.validate()?;
        let capacity =
            usize::try_from(self.frame_count()).map_err(|_| SearchError::TooManyFrames)?;
        let mut frames = Vec::with_capacity(capacity);
        for action in &self.actions {
            match action {
                MacroAction::Move {
                    angle_degrees,
                    magnitude,
                    frames: count,
                } => push_frames(
                    &mut frames,
                    move_frame(*angle_degrees, *magnitude, 0),
                    *count,
                ),
                MacroAction::Roll {
                    angle_degrees,
                    magnitude,
                    button_frame,
                    recovery_frames,
                    spacing,
                } => {
                    let plan = RollOptionPlan {
                        schema: crate::roll_option::ROLL_OPTION_SCHEMA_V1.into(),
                        direction_degrees: *angle_degrees,
                        magnitude: *magnitude,
                        button_frame: *button_frame,
                        recovery_frames: *recovery_frames,
                        spacing: *spacing,
                        cancellation_conditions: Vec::new(),
                    };
                    let realization = plan
                        .realize(frames.len() as u64, None)
                        .map_err(|error| SearchError::NonCanonicalTape(error.to_string()))?;
                    frames.extend(realization.frames);
                }
                MacroAction::Neutral { frames: count } => {
                    push_frames(&mut frames, owned_frame(RawPadState::default()), *count)
                }
                MacroAction::Press {
                    buttons,
                    hold_frames,
                    neutral_frames,
                } => {
                    let buttons = buttons.iter().fold(0, |mask, button| mask | button.mask());
                    let pad = RawPadState {
                        buttons,
                        ..RawPadState::default()
                    };
                    push_frames(&mut frames, owned_frame(pad), *hold_frames);
                    push_frames(
                        &mut frames,
                        owned_frame(RawPadState::default()),
                        *neutral_frames,
                    );
                }
                MacroAction::GameTactic { plan } => {
                    let realization = plan
                        .realize(None)
                        .map_err(|error| SearchError::NonCanonicalTape(error.to_string()))?;
                    frames.extend(realization.frames);
                }
                MacroAction::MotionPath { plan } => {
                    let realization = plan
                        .realize(None)
                        .map_err(|error| SearchError::NonCanonicalTape(error.to_string()))?;
                    frames.extend(realization.frames);
                }
                MacroAction::PadRun { pad, frames: count } => {
                    push_frames(&mut frames, imported_frame((*pad).into()), *count)
                }
            }
        }
        Ok(InputTape {
            boot: self.boot.clone(),
            tick_rate_numerator: 30,
            tick_rate_denominator: 1,
            frames,
        })
    }

    /// Losslessly infers typed boot macros from an absolute raw tape. Only
    /// neutral frames and zero-stick A/B/Start pulses are accepted; analog,
    /// reactive, multi-port, or noncanonical pad state is rejected instead of
    /// being guessed.
    pub fn from_absolute_tape(
        segment: SegmentProfile,
        tape: &InputTape,
    ) -> Result<Self, SearchError> {
        tape.validate()?;
        if tape.tick_rate_numerator != 30 || tape.tick_rate_denominator != 1 {
            return Err(SearchError::NonCanonicalTape(
                "typed candidates currently require a 30/1 tick rate".into(),
            ));
        }
        if tape.frames.is_empty() {
            return Err(SearchError::NonCanonicalTape("tape is empty".into()));
        }
        if matches!(
            segment,
            SegmentProfile::Fsp103ToFsp104 | SegmentProfile::LinkControlToTunnelCrawlStart
        ) {
            return Self::from_movement_tape(segment, tape);
        }
        #[derive(Clone, Copy, Eq, PartialEq)]
        enum State {
            Neutral,
            Press(u16),
        }
        let classify = |frame: &InputFrame| -> Result<State, SearchError> {
            if frame.owned_ports != 0x0f
                || frame.wait_condition != crate::tape::WaitCondition::None
                || frame.wait_timeout_ticks != 0
                || frame.pads[1..] != [RawPadState::default(); 3]
            {
                return Err(SearchError::NonCanonicalTape(
                    "frame ownership, reactive waits, or secondary ports are not safely expressible"
                        .into(),
                ));
            }
            let pad = frame.pads[0];
            let canonical = RawPadState {
                buttons: pad.buttons,
                ..RawPadState::default()
            };
            if pad != canonical {
                return Err(SearchError::NonCanonicalTape(
                    "analog, trigger, disconnected, or error pad state is ambiguous".into(),
                ));
            }
            if pad.buttons & !(BUTTON_A | BUTTON_B | BUTTON_START) != 0 {
                return Err(SearchError::NonCanonicalTape(format!(
                    "button mask 0x{:04x} is not a typed A/B/Start pulse",
                    pad.buttons
                )));
            }
            Ok(if pad.buttons == 0 {
                State::Neutral
            } else {
                State::Press(pad.buttons)
            })
        };
        let mut runs: Vec<(State, u32)> = Vec::new();
        for frame in &tape.frames {
            let state = classify(frame)?;
            if let Some((last, count)) = runs.last_mut()
                && *last == state
            {
                *count = count.checked_add(1).ok_or(SearchError::TooManyFrames)?;
            } else {
                runs.push((state, 1));
            }
        }
        let mut actions = Vec::new();
        let mut index = 0;
        while index < runs.len() {
            match runs[index] {
                (State::Neutral, frames) => {
                    actions.push(MacroAction::Neutral { frames });
                    index += 1;
                }
                (State::Press(mask), hold_frames) => {
                    let neutral_frames = runs
                        .get(index + 1)
                        .and_then(|(state, frames)| (*state == State::Neutral).then_some(*frames))
                        .unwrap_or(0);
                    if neutral_frames > 10_000 {
                        return Err(SearchError::NonCanonicalTape(
                            "post-press neutral run exceeds the typed press limit".into(),
                        ));
                    }
                    let mut buttons = Vec::new();
                    for (button, button_mask) in [
                        (ControllerButton::A, BUTTON_A),
                        (ControllerButton::B, BUTTON_B),
                        (ControllerButton::Start, BUTTON_START),
                    ] {
                        if mask & button_mask != 0 {
                            buttons.push(button);
                        }
                    }
                    // Preserve long raw holds losslessly by splitting them into
                    // adjacent typed actions. This also lets the reducer
                    // canonicalize a mash after deleting intervening frames.
                    let mut remaining = hold_frames;
                    while remaining > 30 {
                        actions.push(MacroAction::Press {
                            buttons: buttons.clone(),
                            hold_frames: 30,
                            neutral_frames: 0,
                        });
                        remaining -= 30;
                    }
                    actions.push(MacroAction::Press {
                        buttons,
                        hold_frames: remaining,
                        neutral_frames,
                    });
                    index += if neutral_frames == 0 { 1 } else { 2 };
                }
            }
        }
        let candidate = Self {
            schema: CANDIDATE_SCHEMA.into(),
            segment,
            boot: tape.boot.clone(),
            actions,
            ancestry: Ancestry {
                generation: 0,
                parent_id: None,
                mutation: Some("lossless absolute-tape import".into()),
                intervention: None,
            },
        };
        candidate.validate()?;
        if candidate.compile()? != *tape {
            return Err(SearchError::NonCanonicalTape(
                "typed inference did not reproduce the source tape exactly".into(),
            ));
        }
        Ok(candidate)
    }

    fn from_movement_tape(segment: SegmentProfile, tape: &InputTape) -> Result<Self, SearchError> {
        let disconnected = RawPadState {
            connected: false,
            error: -1,
            ..RawPadState::default()
        };
        let mut runs: Vec<(SearchPadState, u32)> = Vec::new();
        for frame in &tape.frames {
            if frame.owned_ports != 0x01
                || frame.wait_condition != crate::tape::WaitCondition::None
                || frame.wait_timeout_ticks != 0
                || frame.pads[1..] != [disconnected; 3]
            {
                return Err(SearchError::NonCanonicalTape(
                    "anchored movement imports require absolute port-one ownership, no reactive waits, and canonical disconnected secondary ports"
                        .into(),
                ));
            }
            let pad = SearchPadState::from(frame.pads[0]);
            if let Some((last, frames)) = runs.last_mut()
                && *last == pad
            {
                *frames = frames.checked_add(1).ok_or(SearchError::TooManyFrames)?;
            } else {
                runs.push((pad, 1));
            }
        }
        let candidate = Self {
            schema: CANDIDATE_SCHEMA.into(),
            segment,
            boot: tape.boot.clone(),
            actions: runs
                .into_iter()
                .map(|(pad, frames)| MacroAction::PadRun { pad, frames })
                .collect(),
            ancestry: Ancestry {
                generation: 0,
                parent_id: None,
                mutation: Some("lossless anchored movement-tape import".into()),
                intervention: None,
            },
        };
        candidate.validate()?;
        if candidate.compile()? != *tape {
            return Err(SearchError::NonCanonicalTape(
                "anchored movement import did not reproduce the source tape exactly".into(),
            ));
        }
        Ok(candidate)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PopulationManifest {
    pub schema: String,
    pub segment: SegmentProfile,
    /// Every member shares this authenticated launch origin. Legacy v1
    /// manifests omitted it and therefore deserialize only as process boot.
    #[serde(default)]
    pub boot: TapeBoot,
    pub generation: u32,
    pub rng_seed: u64,
    pub members: Vec<PopulationMember>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PopulationMember {
    pub candidate_id: String,
    pub candidate_file: PathBuf,
    pub tape_file: PathBuf,
    pub frame_count: u64,
    /// Canonical native-field transitions in the compiled absolute tape.
    /// Required by population v3; absent only in legacy manifests.
    #[serde(default)]
    pub input_complexity: Option<u64>,
    pub ancestry: Ancestry,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SearchResults {
    pub schema: String,
    pub segment: SegmentProfile,
    /// Results may only be ranked against a population with this exact origin.
    #[serde(default)]
    pub boot: TapeBoot,
    /// Results are keyed by the content ID from the population manifest.
    pub candidates: BTreeMap<String, CandidateResult>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateResult {
    /// Exact terminal predicate verdict. Required in results v3; legacy
    /// results derive it only from their segment's target depth.
    #[serde(default)]
    pub goal_reached: Option<bool>,
    /// Deepest memory-backed milestone reached by this candidate.
    pub milestone_depth: u16,
    /// Repeated restores/evaluations attempted.
    pub attempts: u32,
    /// Trials which reached `milestone_depth`.
    pub successes: u32,
    /// First-hit ticks for successful trials. Empty when no milestone was hit.
    #[serde(default)]
    pub first_hit_ticks: Vec<u64>,
    /// Count of authenticated risk events. `None` is explicitly unmeasured and
    /// ranks below a measured value; it is never treated as zero.
    #[serde(default)]
    pub risk_events: Option<u64>,
    /// Compatibility of the terminal state with a declared boundary reference.
    #[serde(default)]
    pub boundary_compatibility: BoundaryCompatibility,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryCompatibility {
    Incompatible = 0,
    #[default]
    Unknown = 1,
    Compatible = 2,
    Exact = 3,
}

/// One JSON artifact emitted by the native/PowerShell evaluator.
#[derive(Clone, Debug, Deserialize)]
pub struct EvaluationArtifact {
    pub schema_version: u32,
    pub candidate_id: String,
    pub search_result: CandidateResult,
}

impl CandidateResult {
    pub fn validate(&self) -> Result<(), SearchError> {
        if self.attempts == 0
            || self.successes > self.attempts
            || self.first_hit_ticks.len() != self.successes as usize
            || !(self.successes == 0 || self.successes == self.attempts)
            || (self.milestone_depth == 0) != (self.successes == 0)
            || self
                .first_hit_ticks
                .windows(2)
                .any(|ticks| ticks[0] != ticks[1])
        {
            return Err(SearchError::InvalidResult);
        }
        Ok(())
    }

    pub fn score(&self) -> Result<LexicographicScore, SearchError> {
        self.validate()?;
        let mut ticks = self.first_hit_ticks.clone();
        ticks.sort_unstable();
        let median_first_hit_tick = if ticks.is_empty() {
            u64::MAX
        } else {
            ticks[(ticks.len() - 1) / 2]
        };
        Ok(LexicographicScore {
            goal_feasible: self.goal_reached.unwrap_or(false),
            milestone_depth: self.milestone_depth,
            successes: self.successes,
            attempts: self.attempts,
            median_first_hit_tick,
            best_first_hit_tick: ticks.first().copied().unwrap_or(u64::MAX),
            tape_frames: u64::MAX,
            input_complexity: u64::MAX,
            risk_events: self.risk_events,
            boundary_compatibility: self.boundary_compatibility,
        })
    }

    fn validate_for_segment(
        &self,
        segment: SegmentProfile,
        require_explicit_goal: bool,
    ) -> Result<bool, SearchError> {
        self.validate()?;
        let target_depth = segment.target_depth();
        let implied_goal = self.milestone_depth == target_depth;
        if require_explicit_goal && self.goal_reached.is_none() {
            return Err(SearchError::InvalidResult);
        }
        let goal_reached = self.goal_reached.unwrap_or(implied_goal);
        if goal_reached && self.milestone_depth == 0 {
            return Err(SearchError::InvalidResult);
        }
        Ok(goal_reached)
    }

    fn score_for_segment(
        &self,
        segment: SegmentProfile,
        require_explicit_goal: bool,
        tape_frames: u64,
        input_complexity: u64,
    ) -> Result<LexicographicScore, SearchError> {
        let goal_feasible = self.validate_for_segment(segment, require_explicit_goal)?;
        let mut score = self.score()?;
        score.goal_feasible = goal_feasible;
        score.tape_frames = tape_frames;
        score.input_complexity = input_complexity;
        Ok(score)
    }
}

/// Higher is better under `Ord`. The declared axes are feasibility, goal
/// depth, first-hit tick, tape size, input complexity, measured risk, and
/// boundary compatibility, in that exact order. Repeat stability is an
/// evaluator invariant and never a ranking dimension.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct LexicographicScore {
    pub goal_feasible: bool,
    pub milestone_depth: u16,
    pub successes: u32,
    pub attempts: u32,
    pub median_first_hit_tick: u64,
    pub best_first_hit_tick: u64,
    pub tape_frames: u64,
    pub input_complexity: u64,
    pub risk_events: Option<u64>,
    pub boundary_compatibility: BoundaryCompatibility,
}

impl Ord for LexicographicScore {
    fn cmp(&self, other: &Self) -> Ordering {
        self.goal_feasible
            .cmp(&other.goal_feasible)
            .then_with(|| self.milestone_depth.cmp(&other.milestone_depth))
            .then_with(|| other.median_first_hit_tick.cmp(&self.median_first_hit_tick))
            .then_with(|| other.best_first_hit_tick.cmp(&self.best_first_hit_tick))
            .then_with(|| other.tape_frames.cmp(&self.tape_frames))
            .then_with(|| other.input_complexity.cmp(&self.input_complexity))
            .then_with(|| match (self.risk_events, other.risk_events) {
                (Some(left), Some(right)) => right.cmp(&left),
                (Some(_), None) => Ordering::Greater,
                (None, Some(_)) => Ordering::Less,
                (None, None) => Ordering::Equal,
            })
            .then_with(|| {
                self.boundary_compatibility
                    .cmp(&other.boundary_compatibility)
            })
    }
}

impl PartialOrd for LexicographicScore {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct LeaderboardEntry {
    pub rank: usize,
    pub boot: TapeBoot,
    pub candidate_id: String,
    pub frame_count: u64,
    pub score: LexicographicScore,
}

#[derive(Clone, Copy, Debug)]
pub struct EvolutionConfig {
    pub population_size: usize,
    pub elite_count: usize,
    pub rng_seed: u64,
}

pub fn rank_population(
    manifest: &PopulationManifest,
    results: &SearchResults,
) -> Result<Vec<LeaderboardEntry>, SearchError> {
    validate_pair(manifest, results)?;
    let require_explicit_goal = results.schema == RESULTS_SCHEMA;
    let mut rows = manifest
        .members
        .iter()
        .filter_map(|member| {
            results
                .candidates
                .get(&member.candidate_id)
                .map(|result| (member, result))
        })
        .map(|(member, result)| {
            Ok(LeaderboardEntry {
                rank: 0,
                boot: manifest.boot.clone(),
                candidate_id: member.candidate_id.clone(),
                frame_count: member.frame_count,
                score: result.score_for_segment(
                    manifest.segment,
                    require_explicit_goal,
                    member.frame_count,
                    member.input_complexity.unwrap_or(u64::MAX),
                )?,
            })
        })
        .collect::<Result<Vec<_>, SearchError>>()?;
    rows.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.candidate_id.cmp(&right.candidate_id))
    });
    for (index, row) in rows.iter_mut().enumerate() {
        row.rank = index + 1;
    }
    Ok(rows)
}

/// Collects one or more evaluator artifacts into the generation result schema.
/// If trials reached different depths, success rate and ticks are measured at
/// the deepest observed milestone while all trials remain in the denominator.
pub fn collect_results(
    manifest: &PopulationManifest,
    artifacts: impl IntoIterator<Item = EvaluationArtifact>,
) -> Result<SearchResults, SearchError> {
    validate_population_schema(manifest)?;
    let member_ids: HashSet<_> = manifest
        .members
        .iter()
        .map(|member| member.candidate_id.as_str())
        .collect();
    let mut candidates: BTreeMap<String, CandidateResult> = BTreeMap::new();
    for artifact in artifacts {
        if artifact.schema_version != 1 {
            return Err(SearchError::InvalidEvaluationSchema(
                artifact.schema_version,
            ));
        }
        if !member_ids.contains(artifact.candidate_id.as_str()) {
            return Err(SearchError::UnknownCandidate(artifact.candidate_id));
        }
        let mut search_result = artifact.search_result;
        let goal_reached = search_result.validate_for_segment(manifest.segment, false)?;
        search_result.goal_reached = Some(goal_reached);
        match candidates.entry(artifact.candidate_id) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(search_result);
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                let current = entry.get_mut();
                if current.goal_reached != search_result.goal_reached
                    || current.milestone_depth != search_result.milestone_depth
                    || (current.successes == 0) != (search_result.successes == 0)
                    || current.first_hit_ticks.first() != search_result.first_hit_ticks.first()
                {
                    return Err(SearchError::InvalidResult);
                }
                let total_attempts = current
                    .attempts
                    .checked_add(search_result.attempts)
                    .ok_or(SearchError::InvalidResult)?;
                current.successes = current
                    .successes
                    .checked_add(search_result.successes)
                    .ok_or(SearchError::InvalidResult)?;
                current
                    .first_hit_ticks
                    .extend(search_result.first_hit_ticks);
                current.attempts = total_attempts;
                current.validate()?;
            }
        }
    }
    Ok(SearchResults {
        schema: RESULTS_SCHEMA.into(),
        segment: manifest.segment,
        boot: manifest.boot.clone(),
        candidates,
    })
}

pub fn write_seed_population(
    output: &Path,
    base: Candidate,
    population_size: usize,
    rng_seed: u64,
) -> Result<PopulationManifest, SearchError> {
    if population_size == 0 {
        return Err(SearchError::InvalidPopulation);
    }
    base.validate()?;
    let mut rng = SplitMix64::new(rng_seed);
    let mut candidates = vec![base.clone()];
    let mut ids = HashSet::from([base.id()?]);
    let mut attempts = 0;
    while candidates.len() < population_size {
        attempts += 1;
        if attempts > population_size * 100 {
            return Err(SearchError::PopulationStalled);
        }
        let child = mutate(&base, 0, &mut rng)?;
        if ids.insert(child.id()?) {
            candidates.push(child);
        }
    }
    write_population(output, base.segment, 0, rng_seed, candidates)
}

/// Writes an exact, caller-supplied population without applying mutation.
/// This is used by deterministic reducers which construct every candidate.
pub fn write_explicit_population(
    output: &Path,
    segment: SegmentProfile,
    generation: u32,
    candidates: Vec<Candidate>,
) -> Result<PopulationManifest, SearchError> {
    if candidates.is_empty() {
        return Err(SearchError::InvalidPopulation);
    }
    let mut ids = HashSet::new();
    for candidate in &candidates {
        candidate.validate()?;
        if candidate.segment != segment || !ids.insert(candidate.id()?) {
            return Err(SearchError::InvalidPopulation);
        }
    }
    write_population(output, segment, generation, 0, candidates)
}

pub fn evolve_population(
    source_manifest_path: &Path,
    results: &SearchResults,
    output: &Path,
    config: EvolutionConfig,
) -> Result<PopulationManifest, SearchError> {
    evolve_population_with_proposals(source_manifest_path, results, output, config, &[])
}

pub fn evolve_population_with_proposals(
    source_manifest_path: &Path,
    results: &SearchResults,
    output: &Path,
    config: EvolutionConfig,
    proposals: &[Candidate],
) -> Result<PopulationManifest, SearchError> {
    evolve_population_with_retained_and_proposals(
        source_manifest_path,
        results,
        output,
        config,
        &[],
        proposals,
    )
}

pub fn evolve_population_with_retained_and_proposals(
    source_manifest_path: &Path,
    results: &SearchResults,
    output: &Path,
    config: EvolutionConfig,
    retained: &[Candidate],
    proposals: &[Candidate],
) -> Result<PopulationManifest, SearchError> {
    if config.population_size == 0
        || config.elite_count == 0
        || config.elite_count > config.population_size
    {
        return Err(SearchError::InvalidPopulation);
    }
    let source: PopulationManifest = serde_json::from_slice(&fs::read(source_manifest_path)?)?;
    validate_pair(&source, results)?;
    let leaderboard = rank_population(&source, results)?;
    if leaderboard.is_empty() {
        return Err(SearchError::NoEvaluatedCandidates);
    }
    let root = source_manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let member_by_id: BTreeMap<_, _> = source
        .members
        .iter()
        .map(|member| (member.candidate_id.as_str(), member))
        .collect();
    let elite_count = config.elite_count.min(leaderboard.len());
    let mut elites = Vec::with_capacity(elite_count);
    for row in leaderboard.iter().take(elite_count) {
        let member = member_by_id[row.candidate_id.as_str()];
        let candidate: Candidate =
            serde_json::from_slice(&fs::read(root.join(&member.candidate_file))?)?;
        candidate.validate()?;
        elites.push(candidate);
    }
    let generation = source.generation + 1;
    let mut next = elites.clone();
    let mut ids = next
        .iter()
        .map(Candidate::id)
        .collect::<Result<HashSet<_>, _>>()?;
    for candidate in retained {
        if next.len() >= config.population_size {
            break;
        }
        candidate.validate()?;
        if candidate.segment != source.segment || candidate.boot != source.boot {
            return Err(SearchError::InvalidPopulation);
        }
        if ids.insert(candidate.id()?) {
            next.push(candidate.clone());
        }
    }
    for proposal in proposals {
        if next.len() >= config.population_size {
            break;
        }
        proposal.validate()?;
        if proposal.segment != source.segment
            || proposal.boot != source.boot
            || proposal.ancestry.generation != generation
        {
            return Err(SearchError::InvalidPopulation);
        }
        if ids.insert(proposal.id()?) {
            next.push(proposal.clone());
        }
    }
    let mut rng = SplitMix64::new(config.rng_seed);
    let mut attempts = 0;
    while next.len() < config.population_size {
        attempts += 1;
        if attempts > config.population_size * 200 {
            return Err(SearchError::PopulationStalled);
        }
        let parent_index = rng.usize(elites.len());
        let child = mutate(&elites[parent_index], generation, &mut rng)?;
        if ids.insert(child.id()?) {
            next.push(child);
        }
    }
    write_population(output, source.segment, generation, config.rng_seed, next)
}

fn validate_pair(
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

fn write_population(
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

fn validate_population_schema(manifest: &PopulationManifest) -> Result<(), SearchError> {
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

fn validate_results_schema(results: &SearchResults) -> Result<(), SearchError> {
    if results.schema == RESULTS_SCHEMA
        || results.schema == LEGACY_RESULTS_SCHEMA_V2
        || (results.schema == LEGACY_RESULTS_SCHEMA_V1 && results.boot == TapeBoot::Process)
    {
        Ok(())
    } else {
        Err(SearchError::InvalidSchema(results.schema.clone()))
    }
}

fn mutate(
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
        [0, 4, 6, 7, 8][rng.usize(5)]
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
        8 if imported => {
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
            pad.buttons ^= BUTTON_B;
            description = format!("pad_toggle_b[{index}]");
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

fn intervention_range(parent: &InputTape, child: &InputTape) -> Option<InterventionRange> {
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

fn change_duration(action: &mut MacroAction, delta: i32) {
    fn adjusted(value: u32, delta: i32, minimum: u32) -> u32 {
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
        MacroAction::GameTactic { .. } | MacroAction::MotionPath { .. } => {}
    }
}

fn change_angle(action: &mut MacroAction, delta: i32) {
    match action {
        MacroAction::Move { angle_degrees, .. } | MacroAction::Roll { angle_degrees, .. } => {
            *angle_degrees = normalize_angle(i32::from(*angle_degrees) + delta)
        }
        _ => {}
    }
}

fn change_magnitude(action: &mut MacroAction, delta: i32) {
    match action {
        MacroAction::Move { magnitude, .. } | MacroAction::Roll { magnitude, .. } => {
            *magnitude = (i32::from(*magnitude) + delta).clamp(1, 127) as u8
        }
        _ => {}
    }
}

fn normalize_angle(angle: i32) -> i16 {
    (angle + 180).rem_euclid(360) as i16 - 180
}

fn boot_baseline() -> Vec<MacroAction> {
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

fn route_baseline() -> Vec<MacroAction> {
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
fn tunnel_crawl_baseline() -> Vec<MacroAction> {
    // This profile is anchored to an observed suffix and intentionally has no
    // synthetic baseline. Import the promoted absolute tape losslessly with
    // Candidate::from_absolute_tape; validation rejects this empty placeholder
    // if a generic caller attempts to seed without doing so.
    Vec::new()
}

fn validate_magnitude(magnitude: u8) -> Result<(), SearchError> {
    if magnitude <= 127 {
        Ok(())
    } else {
        Err(SearchError::InvalidMagnitude(magnitude))
    }
}

fn validate_duration(frames: u32) -> Result<(), SearchError> {
    if (1..=10_000).contains(&frames) {
        Ok(())
    } else {
        Err(SearchError::InvalidDuration(frames))
    }
}

fn owned_frame(pad: RawPadState) -> InputFrame {
    let mut frame = InputFrame {
        owned_ports: 0x0f,
        ..InputFrame::default()
    };
    frame.pads[0] = pad;
    frame
}

fn imported_frame(pad: RawPadState) -> InputFrame {
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

fn move_frame(angle_degrees: i16, magnitude: u8, buttons: u16) -> InputFrame {
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

fn push_frames(output: &mut Vec<InputFrame>, frame: InputFrame, count: u32) {
    output.extend(std::iter::repeat_n(frame, count as usize));
}

#[derive(Clone, Copy, Debug)]
struct SplitMix64(u64);

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
        value ^ (value >> 31)
    }

    fn usize(&mut self, exclusive_max: usize) -> usize {
        debug_assert!(exclusive_max > 0);
        (self.next() % exclusive_max as u64) as usize
    }

    fn signed(&mut self, inclusive_magnitude: i32) -> i32 {
        let width = u64::try_from(inclusive_magnitude * 2 + 1).unwrap();
        (self.next() % width) as i32 - inclusive_magnitude
    }
}

#[derive(Debug)]
pub enum SearchError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Tape(crate::tape::TapeError),
    InvalidSchema(String),
    InvalidSegment(String),
    SegmentMismatch,
    BootMismatch,
    InvalidActionCount(usize),
    InvalidMagnitude(u8),
    InvalidDuration(u32),
    InvalidButtonPress,
    TooManyFrames,
    InvalidPopulation,
    PopulationStalled,
    InvalidResult,
    InvalidEvaluationSchema(u32),
    UnknownCandidate(String),
    NoEvaluatedCandidates,
    NonCanonicalTape(String),
}

impl fmt::Display for SearchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "search I/O failed: {error}"),
            Self::Json(error) => write!(formatter, "invalid search JSON: {error}"),
            Self::Tape(error) => write!(formatter, "could not encode candidate tape: {error}"),
            Self::InvalidSchema(schema) => {
                write!(formatter, "unsupported search schema {schema:?}")
            }
            Self::InvalidSegment(segment) => {
                write!(formatter, "unknown search segment {segment:?}")
            }
            Self::SegmentMismatch => formatter.write_str("population and results segment mismatch"),
            Self::BootMismatch => {
                formatter.write_str("population and results boot origin mismatch")
            }
            Self::InvalidActionCount(count) => {
                write!(formatter, "invalid candidate action count {count}")
            }
            Self::InvalidMagnitude(value) => {
                write!(formatter, "stick magnitude {value} exceeds 127")
            }
            Self::InvalidDuration(value) => write!(formatter, "invalid macro duration {value}"),
            Self::InvalidButtonPress => formatter.write_str("invalid button press macro"),
            Self::TooManyFrames => formatter.write_str("candidate expands beyond the frame limit"),
            Self::InvalidPopulation => formatter.write_str("invalid population/elite count"),
            Self::PopulationStalled => {
                formatter.write_str("could not generate a unique population")
            }
            Self::InvalidResult => formatter.write_str("candidate result counts are inconsistent"),
            Self::InvalidEvaluationSchema(version) => {
                write!(formatter, "unsupported evaluator schema version {version}")
            }
            Self::UnknownCandidate(id) => {
                write!(formatter, "evaluation references unknown candidate {id}")
            }
            Self::NoEvaluatedCandidates => {
                formatter.write_str("results contain no population candidates")
            }
            Self::NonCanonicalTape(message) => {
                write!(formatter, "raw tape cannot be inferred safely: {message}")
            }
        }
    }
}

impl Error for SearchError {}

impl From<std::io::Error> for SearchError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for SearchError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl From<crate::tape::TapeError> for SearchError {
    fn from(value: crate::tape::TapeError) -> Self {
        Self::Tape(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macro_ir_compiles_analog_roll_and_press() {
        let candidate = Candidate {
            schema: CANDIDATE_SCHEMA.into(),
            segment: SegmentProfile::Fsp103ToFsp104,
            boot: TapeBoot::Process,
            actions: vec![
                MacroAction::Move {
                    angle_degrees: 90,
                    magnitude: 127,
                    frames: 2,
                },
                MacroAction::Roll {
                    angle_degrees: 0,
                    magnitude: 100,
                    button_frame: 1,
                    recovery_frames: 2,
                    spacing: RollSpacing {
                        period_ticks: 4,
                        phase_tick: 3,
                    },
                },
                MacroAction::Press {
                    buttons: vec![ControllerButton::Start],
                    hold_frames: 1,
                    neutral_frames: 1,
                },
            ],
            ancestry: Ancestry::default(),
        };
        let tape = candidate.compile().unwrap();
        assert_eq!(tape.frames.len(), 8);
        assert_eq!(tape.frames[0].pads[0].stick_x, 127);
        assert_eq!(tape.frames[0].pads[0].stick_y, 0);
        assert_eq!(tape.frames[2].pads[0].buttons, 0);
        assert_eq!(tape.frames[3].pads[0].buttons, BUTTON_B);
        assert_eq!(tape.frames[3].pads[0].stick_y, 100);
        assert_eq!(tape.frames[6].pads[0].buttons, BUTTON_START);
        assert_eq!(tape.frames[7].pads[0].buttons, 0);

        let mut wrong_phase = candidate.clone();
        let MacroAction::Roll { spacing, .. } = &mut wrong_phase.actions[1] else {
            unreachable!()
        };
        spacing.phase_tick = 2;
        assert!(matches!(
            wrong_phase.compile(),
            Err(SearchError::NonCanonicalTape(_))
        ));
    }

    #[test]
    fn legacy_roll_json_defaults_to_first_frame_and_unconstrained_phase() {
        let action: MacroAction = serde_json::from_str(
            r#"{"op":"roll","angle_degrees":0,"magnitude":100,"recovery_frames":2}"#,
        )
        .unwrap();
        assert_eq!(
            action,
            MacroAction::Roll {
                angle_degrees: 0,
                magnitude: 100,
                button_frame: 0,
                recovery_frames: 2,
                spacing: RollSpacing::default(),
            }
        );
    }

    #[test]
    fn typed_game_tactic_is_a_first_class_static_search_macro() {
        let plan = GameTacticPlan::new(crate::game_tactic::GameTactic::Crawl {
            direction_degrees: 90,
            magnitude: 80,
            frames: 3,
            action_held: true,
        });
        let candidate = Candidate {
            schema: CANDIDATE_SCHEMA.into(),
            segment: SegmentProfile::LinkControlToTunnelCrawlStart,
            boot: TapeBoot::Process,
            actions: vec![MacroAction::GameTactic { plan: plan.clone() }],
            ancestry: Ancestry::default(),
        };
        let tape = candidate.compile().unwrap();
        assert_eq!(tape.frames.len(), 3);
        assert_eq!(tape.frames[0].pads[0].stick_x, 80);
        assert_eq!(tape.frames[0].pads[0].buttons, BUTTON_A);

        let mut reactive = plan;
        reactive.cancellation_conditions =
            vec![crate::option_execution::OptionCondition::TargetLost {
                target: "crawl-entry".into(),
            }];
        let candidate = Candidate {
            actions: vec![MacroAction::GameTactic { plan: reactive }],
            ..candidate
        };
        assert!(matches!(
            candidate.compile(),
            Err(SearchError::NonCanonicalTape(_))
        ));
    }

    #[test]
    fn exact_motion_path_is_a_first_class_static_search_macro() {
        use crate::motion_path::{SamplePhase, StickPath, StickPoint};
        let plan = MotionPathPlan {
            schema: crate::motion_path::MOTION_PATH_SCHEMA_V1.into(),
            path: StickPath::Bezier {
                control: [
                    StickPoint { x: 0, y: 0 },
                    StickPoint { x: 0, y: 8 },
                    StickPoint { x: 8, y: 8 },
                    StickPoint { x: 8, y: 0 },
                ],
            },
            duration_ticks: 2,
            sample_phase: SamplePhase {
                numerator: 1,
                denominator: 1,
            },
            cancellation_conditions: Vec::new(),
        };
        let candidate = Candidate {
            schema: CANDIDATE_SCHEMA.into(),
            segment: SegmentProfile::Fsp103ToFsp104,
            boot: TapeBoot::Process,
            actions: vec![MacroAction::MotionPath { plan }],
            ancestry: Ancestry::default(),
        };
        let tape = candidate.compile().unwrap();
        assert_eq!(tape.frames.len(), 2);
        assert_eq!(
            (
                tape.frames[0].pads[0].stick_x,
                tape.frames[0].pads[0].stick_y
            ),
            (4, 6)
        );
        assert_eq!(
            (
                tape.frames[1].pads[0].stick_x,
                tape.frames[1].pads[0].stick_y
            ),
            (8, 0)
        );
    }

    #[test]
    fn absolute_tape_inference_keeps_route_analog_but_boot_rejects_it() {
        let source = Candidate::baseline(SegmentProfile::BootToFsp103)
            .compile()
            .unwrap();
        let imported =
            Candidate::from_absolute_tape(SegmentProfile::BootToFsp103, &source).unwrap();
        assert_eq!(imported.compile().unwrap(), source);
        assert!(
            imported
                .actions
                .iter()
                .any(|action| matches!(action, MacroAction::Press { .. }))
        );

        let mut analog = Candidate::baseline(SegmentProfile::Fsp103ToFsp104)
            .compile()
            .unwrap();
        let disconnected = RawPadState {
            connected: false,
            error: -1,
            ..RawPadState::default()
        };
        for frame in &mut analog.frames {
            frame.owned_ports = 0x01;
            frame.pads[1..].fill(disconnected);
        }
        let imported_route =
            Candidate::from_absolute_tape(SegmentProfile::Fsp103ToFsp104, &analog).unwrap();
        assert_eq!(imported_route.compile().unwrap(), analog);
        assert!(matches!(
            Candidate::from_absolute_tape(SegmentProfile::BootToFsp103, &analog),
            Err(SearchError::NonCanonicalTape(_))
        ));

        let long_hold = Candidate {
            schema: CANDIDATE_SCHEMA.into(),
            segment: SegmentProfile::BootToFsp103,
            boot: TapeBoot::Process,
            actions: vec![
                MacroAction::Press {
                    buttons: vec![ControllerButton::A],
                    hold_frames: 30,
                    neutral_frames: 0,
                },
                MacroAction::Press {
                    buttons: vec![ControllerButton::A],
                    hold_frames: 30,
                    neutral_frames: 1,
                },
            ],
            ancestry: Ancestry::default(),
        }
        .compile()
        .unwrap();
        assert_eq!(
            Candidate::from_absolute_tape(SegmentProfile::BootToFsp103, &long_hold)
                .unwrap()
                .compile()
                .unwrap(),
            long_hold
        );
    }

    #[test]
    fn promoted_tunnel_suffix_imports_losslessly_as_compact_pad_runs() {
        let disconnected = RawPadState {
            connected: false,
            error: -1,
            ..RawPadState::default()
        };
        let tape = InputTape {
            frames: (0..421)
                .map(|index| {
                    let mut frame = InputFrame {
                        owned_ports: 1,
                        pads: [disconnected; 4],
                        ..InputFrame::default()
                    };
                    frame.pads[0] = RawPadState::default();
                    frame.pads[0].stick_y = if index < 200 {
                        96
                    } else if index < 400 {
                        127
                    } else {
                        0
                    };
                    frame
                })
                .collect(),
            ..InputTape::default()
        };
        assert_eq!(tape.frames.len(), 421);
        let candidate =
            Candidate::from_absolute_tape(SegmentProfile::LinkControlToTunnelCrawlStart, &tape)
                .unwrap();
        assert_eq!(candidate.compile().unwrap(), tape);
        assert!(candidate.actions.len() < tape.frames.len());
        assert!(
            candidate
                .actions
                .iter()
                .all(|action| matches!(action, MacroAction::PadRun { .. }))
        );
        assert!(
            Candidate::baseline(SegmentProfile::LinkControlToTunnelCrawlStart)
                .validate()
                .is_err()
        );

        let mut rng = SplitMix64::new(0x7a5_2026);
        let mut stick_mutations = 0;
        let mut button_mutations = 0;
        for _ in 0..256 {
            let child = mutate(&candidate, 1, &mut rng).unwrap();
            let mutation = child.ancestry.mutation.as_deref().unwrap();
            stick_mutations += usize::from(mutation.starts_with("pad_stick["));
            button_mutations += usize::from(mutation.starts_with("pad_toggle_b["));
        }
        assert!(stick_mutations > 20);
        assert!(button_mutations > 20);
    }

    #[test]
    fn boot_mutation_directly_targets_press_gaps() {
        let parent = Candidate::baseline(SegmentProfile::BootToFsp103);
        let mut rng = SplitMix64::new(0x5eed);
        let mut gap_mutations = 0;
        let mut shrink_mutations = 0;
        for _ in 0..256 {
            let child = mutate(&parent, 0, &mut rng).unwrap();
            let mutation = child.ancestry.mutation.as_deref().unwrap();
            if mutation.starts_with("boot_gap[") || mutation.starts_with("boot_shrink[") {
                let changed: Vec<_> = parent
                    .actions
                    .iter()
                    .zip(&child.actions)
                    .enumerate()
                    .filter(|(_, (before, after))| before != after)
                    .collect();
                assert!(changed.len() <= 1);
                if let Some((_, (_, action))) = changed.first() {
                    assert!(matches!(action, MacroAction::Press { .. }));
                }
            }
            gap_mutations += usize::from(mutation.starts_with("boot_gap["));
            shrink_mutations += usize::from(mutation.starts_with("boot_shrink["));
        }
        assert!(gap_mutations > 50);
        assert!(shrink_mutations > 50);
    }

    #[test]
    fn score_is_depth_then_tick_and_fractional_repeats_are_invalid() {
        let score = |depth, successes, attempts, ticks| {
            CandidateResult {
                goal_reached: Some(depth == 4),
                milestone_depth: depth,
                attempts,
                successes,
                first_hit_ticks: ticks,
                risk_events: None,
                boundary_compatibility: BoundaryCompatibility::Unknown,
            }
            .score()
            .unwrap()
        };
        assert!(score(4, 10, 10, vec![500; 10]) > score(3, 10, 10, vec![1; 10]));
        assert!(score(4, 10, 10, vec![99; 10]) > score(4, 10, 10, vec![100; 10]));
        let feasible = CandidateResult {
            goal_reached: Some(true),
            milestone_depth: 1,
            attempts: 1,
            successes: 1,
            first_hit_ticks: vec![10_000],
            risk_events: None,
            boundary_compatibility: BoundaryCompatibility::Unknown,
        }
        .score()
        .unwrap();
        let infeasible = CandidateResult {
            goal_reached: Some(false),
            milestone_depth: u16::MAX,
            attempts: 1,
            successes: 1,
            first_hit_ticks: vec![1],
            risk_events: None,
            boundary_compatibility: BoundaryCompatibility::Unknown,
        }
        .score()
        .unwrap();
        assert!(feasible > infeasible);
        assert!(matches!(
            CandidateResult {
                goal_reached: Some(true),
                milestone_depth: 4,
                attempts: 10,
                successes: 9,
                first_hit_ticks: vec![500; 9],
                risk_events: None,
                boundary_compatibility: BoundaryCompatibility::Unknown,
            }
            .score(),
            Err(SearchError::InvalidResult)
        ));
    }

    #[test]
    fn lexicographic_score_uses_every_declared_axis_in_order() {
        let score = LexicographicScore {
            goal_feasible: true,
            milestone_depth: 4,
            successes: 3,
            attempts: 3,
            median_first_hit_tick: 100,
            best_first_hit_tick: 100,
            tape_frames: 120,
            input_complexity: 12,
            risk_events: Some(2),
            boundary_compatibility: BoundaryCompatibility::Compatible,
        };

        assert!(
            score
                > LexicographicScore {
                    goal_feasible: false,
                    milestone_depth: u16::MAX,
                    median_first_hit_tick: 0,
                    best_first_hit_tick: 0,
                    tape_frames: 0,
                    input_complexity: 0,
                    risk_events: Some(0),
                    boundary_compatibility: BoundaryCompatibility::Exact,
                    ..score
                }
        );
        assert!(
            score
                > LexicographicScore {
                    milestone_depth: 3,
                    median_first_hit_tick: 0,
                    best_first_hit_tick: 0,
                    tape_frames: 0,
                    input_complexity: 0,
                    risk_events: Some(0),
                    boundary_compatibility: BoundaryCompatibility::Exact,
                    ..score
                }
        );
        assert!(
            score
                > LexicographicScore {
                    median_first_hit_tick: 101,
                    best_first_hit_tick: 0,
                    tape_frames: 0,
                    input_complexity: 0,
                    risk_events: Some(0),
                    boundary_compatibility: BoundaryCompatibility::Exact,
                    ..score
                }
        );
        assert!(
            score
                > LexicographicScore {
                    tape_frames: 121,
                    input_complexity: 0,
                    risk_events: Some(0),
                    boundary_compatibility: BoundaryCompatibility::Exact,
                    ..score
                }
        );
        assert!(
            score
                > LexicographicScore {
                    input_complexity: 13,
                    risk_events: Some(0),
                    boundary_compatibility: BoundaryCompatibility::Exact,
                    ..score
                }
        );
        assert!(
            score
                > LexicographicScore {
                    risk_events: Some(3),
                    boundary_compatibility: BoundaryCompatibility::Exact,
                    ..score
                }
        );
        assert!(
            score
                > LexicographicScore {
                    risk_events: None,
                    boundary_compatibility: BoundaryCompatibility::Exact,
                    ..score
                }
        );
        assert!(
            score
                > LexicographicScore {
                    boundary_compatibility: BoundaryCompatibility::Unknown,
                    ..score
                }
        );
        assert!(
            LexicographicScore {
                boundary_compatibility: BoundaryCompatibility::Exact,
                ..score
            } > score
        );
        assert!(
            LexicographicScore {
                boundary_compatibility: BoundaryCompatibility::Unknown,
                ..score
            } > LexicographicScore {
                boundary_compatibility: BoundaryCompatibility::Incompatible,
                ..score
            }
        );
    }

    #[test]
    fn input_complexity_counts_native_transitions_after_compilation() {
        let mut first = InputFrame {
            owned_ports: 0b0101,
            wait_condition: crate::tape::WaitCondition::NameEntryActive,
            wait_timeout_ticks: 20,
            ..InputFrame::default()
        };
        first.pads[0].buttons = 0b0101;
        first.pads[0].stick_x = 1;
        first.pads[0].analog_a = 2;
        first.pads[1].connected = false;
        first.pads[1].error = -1;

        let mut second = first.clone();
        second.owned_ports = 0b0001;
        second.pads[0].buttons = 0b0110;
        second.pads[0].trigger_left = 1;
        let tape = InputTape {
            frames: vec![first, second.clone(), second],
            ..InputTape::default()
        };
        assert_eq!(tape_input_complexity(&tape), 14);
        let decoded = InputTape::decode(&tape.encode().unwrap()).unwrap().tape;
        assert_eq!(tape_input_complexity(&decoded), 14);

        let one_run = Candidate {
            schema: CANDIDATE_SCHEMA.into(),
            segment: SegmentProfile::BootToFsp103,
            boot: TapeBoot::Process,
            actions: vec![MacroAction::Move {
                angle_degrees: 0,
                magnitude: 127,
                frames: 2,
            }],
            ancestry: Ancestry::default(),
        };
        let split_run = Candidate {
            actions: vec![
                MacroAction::Move {
                    angle_degrees: 0,
                    magnitude: 127,
                    frames: 1,
                },
                MacroAction::Move {
                    angle_degrees: 0,
                    magnitude: 127,
                    frames: 1,
                },
            ],
            ..one_run.clone()
        };
        let compiled = one_run.compile().unwrap();
        let split_compiled = split_run.compile().unwrap();
        assert_eq!(compiled, split_compiled);
        assert_eq!(
            tape_input_complexity(&compiled),
            tape_input_complexity(&split_compiled)
        );
    }

    #[test]
    fn population_v3_requires_complexity_while_legacy_populations_remain_readable() {
        let candidate = Candidate::baseline(SegmentProfile::BootToFsp103);
        let member = PopulationMember {
            candidate_id: candidate.id().unwrap(),
            candidate_file: PathBuf::from("candidate.json"),
            tape_file: PathBuf::from("candidate.tape"),
            frame_count: candidate.frame_count(),
            input_complexity: None,
            ancestry: Ancestry::default(),
        };
        let manifest = PopulationManifest {
            schema: POPULATION_SCHEMA.into(),
            segment: candidate.segment,
            boot: candidate.boot,
            generation: 0,
            rng_seed: 1,
            members: vec![member],
        };
        assert!(matches!(
            validate_population_schema(&manifest),
            Err(SearchError::InvalidPopulation)
        ));

        let legacy_v2 = PopulationManifest {
            schema: LEGACY_POPULATION_SCHEMA_V2.into(),
            ..manifest.clone()
        };
        assert!(validate_population_schema(&legacy_v2).is_ok());
        let legacy_v1 = PopulationManifest {
            schema: LEGACY_POPULATION_SCHEMA_V1.into(),
            ..manifest
        };
        assert!(validate_population_schema(&legacy_v1).is_ok());
    }

    #[test]
    fn current_results_require_an_explicit_consistent_goal_verdict() {
        let candidate = Candidate::baseline(SegmentProfile::BootToFsp103);
        let candidate_id = candidate.id().unwrap();
        let manifest = PopulationManifest {
            schema: POPULATION_SCHEMA.into(),
            segment: candidate.segment,
            boot: candidate.boot.clone(),
            generation: 0,
            rng_seed: 1,
            members: vec![PopulationMember {
                candidate_id: candidate_id.clone(),
                candidate_file: PathBuf::from("candidate.json"),
                tape_file: PathBuf::from("candidate.tape"),
                frame_count: candidate.frame_count(),
                input_complexity: Some(0),
                ancestry: Ancestry::default(),
            }],
        };
        let result = |goal_reached, depth| SearchResults {
            schema: RESULTS_SCHEMA.into(),
            segment: manifest.segment,
            boot: manifest.boot.clone(),
            candidates: BTreeMap::from([(
                candidate_id.clone(),
                CandidateResult {
                    goal_reached,
                    milestone_depth: depth,
                    attempts: 1,
                    successes: u32::from(depth != 0),
                    first_hit_ticks: (depth != 0).then_some(10).into_iter().collect(),
                    risk_events: None,
                    boundary_compatibility: BoundaryCompatibility::Unknown,
                },
            )]),
        };
        assert!(matches!(
            rank_population(&manifest, &result(None, 2)),
            Err(SearchError::InvalidResult)
        ));
        assert!(matches!(
            rank_population(&manifest, &result(Some(true), 0)),
            Err(SearchError::InvalidResult)
        ));
        assert!(rank_population(&manifest, &result(Some(true), 2)).is_ok());
    }

    #[test]
    fn evaluator_trials_reject_disagreement() {
        let candidate = Candidate::baseline(SegmentProfile::Fsp103ToFsp104);
        let candidate_id = candidate.id().unwrap();
        let manifest = PopulationManifest {
            schema: POPULATION_SCHEMA.into(),
            segment: candidate.segment,
            boot: candidate.boot.clone(),
            generation: 0,
            rng_seed: 1,
            members: vec![PopulationMember {
                candidate_id: candidate_id.clone(),
                candidate_file: PathBuf::from("candidate.json"),
                tape_file: PathBuf::from("candidate.tape"),
                frame_count: candidate.frame_count(),
                input_complexity: Some(0),
                ancestry: Ancestry::default(),
            }],
        };
        let artifact = |depth, tick| EvaluationArtifact {
            schema_version: 1,
            candidate_id: candidate_id.clone(),
            search_result: CandidateResult {
                goal_reached: None,
                milestone_depth: depth,
                attempts: 1,
                successes: 1,
                first_hit_ticks: vec![tick],
                risk_events: None,
                boundary_compatibility: BoundaryCompatibility::Unknown,
            },
        };
        assert!(matches!(
            collect_results(&manifest, [artifact(3, 570), artifact(4, 603)]),
            Err(SearchError::InvalidResult)
        ));
    }

    #[test]
    fn population_results_and_leaderboard_are_partitioned_by_boot_origin() {
        let root = std::env::temp_dir().join(format!(
            "huntctl-search-boot-partition-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let stage = Candidate::baseline(SegmentProfile::Fsp103ToFsp104);
        let manifest =
            write_explicit_population(&root, stage.segment, 0, vec![stage.clone()]).unwrap();
        assert_eq!(manifest.boot, stage.boot);

        let candidate_id = manifest.members[0].candidate_id.clone();
        let results = SearchResults {
            schema: RESULTS_SCHEMA.into(),
            segment: manifest.segment,
            boot: manifest.boot.clone(),
            candidates: BTreeMap::from([(
                candidate_id.clone(),
                CandidateResult {
                    goal_reached: Some(false),
                    milestone_depth: 1,
                    attempts: 2,
                    successes: 2,
                    first_hit_ticks: vec![42, 42],
                    risk_events: None,
                    boundary_compatibility: BoundaryCompatibility::Unknown,
                },
            )]),
        };
        let leaderboard = rank_population(&manifest, &results).unwrap();
        assert_eq!(leaderboard[0].boot, manifest.boot);

        let process_results = SearchResults {
            boot: TapeBoot::Process,
            ..results.clone()
        };
        assert!(matches!(
            rank_population(&manifest, &process_results),
            Err(SearchError::BootMismatch)
        ));

        let mut process = stage.clone();
        process.boot = TapeBoot::Process;
        assert!(matches!(
            write_explicit_population(&root.join("mixed"), stage.segment, 0, vec![stage, process],),
            Err(SearchError::BootMismatch)
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn seeded_evolution_is_reproducible_and_keeps_champion() {
        let root = std::env::temp_dir().join(format!("huntctl-search-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let first = write_seed_population(
            &root.join("g0"),
            Candidate::baseline(SegmentProfile::Fsp103ToFsp104),
            8,
            42,
        )
        .unwrap();
        let champion = first.members[3].candidate_id.clone();
        let candidates = first
            .members
            .iter()
            .enumerate()
            .map(|(index, member)| {
                (
                    member.candidate_id.clone(),
                    CandidateResult {
                        goal_reached: Some(index == 3),
                        milestone_depth: if index == 3 { 4 } else { 3 },
                        attempts: 2,
                        successes: 2,
                        first_hit_ticks: vec![100 + index as u64; 2],
                        risk_events: None,
                        boundary_compatibility: BoundaryCompatibility::Unknown,
                    },
                )
            })
            .collect();
        let results = SearchResults {
            schema: RESULTS_SCHEMA.into(),
            segment: first.segment,
            boot: first.boot.clone(),
            candidates,
        };
        let config = EvolutionConfig {
            population_size: 8,
            elite_count: 2,
            rng_seed: 99,
        };
        let next = evolve_population(
            &root.join("g0/manifest.json"),
            &results,
            &root.join("g1"),
            config,
        )
        .unwrap();
        let again = evolve_population(
            &root.join("g0/manifest.json"),
            &results,
            &root.join("g1-again"),
            config,
        )
        .unwrap();
        assert_eq!(next.members[0].candidate_id, champion);
        assert_eq!(
            next.members
                .iter()
                .map(|member| &member.candidate_id)
                .collect::<Vec<_>>(),
            again
                .members
                .iter()
                .map(|member| &member.candidate_id)
                .collect::<Vec<_>>()
        );
        assert!(next.members.iter().skip(2).all(|member| {
            member.ancestry.generation == 1 && member.ancestry.parent_id.is_some()
        }));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn evolution_retains_an_exact_archived_candidate_before_new_proposals() {
        let root =
            std::env::temp_dir().join(format!("huntctl-search-retained-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let first = write_seed_population(
            &root.join("g0"),
            Candidate::baseline(SegmentProfile::Fsp103ToFsp104),
            6,
            121,
        )
        .unwrap();
        let retained_member = &first.members[5];
        let retained: Candidate = serde_json::from_slice(
            &fs::read(root.join("g0").join(&retained_member.candidate_file)).unwrap(),
        )
        .unwrap();
        let results = SearchResults {
            schema: RESULTS_SCHEMA.into(),
            segment: first.segment,
            boot: first.boot.clone(),
            candidates: first
                .members
                .iter()
                .enumerate()
                .map(|(index, member)| {
                    (
                        member.candidate_id.clone(),
                        CandidateResult {
                            goal_reached: Some(true),
                            milestone_depth: 2,
                            attempts: 1,
                            successes: 1,
                            first_hit_ticks: vec![100 + index as u64],
                            risk_events: None,
                            boundary_compatibility: BoundaryCompatibility::Unknown,
                        },
                    )
                })
                .collect(),
        };
        let next = evolve_population_with_retained_and_proposals(
            &root.join("g0/manifest.json"),
            &results,
            &root.join("g1"),
            EvolutionConfig {
                population_size: 6,
                elite_count: 1,
                rng_seed: 122,
            },
            std::slice::from_ref(&retained),
            &[],
        )
        .unwrap();
        assert_eq!(next.members[1].candidate_id, retained.id().unwrap());
        assert_eq!(next.members[1].ancestry, retained.ancestry);
        fs::remove_dir_all(root).unwrap();
    }
}
