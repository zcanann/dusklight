//! High-level option selection with deterministic tactic realization proof.

use super::option_values::{OptionActionDescriptor, OptionValueModel, RankedOption};
use crate::game_tactic::{GameTacticError, GameTacticPlan, TacticCancellationHit};
use crate::option_execution::{OptionExecution, TapeRange};
use crate::tape::{InputTape, TapeError};
use serde::Serialize;
use std::error::Error;
use std::fmt;

pub const EXECUTED_OPTION_POLICY_STEP_SCHEMA_V1: &str = "dusklight-executed-option-policy-step/v1";

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TacticOptionCandidate {
    descriptor: OptionActionDescriptor,
    plan: GameTacticPlan,
}

impl TacticOptionCandidate {
    /// Derive the policy action descriptor from the same deterministic plan
    /// that will later emit raw frames; callers cannot author the two apart.
    pub fn new(option_id: String, plan: GameTacticPlan) -> Result<Self, OptionPolicyError> {
        let execution = capture_isolated(&option_id, &plan)?;
        Ok(Self {
            descriptor: descriptor(&execution),
            plan,
        })
    }

    pub fn descriptor(&self) -> &OptionActionDescriptor {
        &self.descriptor
    }

    pub fn plan(&self) -> &GameTacticPlan {
        &self.plan
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ExecutedOptionPolicyStep {
    pub schema: &'static str,
    pub selected: RankedOption,
    pub tape: InputTape,
    pub execution: OptionExecution,
    pub selection_policy: &'static str,
    pub executor: &'static str,
    pub descriptor_matches_execution: bool,
    pub raw_frames_match_tape: bool,
    pub promotion_authority: bool,
}

/// Select the highest-valued typed option, deterministically append its tactic
/// frames, and capture an `OptionExecution` bound to the complete output tape.
pub fn select_and_execute(
    model: &OptionValueModel,
    state: &[f32],
    candidates: &[TacticOptionCandidate],
    tape_prefix: &InputTape,
    cancellation: Option<TacticCancellationHit>,
) -> Result<ExecutedOptionPolicyStep, OptionPolicyError> {
    validate_catalog(model, candidates)?;
    let selected = model
        .rank_options(state)
        .map_err(|error| OptionPolicyError::Values(error.to_string()))?
        .into_iter()
        .next()
        .ok_or(OptionPolicyError::EmptyRanking)?;
    let candidate = candidates
        .iter()
        .find(|candidate| candidate.descriptor == selected.descriptor)
        .ok_or(OptionPolicyError::CatalogMismatch)?;
    let (tape, execution) = execute_candidate(candidate, tape_prefix, cancellation)?;
    Ok(ExecutedOptionPolicyStep {
        schema: EXECUTED_OPTION_POLICY_STEP_SCHEMA_V1,
        selected,
        tape,
        execution,
        selection_policy: "highest_option_value_over_exact_executable_catalog",
        executor: "deterministic_game_tactic_plan",
        descriptor_matches_execution: true,
        raw_frames_match_tape: true,
        promotion_authority: false,
    })
}

fn validate_catalog(
    model: &OptionValueModel,
    candidates: &[TacticOptionCandidate],
) -> Result<(), OptionPolicyError> {
    if candidates.is_empty() || candidates.len() != model.actions().len() {
        return Err(OptionPolicyError::CatalogMismatch);
    }
    let mut model_actions = model
        .actions()
        .iter()
        .map(canonical)
        .collect::<Result<Vec<_>, _>>()?;
    let mut executable_actions = candidates
        .iter()
        .map(|candidate| canonical(&candidate.descriptor))
        .collect::<Result<Vec<_>, _>>()?;
    model_actions.sort();
    executable_actions.sort();
    executable_actions.dedup();
    if model_actions != executable_actions {
        return Err(OptionPolicyError::CatalogMismatch);
    }
    Ok(())
}

fn execute_candidate(
    candidate: &TacticOptionCandidate,
    tape_prefix: &InputTape,
    cancellation: Option<TacticCancellationHit>,
) -> Result<(InputTape, OptionExecution), OptionPolicyError> {
    tape_prefix.validate().map_err(OptionPolicyError::Tape)?;
    let realization = candidate.plan.realize(cancellation)?;
    let start_frame = tape_prefix.frames.len() as u64;
    let end_frame_exclusive = start_frame
        .checked_add(u64::from(realization.realized_ticks))
        .ok_or(OptionPolicyError::RangeOverflow)?;
    let mut tape = tape_prefix.clone();
    tape.frames.extend(realization.frames);
    tape.validate().map_err(OptionPolicyError::Tape)?;
    let execution = candidate.plan.capture_execution(
        candidate.descriptor.option_id.clone(),
        &tape,
        TapeRange {
            start_frame,
            end_frame_exclusive,
        },
        cancellation,
    )?;
    execution
        .validate_against_tape(&tape)
        .map_err(|error| OptionPolicyError::Execution(error.to_string()))?;
    if descriptor(&execution) != candidate.descriptor {
        return Err(OptionPolicyError::DescriptorMismatch);
    }
    Ok((tape, execution))
}

fn capture_isolated(
    option_id: &str,
    plan: &GameTacticPlan,
) -> Result<OptionExecution, OptionPolicyError> {
    let realization = plan.realize(None)?;
    let tape = InputTape {
        frames: realization.frames,
        ..InputTape::default()
    };
    plan.capture_execution(
        option_id.into(),
        &tape,
        TapeRange {
            start_frame: 0,
            end_frame_exclusive: u64::from(realization.realized_ticks),
        },
        None,
    )
    .map_err(OptionPolicyError::from)
}

fn descriptor(execution: &OptionExecution) -> OptionActionDescriptor {
    OptionActionDescriptor {
        option_id: execution.option_id.clone(),
        option_type: execution.option_type.clone(),
        parameters: execution.parameters.clone(),
    }
}

fn canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, OptionPolicyError> {
    serde_json::to_vec(value).map_err(|error| OptionPolicyError::Serialization(error.to_string()))
}

#[derive(Debug)]
pub enum OptionPolicyError {
    CatalogMismatch,
    EmptyRanking,
    DescriptorMismatch,
    RangeOverflow,
    Values(String),
    Execution(String),
    Serialization(String),
    Tactic(GameTacticError),
    Tape(TapeError),
}

impl fmt::Display for OptionPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CatalogMismatch => {
                formatter.write_str("policy and executable option catalogs differ")
            }
            Self::EmptyRanking => formatter.write_str("option policy produced an empty ranking"),
            Self::DescriptorMismatch => {
                formatter.write_str("realized tactic descriptor differs from selected option")
            }
            Self::RangeOverflow => formatter.write_str("realized tactic tape range overflows"),
            Self::Values(message) => write!(formatter, "option policy ranking failed: {message}"),
            Self::Execution(message) => {
                write!(formatter, "option execution proof failed: {message}")
            }
            Self::Serialization(message) => {
                write!(formatter, "option catalog serialization failed: {message}")
            }
            Self::Tactic(error) => write!(formatter, "deterministic tactic failed: {error}"),
            Self::Tape(error) => write!(formatter, "option policy tape is invalid: {error}"),
        }
    }
}

