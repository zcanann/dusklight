//! Authenticated conversion from one realized tactic boundary into the
//! existing semi-Markov option-value sample.

use crate::artifact::Digest;
use crate::fact_snapshot::{FactPhase, FactSnapshot};
use crate::option_values::{OptionActionDescriptor, OptionValueSample};
use crate::tape::InputTape;
use dusklight_control::option_execution::{OptionExecution, TapeRange};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::error::Error;
use std::fmt;

pub const OPTION_TRANSITION_SAMPLE_SCHEMA_V1: &str = "dusklight-option-transition-sample/v1";
pub const MAX_OPTION_INTERMEDIATE_BOUNDARIES: usize = 256;

fn digest_is_zero(value: &Digest) -> bool {
    *value == Digest::ZERO
}

/// The replay row retains the exact facts and execution proof that produced
/// the compact `OptionValueSample`. The Q implementation continues to consume
/// only `value_sample`; checkpoint and PAD provenance remain inspectable and
/// cannot drift away from it.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptionIntermediateBoundary {
    pub episode_shard_sha256: Digest,
    pub offset_ticks: u32,
    pub state_sha256: Digest,
    pub state: FactSnapshot,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptionTransitionSample {
    pub schema: String,
    #[serde(default, skip_serializing_if = "digest_is_zero")]
    pub execution_authority_sha256: Digest,
    pub feature_schema_sha256: Digest,
    pub before_state_sha256: Digest,
    pub after_state_sha256: Digest,
    pub source_checkpoint_sha256: Digest,
    pub next_checkpoint_sha256: Digest,
    pub before: FactSnapshot,
    pub after: FactSnapshot,
    pub execution: OptionExecution,
    pub value_sample: OptionValueSample,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub intermediate_boundaries: Vec<OptionIntermediateBoundary>,
}

impl OptionTransitionSample {
    pub fn replay_identity_sha256(&self) -> Result<Digest, OptionTransitionError> {
        self.validate_and_replay_identity_sha256()
    }

    /// Validate the complete replay row and derive its identity in the same
    /// pass. Callers that must both authenticate and index a row should use
    /// this method instead of validating and then calling
    /// [`Self::replay_identity_sha256`], which would re-authenticate every
    /// retained fact snapshot.
    pub fn validate_and_replay_identity_sha256(&self) -> Result<Digest, OptionTransitionError> {
        self.validate()?;
        self.replay_identity_sha256_after_validation()
    }

    fn replay_identity_sha256_after_validation(&self) -> Result<Digest, OptionTransitionError> {
        let mut hasher = Sha256::new();
        if self.execution_authority_sha256 == Digest::ZERO {
            hasher.update(b"dusklight-option-transition-replay-identity/v1");
        } else {
            hasher.update(b"dusklight-option-transition-replay-identity/v2");
            hasher.update(self.execution_authority_sha256.0);
        }
        hasher.update(self.feature_schema_sha256.0);
        hasher.update(
            self.value_sample
                .replay_identity_sha256()
                .map_err(|error| OptionTransitionError::InvalidOwned(error.to_string()))?
                .0,
        );
        Ok(Digest(hasher.finalize().into()))
    }

