//! Finite-sample search primitives for milestone-backed TAS optimization.

use dusklight_automation_contracts::tape::{InputFrame, InputTape, RawPadState, TapeBoot};
use dusklight_control::game_tactic::GameTacticPlan;
use dusklight_control::motion_path::{
    MAX_PATH_POINTS, MotionPathPlan, SamplePhase, StickPath, StickPoint,
};
use dusklight_control::roll_option::{RollOptionPlan, RollSpacing};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

mod evolution;
pub use evolution::tape_input_complexity;
use evolution::*;

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
/// +90 is right. A roll presses the GameCube A action button on its declared
/// frame and holds its stick for the requested recovery frames.
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
    /// Static motion path with the exact port-zero ownership convention used
    /// by imported anchored suffixes. Keeping this distinct preserves the
    /// established four-port MotionPath identity and direct-stage behavior.
    PortOneMotionPath {
        plan: MotionPathPlan,
    },
    /// Lossless run-length encoded port-zero state used to import an observed
    /// absolute movement tape without quantizing its analog samples.
    PadRun {
        pad: SearchPadState,
        frames: u32,
        /// Exact ownership mask for an imported frame. `None` retains the
        /// canonical port-one-only representation.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        imported_owned_ports: Option<u8>,
        /// Exact secondary-port states for imported frames whose other ports
        /// do not use the canonical disconnected representation.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port_one_secondary_pads: Option<[RawPadState; 3]>,
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
            Self::MotionPath { plan } | Self::PortOneMotionPath { plan } => {
                u64::from(plan.duration_ticks)
            }
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

/// Return the smallest differing parent/child frame interval for compatible
/// absolute tapes. Identical tapes and tapes with different boot/rate identity
/// have no portable intervention.
pub fn tape_intervention(parent: &InputTape, child: &InputTape) -> Option<InterventionRange> {
    if parent.boot != child.boot
        || parent.tick_rate_numerator != child.tick_rate_numerator
        || parent.tick_rate_denominator != child.tick_rate_denominator
    {
        return None;
    }
    let shared = parent.frames.len().min(child.frames.len());
    let start = (0..shared)
        .find(|index| parent.frames[*index] != child.frames[*index])
        .or_else(|| (parent.frames.len() != child.frames.len()).then_some(shared))?;
    let mut parent_end = parent.frames.len();
    let mut child_end = child.frames.len();
    while parent_end > start
        && child_end > start
        && parent.frames[parent_end - 1] == child.frames[child_end - 1]
    {
        parent_end -= 1;
        child_end -= 1;
    }
    Some(InterventionRange {
        start_frame: start as u64,
        end_frame_exclusive: child_end as u64,
        parent_end_frame_exclusive: parent_end as u64,
    })
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
                MacroAction::MotionPath { plan } | MacroAction::PortOneMotionPath { plan } => {
                    if !plan.cancellation_conditions.is_empty() {
                        return Err(SearchError::NonCanonicalTape(
                            "static search paths cannot declare reactive cancellation conditions"
                                .into(),
                        ));
                    }
                    plan.validate()
                        .map_err(|error| SearchError::NonCanonicalTape(error.to_string()))?;
                }
                MacroAction::PadRun {
                    frames,
                    imported_owned_ports,
                    ..
                } => {
                    validate_duration(*frames)?;
                    if imported_owned_ports.is_some_and(|owned| owned & 1 == 0) {
                        return Err(SearchError::NonCanonicalTape(
                            "imported PAD run must retain primary-port ownership".into(),
                        ));
                    }
                }
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
                MacroAction::PortOneMotionPath { plan } => {
                    let realization = plan
                        .realize(None)
                        .map_err(|error| SearchError::NonCanonicalTape(error.to_string()))?;
                    frames.extend(
                        realization
                            .frames
                            .into_iter()
                            .map(|frame| imported_frame(frame.pads[0])),
                    );
                }
                MacroAction::PadRun {
                    pad,
                    frames: count,
                    imported_owned_ports,
                    port_one_secondary_pads,
                } => {
                    let mut frame = imported_frame((*pad).into());
                    if let Some(owned_ports) = imported_owned_ports {
                        frame.owned_ports = *owned_ports;
                    }
                    if let Some(secondary) = port_one_secondary_pads {
                        frame.pads[1..].copy_from_slice(secondary);
                    }
                    push_frames(&mut frames, frame, *count)
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

    /// Losslessly lifts ordinary movement-only spans into typed waypoint
    /// paths while retaining button or otherwise non-path PAD states as exact
    /// runs. This gives continuous refinement semantic duration and path-point
    /// axes without changing the compiled native input or inventing a second
    /// candidate format.
    pub fn from_semantic_movement_tape(
        segment: SegmentProfile,
        tape: &InputTape,
    ) -> Result<Self, SearchError> {
        tape.validate()?;
        if !matches!(
            segment,
            SegmentProfile::Fsp103ToFsp104 | SegmentProfile::LinkControlToTunnelCrawlStart
        ) {
            return Err(SearchError::NonCanonicalTape(
                "semantic motion-path import requires an anchored movement segment".into(),
            ));
        }
        if tape.tick_rate_numerator != 30
            || tape.tick_rate_denominator != 1
            || tape.frames.is_empty()
        {
            return Err(SearchError::NonCanonicalTape(
                "semantic motion-path import requires a nonempty 30/1 tape".into(),
            ));
        }
        let disconnected = RawPadState {
            connected: false,
            error: -1,
            ..RawPadState::default()
        };
        for frame in &tape.frames {
            if frame.owned_ports != 0x01
                || frame.wait_condition != crate::tape::WaitCondition::None
                || frame.wait_timeout_ticks != 0
                || frame.pads[1..] != [disconnected; 3]
            {
                return Err(SearchError::NonCanonicalTape(
                    "semantic motion-path imports require absolute port-one ownership, no reactive waits, and canonical disconnected secondary ports"
                        .into(),
                ));
            }
        }

        let is_path_frame = |frame: &InputFrame| {
            let pad = frame.pads[0];
            frame.pads[1..] == [disconnected; 3]
                && pad
                    == RawPadState {
                        stick_x: pad.stick_x,
                        stick_y: pad.stick_y,
                        ..RawPadState::default()
                    }
        };
        let secondary_payload = |frame: &InputFrame| {
            (frame.pads[1..] != [disconnected; 3])
                .then(|| [frame.pads[1], frame.pads[2], frame.pads[3]])
        };
        let mut actions = Vec::new();
        let mut index = 0;
        while index < tape.frames.len() {
            if is_path_frame(&tape.frames[index]) {
                let start = index;
                while index < tape.frames.len()
                    && is_path_frame(&tape.frames[index])
                    && index - start < MAX_PATH_POINTS
                {
                    index += 1;
                }
                let points = tape.frames[start..index]
                    .iter()
                    .map(|frame| StickPoint {
                        x: i16::from(frame.pads[0].stick_x),
                        y: i16::from(frame.pads[0].stick_y),
                    })
                    .collect::<Vec<_>>();
                actions.push(MacroAction::PortOneMotionPath {
                    plan: MotionPathPlan {
                        schema: crate::motion_path::MOTION_PATH_SCHEMA_V1.into(),
                        duration_ticks: points.len() as u32,
                        path: StickPath::Waypoint { points },
                        sample_phase: SamplePhase::default(),
                        cancellation_conditions: Vec::new(),
                    },
                });
                continue;
            }

            let pad = SearchPadState::from(tape.frames[index].pads[0]);
            let secondary_pads = secondary_payload(&tape.frames[index]);
            let start = index;
            while index < tape.frames.len()
                && tape.frames[index].pads[0] == RawPadState::from(pad)
                && secondary_payload(&tape.frames[index]) == secondary_pads
                && !is_path_frame(&tape.frames[index])
            {
                index += 1;
            }
            actions.push(MacroAction::PadRun {
                pad,
                frames: (index - start) as u32,
                imported_owned_ports: None,
                port_one_secondary_pads: secondary_pads,
            });
        }
        let candidate = Self {
            schema: CANDIDATE_SCHEMA.into(),
            segment,
            boot: tape.boot.clone(),
            actions,
            ancestry: Ancestry {
                generation: 0,
                parent_id: None,
                mutation: Some("lossless semantic motion-path import".into()),
                intervention: None,
            },
        };
        candidate.validate()?;
        if candidate.compile()? != *tape {
            return Err(SearchError::NonCanonicalTape(
                "semantic motion-path import did not reproduce the source tape exactly".into(),
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
        let mut runs: Vec<(SearchPadState, Option<u8>, Option<[RawPadState; 3]>, u32)> = Vec::new();
        for frame in &tape.frames {
            if frame.owned_ports & 0x01 == 0
                || frame.wait_condition != crate::tape::WaitCondition::None
                || frame.wait_timeout_ticks != 0
            {
                return Err(SearchError::NonCanonicalTape(
                    "anchored movement imports require absolute port-one ownership and no reactive waits"
                        .into(),
                ));
            }
            let pad = SearchPadState::from(frame.pads[0]);
            let imported_owned_ports = (frame.owned_ports != 0x01).then_some(frame.owned_ports);
            let secondary_pads = (frame.pads[1..] != [disconnected; 3])
                .then(|| [frame.pads[1], frame.pads[2], frame.pads[3]]);
            if let Some((last, last_owned_ports, last_secondary_pads, frames)) = runs.last_mut()
                && *last == pad
                && *last_owned_ports == imported_owned_ports
                && *last_secondary_pads == secondary_pads
            {
                *frames = frames.checked_add(1).ok_or(SearchError::TooManyFrames)?;
            } else {
                runs.push((pad, imported_owned_ports, secondary_pads, 1));
            }
        }
        let candidate = Self {
            schema: CANDIDATE_SCHEMA.into(),
            segment,
            boot: tape.boot.clone(),
            actions: runs
                .into_iter()
                .map(
                    |(pad, imported_owned_ports, port_one_secondary_pads, frames)| {
                        MacroAction::PadRun {
                            pad,
                            frames,
                            imported_owned_ports,
                            port_one_secondary_pads,
                        }
                    },
                )
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
    write_explicit_population_with_seed(output, segment, generation, 0, candidates)
}

/// Writes an exact caller-supplied population while retaining the proposal
/// seed authenticated by its candidate envelopes. Tournament lanes use this
/// form so extracting a subset cannot silently rewrite proposer lineage.
pub fn write_explicit_population_with_seed(
    output: &Path,
    segment: SegmentProfile,
    generation: u32,
    rng_seed: u64,
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
    write_population(output, segment, generation, rng_seed, candidates)
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
    if next.len() < config.population_size {
        for candidate in coordinated_imported_timing_edits(&elites[0], generation)? {
            if next.len() >= config.population_size {
                break;
            }
            if ids.insert(candidate.id()?) {
                next.push(candidate);
            }
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
#[path = "search/tests.rs"]
mod tests;