impl Error for OptionPolicyError {}

impl From<GameTacticError> for OptionPolicyError {
    fn from(error: GameTacticError) -> Self {
        Self::Tactic(error)
    }
}

#[cfg(test)]
mod tests {
    use super::super::option_values::{OptionValueConfig, OptionValueModel, OptionValueSample};
    use super::*;
    use crate::artifact::Digest;
    use crate::fqi::FqiConfig;
    use crate::game_tactic::GameTactic;
    use crate::tape::InputFrame;

    fn sample(
        candidate: &TacticOptionCandidate,
        state: f32,
        reward: f32,
        tape_byte: u8,
    ) -> OptionValueSample {
        OptionValueSample {
            action: candidate.descriptor.clone(),
            state: vec![state],
            duration_ticks: candidate.plan.planned_ticks().unwrap(),
            reward,
            next_state: vec![state + 1.0],
            terminal: true,
            realized_tape_sha256: Digest([tape_byte; 32]),
        }
    }

    #[test]
    fn highest_option_value_executes_exact_tactic_and_captures_tape_proof() {
        let shield = TacticOptionCandidate::new(
            "shield".into(),
            GameTacticPlan::new(GameTactic::Shield { frames: 2 }),
        )
        .unwrap();
        let attack = TacticOptionCandidate::new(
            "attack".into(),
            GameTacticPlan::new(GameTactic::NormalAttack {
                direction_degrees: 0,
                magnitude: 100,
                press_frames: 1,
                recovery_frames: 1,
            }),
        )
        .unwrap();
        let samples = vec![
            sample(&shield, 0.0, -1.0, 1),
            sample(&attack, 0.0, 5.0, 2),
            sample(&shield, 1.0, -1.0, 3),
            sample(&attack, 1.0, 5.0, 4),
        ];
        let model = OptionValueModel::fit(
            1,
            &samples,
            &[1, 2, 3, 4],
            &OptionValueConfig {
                fitted_q: FqiConfig {
                    iterations: 8,
                    trees_per_action: 7,
                    bootstrap: false,
                    seed: 7,
                    ..FqiConfig::default()
                },
            },
        )
        .unwrap();
        let prefix = InputTape {
            frames: vec![InputFrame::default()],
            ..InputTape::default()
        };
        let step = select_and_execute(&model, &[0.0], &[shield, attack], &prefix, None).unwrap();
        assert_eq!(step.selected.descriptor.option_id, "attack");
        assert_eq!(step.execution.option_id, "attack");
        assert_eq!(step.execution.realized_tape_range.start_frame, 1);
        assert_eq!(step.execution.realized_tape_range.end_frame_exclusive, 3);
        assert_eq!(step.execution.emitted_raw_actions, step.tape.frames[1..3]);
        step.execution.validate_against_tape(&step.tape).unwrap();
        assert!(step.descriptor_matches_execution);
        assert!(step.raw_frames_match_tape);
        assert!(!step.promotion_authority);
    }

    #[test]
    fn refuses_to_rank_a_catalog_without_an_exact_executor_for_every_option() {
        let shield = TacticOptionCandidate::new(
            "shield".into(),
            GameTacticPlan::new(GameTactic::Shield { frames: 2 }),
        )
        .unwrap();
        let attack = TacticOptionCandidate::new(
            "attack".into(),
            GameTacticPlan::new(GameTactic::NormalAttack {
                direction_degrees: 0,
                magnitude: 100,
                press_frames: 1,
                recovery_frames: 1,
            }),
        )
        .unwrap();
        let model = OptionValueModel::fit(
            1,
            &[sample(&shield, 0.0, 0.0, 1), sample(&attack, 0.0, 1.0, 2)],
            &[1, 2],
            &OptionValueConfig::default(),
        )
        .unwrap();
        assert!(matches!(
            select_and_execute(&model, &[0.0], &[shield], &InputTape::default(), None),
            Err(OptionPolicyError::CatalogMismatch)
        ));
    }
}