    /// Build one semi-Markov replay row from the same facts, tape, and
    /// execution record observed at the native tactic boundary.
    #[allow(clippy::too_many_arguments)]
    pub fn capture<E, F>(
        feature_schema_sha256: Digest,
        source_checkpoint_sha256: Digest,
        next_checkpoint_sha256: Digest,
        before: FactSnapshot,
        after: FactSnapshot,
        execution: OptionExecution,
        tape: &InputTape,
        reward: f32,
        terminal: bool,
        encode: F,
    ) -> Result<Self, OptionTransitionError>
    where
        E: fmt::Display,
        F: Fn(&FactSnapshot) -> Result<Vec<f32>, E>,
    {
        before
            .validate()
            .map_err(|error| OptionTransitionError::Facts(error.to_string()))?;
        after
            .validate()
            .map_err(|error| OptionTransitionError::Facts(error.to_string()))?;
        execution
            .validate_against_tape(tape)
            .map_err(|error| OptionTransitionError::Execution(error.to_string()))?;
        let state =
            encode(&before).map_err(|error| OptionTransitionError::Features(error.to_string()))?;
        let next_state =
            encode(&after).map_err(|error| OptionTransitionError::Features(error.to_string()))?;
        let before_state_sha256 = before
            .content_sha256()
            .map_err(|error| OptionTransitionError::Facts(error.to_string()))?;
        let after_state_sha256 = after
            .content_sha256()
            .map_err(|error| OptionTransitionError::Facts(error.to_string()))?;
        let value_sample = OptionValueSample {
            action: descriptor(&execution),
            state,
            duration_ticks: execution.duration.realized_ticks,
            reward,
            next_state,
            terminal,
            before_state_sha256,
            after_state_sha256,
            source_checkpoint_sha256,
            next_checkpoint_sha256,
            realized_tape_range: execution.realized_tape_range,
            realized_tape_sha256: emitted_pad_digest(&execution)?,
        };
        let row = Self {
            schema: OPTION_TRANSITION_SAMPLE_SCHEMA_V1.into(),
            execution_authority_sha256: Digest::ZERO,
            feature_schema_sha256,
            before_state_sha256,
            after_state_sha256,
            source_checkpoint_sha256,
            next_checkpoint_sha256,
            before,
            after,
            execution,
            value_sample,
            intermediate_boundaries: Vec::new(),
        };
        row.validate()?;
        Ok(row)
    }

