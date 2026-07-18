//! Typed, bounded game-specific tactic plans and exact raw realization.

use crate::option_execution::{
    MAX_OPTION_CONDITIONS, OptionCondition, OptionEndReason, OptionExecution, OptionExecutionError,
    OptionParameter, OptionType, TapeRange, validate_condition,
};
use crate::tape::{InputFrame, InputTape, RawPadState};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

pub const GAME_TACTIC_SCHEMA_V1: &str = "dusklight-game-tactic/v1";
pub const MAX_TACTIC_TICKS: u32 = 10_000;
const BUTTON_DOWN: u16 = 0x0004;
const BUTTON_R: u16 = 0x0020;
const BUTTON_L: u16 = 0x0040;
const BUTTON_A: u16 = 0x0100;
const BUTTON_B: u16 = 0x0200;
const BUTTON_X: u16 = 0x0400;
const BUTTON_Y: u16 = 0x0800;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemSlot {
    X,
    Y,
}

impl ItemSlot {
    const fn button(self) -> u16 {
        match self {
            Self::X => BUTTON_X,
            Self::Y => BUTTON_Y,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::X => "x",
            Self::Y => "y",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GameTactic {
    JumpAttack {
        direction_degrees: i16,
        magnitude: u8,
        windup_frames: u32,
        press_frames: u32,
        recovery_frames: u32,
    },
    NormalAttack {
        direction_degrees: i16,
        magnitude: u8,
        press_frames: u32,
        recovery_frames: u32,
    },
    AttackCombo {
        direction_degrees: i16,
        magnitude: u8,
        hits: u8,
        press_frames: u32,
        gap_frames: u32,
        recovery_frames: u32,
    },
    Shield {
        frames: u32,
    },
    Target {
        frames: u32,
    },
    Interact {
        press_frames: u32,
        recovery_frames: u32,
    },
    ItemUse {
        slot: ItemSlot,
        hold_frames: u32,
        recovery_frames: u32,
    },
    Transform {
        press_frames: u32,
        recovery_frames: u32,
    },
    Crawl {
        direction_degrees: i16,
        magnitude: u8,
        frames: u32,
        action_held: bool,
    },
    Climb {
        direction_degrees: i16,
        magnitude: u8,
        frames: u32,
        action_held: bool,
    },
    Swim {
        direction_degrees: i16,
        magnitude: u8,
        frames: u32,
        action_held: bool,
    },
    Epona {
        direction_degrees: i16,
        magnitude: u8,
        frames: u32,
        spur_frames: u32,
    },
    Boomerang {
        slot: ItemSlot,
        hold_frames: u32,
        recovery_frames: u32,
    },
    Clawshot {
        slot: ItemSlot,
        hold_frames: u32,
        recovery_frames: u32,
    },
    Spinner {
        slot: ItemSlot,
        hold_frames: u32,
        recovery_frames: u32,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GameTacticPlan {
    pub schema: String,
    pub tactic: GameTactic,
    pub cancellation_conditions: Vec<OptionCondition>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticCancellationHit {
    /// Cancellation is sampled before input for this zero-based local tick.
    pub tick: u32,
    pub condition_index: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GameTacticRealization {
    pub planned_ticks: u32,
    pub realized_ticks: u32,
    pub end_reason: OptionEndReason,
    pub frames: Vec<InputFrame>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GameTacticError {
    UnsupportedSchema,
    InvalidDirection,
    InvalidMagnitude,
    InvalidTiming,
    InvalidCombo,
    InvalidCancellation,
    FrameMismatch,
    RangeOverflow,
    Execution(OptionExecutionError),
}

impl fmt::Display for GameTacticError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema => formatter.write_str("unsupported game tactic schema"),
            Self::InvalidDirection => {
                formatter.write_str("tactic direction must be in [-180, 180] degrees")
            }
            Self::InvalidMagnitude => formatter.write_str("tactic magnitude must be in 1..=127"),
            Self::InvalidTiming => formatter.write_str("tactic timing is invalid or unbounded"),
            Self::InvalidCombo => formatter.write_str("attack combo must contain 2..=8 hits"),
            Self::InvalidCancellation => formatter.write_str("tactic cancellation hit is invalid"),
            Self::FrameMismatch => {
                formatter.write_str("tactic realization does not match the tape range")
            }
            Self::RangeOverflow => formatter.write_str("tactic tape range overflows"),
            Self::Execution(source) => write!(formatter, "tactic execution is invalid: {source}"),
        }
    }
}

impl Error for GameTacticError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Execution(source) => Some(source),
            _ => None,
        }
    }
}

impl From<OptionExecutionError> for GameTacticError {
    fn from(value: OptionExecutionError) -> Self {
        Self::Execution(value)
    }
}

impl GameTacticPlan {
    pub fn new(tactic: GameTactic) -> Self {
        Self {
            schema: GAME_TACTIC_SCHEMA_V1.into(),
            tactic,
            cancellation_conditions: Vec::new(),
        }
    }

    pub fn option_type(&self) -> OptionType {
        match self.tactic {
            GameTactic::JumpAttack { .. } => OptionType::JumpAttack,
            GameTactic::NormalAttack { .. } | GameTactic::AttackCombo { .. } => OptionType::Attack,
            GameTactic::Shield { .. } => OptionType::Shield,
            GameTactic::Target { .. } => OptionType::Target,
            GameTactic::Interact { .. } => OptionType::Interact,
            GameTactic::ItemUse { .. } => OptionType::ItemUse,
            GameTactic::Transform { .. } => OptionType::Transform,
            GameTactic::Crawl { .. } => OptionType::Crawl,
            GameTactic::Climb { .. } => OptionType::Climb,
            GameTactic::Swim { .. } => OptionType::Swim,
            GameTactic::Epona { .. } => OptionType::Mount,
            GameTactic::Boomerang { .. } => OptionType::Boomerang,
            GameTactic::Clawshot { .. } => OptionType::Clawshot,
            GameTactic::Spinner { .. } => OptionType::Spinner,
        }
    }

    pub fn validate(&self) -> Result<(), GameTacticError> {
        if self.schema != GAME_TACTIC_SCHEMA_V1 {
            return Err(GameTacticError::UnsupportedSchema);
        }
        self.planned_ticks()?;
        if self.cancellation_conditions.len() > MAX_OPTION_CONDITIONS {
            return Err(GameTacticError::InvalidCancellation);
        }
        for condition in &self.cancellation_conditions {
            validate_condition(condition)?;
        }
        Ok(())
    }

    pub fn planned_ticks(&self) -> Result<u32, GameTacticError> {
        let checked_direction = |direction: i16, magnitude: u8| {
            if !(-180..=180).contains(&direction) {
                Err(GameTacticError::InvalidDirection)
            } else if !(1..=127).contains(&magnitude) {
                Err(GameTacticError::InvalidMagnitude)
            } else {
                Ok(())
            }
        };
        let add = |parts: &[u32]| {
            parts
                .iter()
                .try_fold(0_u32, |sum, value| sum.checked_add(*value))
                .filter(|ticks| (1..=MAX_TACTIC_TICKS).contains(ticks))
                .ok_or(GameTacticError::InvalidTiming)
        };
        match self.tactic {
            GameTactic::JumpAttack {
                direction_degrees,
                magnitude,
                windup_frames,
                press_frames,
                recovery_frames,
            } => {
                checked_direction(direction_degrees, magnitude)?;
                if press_frames == 0 {
                    return Err(GameTacticError::InvalidTiming);
                }
                add(&[windup_frames, press_frames, recovery_frames])
            }
            GameTactic::NormalAttack {
                direction_degrees,
                magnitude,
                press_frames,
                recovery_frames,
            } => {
                checked_direction(direction_degrees, magnitude)?;
                if press_frames == 0 {
                    return Err(GameTacticError::InvalidTiming);
                }
                add(&[press_frames, recovery_frames])
            }
            GameTactic::AttackCombo {
                direction_degrees,
                magnitude,
                hits,
                press_frames,
                gap_frames,
                recovery_frames,
            } => {
                checked_direction(direction_degrees, magnitude)?;
                if !(2..=8).contains(&hits) {
                    return Err(GameTacticError::InvalidCombo);
                }
                if press_frames == 0 {
                    return Err(GameTacticError::InvalidTiming);
                }
                let presses = press_frames
                    .checked_mul(u32::from(hits))
                    .ok_or(GameTacticError::InvalidTiming)?;
                let gaps = gap_frames
                    .checked_mul(u32::from(hits - 1))
                    .ok_or(GameTacticError::InvalidTiming)?;
                add(&[presses, gaps, recovery_frames])
            }
            GameTactic::Shield { frames } | GameTactic::Target { frames } => add(&[frames]),
            GameTactic::Interact {
                press_frames,
                recovery_frames,
            }
            | GameTactic::Transform {
                press_frames,
                recovery_frames,
            } => {
                if press_frames == 0 {
                    return Err(GameTacticError::InvalidTiming);
                }
                add(&[press_frames, recovery_frames])
            }
            GameTactic::ItemUse {
                hold_frames,
                recovery_frames,
                ..
            }
            | GameTactic::Boomerang {
                hold_frames,
                recovery_frames,
                ..
            }
            | GameTactic::Clawshot {
                hold_frames,
                recovery_frames,
                ..
            }
            | GameTactic::Spinner {
                hold_frames,
                recovery_frames,
                ..
            } => {
                if hold_frames == 0 {
                    return Err(GameTacticError::InvalidTiming);
                }
                add(&[hold_frames, recovery_frames])
            }
            GameTactic::Crawl {
                direction_degrees,
                magnitude,
                frames,
                ..
            }
            | GameTactic::Climb {
                direction_degrees,
                magnitude,
                frames,
                ..
            }
            | GameTactic::Swim {
                direction_degrees,
                magnitude,
                frames,
                ..
            } => {
                checked_direction(direction_degrees, magnitude)?;
                add(&[frames])
            }
            GameTactic::Epona {
                direction_degrees,
                magnitude,
                frames,
                spur_frames,
            } => {
                checked_direction(direction_degrees, magnitude)?;
                if spur_frames > frames {
                    return Err(GameTacticError::InvalidTiming);
                }
                add(&[frames])
            }
        }
    }

    pub fn realize(
        &self,
        cancellation: Option<TacticCancellationHit>,
    ) -> Result<GameTacticRealization, GameTacticError> {
        self.validate()?;
        let planned_ticks = self.planned_ticks()?;
        let (realized_ticks, end_reason) = match cancellation {
            Some(hit)
                if hit.tick > 0
                    && hit.tick < planned_ticks
                    && (hit.condition_index as usize) < self.cancellation_conditions.len() =>
            {
                (
                    hit.tick,
                    OptionEndReason::Cancelled {
                        condition_index: hit.condition_index,
                    },
                )
            }
            Some(_) => return Err(GameTacticError::InvalidCancellation),
            None => (planned_ticks, OptionEndReason::Completed),
        };
        let mut frames = self.compile_frames()?;
        frames.truncate(realized_ticks as usize);
        Ok(GameTacticRealization {
            planned_ticks,
            realized_ticks,
            end_reason,
            frames,
        })
    }

    pub fn capture_execution(
        &self,
        option_id: String,
        tape: &InputTape,
        range: TapeRange,
        cancellation: Option<TacticCancellationHit>,
    ) -> Result<OptionExecution, GameTacticError> {
        let realization = self.realize(cancellation)?;
        let expected_end = range
            .start_frame
            .checked_add(u64::from(realization.realized_ticks))
            .ok_or(GameTacticError::RangeOverflow)?;
        let start =
            usize::try_from(range.start_frame).map_err(|_| GameTacticError::RangeOverflow)?;
        let end = usize::try_from(expected_end).map_err(|_| GameTacticError::RangeOverflow)?;
        if range.end_frame_exclusive != expected_end
            || end > tape.frames.len()
            || tape.frames[start..end] != realization.frames
        {
            return Err(GameTacticError::FrameMismatch);
        }
        Ok(OptionExecution::capture(
            option_id,
            self.option_type(),
            self.parameters(),
            1,
            self.planned_ticks()?,
            OptionCondition::DurationElapsed,
            self.cancellation_conditions.clone(),
            realization.end_reason,
            tape,
            range,
        )?)
    }

    fn compile_frames(&self) -> Result<Vec<InputFrame>, GameTacticError> {
        let capacity = self.planned_ticks()? as usize;
        let mut frames = Vec::with_capacity(capacity);
        match self.tactic {
            GameTactic::JumpAttack {
                direction_degrees,
                magnitude,
                windup_frames,
                press_frames,
                recovery_frames,
            } => {
                let pad = direction_pad(direction_degrees, magnitude);
                push_phase(&mut frames, pad, BUTTON_L, windup_frames);
                push_phase(&mut frames, pad, BUTTON_L | BUTTON_A, press_frames);
                push_phase(&mut frames, pad, BUTTON_L, recovery_frames);
            }
            GameTactic::NormalAttack {
                direction_degrees,
                magnitude,
                press_frames,
                recovery_frames,
            } => {
                let pad = direction_pad(direction_degrees, magnitude);
                push_phase(&mut frames, pad, BUTTON_B, press_frames);
                push_phase(&mut frames, pad, 0, recovery_frames);
            }
            GameTactic::AttackCombo {
                direction_degrees,
                magnitude,
                hits,
                press_frames,
                gap_frames,
                recovery_frames,
            } => {
                let pad = direction_pad(direction_degrees, magnitude);
                for hit in 0..hits {
                    push_phase(&mut frames, pad, BUTTON_B, press_frames);
                    if hit + 1 < hits {
                        push_phase(&mut frames, pad, 0, gap_frames);
                    }
                }
                push_phase(&mut frames, pad, 0, recovery_frames);
            }
            GameTactic::Shield { frames: count } => {
                push_phase(&mut frames, RawPadState::default(), BUTTON_R, count)
            }
            GameTactic::Target { frames: count } => {
                push_phase(&mut frames, RawPadState::default(), BUTTON_L, count)
            }
            GameTactic::Interact {
                press_frames,
                recovery_frames,
            } => {
                push_phase(&mut frames, RawPadState::default(), BUTTON_A, press_frames);
                push_phase(&mut frames, RawPadState::default(), 0, recovery_frames);
            }
            GameTactic::ItemUse {
                slot,
                hold_frames,
                recovery_frames,
            }
            | GameTactic::Boomerang {
                slot,
                hold_frames,
                recovery_frames,
            }
            | GameTactic::Clawshot {
                slot,
                hold_frames,
                recovery_frames,
            }
            | GameTactic::Spinner {
                slot,
                hold_frames,
                recovery_frames,
            } => {
                push_phase(
                    &mut frames,
                    RawPadState::default(),
                    slot.button(),
                    hold_frames,
                );
                push_phase(&mut frames, RawPadState::default(), 0, recovery_frames);
            }
            GameTactic::Transform {
                press_frames,
                recovery_frames,
            } => {
                push_phase(
                    &mut frames,
                    RawPadState::default(),
                    BUTTON_DOWN,
                    press_frames,
                );
                push_phase(&mut frames, RawPadState::default(), 0, recovery_frames);
            }
            GameTactic::Crawl {
                direction_degrees,
                magnitude,
                frames: count,
                action_held,
            }
            | GameTactic::Climb {
                direction_degrees,
                magnitude,
                frames: count,
                action_held,
            }
            | GameTactic::Swim {
                direction_degrees,
                magnitude,
                frames: count,
                action_held,
            } => push_phase(
                &mut frames,
                direction_pad(direction_degrees, magnitude),
                if action_held { BUTTON_A } else { 0 },
                count,
            ),
            GameTactic::Epona {
                direction_degrees,
                magnitude,
                frames: count,
                spur_frames,
            } => {
                let pad = direction_pad(direction_degrees, magnitude);
                push_phase(&mut frames, pad, BUTTON_A, spur_frames);
                push_phase(&mut frames, pad, 0, count - spur_frames);
            }
        }
        debug_assert_eq!(frames.len(), capacity);
        Ok(frames)
    }

    fn parameters(&self) -> BTreeMap<String, OptionParameter> {
        let mut output = BTreeMap::new();
        let signed = |output: &mut BTreeMap<String, OptionParameter>, name: &str, value: i16| {
            output.insert(name.into(), OptionParameter::Signed(i64::from(value)));
        };
        let unsigned = |output: &mut BTreeMap<String, OptionParameter>, name: &str, value: u64| {
            output.insert(name.into(), OptionParameter::Unsigned(value));
        };
        let movement = |output: &mut BTreeMap<String, OptionParameter>, direction, magnitude| {
            signed(output, "direction_degrees", direction);
            unsigned(output, "magnitude", u64::from(magnitude));
        };
        match self.tactic {
            GameTactic::JumpAttack {
                direction_degrees,
                magnitude,
                windup_frames,
                press_frames,
                recovery_frames,
            } => {
                movement(&mut output, direction_degrees, magnitude);
                unsigned(&mut output, "windup_frames", u64::from(windup_frames));
                unsigned(&mut output, "press_frames", u64::from(press_frames));
                unsigned(&mut output, "recovery_frames", u64::from(recovery_frames));
            }
            GameTactic::NormalAttack {
                direction_degrees,
                magnitude,
                press_frames,
                recovery_frames,
            } => {
                movement(&mut output, direction_degrees, magnitude);
                unsigned(&mut output, "press_frames", u64::from(press_frames));
                unsigned(&mut output, "recovery_frames", u64::from(recovery_frames));
            }
            GameTactic::AttackCombo {
                direction_degrees,
                magnitude,
                hits,
                press_frames,
                gap_frames,
                recovery_frames,
            } => {
                movement(&mut output, direction_degrees, magnitude);
                unsigned(&mut output, "hits", u64::from(hits));
                unsigned(&mut output, "press_frames", u64::from(press_frames));
                unsigned(&mut output, "gap_frames", u64::from(gap_frames));
                unsigned(&mut output, "recovery_frames", u64::from(recovery_frames));
            }
            GameTactic::Shield { frames } | GameTactic::Target { frames } => {
                unsigned(&mut output, "frames", u64::from(frames));
            }
            GameTactic::Interact {
                press_frames,
                recovery_frames,
            }
            | GameTactic::Transform {
                press_frames,
                recovery_frames,
            } => {
                unsigned(&mut output, "press_frames", u64::from(press_frames));
                unsigned(&mut output, "recovery_frames", u64::from(recovery_frames));
            }
            GameTactic::ItemUse {
                slot,
                hold_frames,
                recovery_frames,
            }
            | GameTactic::Boomerang {
                slot,
                hold_frames,
                recovery_frames,
            }
            | GameTactic::Clawshot {
                slot,
                hold_frames,
                recovery_frames,
            }
            | GameTactic::Spinner {
                slot,
                hold_frames,
                recovery_frames,
            } => {
                output.insert("slot".into(), OptionParameter::Text(slot.name().into()));
                unsigned(&mut output, "hold_frames", u64::from(hold_frames));
                unsigned(&mut output, "recovery_frames", u64::from(recovery_frames));
            }
            GameTactic::Crawl {
                direction_degrees,
                magnitude,
                frames,
                action_held,
            }
            | GameTactic::Climb {
                direction_degrees,
                magnitude,
                frames,
                action_held,
            }
            | GameTactic::Swim {
                direction_degrees,
                magnitude,
                frames,
                action_held,
            } => {
                movement(&mut output, direction_degrees, magnitude);
                unsigned(&mut output, "frames", u64::from(frames));
                output.insert("action_held".into(), OptionParameter::Bool(action_held));
            }
            GameTactic::Epona {
                direction_degrees,
                magnitude,
                frames,
                spur_frames,
            } => {
                movement(&mut output, direction_degrees, magnitude);
                unsigned(&mut output, "frames", u64::from(frames));
                unsigned(&mut output, "spur_frames", u64::from(spur_frames));
            }
        }
        output
    }
}

fn direction_pad(direction_degrees: i16, magnitude: u8) -> RawPadState {
    let radians = f64::from(direction_degrees).to_radians();
    let magnitude = f64::from(magnitude);
    RawPadState {
        stick_x: (radians.sin() * magnitude).round().clamp(-127.0, 127.0) as i8,
        stick_y: (radians.cos() * magnitude).round().clamp(-127.0, 127.0) as i8,
        ..RawPadState::default()
    }
}

fn push_phase(output: &mut Vec<InputFrame>, mut pad: RawPadState, buttons: u16, frames: u32) {
    pad.buttons = buttons;
    let mut frame = InputFrame {
        owned_ports: 0x0f,
        ..InputFrame::default()
    };
    frame.pads[0] = pad;
    output.extend(std::iter::repeat_n(frame, frames as usize));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tape::TapeBoot;

    fn first_buttons(tactic: GameTactic) -> u16 {
        GameTacticPlan::new(tactic).realize(None).unwrap().frames[0].pads[0].buttons
    }

    #[test]
    fn fixed_combat_interaction_and_item_bindings_are_exact() {
        let jump = GameTacticPlan::new(GameTactic::JumpAttack {
            direction_degrees: 0,
            magnitude: 100,
            windup_frames: 1,
            press_frames: 1,
            recovery_frames: 1,
        })
        .realize(None)
        .unwrap();
        assert_eq!(jump.frames[0].pads[0].buttons, BUTTON_L);
        assert_eq!(jump.frames[1].pads[0].buttons, BUTTON_L | BUTTON_A);
        assert_eq!(jump.frames[2].pads[0].buttons, BUTTON_L);
        assert_eq!(jump.frames[1].pads[0].stick_y, 100);

        assert_eq!(
            first_buttons(GameTactic::NormalAttack {
                direction_degrees: 0,
                magnitude: 1,
                press_frames: 1,
                recovery_frames: 0,
            }),
            BUTTON_B
        );
        let combo = GameTacticPlan::new(GameTactic::AttackCombo {
            direction_degrees: 0,
            magnitude: 1,
            hits: 3,
            press_frames: 1,
            gap_frames: 1,
            recovery_frames: 1,
        })
        .realize(None)
        .unwrap();
        assert_eq!(
            combo
                .frames
                .iter()
                .filter(|frame| frame.pads[0].buttons == BUTTON_B)
                .count(),
            3
        );
        assert_eq!(first_buttons(GameTactic::Shield { frames: 1 }), BUTTON_R);
        assert_eq!(first_buttons(GameTactic::Target { frames: 1 }), BUTTON_L);
        assert_eq!(
            first_buttons(GameTactic::Interact {
                press_frames: 1,
                recovery_frames: 0,
            }),
            BUTTON_A
        );
        assert_eq!(
            first_buttons(GameTactic::ItemUse {
                slot: ItemSlot::X,
                hold_frames: 1,
                recovery_frames: 0,
            }),
            BUTTON_X
        );
        assert_eq!(
            first_buttons(GameTactic::Transform {
                press_frames: 1,
                recovery_frames: 0,
            }),
            BUTTON_DOWN
        );
    }

    #[test]
    fn traversal_mount_and_named_item_tactics_cover_each_typed_family() {
        for tactic in [
            GameTactic::Crawl {
                direction_degrees: -90,
                magnitude: 80,
                frames: 2,
                action_held: true,
            },
            GameTactic::Climb {
                direction_degrees: 0,
                magnitude: 81,
                frames: 2,
                action_held: false,
            },
            GameTactic::Swim {
                direction_degrees: 90,
                magnitude: 82,
                frames: 2,
                action_held: true,
            },
        ] {
            let realization = GameTacticPlan::new(tactic).realize(None).unwrap();
            assert_eq!(realization.frames.len(), 2);
            assert_ne!(
                (
                    realization.frames[0].pads[0].stick_x,
                    realization.frames[0].pads[0].stick_y
                ),
                (0, 0)
            );
        }
        let epona = GameTacticPlan::new(GameTactic::Epona {
            direction_degrees: 0,
            magnitude: 100,
            frames: 3,
            spur_frames: 1,
        })
        .realize(None)
        .unwrap();
        assert_eq!(epona.frames[0].pads[0].buttons, BUTTON_A);
        assert_eq!(epona.frames[1].pads[0].buttons, 0);

        assert_eq!(
            first_buttons(GameTactic::Boomerang {
                slot: ItemSlot::Y,
                hold_frames: 1,
                recovery_frames: 0,
            }),
            BUTTON_Y
        );
        assert_eq!(
            first_buttons(GameTactic::Clawshot {
                slot: ItemSlot::X,
                hold_frames: 1,
                recovery_frames: 0,
            }),
            BUTTON_X
        );
        assert_eq!(
            first_buttons(GameTactic::Spinner {
                slot: ItemSlot::Y,
                hold_frames: 1,
                recovery_frames: 0,
            }),
            BUTTON_Y
        );
    }

    #[test]
    fn pre_input_cancellation_and_capture_preserve_exact_typed_evidence() {
        let mut plan = GameTacticPlan::new(GameTactic::AttackCombo {
            direction_degrees: 0,
            magnitude: 100,
            hits: 3,
            press_frames: 1,
            gap_frames: 1,
            recovery_frames: 1,
        });
        plan.cancellation_conditions = vec![OptionCondition::Observation {
            observation_schema_sha256: crate::artifact::Digest([1; 32]),
            expression_sha256: crate::artifact::Digest([2; 32]),
        }];
        let hit = TacticCancellationHit {
            tick: 2,
            condition_index: 0,
        };
        let realization = plan.realize(Some(hit)).unwrap();
        assert_eq!(realization.realized_ticks, 2);
        assert_eq!(
            realization.end_reason,
            OptionEndReason::Cancelled { condition_index: 0 }
        );

        let mut frames = vec![InputFrame::default()];
        frames.extend(realization.frames.clone());
        let tape = InputTape {
            boot: TapeBoot::Process,
            frames,
            ..InputTape::default()
        };
        let range = TapeRange {
            start_frame: 1,
            end_frame_exclusive: 3,
        };
        let execution = plan
            .capture_execution("combo-cancel".into(), &tape, range, Some(hit))
            .unwrap();
        assert_eq!(execution.option_type, OptionType::Attack);
        assert_eq!(execution.emitted_raw_actions, realization.frames);
        execution.validate_against_tape(&tape).unwrap();
    }

    #[test]
    fn invalid_directions_combos_timing_and_mount_spurs_are_rejected() {
        assert_eq!(
            GameTacticPlan::new(GameTactic::NormalAttack {
                direction_degrees: 181,
                magnitude: 1,
                press_frames: 1,
                recovery_frames: 0,
            })
            .validate(),
            Err(GameTacticError::InvalidDirection)
        );
        assert_eq!(
            GameTacticPlan::new(GameTactic::AttackCombo {
                direction_degrees: 0,
                magnitude: 1,
                hits: 1,
                press_frames: 1,
                gap_frames: 1,
                recovery_frames: 0,
            })
            .validate(),
            Err(GameTacticError::InvalidCombo)
        );
        assert_eq!(
            GameTacticPlan::new(GameTactic::Shield { frames: 0 }).validate(),
            Err(GameTacticError::InvalidTiming)
        );
        assert_eq!(
            GameTacticPlan::new(GameTactic::Epona {
                direction_degrees: 0,
                magnitude: 1,
                frames: 2,
                spur_frames: 3,
            })
            .validate(),
            Err(GameTacticError::InvalidTiming)
        );
    }
}
