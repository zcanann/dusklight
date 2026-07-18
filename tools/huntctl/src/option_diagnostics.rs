//! Per-tick diagnostics for authenticated option executions.

use crate::artifact::Digest;
use crate::option_execution::{
    OptionEndReason, OptionExecution, OptionExecutionError, OptionParameter, OptionType,
};
use crate::tape::{InputFrame, InputTape};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

pub const OPTION_DIAGNOSTIC_SCHEMA_V1: &str = "dusklight-option-diagnostic/v1";
pub const OPTION_DIAGNOSTIC_BUNDLE_SCHEMA_V1: &str = "dusklight-option-diagnostic-bundle/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum IntendedTarget {
    None,
    Coordinate {
        value_f32_bits: [u32; 3],
    },
    Heading {
        radians_f32_bits: u32,
    },
    Actor {
        selector: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        runtime_process_id: Option<u64>,
    },
    Semantic {
        name: String,
        parameters: BTreeMap<String, OptionParameter>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActionMaskDecision {
    pub guidance_schema: String,
    pub guidance_sha256: Digest,
    pub action_id: u16,
    pub recommended: bool,
    /// Must remain true: guidance never restricts proof execution.
    pub proof_unrestricted: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClampDisposition {
    pub main_x: bool,
    pub main_y: bool,
    pub substick_x: bool,
    pub substick_y: bool,
    pub trigger_left: bool,
    pub trigger_right: bool,
    pub analog_a: bool,
    pub analog_b: bool,
}

/// Normalized viewport coordinates, where `(0, 0)` is the top-left and
/// `(65535, 65535)` is the bottom-right of the captured gameplay image.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ViewportPoint {
    pub x_u16: u16,
    pub y_u16: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptionContact {
    pub kind: String,
    pub position_f32_bits: [u32; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal_f32_bits: Option<[u32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stable_surface_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<ViewportPoint>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptionGoalProgress {
    pub goal: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_f32_bits: Option<u32>,
    pub satisfied: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptionTickDiagnostic {
    pub local_tick: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_vector_f32_bits: Option<[u32; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_mask: Option<ActionMaskDecision>,
    pub raw_output: InputFrame,
    pub clamps: ClampDisposition,
    pub game_consumed_input: InputFrame,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contacts: Vec<OptionContact>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub goal_progress: Vec<OptionGoalProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_viewport: Option<ViewportPoint>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptionDiagnostic {
    pub schema: String,
    pub execution: OptionExecution,
    pub intended_target: IntendedTarget,
    pub ticks: Vec<OptionTickDiagnostic>,
}

/// Authenticated diagnostics stored next to a canonical tape as
/// `<artifact>.options.json`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptionDiagnosticBundle {
    pub schema: String,
    pub tape_sha256: Digest,
    pub options: Vec<OptionDiagnostic>,
}

#[derive(Clone, Debug, Serialize)]
pub struct OptionVisualization {
    pub option_id: String,
    #[serde(rename = "type")]
    pub option_type: OptionType,
    pub start_frame: u64,
    pub end_frame_exclusive: u64,
    pub end_reason: OptionEndReason,
    pub intended_target: IntendedTarget,
    pub curve: Vec<OptionCurvePoint>,
    pub contacts: Vec<OptionContactSample>,
    pub goal_progress: Vec<OptionGoalProgressSample>,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct OptionCurvePoint {
    pub frame: u64,
    pub main_x: i8,
    pub main_y: i8,
    pub camera_x: i8,
    pub camera_y: i8,
    pub clamped: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_viewport: Option<ViewportPoint>,
}

#[derive(Clone, Debug, Serialize)]
pub struct OptionContactSample {
    pub frame: u64,
    #[serde(flatten)]
    pub contact: OptionContact,
}

#[derive(Clone, Debug, Serialize)]
pub struct OptionGoalProgressSample {
    pub frame: u64,
    #[serde(flatten)]
    pub progress: OptionGoalProgress,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OptionDiagnosticError {
    UnsupportedSchema,
    Execution(OptionExecutionError),
    InvalidTarget,
    TickCount,
    TickOrder,
    RawOutputMismatch,
    InvalidActionMask,
    NonFiniteErrorVector,
    ReactiveConsumedInput,
    InvalidContact,
    InvalidGoalProgress,
}

impl fmt::Display for OptionDiagnosticError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema => formatter.write_str("unsupported option diagnostic schema"),
            Self::Execution(source) => write!(formatter, "invalid option execution: {source}"),
            Self::InvalidTarget => formatter.write_str("intended option target is invalid"),
            Self::TickCount => {
                formatter.write_str("diagnostic tick count does not match execution")
            }
            Self::TickOrder => formatter.write_str("diagnostic local ticks are not contiguous"),
            Self::RawOutputMismatch => {
                formatter.write_str("diagnostic raw output does not match option execution")
            }
            Self::InvalidActionMask => formatter.write_str(
                "diagnostic action-mask evidence is invalid or attempts to restrict proof",
            ),
            Self::NonFiniteErrorVector => {
                formatter.write_str("diagnostic error vector contains a non-finite component")
            }
            Self::ReactiveConsumedInput => {
                formatter.write_str("game-consumed diagnostic input must be absolute")
            }
            Self::InvalidContact => formatter.write_str("option contact evidence is invalid"),
            Self::InvalidGoalProgress => {
                formatter.write_str("option goal-progress evidence is invalid")
            }
        }
    }
}

impl Error for OptionDiagnosticError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Execution(source) => Some(source),
            _ => None,
        }
    }
}

impl From<OptionExecutionError> for OptionDiagnosticError {
    fn from(value: OptionExecutionError) -> Self {
        Self::Execution(value)
    }
}

impl OptionDiagnostic {
    pub fn capture(
        execution: OptionExecution,
        intended_target: IntendedTarget,
        ticks: Vec<OptionTickDiagnostic>,
    ) -> Result<Self, OptionDiagnosticError> {
        let diagnostic = Self {
            schema: OPTION_DIAGNOSTIC_SCHEMA_V1.into(),
            execution,
            intended_target,
            ticks,
        };
        diagnostic.validate()?;
        Ok(diagnostic)
    }

    pub fn validate(&self) -> Result<(), OptionDiagnosticError> {
        if self.schema != OPTION_DIAGNOSTIC_SCHEMA_V1 {
            return Err(OptionDiagnosticError::UnsupportedSchema);
        }
        self.execution.validate()?;
        validate_target(&self.intended_target)?;
        if self.ticks.len() != self.execution.emitted_raw_actions.len() {
            return Err(OptionDiagnosticError::TickCount);
        }
        for (index, tick) in self.ticks.iter().enumerate() {
            if tick.local_tick as usize != index {
                return Err(OptionDiagnosticError::TickOrder);
            }
            if tick.raw_output != self.execution.emitted_raw_actions[index] {
                return Err(OptionDiagnosticError::RawOutputMismatch);
            }
            if tick.error_vector_f32_bits.is_some_and(|vector| {
                vector
                    .into_iter()
                    .any(|component| !f32::from_bits(component).is_finite())
            }) {
                return Err(OptionDiagnosticError::NonFiniteErrorVector);
            }
            if tick.game_consumed_input.wait_condition != crate::tape::WaitCondition::None
                || tick.game_consumed_input.wait_timeout_ticks != 0
            {
                return Err(OptionDiagnosticError::ReactiveConsumedInput);
            }
            if let Some(mask) = &tick.action_mask
                && (mask.guidance_schema.is_empty()
                    || mask.guidance_schema.len() > 128
                    || mask.guidance_sha256 == Digest::ZERO
                    || !mask.proof_unrestricted)
            {
                return Err(OptionDiagnosticError::InvalidActionMask);
            }
            for contact in &tick.contacts {
                validate_contact(contact)?;
            }
            for progress in &tick.goal_progress {
                validate_goal_progress(progress)?;
            }
        }
        Ok(())
    }

    pub fn start_frame(&self) -> u64 {
        self.execution.realized_tape_range.start_frame
    }

    pub fn end_frame_exclusive(&self) -> u64 {
        self.execution.realized_tape_range.end_frame_exclusive
    }
}

impl OptionDiagnosticBundle {
    pub fn capture(
        tape: &InputTape,
        options: Vec<OptionDiagnostic>,
    ) -> Result<Self, OptionDiagnosticError> {
        let bundle = Self {
            schema: OPTION_DIAGNOSTIC_BUNDLE_SCHEMA_V1.into(),
            tape_sha256: Digest(
                Sha256::digest(tape.encode().map_err(|error| {
                    OptionDiagnosticError::Execution(OptionExecutionError::Tape(error))
                })?)
                .into(),
            ),
            options,
        };
        bundle.validate_against_tape(tape)?;
        Ok(bundle)
    }

    pub fn validate_against_tape(&self, tape: &InputTape) -> Result<(), OptionDiagnosticError> {
        if self.schema != OPTION_DIAGNOSTIC_BUNDLE_SCHEMA_V1 {
            return Err(OptionDiagnosticError::UnsupportedSchema);
        }
        let digest = Digest(
            Sha256::digest(tape.encode().map_err(|error| {
                OptionDiagnosticError::Execution(OptionExecutionError::Tape(error))
            })?)
            .into(),
        );
        if self.tape_sha256 != digest {
            return Err(OptionDiagnosticError::Execution(
                OptionExecutionError::TapeDigestMismatch,
            ));
        }
        let mut end = 0_u64;
        for diagnostic in &self.options {
            diagnostic.validate()?;
            diagnostic.execution.validate_against_tape(tape)?;
            if diagnostic.start_frame() < end {
                return Err(OptionDiagnosticError::TickOrder);
            }
            end = diagnostic.end_frame_exclusive();
        }
        Ok(())
    }

    pub fn visualization(&self) -> Vec<OptionVisualization> {
        self.options
            .iter()
            .map(|diagnostic| {
                let start = diagnostic.start_frame();
                let mut contacts = Vec::new();
                let mut goal_progress = Vec::new();
                let curve = diagnostic
                    .ticks
                    .iter()
                    .map(|tick| {
                        let frame = start + u64::from(tick.local_tick);
                        contacts.extend(
                            tick.contacts
                                .iter()
                                .cloned()
                                .map(|contact| OptionContactSample { frame, contact }),
                        );
                        goal_progress.extend(
                            tick.goal_progress
                                .iter()
                                .cloned()
                                .map(|progress| OptionGoalProgressSample { frame, progress }),
                        );
                        let pad = tick.game_consumed_input.pads[0];
                        OptionCurvePoint {
                            frame,
                            main_x: pad.stick_x,
                            main_y: pad.stick_y,
                            camera_x: pad.substick_x,
                            camera_y: pad.substick_y,
                            clamped: tick.clamps != ClampDisposition::default(),
                            target_viewport: tick.target_viewport,
                        }
                    })
                    .collect();
                OptionVisualization {
                    option_id: diagnostic.execution.option_id.clone(),
                    option_type: diagnostic.execution.option_type.clone(),
                    start_frame: start,
                    end_frame_exclusive: diagnostic.end_frame_exclusive(),
                    end_reason: diagnostic.execution.end_reason,
                    intended_target: diagnostic.intended_target.clone(),
                    curve,
                    contacts,
                    goal_progress,
                }
            })
            .collect()
    }
}

fn validate_contact(contact: &OptionContact) -> Result<(), OptionDiagnosticError> {
    let finite_vector = |bits: &[u32; 3]| {
        bits.iter()
            .all(|component| f32::from_bits(*component).is_finite())
    };
    if contact.kind.is_empty()
        || contact.kind.len() > 96
        || !finite_vector(&contact.position_f32_bits)
        || contact
            .normal_f32_bits
            .as_ref()
            .is_some_and(|normal| !finite_vector(normal))
        || contact
            .stable_surface_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 256)
    {
        return Err(OptionDiagnosticError::InvalidContact);
    }
    Ok(())
}

fn validate_goal_progress(progress: &OptionGoalProgress) -> Result<(), OptionDiagnosticError> {
    if progress.goal.is_empty()
        || progress.goal.len() > 128
        || progress
            .value_f32_bits
            .is_some_and(|bits| !f32::from_bits(bits).is_finite())
    {
        return Err(OptionDiagnosticError::InvalidGoalProgress);
    }
    Ok(())
}

fn validate_target(target: &IntendedTarget) -> Result<(), OptionDiagnosticError> {
    let finite = |bits: u32| f32::from_bits(bits).is_finite();
    match target {
        IntendedTarget::None => Ok(()),
        IntendedTarget::Coordinate { value_f32_bits } => value_f32_bits
            .iter()
            .copied()
            .all(finite)
            .then_some(())
            .ok_or(OptionDiagnosticError::InvalidTarget),
        IntendedTarget::Heading { radians_f32_bits } => finite(*radians_f32_bits)
            .then_some(())
            .ok_or(OptionDiagnosticError::InvalidTarget),
        IntendedTarget::Actor {
            selector,
            runtime_process_id,
        } if !selector.is_empty()
            && selector.len() <= 256
            && runtime_process_id.is_none_or(|id| id != 0) =>
        {
            Ok(())
        }
        IntendedTarget::Semantic { name, parameters }
            if !name.is_empty() && name.len() <= 96 && parameters.len() <= 64 =>
        {
            Ok(())
        }
        _ => Err(OptionDiagnosticError::InvalidTarget),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::option_execution::{OptionCondition, OptionEndReason, OptionType, TapeRange};
    use crate::tape::{InputTape, RawPadState};
    use std::collections::BTreeMap;

    fn execution() -> OptionExecution {
        let mut frame = InputFrame {
            owned_ports: 1,
            ..InputFrame::default()
        };
        frame.pads[0] = RawPadState {
            stick_y: 100,
            ..RawPadState::default()
        };
        let tape = InputTape {
            frames: vec![frame.clone(), frame],
            ..InputTape::default()
        };
        OptionExecution::capture(
            "move-door".into(),
            OptionType::Move,
            BTreeMap::new(),
            1,
            2,
            OptionCondition::DurationElapsed,
            Vec::new(),
            OptionEndReason::Completed,
            &tape,
            TapeRange {
                start_frame: 0,
                end_frame_exclusive: 2,
            },
        )
        .unwrap()
    }

    #[test]
    fn captures_aligned_raw_clamp_mask_error_and_consumed_input() {
        let execution = execution();
        let ticks = execution
            .emitted_raw_actions
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, raw_output)| OptionTickDiagnostic {
                local_tick: index as u32,
                error_vector_f32_bits: Some([
                    1.0_f32.to_bits(),
                    0.0_f32.to_bits(),
                    2.0_f32.to_bits(),
                ]),
                action_mask: Some(ActionMaskDecision {
                    guidance_schema: "dusklight-action-guidance/movement-v1".into(),
                    guidance_sha256: Digest([7; 32]),
                    action_id: 3,
                    recommended: index == 0,
                    proof_unrestricted: true,
                }),
                raw_output: raw_output.clone(),
                clamps: ClampDisposition {
                    main_y: index == 1,
                    ..ClampDisposition::default()
                },
                game_consumed_input: raw_output,
                contacts: vec![OptionContact {
                    kind: "floor".into(),
                    position_f32_bits: [1.0_f32.to_bits(), 2.0_f32.to_bits(), 3.0_f32.to_bits()],
                    normal_f32_bits: Some([0, 1.0_f32.to_bits(), 0]),
                    stable_surface_id: Some("room-0/floor-3".into()),
                    viewport: Some(ViewportPoint {
                        x_u16: 32_768,
                        y_u16: 48_000,
                    }),
                }],
                goal_progress: vec![OptionGoalProgress {
                    goal: "reach-door".into(),
                    value_f32_bits: Some((index as f32 / 2.0).to_bits()),
                    satisfied: index == 1,
                }],
                target_viewport: Some(ViewportPoint {
                    x_u16: 50_000,
                    y_u16: 20_000,
                }),
            })
            .collect();
        let diagnostic = OptionDiagnostic::capture(
            execution,
            IntendedTarget::Coordinate {
                value_f32_bits: [10.0_f32.to_bits(), 0, 20.0_f32.to_bits()],
            },
            ticks,
        )
        .unwrap();
        assert_eq!(diagnostic.start_frame(), 0);
        assert_eq!(diagnostic.end_frame_exclusive(), 2);
        assert!(diagnostic.ticks[1].clamps.main_y);
        assert_eq!(diagnostic.ticks[0].contacts[0].kind, "floor");

        let bundle = OptionDiagnosticBundle::capture(
            &InputTape {
                frames: diagnostic.execution.emitted_raw_actions.clone(),
                ..InputTape::default()
            },
            vec![diagnostic],
        )
        .unwrap();
        let view = bundle.visualization();
        assert_eq!(view[0].curve.len(), 2);
        assert_eq!(view[0].contacts.len(), 2);
        assert!(view[0].goal_progress[1].progress.satisfied);
    }

    #[test]
    fn rejects_misaligned_or_proof_restricting_diagnostics() {
        let execution = execution();
        let raw = execution.emitted_raw_actions[0].clone();
        let mut tick = OptionTickDiagnostic {
            local_tick: 0,
            error_vector_f32_bits: None,
            action_mask: None,
            raw_output: raw.clone(),
            clamps: ClampDisposition::default(),
            game_consumed_input: raw,
            contacts: Vec::new(),
            goal_progress: Vec::new(),
            target_viewport: None,
        };
        assert!(matches!(
            OptionDiagnostic::capture(execution.clone(), IntendedTarget::None, vec![tick.clone()]),
            Err(OptionDiagnosticError::TickCount)
        ));
        tick.action_mask = Some(ActionMaskDecision {
            guidance_schema: "mask/v1".into(),
            guidance_sha256: Digest([1; 32]),
            action_id: 0,
            recommended: false,
            proof_unrestricted: false,
        });
        let mut second = tick.clone();
        second.local_tick = 1;
        assert!(matches!(
            OptionDiagnostic::capture(execution, IntendedTarget::None, vec![tick, second]),
            Err(OptionDiagnosticError::InvalidActionMask)
        ));
    }
}