    pub fn validate(&self) -> Result<(), OptionTransitionError> {
        if self.schema != OPTION_TRANSITION_SAMPLE_SCHEMA_V1
            || self.feature_schema_sha256 == Digest::ZERO
            || self.before_state_sha256 == Digest::ZERO
            || self.after_state_sha256 == Digest::ZERO
            || self.source_checkpoint_sha256 == Digest::ZERO
            || self.next_checkpoint_sha256 == Digest::ZERO
        {
            return Err(OptionTransitionError::Invalid(
                "transition identity is missing or unsupported",
            ));
        }
        self.execution
            .validate()
            .map_err(|error| OptionTransitionError::Execution(error.to_string()))?;
        let before_state_sha256 = self
            .before
            .content_sha256()
            .map_err(|error| OptionTransitionError::Facts(error.to_string()))?;
        let after_state_sha256 = self
            .after
            .content_sha256()
            .map_err(|error| OptionTransitionError::Facts(error.to_string()))?;
        if self.before_state_sha256 != before_state_sha256
            || self.after_state_sha256 != after_state_sha256
        {
            return Err(OptionTransitionError::Invalid(
                "fact identity is detached from the transition",
            ));
        }

        let duration = u64::from(self.execution.duration.realized_ticks);
        let expected_end = self
            .execution
            .realized_tape_range
            .start_frame
            .checked_add(duration)
            .ok_or(OptionTransitionError::Invalid(
                "realized tape range overflows",
            ))?;
        let expected_simulation_end = self.before.simulation_tick.checked_add(duration).ok_or(
            OptionTransitionError::Invalid("simulation tick range overflows"),
        )?;
        let after_simulation_boundary = match self.after.phase {
            FactPhase::PostSimulation => self.after.simulation_tick.checked_add(1),
            FactPhase::PreInput | FactPhase::TacticBoundary => Some(self.after.simulation_tick),
        }
        .ok_or(OptionTransitionError::Invalid(
            "after-fact simulation boundary overflows",
        ))?;
        let after_boundary_frame = match self.after.phase {
            FactPhase::PostSimulation => self.after.tape_frame.checked_add(1),
            FactPhase::PreInput | FactPhase::TacticBoundary => Some(self.after.tape_frame),
        }
        .ok_or(OptionTransitionError::Invalid(
            "after-fact tape boundary overflows",
        ))?;
        if self.execution.realized_tape_range
            != (TapeRange {
                start_frame: self.before.tape_frame,
                end_frame_exclusive: after_boundary_frame,
            })
            || expected_end != after_boundary_frame
            || expected_simulation_end != after_simulation_boundary
            || self.after.boundary_index < self.before.boundary_index
        {
            return Err(OptionTransitionError::Invalid(
                "facts, duration, and realized PAD range are not one boundary",
            ));
        }

        if self.after.terminal.configured != Some(true)
            || self.after.terminal.reached != Some(self.value_sample.terminal)
        {
            return Err(OptionTransitionError::Invalid(
                "sample terminal does not come from the after-fact verdict",
            ));
        }
        if self.value_sample.action != descriptor(&self.execution)
            || self.value_sample.duration_ticks != self.execution.duration.realized_ticks
            || self.value_sample.before_state_sha256 != self.before_state_sha256
            || self.value_sample.after_state_sha256 != self.after_state_sha256
            || self.value_sample.source_checkpoint_sha256 != self.source_checkpoint_sha256
            || self.value_sample.next_checkpoint_sha256 != self.next_checkpoint_sha256
            || self.value_sample.realized_tape_range != self.execution.realized_tape_range
            || self.value_sample.realized_tape_sha256 != emitted_pad_digest(&self.execution)?
            || self.value_sample.state.is_empty()
            || self.value_sample.state.len() != self.value_sample.next_state.len()
            || self
                .value_sample
                .state
                .iter()
                .chain(&self.value_sample.next_state)
                .any(|value| !value.is_finite())
            || !self.value_sample.reward.is_finite()
        {
            return Err(OptionTransitionError::Invalid(
                "value sample is detached from its realized transition",
            ));
        }
        if self.intermediate_boundaries.len() > MAX_OPTION_INTERMEDIATE_BOUNDARIES {
            return Err(OptionTransitionError::Invalid(
                "transition has too many intermediate boundaries",
            ));
        }
        let mut previous_offset = 0_u32;
        for boundary in &self.intermediate_boundaries {
            let expected_tape_frame = self
                .execution
                .realized_tape_range
                .start_frame
                .checked_add(u64::from(boundary.offset_ticks))
                .ok_or(OptionTransitionError::Invalid(
                    "intermediate tape frame overflows",
                ))?;
            let expected_simulation_tick = self
                .before
                .simulation_tick
                .checked_add(u64::from(boundary.offset_ticks))
                .ok_or(OptionTransitionError::Invalid(
                    "intermediate simulation tick overflows",
                ))?;
            if boundary.episode_shard_sha256 == Digest::ZERO
                || boundary.state_sha256 == Digest::ZERO
            {
                return Err(OptionTransitionError::Invalid(
                    "intermediate boundary has no evidence identity",
                ));
            }
            if boundary.offset_ticks <= previous_offset
                || boundary.offset_ticks >= self.execution.duration.realized_ticks
            {
                return Err(OptionTransitionError::Invalid(
                    "intermediate boundary offset is outside its native option",
                ));
            }
            if boundary.state.phase != FactPhase::PreInput {
                return Err(OptionTransitionError::Invalid(
                    "intermediate boundary is not a pre-input state",
                ));
            }
            if boundary.state.tape_frame != expected_tape_frame {
                return Err(OptionTransitionError::Invalid(
                    "intermediate boundary tape frame is detached from its native option",
                ));
            }
            if boundary.state.simulation_tick != expected_simulation_tick {
                return Err(OptionTransitionError::Invalid(
                    "intermediate boundary simulation tick is detached from its native option",
                ));
            }
            if boundary.state.terminal.configured != Some(true)
                || boundary.state.terminal.reached != Some(false)
            {
                return Err(OptionTransitionError::Invalid(
                    "intermediate boundary has invalid terminal evidence",
                ));
            }
            let state_sha256 = boundary
                .state
                .content_sha256()
                .map_err(|error| OptionTransitionError::Facts(error.to_string()))?;
            if boundary.state_sha256 != state_sha256 {
                return Err(OptionTransitionError::Invalid(
                    "intermediate boundary state digest is stale",
                ));
            }
            previous_offset = boundary.offset_ticks;
        }
        Ok(())
    }
}

fn descriptor(execution: &OptionExecution) -> OptionActionDescriptor {
    OptionActionDescriptor {
        option_id: execution.option_id.clone(),
        option_type: execution.option_type.clone(),
        parameters: execution.parameters.clone(),
    }
}

