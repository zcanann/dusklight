//! Reusable, context-labelled regression cases for game-specific tactics.

use crate::game_tactic::{GameTactic, GameTacticError, GameTacticPlan, ItemSlot};
use crate::option_execution::OptionType;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;

pub const TACTIC_TEST_SCHEMA_V1: &str = "dusklight-tactic-test/v1";

pub const MODE_CLIMB: u32 = 0x0000_0010;
pub const MODE_RIDING: u32 = 0x0000_0400;
pub const MODE_VINE_CLIMB: u32 = 0x0001_0000;
pub const MODE_SWIMMING: u32 = 0x0004_0000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerGameMode {
    Human,
    Wolf,
    Horse,
    Crawl,
    Climb,
    Swim,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticProcedureContext {
    /// Exact native `daAlink_PROC` token, never a session-dependent numeric ID.
    pub procedure: String,
    pub game_mode: PlayerGameMode,
    pub required_mode_flags: u32,
    pub forbidden_mode_flags: u32,
}

impl TacticProcedureContext {
    pub fn matches(&self, procedure: &str, mode_flags: u32) -> bool {
        self.procedure == procedure
            && mode_flags & self.required_mode_flags == self.required_mode_flags
            && mode_flags & self.forbidden_mode_flags == 0
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticPadExpectation {
    pub local_tick: u32,
    pub buttons: u16,
    pub stick_x: i8,
    pub stick_y: i8,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReusableTacticTest {
    pub schema: String,
    pub id: String,
    pub context: TacticProcedureContext,
    pub plan: GameTacticPlan,
    pub expected_option_type: OptionType,
    pub expected_ticks: u32,
    pub pad_expectations: Vec<TacticPadExpectation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TacticTestError {
    UnsupportedSchema,
    InvalidId,
    InvalidProcedure,
    InvalidModeFlags,
    InvalidProcedureMode,
    DuplicateExpectation,
    ExpectationOutOfRange,
    WrongOptionType,
    WrongDuration,
    PadMismatch(u32),
    Tactic(GameTacticError),
}

impl fmt::Display for TacticTestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema => formatter.write_str("unsupported tactic-test schema"),
            Self::InvalidId => formatter.write_str("tactic-test ID is invalid"),
            Self::InvalidProcedure => formatter.write_str("player procedure token is invalid"),
            Self::InvalidModeFlags => {
                formatter.write_str("required and forbidden mode flags overlap")
            }
            Self::InvalidProcedureMode => {
                formatter.write_str("player procedure does not belong to the declared game mode")
            }
            Self::DuplicateExpectation => {
                formatter.write_str("pad expectations repeat a local tick")
            }
            Self::ExpectationOutOfRange => {
                formatter.write_str("pad expectation lies outside the tactic realization")
            }
            Self::WrongOptionType => formatter.write_str("tactic produced the wrong option type"),
            Self::WrongDuration => {
                formatter.write_str("tactic produced the wrong planned duration")
            }
            Self::PadMismatch(tick) => {
                write!(formatter, "tactic pad output differs at local tick {tick}")
            }
            Self::Tactic(source) => write!(formatter, "invalid tactic plan: {source}"),
        }
    }
}

impl Error for TacticTestError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Tactic(source) => Some(source),
            _ => None,
        }
    }
}

impl From<GameTacticError> for TacticTestError {
    fn from(value: GameTacticError) -> Self {
        Self::Tactic(value)
    }
}

