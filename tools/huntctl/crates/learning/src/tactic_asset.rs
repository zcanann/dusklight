//! One runtime adapter contract over Dusklight's existing tactic representations.
//!
//! This module deliberately does not introduce another serialized action language.
//! Each adapter validates, identifies, describes, and—when possible—realizes the
//! existing source plan using its current canonical encoding and evaluator.

use crate::artifact::Digest;
use crate::native_generic_tactic::{
    GenericTactic, NATIVE_GENERIC_TACTIC_SCHEMA_V1, NativeGenericTacticPlan,
};
use crate::option_policy::TacticOptionCandidate;
use crate::option_values::OptionActionDescriptor;
use dusklight_control::controller_compilation::{
    ControllerObservationField, ControllerObservationProvenance, StaticControllerError,
    compile_static_controller,
};
use dusklight_control::controller_program::{ControllerProgram, VERSION_MAJOR, VERSION_MINOR};
use dusklight_control::game_tactic::{GAME_TACTIC_SCHEMA_V1, GameTacticPlan};
use dusklight_control::motion_path::{MOTION_PATH_SCHEMA_V1, MotionPathPlan};
use dusklight_control::option_execution::{
    MAX_OPTION_CONDITIONS, OptionCondition, OptionEndReason, OptionExecution, OptionParameter,
    OptionType, TapeRange, validate_condition,
};
use dusklight_control::tape::{InputFrame, InputTape};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const TACTIC_ASSET_ADAPTER_SCHEMA_V1: &str = "dusklight-tactic-asset-adapter/v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TacticAssetKind {
    GameTactic,
    NativeGenericTactic,
    MotionPath,
    ReactiveController,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TacticObservationRequirement {
    SimulationTick,
    PlayerPosition,
    PlayerYaw,
    PlayerVelocity,
    PlayerProcedure,
    PlayerActionLane,
    CameraYaw,
    StageName,
    ActorIdentity,
    ActorPosition,
    ActorSnapshotCompleteness,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TacticApplicability {
    /// The plan can always emit its input recipe. Gameplay acceptance is not
    /// implied.
    InputOnly,
    /// The plan needs the listed read-only observation families to emit input.
    ObservationBound,
    /// The input recipe is valid, but a procedure/mode predicate must decide
    /// whether the game can currently accept it.
    GameContextRequired,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TacticExecutor {
    StaticPlan,
    NativeGenericObservationLoop,
    ReactiveControllerProgram,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticDurationBounds {
    pub minimum_ticks: u32,
    pub maximum_ticks: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticStoppingContract {
    pub termination: OptionCondition,
    pub cancellation: Vec<OptionCondition>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticAssetDescription {
    pub schema: String,
    pub kind: TacticAssetKind,
    pub source_schema: String,
    pub content_sha256: Digest,
    pub option: OptionActionDescriptor,
    pub duration: TacticDurationBounds,
    pub applicability: TacticApplicability,
    pub required_observations: BTreeSet<TacticObservationRequirement>,
    pub executor: TacticExecutor,
    pub stopping: TacticStoppingContract,
    pub statically_realizable: bool,
}

impl TacticAssetDescription {
    pub fn validate(&self) -> Result<(), TacticAssetError> {
        if self.schema != TACTIC_ASSET_ADAPTER_SCHEMA_V1
            || self.content_sha256 == Digest::ZERO
            || self.duration.minimum_ticks == 0
            || self.duration.minimum_ticks > self.duration.maximum_ticks
        {
            return Err(invalid(
                "tactic description identity or duration is invalid",
            ));
        }
        self.option
            .validate()
            .map_err(|error| invalid(error.to_string()))?;
        validate_condition(&self.stopping.termination)
            .map_err(|error| invalid(error.to_string()))?;
        if self.stopping.cancellation.len() > MAX_OPTION_CONDITIONS {
            return Err(invalid("tactic has too many cancellation conditions"));
        }
        for condition in &self.stopping.cancellation {
            validate_condition(condition).map_err(|error| invalid(error.to_string()))?;
        }
        match self.applicability {
            TacticApplicability::InputOnly if !self.required_observations.is_empty() => {
                return Err(invalid(
                    "input-only tactic cannot declare observation requirements",
                ));
            }
            TacticApplicability::ObservationBound if self.required_observations.is_empty() => {
                return Err(invalid(
                    "observation-bound tactic must declare queried observations",
                ));
            }
            _ => {}
        }
        if self.statically_realizable != matches!(self.executor, TacticExecutor::StaticPlan) {
            return Err(invalid(
                "static realization and selected tactic executor disagree",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExactTacticRealization {
    pub tape: InputTape,
    pub execution: OptionExecution,
}

impl ExactTacticRealization {
    pub fn validate_against(
        &self,
        description: &TacticAssetDescription,
    ) -> Result<(), TacticAssetError> {
        description.validate()?;
        self.execution
            .validate_against_tape(&self.tape)
            .map_err(|error| invalid(error.to_string()))?;
        if descriptor(&self.execution) != description.option
            || self.execution.duration.minimum_ticks != description.duration.minimum_ticks
            || self.execution.duration.maximum_ticks != description.duration.maximum_ticks
            || self.execution.termination_condition != description.stopping.termination
            || self.execution.cancellation_conditions != description.stopping.cancellation
        {
            return Err(invalid(
                "exact realization differs from its tactic asset description",
            ));
        }
        Ok(())
    }
}

/// Runtime adapter implemented directly by every existing tactic plan type.
///
/// `canonical_bytes` always returns that plan's existing representation:
/// canonical JSON for typed Rust plans and canonical DUSKCTRL bytes for reactive
/// controllers.
pub trait TacticAssetAdapter {
    fn describe(&self, option_id: &str) -> Result<TacticAssetDescription, TacticAssetError>;

    fn canonical_bytes(&self) -> Result<Vec<u8>, TacticAssetError>;

    /// Returns exact raw frames only when the existing plan is observation-free.
    /// Reactive plans return `None` and remain owned by their native evaluator.
    fn static_frames(&self) -> Result<Option<Vec<InputFrame>>, TacticAssetError>;

    /// Captures the exact existing `OptionExecution` and authenticated tape for
    /// an observation-free plan. Observation-driven plans return `None` and
    /// name their existing executor in `describe`.
    fn exact_static_realization(
        &self,
        option_id: &str,
    ) -> Result<Option<ExactTacticRealization>, TacticAssetError>;
}

impl TacticAssetAdapter for GameTacticPlan {
    fn describe(&self, option_id: &str) -> Result<TacticAssetDescription, TacticAssetError> {
        validate_option_id(option_id)?;
        let candidate = TacticOptionCandidate::new(option_id.into(), self.clone())
            .map_err(|error| invalid(error.to_string()))?;
        let maximum_ticks = self
            .planned_ticks()
            .map_err(|error| invalid(error.to_string()))?;
        let canonical = self.canonical_bytes()?;
        checked(TacticAssetDescription {
            schema: TACTIC_ASSET_ADAPTER_SCHEMA_V1.into(),
            kind: TacticAssetKind::GameTactic,
            source_schema: GAME_TACTIC_SCHEMA_V1.into(),
            content_sha256: digest(&canonical),
            option: candidate.descriptor().clone(),
            duration: TacticDurationBounds {
                minimum_ticks: 1,
                maximum_ticks,
            },
            applicability: TacticApplicability::GameContextRequired,
            required_observations: BTreeSet::new(),
            executor: TacticExecutor::StaticPlan,
            stopping: TacticStoppingContract {
                termination: OptionCondition::DurationElapsed,
                cancellation: self.cancellation_conditions.clone(),
            },
            statically_realizable: true,
        })
    }

    fn canonical_bytes(&self) -> Result<Vec<u8>, TacticAssetError> {
        self.validate()
            .map_err(|error| invalid(error.to_string()))?;
        serde_json::to_vec(self).map_err(serialization)
    }

    fn static_frames(&self) -> Result<Option<Vec<InputFrame>>, TacticAssetError> {
        Ok(Some(
            self.realize(None)
                .map_err(|error| invalid(error.to_string()))?
                .frames,
        ))
    }

    fn exact_static_realization(
        &self,
        option_id: &str,
    ) -> Result<Option<ExactTacticRealization>, TacticAssetError> {
        let description = self.describe(option_id)?;
        let realization = self
            .realize(None)
            .map_err(|error| invalid(error.to_string()))?;
        let tape = tape(realization.frames);
        let execution = self
            .capture_execution(
                option_id.into(),
                &tape,
                TapeRange {
                    start_frame: 0,
                    end_frame_exclusive: tape.frames.len() as u64,
                },
                None,
            )
            .map_err(|error| invalid(error.to_string()))?;
        let exact = ExactTacticRealization { tape, execution };
        exact.validate_against(&description)?;
        Ok(Some(exact))
    }
}

impl TacticAssetAdapter for MotionPathPlan {
    fn describe(&self, option_id: &str) -> Result<TacticAssetDescription, TacticAssetError> {
        validate_option_id(option_id)?;
        let realization = self
            .realize(None)
            .map_err(|error| invalid(error.to_string()))?;
        let tape = tape(realization.frames);
        let execution = self
            .capture_execution(
                option_id.into(),
                &tape,
                TapeRange {
                    start_frame: 0,
                    end_frame_exclusive: tape.frames.len() as u64,
                },
                None,
            )
            .map_err(|error| invalid(error.to_string()))?;
        let canonical = self.canonical_bytes()?;
        checked(TacticAssetDescription {
            schema: TACTIC_ASSET_ADAPTER_SCHEMA_V1.into(),
            kind: TacticAssetKind::MotionPath,
            source_schema: MOTION_PATH_SCHEMA_V1.into(),
            content_sha256: digest(&canonical),
            option: descriptor(&execution),
            duration: TacticDurationBounds {
                minimum_ticks: 1,
                maximum_ticks: self.duration_ticks,
            },
            applicability: TacticApplicability::InputOnly,
            required_observations: BTreeSet::new(),
            executor: TacticExecutor::StaticPlan,
            stopping: TacticStoppingContract {
                termination: execution.termination_condition.clone(),
                cancellation: execution.cancellation_conditions.clone(),
            },
            statically_realizable: true,
        })
    }

    fn canonical_bytes(&self) -> Result<Vec<u8>, TacticAssetError> {
        self.validate()
            .map_err(|error| invalid(error.to_string()))?;
        serde_json::to_vec(self).map_err(serialization)
    }

    fn static_frames(&self) -> Result<Option<Vec<InputFrame>>, TacticAssetError> {
        Ok(Some(
            self.realize(None)
                .map_err(|error| invalid(error.to_string()))?
                .frames,
        ))
    }

    fn exact_static_realization(
        &self,
        option_id: &str,
    ) -> Result<Option<ExactTacticRealization>, TacticAssetError> {
        let description = self.describe(option_id)?;
        let realization = self
            .realize(None)
            .map_err(|error| invalid(error.to_string()))?;
        let tape = tape(realization.frames);
        let execution = self
            .capture_execution(
                option_id.into(),
                &tape,
                TapeRange {
                    start_frame: 0,
                    end_frame_exclusive: tape.frames.len() as u64,
                },
                None,
            )
            .map_err(|error| invalid(error.to_string()))?;
        let exact = ExactTacticRealization { tape, execution };
        exact.validate_against(&description)?;
        Ok(Some(exact))
    }
}

impl TacticAssetAdapter for NativeGenericTacticPlan {
    fn describe(&self, option_id: &str) -> Result<TacticAssetDescription, TacticAssetError> {
        validate_option_id(option_id)?;
        let option = self
            .descriptor(option_id.into())
            .map_err(|error| invalid(error.to_string()))?;
        let canonical = self.canonical_bytes()?;
        checked(TacticAssetDescription {
            schema: TACTIC_ASSET_ADAPTER_SCHEMA_V1.into(),
            kind: TacticAssetKind::NativeGenericTactic,
            source_schema: NATIVE_GENERIC_TACTIC_SCHEMA_V1.into(),
            content_sha256: digest(&canonical),
            option,
            duration: TacticDurationBounds {
                minimum_ticks: self.minimum_ticks,
                maximum_ticks: self.maximum_ticks,
            },
            applicability: TacticApplicability::ObservationBound,
            required_observations: native_requirements(&self.tactic),
            executor: TacticExecutor::NativeGenericObservationLoop,
            stopping: TacticStoppingContract {
                termination: self
                    .termination_condition()
                    .map_err(|error| invalid(error.to_string()))?,
                cancellation: Vec::new(),
            },
            statically_realizable: false,
        })
    }

    fn canonical_bytes(&self) -> Result<Vec<u8>, TacticAssetError> {
        self.validate()
            .map_err(|error| invalid(error.to_string()))?;
        serde_json::to_vec(self).map_err(serialization)
    }

    fn static_frames(&self) -> Result<Option<Vec<InputFrame>>, TacticAssetError> {
        self.validate()
            .map_err(|error| invalid(error.to_string()))?;
        Ok(None)
    }

    fn exact_static_realization(
        &self,
        option_id: &str,
    ) -> Result<Option<ExactTacticRealization>, TacticAssetError> {
        self.describe(option_id)?;
        Ok(None)
    }
}

impl TacticAssetAdapter for ControllerProgram {
    fn describe(&self, option_id: &str) -> Result<TacticAssetDescription, TacticAssetError> {
        validate_option_id(option_id)?;
        self.validate()
            .map_err(|error| invalid(error.to_string()))?;
        let canonical = self.canonical_bytes()?;
        let provenance = ControllerObservationProvenance::for_program(self);
        let required_observations = controller_requirements(&provenance);
        let statically_realizable = provenance.is_static();
        let mut parameters = BTreeMap::new();
        parameters.insert(
            "program_sha256".into(),
            OptionParameter::Digest(digest(&canonical)),
        );
        parameters.insert(
            "duration_ticks".into(),
            OptionParameter::Unsigned(u64::from(self.duration_frames)),
        );
        checked(TacticAssetDescription {
            schema: TACTIC_ASSET_ADAPTER_SCHEMA_V1.into(),
            kind: TacticAssetKind::ReactiveController,
            source_schema: format!("DUSKCTRL/{VERSION_MAJOR}.{VERSION_MINOR}"),
            content_sha256: digest(&canonical),
            option: OptionActionDescriptor {
                option_id: option_id.into(),
                option_type: OptionType::Custom("reactive_controller".into()),
                parameters,
            },
            duration: TacticDurationBounds {
                minimum_ticks: 1,
                maximum_ticks: self.duration_frames,
            },
            applicability: if statically_realizable {
                TacticApplicability::InputOnly
            } else {
                TacticApplicability::ObservationBound
            },
            required_observations,
            executor: if statically_realizable {
                TacticExecutor::StaticPlan
            } else {
                TacticExecutor::ReactiveControllerProgram
            },
            stopping: TacticStoppingContract {
                termination: OptionCondition::DurationElapsed,
                cancellation: Vec::new(),
            },
            statically_realizable,
        })
    }

    fn canonical_bytes(&self) -> Result<Vec<u8>, TacticAssetError> {
        self.encode().map_err(|error| invalid(error.to_string()))
    }

    fn static_frames(&self) -> Result<Option<Vec<InputFrame>>, TacticAssetError> {
        match compile_static_controller(self) {
            Ok(tape) => Ok(Some(tape.frames)),
            Err(StaticControllerError::Reactive(_)) => Ok(None),
            Err(StaticControllerError::InvalidProgram(message)) => Err(invalid(message)),
        }
    }

    fn exact_static_realization(
        &self,
        option_id: &str,
    ) -> Result<Option<ExactTacticRealization>, TacticAssetError> {
        let description = self.describe(option_id)?;
        let Some(frames) = self.static_frames()? else {
            return Ok(None);
        };
        let tape = tape(frames);
        let execution = OptionExecution::capture(
            description.option.option_id.clone(),
            description.option.option_type.clone(),
            description.option.parameters.clone(),
            description.duration.minimum_ticks,
            description.duration.maximum_ticks,
            description.stopping.termination.clone(),
            description.stopping.cancellation.clone(),
            OptionEndReason::Completed,
            &tape,
            TapeRange {
                start_frame: 0,
                end_frame_exclusive: tape.frames.len() as u64,
            },
        )
        .map_err(|error| invalid(error.to_string()))?;
        let exact = ExactTacticRealization { tape, execution };
        exact.validate_against(&description)?;
        Ok(Some(exact))
    }
}

fn checked(
    description: TacticAssetDescription,
) -> Result<TacticAssetDescription, TacticAssetError> {
    description.validate()?;
    Ok(description)
}

fn descriptor(
    execution: &dusklight_control::option_execution::OptionExecution,
) -> OptionActionDescriptor {
    OptionActionDescriptor {
        option_id: execution.option_id.clone(),
        option_type: execution.option_type.clone(),
        parameters: execution.parameters.clone(),
    }
}

fn tape(frames: Vec<InputFrame>) -> InputTape {
    InputTape {
        frames,
        ..InputTape::default()
    }
}

fn digest(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

fn validate_option_id(value: &str) -> Result<(), TacticAssetError> {
    if value.is_empty()
        || value.len() > 96
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/'))
    {
        return Err(TacticAssetError::InvalidOptionId);
    }
    Ok(())
}

fn native_requirements(tactic: &GenericTactic) -> BTreeSet<TacticObservationRequirement> {
    use TacticObservationRequirement as Requirement;
    match tactic {
        GenericTactic::MaintainRelativeHeading { .. } => {
            [Requirement::PlayerYaw, Requirement::CameraYaw]
                .into_iter()
                .collect()
        }
        GenericTactic::MaintainOffset { .. } | GenericTactic::SeekActor { .. } => [
            Requirement::PlayerPosition,
            Requirement::CameraYaw,
            Requirement::StageName,
            Requirement::ActorIdentity,
            Requirement::ActorPosition,
            Requirement::ActorSnapshotCompleteness,
        ]
        .into_iter()
        .collect(),
        GenericTactic::SeekCoordinate { .. } => {
            [Requirement::PlayerPosition, Requirement::CameraYaw]
                .into_iter()
                .collect()
        }
        GenericTactic::ShortCurve { .. } => [Requirement::SimulationTick].into_iter().collect(),
        GenericTactic::CameraWhileMoving { .. } => [Requirement::CameraYaw].into_iter().collect(),
        GenericTactic::SynchronizeButtonEdge { .. } => {
            [Requirement::PlayerProcedure, Requirement::PlayerActionLane]
                .into_iter()
                .collect()
        }
    }
}

fn controller_requirements(
    provenance: &ControllerObservationProvenance,
) -> BTreeSet<TacticObservationRequirement> {
    provenance
        .reactive_layers
        .iter()
        .flat_map(|layer| &layer.fields)
        .map(|field| match field {
            ControllerObservationField::PlayerPosition => {
                TacticObservationRequirement::PlayerPosition
            }
            ControllerObservationField::PlayerYaw => TacticObservationRequirement::PlayerYaw,
            ControllerObservationField::PlayerVelocity => {
                TacticObservationRequirement::PlayerVelocity
            }
            ControllerObservationField::CameraYaw => TacticObservationRequirement::CameraYaw,
            ControllerObservationField::StageName => TacticObservationRequirement::StageName,
            ControllerObservationField::ActorIdentity => {
                TacticObservationRequirement::ActorIdentity
            }
            ControllerObservationField::ActorPosition => {
                TacticObservationRequirement::ActorPosition
            }
            ControllerObservationField::ActorSnapshotCompleteness => {
                TacticObservationRequirement::ActorSnapshotCompleteness
            }
        })
        .collect()
}

fn invalid(message: impl Into<String>) -> TacticAssetError {
    TacticAssetError::InvalidAsset(message.into())
}

fn serialization(error: serde_json::Error) -> TacticAssetError {
    TacticAssetError::Serialization(error.to_string())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TacticAssetError {
    InvalidOptionId,
    InvalidAsset(String),
    Serialization(String),
}

impl fmt::Display for TacticAssetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOptionId => formatter.write_str("tactic option ID is invalid"),
            Self::InvalidAsset(message) => write!(formatter, "tactic asset is invalid: {message}"),
            Self::Serialization(message) => {
                write!(formatter, "tactic asset serialization failed: {message}")
            }
        }
    }
}

impl Error for TacticAssetError {}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_control::controller_program::ControllerProgram;
    use dusklight_control::game_tactic::{GameTactic, GameTacticPlan};
    use dusklight_control::motion_path::{MotionPathPlan, SamplePhase, StickPath, StickPoint};

    #[test]
    fn existing_plan_types_share_one_adapter_without_changing_realization() {
        let game = GameTacticPlan::new(GameTactic::Interact {
            press_frames: 1,
            recovery_frames: 2,
        });
        let path = MotionPathPlan {
            schema: MOTION_PATH_SCHEMA_V1.into(),
            path: StickPath::Bezier {
                control: [
                    StickPoint { x: 0, y: 127 },
                    StickPoint { x: 20, y: 100 },
                    StickPoint { x: 40, y: 80 },
                    StickPoint { x: 60, y: 60 },
                ],
            },
            duration_ticks: 4,
            sample_phase: SamplePhase::default(),
            cancellation_conditions: Vec::new(),
        };
        let native = NativeGenericTacticPlan::new(
            GenericTactic::SeekCoordinate {
                coordinate_f32_bits: [1.0_f32.to_bits(), 2.0_f32.to_bits(), 3.0_f32.to_bits()],
                tolerance_f32_bits: 0.5_f32.to_bits(),
                magnitude: 100,
            },
            8,
        );
        let controller = ControllerProgram::parse(
            "duskcontrol 1\nframes 4\nbezier replace from 0 for 4 p0 0 127 p1 20 100 p2 40 80 p3 60 60\n",
        )
        .unwrap();

        let game_description = game.describe("interact").unwrap();
        let path_description = path.describe("curve").unwrap();
        let native_description = native.describe("seek").unwrap();
        let controller_description = controller.describe("controller").unwrap();

        for description in [
            &game_description,
            &path_description,
            &native_description,
            &controller_description,
        ] {
            description.validate().unwrap();
            let encoded = serde_json::to_vec(description).unwrap();
            let decoded: TacticAssetDescription = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(&decoded, description);
        }
        assert_eq!(game_description.kind, TacticAssetKind::GameTactic);
        assert_eq!(path_description.option.option_type, OptionType::Bezier);
        assert_eq!(native_description.option.option_type, OptionType::Move);
        assert_eq!(
            controller_description.option.option_type,
            OptionType::Custom("reactive_controller".into())
        );
        assert!(game.static_frames().unwrap().is_some());
        let game_exact = game.exact_static_realization("interact").unwrap().unwrap();
        game_exact.validate_against(&game_description).unwrap();
        assert_eq!(
            path.static_frames().unwrap().unwrap(),
            path.realize(None).unwrap().frames
        );
        let path_exact = path.exact_static_realization("curve").unwrap().unwrap();
        path_exact.validate_against(&path_description).unwrap();
        assert!(native.static_frames().unwrap().is_none());
        assert!(native.exact_static_realization("seek").unwrap().is_none());
        assert!(controller.static_frames().unwrap().is_some());
        let controller_exact = controller
            .exact_static_realization("controller")
            .unwrap()
            .unwrap();
        controller_exact
            .validate_against(&controller_description)
            .unwrap();
        assert_eq!(
            controller_exact.execution.emitted_raw_actions,
            controller.static_frames().unwrap().unwrap()
        );
        assert_eq!(game_description.executor, TacticExecutor::StaticPlan);
        assert_eq!(
            native_description.executor,
            TacticExecutor::NativeGenericObservationLoop
        );
        assert_eq!(
            native_description.stopping.termination,
            native.termination_condition().unwrap()
        );
        assert_eq!(
            [
                game_description.content_sha256,
                path_description.content_sha256,
                native_description.content_sha256,
                controller_description.content_sha256,
            ]
            .into_iter()
            .collect::<BTreeSet<_>>()
            .len(),
            4
        );
    }

    #[test]
    fn reactive_controller_and_native_tactic_declare_exact_observation_families() {
        let controller = ControllerProgram::parse(
            "duskcontrol 1\nframes 3\nseek coordinate replace from 0 for 3 frame world target 1 2 3 offset 0 0 0 magnitude 100 stop 1\n",
        )
        .unwrap();
        let controller_description = controller.describe("seek-world").unwrap();
        assert!(!controller_description.statically_realizable);
        assert_eq!(
            controller_description.executor,
            TacticExecutor::ReactiveControllerProgram
        );
        assert_eq!(
            controller_description.required_observations,
            [
                TacticObservationRequirement::PlayerPosition,
                TacticObservationRequirement::CameraYaw,
            ]
            .into_iter()
            .collect()
        );
        assert!(controller.static_frames().unwrap().is_none());
        assert!(
            controller
                .exact_static_realization("seek-world")
                .unwrap()
                .is_none()
        );

        let native = NativeGenericTacticPlan::new(
            GenericTactic::SynchronizeButtonEdge {
                button_mask: 0x0100,
                procedure_id: 7,
                animation_resource_id: 12,
                phase_f32_bits: 4.0_f32.to_bits(),
                movement_heading_radians_f32_bits: None,
                movement_magnitude: 0,
            },
            10,
        );
        assert_eq!(
            native.describe("sync").unwrap().required_observations,
            [
                TacticObservationRequirement::PlayerProcedure,
                TacticObservationRequirement::PlayerActionLane,
            ]
            .into_iter()
            .collect()
        );
    }

    #[test]
    fn adapter_rejects_nonportable_option_ids() {
        let tactic = GameTacticPlan::new(GameTactic::Shield { frames: 2 });
        assert_eq!(
            tactic.describe("contains spaces").unwrap_err(),
            TacticAssetError::InvalidOptionId
        );
    }

    #[test]
    fn exact_realization_rejects_descriptor_or_stopping_drift() {
        let tactic = GameTacticPlan::new(GameTactic::Shield { frames: 2 });
        let realization = tactic.exact_static_realization("shield").unwrap().unwrap();
        let mut description = tactic.describe("shield").unwrap();
        description
            .option
            .parameters
            .insert("frames".into(), OptionParameter::Unsigned(3));
        assert!(realization.validate_against(&description).is_err());

        let mut description = tactic.describe("shield").unwrap();
        description.stopping.termination = OptionCondition::TargetReached {
            target: "unrelated".into(),
        };
        assert!(realization.validate_against(&description).is_err());
    }
}
