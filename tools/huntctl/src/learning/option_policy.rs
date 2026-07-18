//! High-level option selection joined to deterministic low-level tape proof.

use super::option_values::{
    OptionActionDescriptor, OptionValueError, OptionValueModel, RankedOption,
};
use crate::game_tactic::{GameTacticError, GameTacticPlan, TacticCancellationHit};
use crate::option_execution::{OptionExecution, OptionExecutionError, TapeRange};
use crate::tape::InputTape;
use serde::Serialize;
use std::error::Error;
use std::fmt;

pub const PROVED_OPTION_POLICY_STEP_SCHEMA_V1: &str = "dusklight-proved-option-policy-step/v1";

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SelectedOption {
    pub selection: RankedOption,
    pub policy_layer: &'static str,
    pub low_level_contract: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProvedOptionPolicyStep {
    pub schema: &'static str,
    pub selection: RankedOption,
    pub execution: OptionExecution,
    pub policy_layer: &'static str,
    pub executor_layer: &'static str,
    pub proof_layer: &'static str,
    pub promotion_authority: bool,
}

impl SelectedOption {
    pub fn select(model: &OptionValueModel, state: &[f32]) -> Result<Self, OptionPolicyError> {
        let selection = model
            .rank_options(state)?
            .into_iter()
            .next()
            .ok_or(OptionPolicyError::NoAvailableOption)?;
        Ok(Self {
            selection,
            policy_layer: "learned_high_level_option_value",
            low_level_contract: "deterministic_tactic_then_exact_realized_tape",
        })
    }

    pub fn execute_game_tactic(
        self,
        plan: &GameTacticPlan,
        tape: &InputTape,
        range: TapeRange,
        cancellation: Option<TacticCancellationHit>,
    ) -> Result<ProvedOptionPolicyStep, OptionPolicyError> {
        let execution = plan.capture_execution(
            self.selection.descriptor.option_id.clone(),
            tape,
            range,
            cancellation,
        )?;
        self.prove_execution(execution, tape, "deterministic_game_tactic_plan")
    }

    /// Common proof boundary for other deterministic option executors.
    pub fn prove_execution(
        self,
        execution: OptionExecution,
        tape: &InputTape,
        executor_layer: &'static str,
    ) -> Result<ProvedOptionPolicyStep, OptionPolicyError> {
        execution.validate_against_tape(tape)?;
        let realized_descriptor = OptionActionDescriptor {
            option_id: execution.option_id.clone(),
            option_type: execution.option_type.clone(),
            parameters: execution.parameters.clone(),
        };
        if realized_descriptor != self.selection.descriptor {
            return Err(OptionPolicyError::DescriptorMismatch {
                selected: self.selection.descriptor,
                realized: realized_descriptor,
            });
        }
        Ok(ProvedOptionPolicyStep {
            schema: PROVED_OPTION_POLICY_STEP_SCHEMA_V1,
            selection: self.selection,
            execution,
            policy_layer: self.policy_layer,
            executor_layer,
            proof_layer: "option_execution_validated_against_complete_canonical_tape",
            promotion_authority: false,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum OptionPolicyError {
    Values(String),
    Tactic(String),
    Execution(String),
    NoAvailableOption,
    DescriptorMismatch {
        selected: OptionActionDescriptor,
        realized: OptionActionDescriptor,
    },
}

impl fmt::Display for OptionPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Values(message) => write!(formatter, "option policy ranking failed: {message}"),
            Self::Tactic(message) => write!(formatter, "option tactic execution failed: {message}"),
            Self::Execution(message) => {
                write!(formatter, "option execution proof failed: {message}")
            }
            Self::NoAvailableOption => formatter.write_str("option policy has no available option"),
            Self::DescriptorMismatch { .. } => formatter
                .write_str("selected option descriptor differs from realized tactic descriptor"),
        }
    }
}

impl Error for OptionPolicyError {}

impl From<OptionValueError> for OptionPolicyError {
    fn from(error: OptionValueError) -> Self {
        Self::Values(error.to_string())
    }
}

impl From<GameTacticError> for OptionPolicyError {
    fn from(error: GameTacticError) -> Self {
        Self::Tactic(error.to_string())
    }
}

impl From<OptionExecutionError> for OptionPolicyError {
    fn from(error: OptionExecutionError) -> Self {
        Self::Execution(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::Digest;
    use crate::game_tactic::GameTactic;
    use crate::learning::fqi::FqiConfig;
    use crate::learning::option_values::{OptionValueConfig, OptionValueSample};
    use crate::tape::{InputFrame, TapeBoot};

    fn sample(
        descriptor: OptionActionDescriptor,
        reward: f32,
        tape_digest: u8,
    ) -> OptionValueSample {
        OptionValueSample {
            action: descriptor,
            state: vec![0.0],
            duration_ticks: 2,
            reward,
            next_state: vec![1.0],
            terminal: true,
            realized_tape_sha256: Digest([tape_digest; 32]),
        }
    }

    fn trained_policy_and_tape() -> (OptionValueModel, GameTacticPlan, InputTape, TapeRange) {
        let plan = GameTacticPlan::new(GameTactic::NormalAttack {
            direction_degrees: 0,
            magnitude: 100,
            press_frames: 1,
            recovery_frames: 1,
        });
        let realization = plan.realize(None).unwrap();
        let mut frames = vec![InputFrame::default()];
        frames.extend(realization.frames);
        frames.push(InputFrame::default());
        let tape = InputTape {
            boot: TapeBoot::Process,
            frames,
            ..InputTape::default()
        };
        let range = TapeRange {
            start_frame: 1,
            end_frame_exclusive: 3,
        };
        let attack = plan
            .capture_execution("attack".into(), &tape, range, None)
            .unwrap();
        let attack_descriptor = OptionActionDescriptor {
            option_id: attack.option_id,
            option_type: attack.option_type,
            parameters: attack.parameters,
        };
        let mut wait_descriptor = attack_descriptor.clone();
        wait_descriptor.option_id = "wait".into();
        wait_descriptor.option_type = crate::option_execution::OptionType::Neutral;
        wait_descriptor.parameters.clear();
        let config = OptionValueConfig {
            fitted_q: FqiConfig {
                iterations: 8,
                trees_per_action: 5,
                max_tree_depth: 2,
                bootstrap: false,
                seed: 7,
                ..FqiConfig::default()
            },
        };
        let model = OptionValueModel::fit(
            1,
            &[
                sample(attack_descriptor.clone(), 5.0, 1),
                sample(wait_descriptor.clone(), -1.0, 2),
                sample(attack_descriptor, 5.0, 3),
                sample(wait_descriptor, -1.0, 4),
            ],
            &[1, 2, 3, 4],
            &config,
        )
        .unwrap();
        (model, plan, tape, range)
    }

    #[test]
    fn selected_tactic_is_deterministically_realized_and_tape_proved() {
        let (model, plan, tape, range) = trained_policy_and_tape();
        let selected = SelectedOption::select(&model, &[0.0]).unwrap();
        assert_eq!(selected.selection.descriptor.option_id, "attack");
        let step = selected
            .execute_game_tactic(&plan, &tape, range, None)
            .unwrap();
        assert_eq!(
            step.selection.descriptor.option_id,
            step.execution.option_id
        );
        assert_eq!(step.execution.emitted_raw_actions, tape.frames[1..3]);
        assert_eq!(step.executor_layer, "deterministic_game_tactic_plan");
        assert!(!step.promotion_authority);
    }

    #[test]
    fn descriptor_or_complete_tape_mismatch_cannot_be_proved() {
        let (model, plan, tape, range) = trained_policy_and_tape();
        let mismatched = plan
            .capture_execution("different-id".into(), &tape, range, None)
            .unwrap();
        assert!(matches!(
            SelectedOption::select(&model, &[0.0])
                .unwrap()
                .prove_execution(mismatched, &tape, "test"),
            Err(OptionPolicyError::DescriptorMismatch { .. })
        ));

        let execution = plan
            .capture_execution("attack".into(), &tape, range, None)
            .unwrap();
        let mut changed_tape = tape;
        changed_tape.frames[0].pads[0].buttons = 1;
        assert!(
            SelectedOption::select(&model, &[0.0])
                .unwrap()
                .prove_execution(execution, &changed_tape, "test")
                .is_err()
        );
    }
}