impl ReusableTacticTest {
    pub fn verify(&self) -> Result<(), TacticTestError> {
        if self.schema != TACTIC_TEST_SCHEMA_V1 {
            return Err(TacticTestError::UnsupportedSchema);
        }
        if !valid_name(&self.id) {
            return Err(TacticTestError::InvalidId);
        }
        if !valid_procedure(&self.context.procedure) {
            return Err(TacticTestError::InvalidProcedure);
        }
        if self.context.required_mode_flags & self.context.forbidden_mode_flags != 0 {
            return Err(TacticTestError::InvalidModeFlags);
        }
        if !procedure_belongs_to_mode(&self.context.procedure, self.context.game_mode) {
            return Err(TacticTestError::InvalidProcedureMode);
        }
        self.plan.validate()?;
        if self.plan.option_type() != self.expected_option_type {
            return Err(TacticTestError::WrongOptionType);
        }
        let realization = self.plan.realize(None)?;
        if realization.planned_ticks != self.expected_ticks {
            return Err(TacticTestError::WrongDuration);
        }
        let mut previous = None;
        for expectation in &self.pad_expectations {
            if previous.is_some_and(|tick| tick >= expectation.local_tick) {
                return Err(TacticTestError::DuplicateExpectation);
            }
            previous = Some(expectation.local_tick);
            let Some(frame) = realization.frames.get(expectation.local_tick as usize) else {
                return Err(TacticTestError::ExpectationOutOfRange);
            };
            let pad = frame.pads[0];
            if pad.buttons != expectation.buttons
                || pad.stick_x != expectation.stick_x
                || pad.stick_y != expectation.stick_y
            {
                return Err(TacticTestError::PadMismatch(expectation.local_tick));
            }
        }
        Ok(())
    }
}

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn valid_procedure(value: &str) -> bool {
    value.starts_with("PROC_")
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

fn procedure_belongs_to_mode(procedure: &str, mode: PlayerGameMode) -> bool {
    match mode {
        PlayerGameMode::Wolf => procedure.starts_with("PROC_WOLF_"),
        PlayerGameMode::Horse => procedure.starts_with("PROC_HORSE_"),
        PlayerGameMode::Crawl => procedure.starts_with("PROC_CRAWL_"),
        PlayerGameMode::Climb => procedure.starts_with("PROC_CLIMB_"),
        PlayerGameMode::Swim => procedure.starts_with("PROC_SWIM_"),
        PlayerGameMode::Human => ![
            "PROC_WOLF_",
            "PROC_HORSE_",
            "PROC_CRAWL_",
            "PROC_CLIMB_",
            "PROC_SWIM_",
        ]
        .iter()
        .any(|prefix| procedure.starts_with(prefix)),
    }
}

const BUTTON_DOWN: u16 = 0x0004;
const BUTTON_R: u16 = 0x0020;
const BUTTON_L: u16 = 0x0040;
const BUTTON_A: u16 = 0x0100;
const BUTTON_B: u16 = 0x0200;
const BUTTON_X: u16 = 0x0400;
const BUTTON_Y: u16 = 0x0800;

fn context(procedure: &str, game_mode: PlayerGameMode) -> TacticProcedureContext {
    let (required_mode_flags, forbidden_mode_flags) = match game_mode {
        PlayerGameMode::Horse => (MODE_RIDING, MODE_SWIMMING | MODE_CLIMB | MODE_VINE_CLIMB),
        PlayerGameMode::Swim => (MODE_SWIMMING, MODE_RIDING | MODE_CLIMB | MODE_VINE_CLIMB),
        PlayerGameMode::Climb => (MODE_CLIMB, MODE_RIDING | MODE_SWIMMING),
        PlayerGameMode::Human => (
            0,
            MODE_RIDING | MODE_SWIMMING | MODE_CLIMB | MODE_VINE_CLIMB,
        ),
        PlayerGameMode::Wolf | PlayerGameMode::Crawl => (0, MODE_RIDING | MODE_SWIMMING),
    };
    TacticProcedureContext {
        procedure: procedure.into(),
        game_mode,
        required_mode_flags,
        forbidden_mode_flags,
    }
}

fn test(
    id: &str,
    procedure: &str,
    mode: PlayerGameMode,
    tactic: GameTactic,
    expected_option_type: OptionType,
    expected_ticks: u32,
    pad_expectations: &[(u32, u16, i8, i8)],
) -> ReusableTacticTest {
    ReusableTacticTest {
        schema: TACTIC_TEST_SCHEMA_V1.into(),
        id: id.into(),
        context: context(procedure, mode),
        plan: GameTacticPlan::new(tactic),
        expected_option_type,
        expected_ticks,
        pad_expectations: pad_expectations
            .iter()
            .map(
                |&(local_tick, buttons, stick_x, stick_y)| TacticPadExpectation {
                    local_tick,
                    buttons,
                    stick_x,
                    stick_y,
                },
            )
            .collect(),
    }
}

/// Canonical reusable smoke cases. They assert deterministic PAD recipes and
/// label the exact native procedure/game-mode context in which a gameplay
/// fixture must exercise each tactic.
pub fn reusable_tactic_test_catalog() -> Vec<ReusableTacticTest> {
    vec![
        test(
            "human-wait-jump-attack",
            "PROC_WAIT",
            PlayerGameMode::Human,
            GameTactic::JumpAttack {
                direction_degrees: 0,
                magnitude: 100,
                windup_frames: 1,
                press_frames: 1,
                recovery_frames: 1,
            },
            OptionType::JumpAttack,
            3,
            &[(0, BUTTON_L, 0, 100), (1, BUTTON_L | BUTTON_A, 0, 100)],
        ),
        test(
            "human-move-normal-attack",
            "PROC_MOVE",
            PlayerGameMode::Human,
            GameTactic::NormalAttack {
                direction_degrees: 90,
                magnitude: 80,
                press_frames: 1,
                recovery_frames: 1,
            },
            OptionType::Attack,
            2,
            &[(0, BUTTON_B, 80, 0), (1, 0, 80, 0)],
        ),
        test(
            "human-attention-combo",
            "PROC_ATN_MOVE",
            PlayerGameMode::Human,
            GameTactic::AttackCombo {
                direction_degrees: 0,
                magnitude: 64,
                hits: 2,
                press_frames: 1,
                gap_frames: 1,
                recovery_frames: 1,
            },
            OptionType::Attack,
            4,
            &[(0, BUTTON_B, 0, 64), (1, 0, 0, 64), (2, BUTTON_B, 0, 64)],
        ),
        test(
            "human-wait-shield",
            "PROC_WAIT",
            PlayerGameMode::Human,
            GameTactic::Shield { frames: 2 },
            OptionType::Shield,
            2,
            &[(0, BUTTON_R, 0, 0)],
        ),
        test(
            "human-attention-target",
            "PROC_ATN_ACTOR_WAIT",
            PlayerGameMode::Human,
            GameTactic::Target { frames: 2 },
            OptionType::Target,
            2,
            &[(0, BUTTON_L, 0, 0)],
        ),
        test(
            "human-wait-interact",
            "PROC_WAIT",
            PlayerGameMode::Human,
            GameTactic::Interact {
                press_frames: 1,
                recovery_frames: 1,
            },
            OptionType::Interact,
            2,
            &[(0, BUTTON_A, 0, 0), (1, 0, 0, 0)],
        ),
        test(
            "human-wait-item-x",
            "PROC_WAIT",
            PlayerGameMode::Human,
            GameTactic::ItemUse {
                slot: ItemSlot::X,
                hold_frames: 1,
                recovery_frames: 1,
            },
            OptionType::ItemUse,
            2,
            &[(0, BUTTON_X, 0, 0)],
        ),
        test(
            "human-wait-transform",
            "PROC_WAIT",
            PlayerGameMode::Human,
            GameTactic::Transform {
                press_frames: 1,
                recovery_frames: 1,
            },
            OptionType::Transform,
            2,
            &[(0, BUTTON_DOWN, 0, 0)],
        ),
        test(
            "wolf-wait-transform",
            "PROC_WOLF_WAIT",
            PlayerGameMode::Wolf,
            GameTactic::Transform {
                press_frames: 1,
                recovery_frames: 1,
            },
            OptionType::Transform,
            2,
            &[(0, BUTTON_DOWN, 0, 0)],
        ),
        test(
            "crawl-move-forward",
            "PROC_CRAWL_MOVE",
            PlayerGameMode::Crawl,
            GameTactic::Crawl {
                direction_degrees: 0,
                magnitude: 70,
                frames: 2,
                action_held: true,
            },
            OptionType::Crawl,
            2,
            &[(0, BUTTON_A, 0, 70)],
        ),
        test(
            "climb-wait-up",
            "PROC_CLIMB_WAIT",
            PlayerGameMode::Climb,
            GameTactic::Climb {
                direction_degrees: 0,
                magnitude: 70,
                frames: 2,
                action_held: false,
            },
            OptionType::Climb,
            2,
            &[(0, 0, 0, 70)],
        ),
        test(
            "climb-side-right",
            "PROC_CLIMB_MOVE_SIDE",
            PlayerGameMode::Climb,
            GameTactic::Climb {
                direction_degrees: 90,
                magnitude: 70,
                frames: 2,
                action_held: true,
            },
            OptionType::Climb,
            2,
            &[(0, BUTTON_A, 70, 0)],
        ),
        test(
            "swim-wait-forward",
            "PROC_SWIM_WAIT",
            PlayerGameMode::Swim,
            GameTactic::Swim {
                direction_degrees: 0,
                magnitude: 90,
                frames: 2,
                action_held: true,
            },
            OptionType::Swim,
            2,
            &[(0, BUTTON_A, 0, 90)],
        ),
        test(
            "swim-move-hookshot",
            "PROC_SWIM_MOVE",
            PlayerGameMode::Swim,
            GameTactic::Clawshot {
                slot: ItemSlot::X,
                hold_frames: 1,
                recovery_frames: 1,
            },
            OptionType::Clawshot,
            2,
            &[(0, BUTTON_X, 0, 0)],
        ),
        test(
            "horse-wait-spur",
            "PROC_HORSE_WAIT",
            PlayerGameMode::Horse,
            GameTactic::Epona {
                direction_degrees: 0,
                magnitude: 100,
                frames: 3,
                spur_frames: 1,
            },
            OptionType::Mount,
            3,
            &[(0, BUTTON_A, 0, 100), (1, 0, 0, 100)],
        ),
        test(
            "horse-wait-boomerang",
            "PROC_HORSE_WAIT",
            PlayerGameMode::Horse,
            GameTactic::Boomerang {
                slot: ItemSlot::Y,
                hold_frames: 1,
                recovery_frames: 1,
            },
            OptionType::Boomerang,
            2,
            &[(0, BUTTON_Y, 0, 0)],
        ),
        test(
            "human-wait-clawshot",
            "PROC_WAIT",
            PlayerGameMode::Human,
            GameTactic::Clawshot {
                slot: ItemSlot::X,
                hold_frames: 1,
                recovery_frames: 1,
            },
            OptionType::Clawshot,
            2,
            &[(0, BUTTON_X, 0, 0)],
        ),
        test(
            "human-wait-spinner",
            "PROC_WAIT",
            PlayerGameMode::Human,
            GameTactic::Spinner {
                slot: ItemSlot::Y,
                hold_frames: 1,
                recovery_frames: 1,
            },
            OptionType::Spinner,
            2,
            &[(0, BUTTON_Y, 0, 0)],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn reusable_catalog_is_unique_valid_and_covers_every_tactic_family_and_mode() {
        let catalog = reusable_tactic_test_catalog();
        let mut ids = BTreeSet::new();
        let mut modes = BTreeSet::new();
        let mut families = BTreeSet::new();
        for case in &catalog {
            case.verify().unwrap();
            assert!(ids.insert(case.id.clone()));
            modes.insert(format!("{:?}", case.context.game_mode));
            families.insert(match case.plan.tactic {
                GameTactic::JumpAttack { .. } => "jump_attack",
                GameTactic::NormalAttack { .. } => "normal_attack",
                GameTactic::AttackCombo { .. } => "attack_combo",
                GameTactic::Shield { .. } => "shield",
                GameTactic::Target { .. } => "target",
                GameTactic::Interact { .. } => "interact",
                GameTactic::ItemUse { .. } => "item_use",
                GameTactic::Transform { .. } => "transform",
                GameTactic::Crawl { .. } => "crawl",
                GameTactic::Climb { .. } => "climb",
                GameTactic::Swim { .. } => "swim",
                GameTactic::Epona { .. } => "epona",
                GameTactic::Boomerang { .. } => "boomerang",
                GameTactic::Clawshot { .. } => "clawshot",
                GameTactic::Spinner { .. } => "spinner",
            });
        }
        assert_eq!(modes.len(), 6);
        assert_eq!(families.len(), 15);
    }

    #[test]
    fn context_matching_requires_exact_procedure_and_mode_flags() {
        let horse = reusable_tactic_test_catalog()
            .into_iter()
            .find(|case| case.id == "horse-wait-spur")
            .unwrap();
        assert!(horse.context.matches("PROC_HORSE_WAIT", MODE_RIDING));
        assert!(!horse.context.matches("PROC_HORSE_RUN", MODE_RIDING));
        assert!(!horse.context.matches("PROC_HORSE_WAIT", 0));
        assert!(
            !horse
                .context
                .matches("PROC_HORSE_WAIT", MODE_RIDING | MODE_SWIMMING)
        );
    }
}
