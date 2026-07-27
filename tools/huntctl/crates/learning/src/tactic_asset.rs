//! One runtime adapter contract over Dusklight's existing tactic representations.
//!
//! This module deliberately does not introduce another serialized action language.
//! Each adapter validates, identifies, describes, and—when possible—realizes the
//! existing source plan using its current canonical encoding and evaluator.

use crate::artifact::Digest;
use crate::native_generic_tactic::{
    GenericTactic, NATIVE_GENERIC_TACTIC_SCHEMA_V1, NativeGenericTacticCandidate,
    NativeGenericTacticPlan,
};
use crate::option_policy::TacticOptionCandidate;
use crate::option_values::OptionActionDescriptor;
use dusklight_control::controller_compilation::{
    ControllerObservationField, ControllerObservationProvenance, StaticControllerError,
    compile_static_controller,
};
use dusklight_control::controller_program::{
    ActorSelector, ControllerProgram, Operation, VERSION_MAJOR, VERSION_MINOR,
};
use dusklight_control::game_tactic::{GAME_TACTIC_SCHEMA_V1, GameTacticPlan};
use dusklight_control::motion_path::{MOTION_PATH_SCHEMA_V1, MotionPathPlan};
use dusklight_control::option_execution::{
    MAX_OPTION_CONDITIONS, MAX_OPTION_TICKS, OptionCondition, OptionEndReason, OptionExecution,
    OptionParameter, OptionType, TapeRange, validate_condition,
};
use dusklight_control::roll_option::{ROLL_OPTION_SCHEMA_V1, RollOptionPlan};
use dusklight_control::tape::{InputFrame, InputTape};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const TACTIC_ASSET_ADAPTER_SCHEMA_V1: &str = "dusklight-tactic-asset-adapter/v1";
pub const MAX_TACTIC_CATALOG_ENTRIES: usize = 512;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TacticAssetKind {
    GameTactic,
    NativeGenericTactic,
    MotionPath,
    Roll,
    ReactiveController,
    RecordedTape,
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

/// In-memory sum of the existing plan types. It is intentionally not
/// serializable: each variant keeps using its source plan's current encoding.
#[derive(Clone, Debug, PartialEq)]
pub enum TacticAssetSource {
    GameTactic(GameTacticPlan),
    NativeGenericTactic(NativeGenericTacticPlan),
    MotionPath(MotionPathPlan),
    Roll(RollOptionPlan),
    ReactiveController(ControllerProgram),
    RecordedTape(InputTape),
}

impl TacticAssetAdapter for TacticAssetSource {
    fn describe(&self, option_id: &str) -> Result<TacticAssetDescription, TacticAssetError> {
        match self {
            Self::GameTactic(plan) => plan.describe(option_id),
            Self::NativeGenericTactic(plan) => plan.describe(option_id),
            Self::MotionPath(plan) => plan.describe(option_id),
            Self::Roll(plan) => plan.describe(option_id),
            Self::ReactiveController(plan) => plan.describe(option_id),
            Self::RecordedTape(tape) => tape.describe(option_id),
        }
    }

    fn canonical_bytes(&self) -> Result<Vec<u8>, TacticAssetError> {
        match self {
            Self::GameTactic(plan) => TacticAssetAdapter::canonical_bytes(plan),
            Self::NativeGenericTactic(plan) => TacticAssetAdapter::canonical_bytes(plan),
            Self::MotionPath(plan) => TacticAssetAdapter::canonical_bytes(plan),
            Self::Roll(plan) => TacticAssetAdapter::canonical_bytes(plan),
            Self::ReactiveController(plan) => TacticAssetAdapter::canonical_bytes(plan),
            Self::RecordedTape(tape) => TacticAssetAdapter::canonical_bytes(tape),
        }
    }

    fn static_frames(&self) -> Result<Option<Vec<InputFrame>>, TacticAssetError> {
        match self {
            Self::GameTactic(plan) => plan.static_frames(),
            Self::NativeGenericTactic(plan) => plan.static_frames(),
            Self::MotionPath(plan) => plan.static_frames(),
            Self::Roll(plan) => plan.static_frames(),
            Self::ReactiveController(plan) => plan.static_frames(),
            Self::RecordedTape(tape) => tape.static_frames(),
        }
    }

    fn exact_static_realization(
        &self,
        option_id: &str,
    ) -> Result<Option<ExactTacticRealization>, TacticAssetError> {
        match self {
            Self::GameTactic(plan) => plan.exact_static_realization(option_id),
            Self::NativeGenericTactic(plan) => plan.exact_static_realization(option_id),
            Self::MotionPath(plan) => plan.exact_static_realization(option_id),
            Self::Roll(plan) => plan.exact_static_realization(option_id),
            Self::ReactiveController(plan) => plan.exact_static_realization(option_id),
            Self::RecordedTape(tape) => tape.exact_static_realization(option_id),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TacticCatalogEntry {
    option_id: String,
    source: TacticAssetSource,
    description: TacticAssetDescription,
}

impl TacticCatalogEntry {
    pub fn new(
        option_id: impl Into<String>,
        source: TacticAssetSource,
    ) -> Result<Self, TacticAssetError> {
        let option_id = option_id.into();
        let description = source.describe(&option_id)?;
        let entry = Self {
            option_id,
            source,
            description,
        };
        entry.validate()?;
        Ok(entry)
    }

    pub fn option_id(&self) -> &str {
        &self.option_id
    }

    pub fn source(&self) -> &TacticAssetSource {
        &self.source
    }

    pub fn description(&self) -> &TacticAssetDescription {
        &self.description
    }

    pub fn exact_static_realization(
        &self,
    ) -> Result<Option<ExactTacticRealization>, TacticAssetError> {
        let realization = self.source.exact_static_realization(&self.option_id)?;
        if let Some(realization) = &realization {
            realization.validate_against(&self.description)?;
        } else if self.description.statically_realizable {
            return Err(invalid(
                "catalog entry promised a static realization but produced none",
            ));
        }
        Ok(realization)
    }

    fn validate(&self) -> Result<(), TacticAssetError> {
        validate_option_id(&self.option_id)?;
        self.description.validate()?;
        if self.description.option.option_id != self.option_id
            || self.source.describe(&self.option_id)? != self.description
        {
            return Err(invalid(
                "catalog entry description differs from its source plan",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TacticAssetCatalog {
    entries: Vec<TacticCatalogEntry>,
    action_schema_sha256: Digest,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PreparedTacticExecution<'a> {
    Static(ExactTacticRealization),
    NativeGeneric(NativeGenericTacticCandidate),
    ReactiveController(&'a ControllerProgram),
}

impl TacticAssetCatalog {
    pub fn new(mut entries: Vec<TacticCatalogEntry>) -> Result<Self, TacticAssetError> {
        if entries.is_empty() {
            return Err(TacticAssetError::EmptyCatalog);
        }
        if entries.len() > MAX_TACTIC_CATALOG_ENTRIES {
            return Err(TacticAssetError::CatalogTooLarge);
        }
        entries.sort_by(|left, right| left.option_id.cmp(&right.option_id));
        if entries
            .windows(2)
            .any(|pair| pair[0].option_id == pair[1].option_id)
        {
            return Err(TacticAssetError::DuplicateOptionId);
        }
        for entry in &entries {
            entry.validate()?;
        }
        let action_schema_sha256 = digest(
            &serde_json::to_vec(
                &entries
                    .iter()
                    .map(|entry| &entry.description.option)
                    .collect::<Vec<_>>(),
            )
            .map_err(serialization)?,
        );
        Ok(Self {
            entries,
            action_schema_sha256,
        })
    }

    pub fn entries(&self) -> &[TacticCatalogEntry] {
        &self.entries
    }

    pub fn entry(&self, option_id: &str) -> Option<&TacticCatalogEntry> {
        self.entries
            .binary_search_by_key(&option_id, |entry| entry.option_id.as_str())
            .ok()
            .map(|index| &self.entries[index])
    }

    pub fn descriptions(&self) -> impl ExactSizeIterator<Item = &TacticAssetDescription> {
        self.entries.iter().map(|entry| &entry.description)
    }

    pub fn option_descriptors(&self) -> impl ExactSizeIterator<Item = &OptionActionDescriptor> {
        self.entries.iter().map(|entry| &entry.description.option)
    }

    pub fn action_schema_sha256(&self) -> Digest {
        self.action_schema_sha256
    }

    pub fn prepare_execution(
        &self,
        option_id: &str,
    ) -> Result<PreparedTacticExecution<'_>, TacticAssetError> {
        let entry = self
            .entry(option_id)
            .ok_or_else(|| TacticAssetError::UnknownOptionId(option_id.into()))?;
        match entry.description.executor {
            TacticExecutor::StaticPlan => entry
                .exact_static_realization()?
                .map(PreparedTacticExecution::Static)
                .ok_or_else(|| {
                    invalid("static tactic catalog entry produced no exact realization")
                }),
            TacticExecutor::NativeGenericObservationLoop => match &entry.source {
                TacticAssetSource::NativeGenericTactic(plan) => {
                    NativeGenericTacticCandidate::new(entry.option_id.clone(), plan.clone())
                        .map(PreparedTacticExecution::NativeGeneric)
                        .map_err(|error| invalid(error.to_string()))
                }
                _ => Err(invalid(
                    "native generic executor does not own a native generic tactic plan",
                )),
            },
            TacticExecutor::ReactiveControllerProgram => match &entry.source {
                TacticAssetSource::ReactiveController(program) => {
                    Ok(PreparedTacticExecution::ReactiveController(program))
                }
                _ => Err(invalid(
                    "reactive controller executor does not own a controller program",
                )),
            },
        }
    }
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

impl TacticAssetAdapter for InputTape {
    fn describe(&self, option_id: &str) -> Result<TacticAssetDescription, TacticAssetError> {
        validate_option_id(option_id)?;
        let exact = recorded_tape_realization(self, option_id)?;
        let canonical = self.canonical_bytes()?;
        checked(TacticAssetDescription {
            schema: TACTIC_ASSET_ADAPTER_SCHEMA_V1.into(),
            kind: TacticAssetKind::RecordedTape,
            source_schema: "dusklight-input-tape/binary".into(),
            content_sha256: digest(&canonical),
            option: descriptor(&exact.execution),
            duration: TacticDurationBounds {
                minimum_ticks: exact.execution.duration.minimum_ticks,
                maximum_ticks: exact.execution.duration.maximum_ticks,
            },
            applicability: TacticApplicability::InputOnly,
            required_observations: BTreeSet::new(),
            executor: TacticExecutor::StaticPlan,
            stopping: TacticStoppingContract {
                termination: exact.execution.termination_condition.clone(),
                cancellation: exact.execution.cancellation_conditions.clone(),
            },
            statically_realizable: true,
        })
    }

    fn canonical_bytes(&self) -> Result<Vec<u8>, TacticAssetError> {
        self.validate()
            .map_err(|error| invalid(error.to_string()))?;
        if self.frames.is_empty() || self.frames.len() > MAX_OPTION_TICKS as usize {
            return Err(invalid("recorded tactic tape has an invalid duration"));
        }
        self.encode().map_err(|error| invalid(error.to_string()))
    }

    fn static_frames(&self) -> Result<Option<Vec<InputFrame>>, TacticAssetError> {
        self.canonical_bytes()?;
        Ok(Some(self.frames.clone()))
    }

    fn exact_static_realization(
        &self,
        option_id: &str,
    ) -> Result<Option<ExactTacticRealization>, TacticAssetError> {
        let description = self.describe(option_id)?;
        let exact = recorded_tape_realization(self, option_id)?;
        exact.validate_against(&description)?;
        Ok(Some(exact))
    }
}

fn recorded_tape_realization(
    tape: &InputTape,
    option_id: &str,
) -> Result<ExactTacticRealization, TacticAssetError> {
    tape.validate()
        .map_err(|error| invalid(error.to_string()))?;
    let duration = u32::try_from(tape.frames.len())
        .ok()
        .filter(|duration| *duration > 0 && *duration <= MAX_OPTION_TICKS)
        .ok_or_else(|| invalid("recorded tactic tape has an invalid duration"))?;
    let canonical = tape.encode().map_err(|error| invalid(error.to_string()))?;
    let mut parameters = BTreeMap::new();
    parameters.insert(
        "input_tape_sha256".into(),
        OptionParameter::Digest(digest(&canonical)),
    );
    parameters.insert(
        "duration_ticks".into(),
        OptionParameter::Unsigned(u64::from(duration)),
    );
    let execution = OptionExecution::capture(
        option_id.into(),
        OptionType::Custom("recorded_tape".into()),
        parameters,
        duration,
        duration,
        OptionCondition::DurationElapsed,
        Vec::new(),
        OptionEndReason::Completed,
        tape,
        TapeRange {
            start_frame: 0,
            end_frame_exclusive: u64::from(duration),
        },
    )
    .map_err(|error| invalid(error.to_string()))?;
    Ok(ExactTacticRealization {
        tape: tape.clone(),
        execution,
    })
}

impl TacticAssetAdapter for RollOptionPlan {
    fn describe(&self, option_id: &str) -> Result<TacticAssetDescription, TacticAssetError> {
        validate_option_id(option_id)?;
        self.validate()
            .map_err(|error| invalid(error.to_string()))?;
        let exact = exact_roll_realization(self, option_id)?;
        let canonical = self.canonical_bytes()?;
        checked(TacticAssetDescription {
            schema: TACTIC_ASSET_ADAPTER_SCHEMA_V1.into(),
            kind: TacticAssetKind::Roll,
            source_schema: ROLL_OPTION_SCHEMA_V1.into(),
            content_sha256: digest(&canonical),
            option: descriptor(&exact.execution),
            duration: TacticDurationBounds {
                minimum_ticks: 1,
                maximum_ticks: self
                    .planned_ticks()
                    .map_err(|error| invalid(error.to_string()))?,
            },
            applicability: TacticApplicability::GameContextRequired,
            required_observations: BTreeSet::new(),
            executor: TacticExecutor::StaticPlan,
            stopping: TacticStoppingContract {
                termination: exact.execution.termination_condition.clone(),
                cancellation: exact.execution.cancellation_conditions.clone(),
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
        let start = compatible_roll_start(self);
        Ok(Some(
            self.realize(start, None)
                .map_err(|error| invalid(error.to_string()))?
                .frames,
        ))
    }

    fn exact_static_realization(
        &self,
        option_id: &str,
    ) -> Result<Option<ExactTacticRealization>, TacticAssetError> {
        let description = self.describe(option_id)?;
        let exact = exact_roll_realization(self, option_id)?;
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
        insert_controller_command_summary(&mut parameters, self);
        if let Some((mask, period_ticks, phase_tick)) = periodic_button_overlay(self) {
            let mut base_program = self.clone();
            base_program
                .layers
                .retain(|layer| !matches!(layer.operation, Operation::Buttons { .. }));
            let base_program_bytes = base_program
                .encode()
                .map_err(|error| invalid(error.to_string()))?;
            parameters.insert(
                "controller_base_sha256".into(),
                OptionParameter::Digest(digest(&base_program_bytes)),
            );
            parameters.insert(
                "button_pulse_mask".into(),
                OptionParameter::Unsigned(u64::from(mask)),
            );
            parameters.insert(
                "button_pulse_period_ticks".into(),
                OptionParameter::Unsigned(u64::from(period_ticks)),
            );
            parameters.insert(
                "button_pulse_phase_tick".into(),
                OptionParameter::Unsigned(u64::from(phase_tick)),
            );
            if let Some(waypoint_switch_radius) =
                uniform_sequence_waypoint_switch_radius(&base_program)
            {
                let mut structural_program = base_program.clone();
                for layer in &mut structural_program.layers {
                    if let Operation::SeekCoordinateSequence {
                        intermediate_stop_radius,
                        ..
                    } = &mut layer.operation
                    {
                        *intermediate_stop_radius = 0.0;
                    }
                }
                let structural_bytes = structural_program
                    .encode()
                    .map_err(|error| invalid(error.to_string()))?;
                parameters.insert(
                    "controller_structure_sha256".into(),
                    OptionParameter::Digest(digest(&structural_bytes)),
                );
                parameters.insert(
                    "waypoint_switch_radius".into(),
                    OptionParameter::F32Bits(waypoint_switch_radius.to_bits()),
                );
            }
        }
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
                cancellation: self
                    .layers
                    .iter()
                    .any(|layer| {
                        matches!(
                            &layer.operation,
                            Operation::SeekActor {
                                selector: ActorSelector::Process { .. }
                                    | ActorSelector::Placed { .. },
                                ..
                            }
                        )
                    })
                    .then(|| {
                        vec![OptionCondition::TargetLost {
                            target: "controller_exact_actor".into(),
                        }]
                    })
                    .unwrap_or_default(),
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

/// Expose generic executable-command factors for cross-action learning.
///
/// Opaque program digests remain evidence identities. These typed summaries
/// are deliberately independent of option IDs and route names so a learner can
/// share observations about direction changes, magnitude, targeting, and
/// button use with controller instances it has never executed.
fn insert_controller_command_summary(
    parameters: &mut BTreeMap<String, OptionParameter>,
    program: &ControllerProgram,
) {
    let button_layers = program
        .layers
        .iter()
        .filter_map(|layer| match layer.operation {
            Operation::Buttons { mask } => Some((layer.start_frame, layer.duration_frames, mask)),
            _ => None,
        })
        .collect::<Vec<_>>();
    if !button_layers.is_empty() {
        let mask = button_layers
            .iter()
            .fold(0_u16, |combined, (_, _, mask)| combined | *mask);
        let active_ticks = button_layers.iter().fold(0_u64, |total, (_, duration, _)| {
            total.saturating_add(u64::from(*duration))
        });
        parameters.insert(
            "command_button_mask".into(),
            OptionParameter::Unsigned(u64::from(mask)),
        );
        parameters.insert(
            "command_button_pulse_count".into(),
            OptionParameter::Unsigned(button_layers.len() as u64),
        );
        parameters.insert(
            "command_button_active_fraction".into(),
            OptionParameter::F32Bits(
                (active_ticks as f32 / program.duration_frames as f32)
                    .clamp(0.0, 1.0)
                    .to_bits(),
            ),
        );
        if button_layers.len() >= 2 {
            let interval_total = button_layers.windows(2).fold(0_u64, |total, pair| {
                total.saturating_add(u64::from(pair[1].0.saturating_sub(pair[0].0)))
            });
            parameters.insert(
                "command_button_mean_interval_ticks".into(),
                OptionParameter::F32Bits(
                    (interval_total as f32 / (button_layers.len() - 1) as f32).to_bits(),
                ),
            );
        }
    }

    let coordinate_plan = program
        .layers
        .iter()
        .filter_map(|layer| match &layer.operation {
            Operation::SeekCoordinateSequence {
                coordinates_xz,
                magnitude,
                ..
            } if !coordinates_xz.is_empty() => {
                Some((layer.duration_frames, coordinates_xz, *magnitude))
            }
            _ => None,
        })
        .max_by_key(|(duration, coordinates, _)| (*duration, coordinates.len()));
    let Some((_, coordinates, magnitude)) = coordinate_plan else {
        return;
    };
    let first = coordinates[0];
    let second = coordinates.get(1).copied();
    let last = coordinates[coordinates.len() - 1];
    let internal_path_length = coordinates
        .windows(2)
        .map(|pair| (pair[1][0] - pair[0][0]).hypot(pair[1][1] - pair[0][1]))
        .sum::<f32>();
    let internal_displacement = (last[0] - first[0]).hypot(last[1] - first[1]);
    let internal_turn_radians = coordinates
        .windows(3)
        .map(|points| {
            let left = [points[1][0] - points[0][0], points[1][1] - points[0][1]];
            let right = [points[2][0] - points[1][0], points[2][1] - points[1][1]];
            let cross = left[0] * right[1] - left[1] * right[0];
            let dot = left[0] * right[0] + left[1] * right[1];
            cross.atan2(dot).abs()
        })
        .sum::<f32>();
    for (name, value) in [
        ("command_target_first_x", first[0]),
        ("command_target_first_z", first[1]),
        ("command_target_last_x", last[0]),
        ("command_target_last_z", last[1]),
        ("command_internal_path_length", internal_path_length),
        ("command_internal_displacement", internal_displacement),
        ("command_internal_turn_radians", internal_turn_radians),
    ] {
        parameters.insert(name.into(), OptionParameter::F32Bits(value.to_bits()));
    }
    if let Some(second) = second {
        parameters.insert(
            "command_target_second_x".into(),
            OptionParameter::F32Bits(second[0].to_bits()),
        );
        parameters.insert(
            "command_target_second_z".into(),
            OptionParameter::F32Bits(second[1].to_bits()),
        );
    }
    parameters.insert(
        "command_target_point_count".into(),
        OptionParameter::Unsigned(coordinates.len() as u64),
    );
    parameters.insert(
        "command_stick_magnitude".into(),
        OptionParameter::Unsigned(u64::from(magnitude)),
    );
}

/// Describe the common controller composition of a movement layer plus one
/// evenly spaced button pulse. Exposing these values as typed option
/// parameters lets acquisition compare nearby controller variants without
/// parsing option IDs or treating the opaque program digest as geometry.
fn periodic_button_overlay(program: &ControllerProgram) -> Option<(u16, u32, u32)> {
    let pulses = program
        .layers
        .iter()
        .filter_map(|layer| match layer.operation {
            Operation::Buttons { mask } if layer.duration_frames == 1 => {
                Some((layer.start_frame, mask))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if pulses.len() < 2
        || pulses.iter().any(|(_, mask)| *mask != pulses[0].1)
        || pulses
            .windows(2)
            .any(|pair| pair[1].0.saturating_sub(pair[0].0) != pulses[1].0 - pulses[0].0)
    {
        return None;
    }
    let period_ticks = pulses[1].0 - pulses[0].0;
    (period_ticks > 0).then_some((pulses[0].1, period_ticks, pulses[0].0))
}

fn uniform_sequence_waypoint_switch_radius(program: &ControllerProgram) -> Option<f32> {
    let mut radii = program.layers.iter().filter_map(|layer| {
        if let Operation::SeekCoordinateSequence {
            intermediate_stop_radius,
            ..
        } = layer.operation
        {
            Some(intermediate_stop_radius)
        } else {
            None
        }
    });
    let radius = radii.next()?;
    radii
        .all(|candidate| candidate.to_bits() == radius.to_bits())
        .then_some(radius)
}

fn checked(
    description: TacticAssetDescription,
) -> Result<TacticAssetDescription, TacticAssetError> {
    description.validate()?;
    Ok(description)
}

fn compatible_roll_start(plan: &RollOptionPlan) -> u64 {
    let period = u64::from(plan.spacing.period_ticks);
    let button_phase = u64::from(plan.button_frame) % period;
    (u64::from(plan.spacing.phase_tick) + period - button_phase) % period
}

fn exact_roll_realization(
    plan: &RollOptionPlan,
    option_id: &str,
) -> Result<ExactTacticRealization, TacticAssetError> {
    validate_option_id(option_id)?;
    let start_frame = compatible_roll_start(plan);
    let realization = plan
        .realize(start_frame, None)
        .map_err(|error| invalid(error.to_string()))?;
    let mut frames = vec![InputFrame::default(); start_frame as usize];
    frames.extend(realization.frames);
    let tape = tape(frames);
    let range = TapeRange {
        start_frame,
        end_frame_exclusive: tape.frames.len() as u64,
    };
    let execution = plan
        .capture_execution(option_id.into(), &tape, range, None)
        .map_err(|error| invalid(error.to_string()))?;
    Ok(ExactTacticRealization { tape, execution })
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
        GenericTactic::SeekCoordinate { .. } | GenericTactic::SeekCoordinateSequence { .. } => {
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
    EmptyCatalog,
    CatalogTooLarge,
    DuplicateOptionId,
    UnknownOptionId(String),
    InvalidAsset(String),
    Serialization(String),
}

impl fmt::Display for TacticAssetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOptionId => formatter.write_str("tactic option ID is invalid"),
            Self::EmptyCatalog => formatter.write_str("tactic catalog is empty"),
            Self::CatalogTooLarge => formatter.write_str("tactic catalog exceeds its finite bound"),
            Self::DuplicateOptionId => {
                formatter.write_str("tactic catalog option IDs are not unique")
            }
            Self::UnknownOptionId(option_id) => {
                write!(formatter, "tactic catalog has no option named {option_id}")
            }
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
    use dusklight_control::controller_program::{ControllerProgram, Layer, StickBlend};
    use dusklight_control::game_tactic::{GameTactic, GameTacticPlan};
    use dusklight_control::motion_path::{MotionPathPlan, SamplePhase, StickPath, StickPoint};
    use dusklight_control::roll_option::{RollOptionPlan, RollSpacing};

    fn parameter_f32(description: &TacticAssetDescription, name: &str) -> f32 {
        match description.option.parameters.get(name).unwrap() {
            OptionParameter::F32Bits(bits) => f32::from_bits(*bits),
            parameter => panic!("{name} was not an f32 parameter: {parameter:?}"),
        }
    }

    #[test]
    fn reactive_controller_describes_generic_motion_and_button_factors() {
        let controller = ControllerProgram {
            duration_frames: 12,
            layers: vec![
                Layer {
                    start_frame: 0,
                    duration_frames: 12,
                    operation: Operation::SeekCoordinateSequence {
                        blend: StickBlend::Replace,
                        coordinates_xz: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]],
                        intermediate_stop_radius: 1.0,
                        final_stop_radius: 0.0,
                        magnitude: 127,
                    },
                },
                Layer {
                    start_frame: 1,
                    duration_frames: 1,
                    operation: Operation::Buttons { mask: 0x0100 },
                },
                Layer {
                    start_frame: 5,
                    duration_frames: 1,
                    operation: Operation::Buttons { mask: 0x0100 },
                },
            ],
        };

        let description = controller.describe("opaque-controller-id").unwrap();

        assert_eq!(
            description.option.parameters.get("command_button_mask"),
            Some(&OptionParameter::Unsigned(0x0100))
        );
        assert_eq!(
            description
                .option
                .parameters
                .get("command_target_point_count"),
            Some(&OptionParameter::Unsigned(3))
        );
        assert_eq!(
            description.option.parameters.get("command_stick_magnitude"),
            Some(&OptionParameter::Unsigned(127))
        );
        assert_eq!(
            parameter_f32(&description, "command_internal_path_length"),
            20.0
        );
        assert!(
            (parameter_f32(&description, "command_internal_displacement") - 10.0_f32.hypot(10.0))
                .abs()
                < 1.0e-5
        );
        assert!(
            (parameter_f32(&description, "command_internal_turn_radians")
                - std::f32::consts::FRAC_PI_2)
                .abs()
                < 1.0e-5
        );
        assert_eq!(
            parameter_f32(&description, "command_button_mean_interval_ticks"),
            4.0
        );
    }

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
        let roll = RollOptionPlan::new(0, 100, 2);
        let controller = ControllerProgram::parse(
            "duskcontrol 1\nframes 4\nbezier replace from 0 for 4 p0 0 127 p1 20 100 p2 40 80 p3 60 60\n",
        )
        .unwrap();

        let game_description = game.describe("interact").unwrap();
        let path_description = path.describe("curve").unwrap();
        let native_description = native.describe("seek").unwrap();
        let roll_description = roll.describe("roll").unwrap();
        let controller_description = controller.describe("controller").unwrap();

        for description in [
            &game_description,
            &path_description,
            &native_description,
            &roll_description,
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
        assert_eq!(roll_description.option.option_type, OptionType::Roll);
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
        let roll_exact = roll.exact_static_realization("roll").unwrap().unwrap();
        roll_exact.validate_against(&roll_description).unwrap();
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
                roll_description.content_sha256,
                controller_description.content_sha256,
            ]
            .into_iter()
            .collect::<BTreeSet<_>>()
            .len(),
            5
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

        let exact_actor = ControllerProgram::parse(
            "duskcontrol 1\nframes 3\nseek actor replace from 0 for 3 actor 42 set 7 room 1 stage F_SP103 offset 0 0 0 magnitude 100 stop 1\n",
        )
        .unwrap()
        .describe("seek-exact-actor")
        .unwrap();
        assert_eq!(
            exact_actor.stopping.cancellation,
            vec![OptionCondition::TargetLost {
                target: "controller_exact_actor".into(),
            }]
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

    #[test]
    fn one_finite_catalog_holds_all_existing_plan_families() {
        let game = TacticCatalogEntry::new(
            "game.interact",
            TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Interact {
                press_frames: 1,
                recovery_frames: 1,
            })),
        )
        .unwrap();
        let path = TacticCatalogEntry::new(
            "path.waypoint",
            TacticAssetSource::MotionPath(MotionPathPlan::new(
                StickPath::Waypoint {
                    points: vec![StickPoint { x: 0, y: 100 }],
                },
                2,
            )),
        )
        .unwrap();
        let native = TacticCatalogEntry::new(
            "native.heading",
            TacticAssetSource::NativeGenericTactic(NativeGenericTacticPlan::new(
                GenericTactic::MaintainRelativeHeading {
                    heading_radians_f32_bits: 0.0_f32.to_bits(),
                    magnitude: 90,
                },
                3,
            )),
        )
        .unwrap();
        let mut roll_plan = RollOptionPlan::new(0, 100, 1);
        roll_plan.button_frame = 1;
        roll_plan.spacing = RollSpacing {
            period_ticks: 4,
            phase_tick: 3,
        };
        let roll =
            TacticCatalogEntry::new("roll.forward", TacticAssetSource::Roll(roll_plan)).unwrap();
        let controller = TacticCatalogEntry::new(
            "controller.buttons",
            TacticAssetSource::ReactiveController(
                ControllerProgram::parse("duskcontrol 1\nframes 2\nbuttons from 0 for 2 B\n")
                    .unwrap(),
            ),
        )
        .unwrap();

        let catalog = TacticAssetCatalog::new(vec![path, native, controller, game, roll]).unwrap();
        assert_eq!(
            catalog
                .entries()
                .iter()
                .map(TacticCatalogEntry::option_id)
                .collect::<Vec<_>>(),
            vec![
                "controller.buttons",
                "game.interact",
                "native.heading",
                "path.waypoint",
                "roll.forward",
            ]
        );
        assert_eq!(catalog.descriptions().len(), 5);
        for option_id in [
            "controller.buttons",
            "game.interact",
            "path.waypoint",
            "roll.forward",
        ] {
            let PreparedTacticExecution::Static(realization) =
                catalog.prepare_execution(option_id).unwrap()
            else {
                panic!("expected exact static execution");
            };
            realization
                .validate_against(catalog.entry(option_id).unwrap().description())
                .unwrap();
        }
        let PreparedTacticExecution::NativeGeneric(native) =
            catalog.prepare_execution("native.heading").unwrap()
        else {
            panic!("expected native generic executor input");
        };
        assert_eq!(native.descriptor().option_id, "native.heading");
        assert_eq!(
            catalog.option_descriptors().count(),
            catalog.entries().len()
        );
        assert_ne!(catalog.action_schema_sha256(), Digest::ZERO);
        assert_eq!(
            catalog.prepare_execution("missing").unwrap_err(),
            TacticAssetError::UnknownOptionId("missing".into())
        );
    }

    #[test]
    fn catalog_dispatches_observation_driven_controller_to_existing_program() {
        let catalog = TacticAssetCatalog::new(vec![
            TacticCatalogEntry::new(
                "controller.seek",
                TacticAssetSource::ReactiveController(
                    ControllerProgram::parse(
                        "duskcontrol 1\nframes 3\nseek coordinate replace from 0 for 3 frame world target 1 2 3 offset 0 0 0 magnitude 100 stop 1\n",
                    )
                    .unwrap(),
                ),
            )
            .unwrap(),
        ])
        .unwrap();

        let PreparedTacticExecution::ReactiveController(program) =
            catalog.prepare_execution("controller.seek").unwrap()
        else {
            panic!("expected reactive controller executor input");
        };
        assert_eq!(program.duration_frames, 3);
    }

    #[test]
    fn recorded_binary_tape_is_an_exact_static_tactic() {
        let mut frame = InputFrame::default();
        frame.owned_ports = 1;
        frame.pads[0].stick_x = 73;
        frame.pads[0].buttons = 0x0100;
        let tape = InputTape {
            frames: vec![frame.clone(), frame, InputFrame::default()],
            ..InputTape::default()
        };
        let entry = TacticCatalogEntry::new(
            "promoted/example",
            TacticAssetSource::RecordedTape(tape.clone()),
        )
        .unwrap();
        assert_eq!(entry.description().kind, TacticAssetKind::RecordedTape);
        assert_eq!(entry.description().duration.maximum_ticks, 3);
        assert_eq!(
            entry
                .description()
                .option
                .parameters
                .get("input_tape_sha256"),
            Some(&OptionParameter::Digest(digest(&tape.encode().unwrap())))
        );
        let catalog = TacticAssetCatalog::new(vec![entry]).unwrap();
        let PreparedTacticExecution::Static(realized) =
            catalog.prepare_execution("promoted/example").unwrap()
        else {
            panic!("recorded tape must use the exact static executor");
        };
        assert_eq!(realized.tape, tape);
        assert_eq!(realized.execution.emitted_raw_actions, tape.frames);
        realized
            .validate_against(catalog.entry("promoted/example").unwrap().description())
            .unwrap();
    }

    #[test]
    fn catalog_rejects_duplicate_concrete_option_identity() {
        let entry = || {
            TacticCatalogEntry::new(
                "duplicate",
                TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield {
                    frames: 1,
                })),
            )
            .unwrap()
        };
        assert_eq!(
            TacticAssetCatalog::new(vec![entry(), entry()]).unwrap_err(),
            TacticAssetError::DuplicateOptionId
        );
    }
}