fn emitted_pad_digest(execution: &OptionExecution) -> Result<Digest, OptionTransitionError> {
    let encoded = serde_json::to_vec(&execution.emitted_raw_actions)
        .map_err(|error| OptionTransitionError::Serialization(error.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(OPTION_TRANSITION_SAMPLE_SCHEMA_V1.as_bytes());
    hasher.update((encoded.len() as u64).to_le_bytes());
    hasher.update(encoded);
    Ok(Digest(hasher.finalize().into()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OptionTransitionError {
    Invalid(&'static str),
    InvalidOwned(String),
    Facts(String),
    Execution(String),
    Features(String),
    Serialization(String),
}

impl fmt::Display for OptionTransitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => write!(formatter, "invalid option transition: {message}"),
            Self::InvalidOwned(message) => {
                write!(formatter, "invalid option transition: {message}")
            }
            Self::Facts(message) => write!(formatter, "option transition facts failed: {message}"),
            Self::Execution(message) => {
                write!(formatter, "option transition execution failed: {message}")
            }
            Self::Features(message) => {
                write!(
                    formatter,
                    "option transition feature projection failed: {message}"
                )
            }
            Self::Serialization(message) => {
                write!(
                    formatter,
                    "option transition serialization failed: {message}"
                )
            }
        }
    }
}

impl Error for OptionTransitionError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fact_snapshot::FactTerminalReason;
    use crate::tape::{InputFrame, InputTape};
    use dusklight_control::option_execution::{
        OptionCondition, OptionEndReason, OptionParameter, OptionType,
    };
    use dusklight_evidence::native_episode_shard::NativeEpisodeShard;
    use std::collections::BTreeMap;

    fn row() -> OptionTransitionSample {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        let step = &shard.episodes[0].steps[0];
        let mut before =
            FactSnapshot::from_native_learning(&step.pre_input, &[], None, Vec::new()).unwrap();
        let mut after = FactSnapshot::from_native_learning(
            &step.post_simulation,
            &[step.pre_input.clone()],
            None,
            Vec::new(),
        )
        .unwrap();
        // The fixture's goal evidence is authoritative but may represent a
        // different historical objective. Bind this unit transition to a
        // configured, unreached test objective without changing native facts.
        before.terminal.configured = Some(true);
        before.terminal.reached = Some(false);
        before.terminal.reason = FactTerminalReason::None;
        after.terminal.configured = Some(true);
        after.terminal.reached = Some(false);
        after.terminal.reason = FactTerminalReason::None;

        assert_eq!(after.tape_frame, before.tape_frame);
        assert_eq!(after.simulation_tick, before.simulation_tick);
        let mut tape = InputTape {
            frames: vec![InputFrame::default(); after.tape_frame as usize + 1],
            ..InputTape::default()
        };
        tape.frames[before.tape_frame as usize] = InputFrame::default();
        let execution = OptionExecution::capture(
            "wait".into(),
            OptionType::Neutral,
            BTreeMap::<String, OptionParameter>::new(),
            1,
            1,
            OptionCondition::DurationElapsed,
            Vec::new(),
            OptionEndReason::Completed,
            &tape,
            TapeRange {
                start_frame: before.tape_frame,
                end_frame_exclusive: after.tape_frame + 1,
            },
        )
        .unwrap();
        OptionTransitionSample::capture(
            Digest([1; 32]),
            Digest([2; 32]),
            Digest([3; 32]),
            before,
            after,
            execution,
            &tape,
            -0.01,
            false,
            |facts| Ok::<_, &'static str>(vec![facts.player.position_f32_bits[0] as f32]),
        )
        .unwrap()
    }

    fn row_with_intermediate_boundary() -> OptionTransitionSample {
        let base = row();
        let before = base.before;
        let mut after = base.after;
        after.phase = FactPhase::PreInput;
        after.boundary_index = before.boundary_index + 8;
        after.simulation_tick = before.simulation_tick + 8;
        after.tape_frame = before.tape_frame + 8;
        after.state_identity = [8; 16];
        let tape = InputTape {
            frames: vec![InputFrame::default(); after.tape_frame as usize],
            ..InputTape::default()
        };
        let execution = OptionExecution::capture(
            "wait-eight".into(),
            OptionType::Neutral,
            BTreeMap::<String, OptionParameter>::new(),
            8,
            8,
            OptionCondition::DurationElapsed,
            Vec::new(),
            OptionEndReason::Completed,
            &tape,
            TapeRange {
                start_frame: before.tape_frame,
                end_frame_exclusive: after.tape_frame,
            },
        )
        .unwrap();
        let mut row = OptionTransitionSample::capture(
            Digest([1; 32]),
            Digest([2; 32]),
            Digest([3; 32]),
            before.clone(),
            after,
            execution,
            &tape,
            -0.08,
            false,
            |facts| Ok::<_, &'static str>(vec![facts.player.position_f32_bits[0] as f32]),
        )
        .unwrap();
        let mut state = before;
        state.boundary_index += 4;
        state.simulation_tick += 4;
        state.tape_frame += 4;
        state.state_identity = [4; 16];
        let state_sha256 = state.content_sha256().unwrap();
        row.intermediate_boundaries = vec![OptionIntermediateBoundary {
            episode_shard_sha256: Digest([9; 32]),
            offset_ticks: 4,
            state_sha256,
            state,
        }];
        row
    }

    #[test]
    fn converts_exact_option_boundary_into_existing_value_sample() {
        let row = row();
        row.validate().unwrap();
        assert_eq!(row.value_sample.action.option_id, "wait");
        assert_eq!(row.value_sample.duration_ticks, 1);
        assert_eq!(
            row.execution.realized_tape_range,
            TapeRange {
                start_frame: row.before.tape_frame,
                end_frame_exclusive: row.after.tape_frame + 1,
            }
        );
        assert_ne!(row.value_sample.realized_tape_sha256, Digest::ZERO);
    }

    #[test]
    fn execution_authority_partitions_replay_identity_without_changing_legacy_rows() {
        let legacy = row();
        let legacy_identity = legacy.replay_identity_sha256().unwrap();
        assert_eq!(
            legacy.validate_and_replay_identity_sha256().unwrap(),
            legacy_identity
        );
        assert!(
            !serde_json::to_vec(&legacy)
                .unwrap()
                .windows("execution_authority_sha256".len())
                .any(|window| window == b"execution_authority_sha256")
        );

        let mut first_plan = legacy.clone();
        first_plan.execution_authority_sha256 = Digest([8; 32]);
        let mut second_plan = legacy;
        second_plan.execution_authority_sha256 = Digest([9; 32]);

        assert_ne!(
            first_plan.replay_identity_sha256().unwrap(),
            legacy_identity
        );
        assert_ne!(
            first_plan.replay_identity_sha256().unwrap(),
            second_plan.replay_identity_sha256().unwrap()
        );
    }

    #[test]
    fn rejects_terminal_duration_checkpoint_and_pad_detachment() {
        let original = row();

        let mut tampered = original.clone();
        tampered.value_sample.terminal = true;
        assert!(tampered.validate().is_err());

        tampered = original.clone();
        tampered.value_sample.duration_ticks = 2;
        assert!(tampered.validate().is_err());

        tampered = original.clone();
        tampered.source_checkpoint_sha256 = Digest::ZERO;
        assert!(tampered.validate().is_err());

        tampered = original;
        tampered.value_sample.realized_tape_sha256 = Digest([9; 32]);
        assert!(tampered.validate().is_err());
    }

    #[test]
    fn validates_exact_intermediate_native_boundaries() {
        let original = row_with_intermediate_boundary();
        original.validate().unwrap();
        assert_eq!(original.intermediate_boundaries[0].offset_ticks, 4);

        let mut tampered = original.clone();
        tampered.intermediate_boundaries[0].offset_ticks = 8;
        assert!(tampered.validate().is_err());

        tampered = original;
        tampered.intermediate_boundaries[0].state.tape_frame += 1;
        assert!(tampered.validate().is_err());
    }
}
