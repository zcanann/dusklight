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
mod error;

pub use error::TacticAssetError;
use error::{checked, invalid, serialization, validate_option_id};

pub const TACTIC_ASSET_ADAPTER_SCHEMA_V1: &str = "dusklight-tactic-asset-adapter/v1";
pub const ENCODED_TACTIC_ASSET_SOURCE_SCHEMA_V1: &str = "dusklight-encoded-tactic-asset-source/v1";
pub const MAX_TACTIC_CATALOG_ENTRIES: usize = 512;
pub const MAX_ENCODED_TACTIC_ASSET_SOURCE_BYTES: usize = 4 * 1024 * 1024;

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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EncodedTacticAssetSource {
    pub schema: String,
    pub kind: TacticAssetKind,
    pub canonical_bytes: Vec<u8>,
}

impl EncodedTacticAssetSource {
    pub fn capture(source: &TacticAssetSource) -> Result<Self, TacticAssetError> {
        let encoded = Self {
            schema: ENCODED_TACTIC_ASSET_SOURCE_SCHEMA_V1.into(),
            kind: source.kind(),
            canonical_bytes: source.canonical_bytes()?,
        };
        encoded.decode()?;
        Ok(encoded)
    }

    pub fn decode(&self) -> Result<TacticAssetSource, TacticAssetError> {
        if self.schema != ENCODED_TACTIC_ASSET_SOURCE_SCHEMA_V1
            || self.canonical_bytes.is_empty()
            || self.canonical_bytes.len() > MAX_ENCODED_TACTIC_ASSET_SOURCE_BYTES
        {
            return Err(invalid("encoded tactic source is invalid or oversized"));
        }
        let source = match self.kind {
            TacticAssetKind::GameTactic => TacticAssetSource::GameTactic(
                serde_json::from_slice(&self.canonical_bytes).map_err(serialization)?,
            ),
            TacticAssetKind::NativeGenericTactic => TacticAssetSource::NativeGenericTactic(
                serde_json::from_slice(&self.canonical_bytes).map_err(serialization)?,
            ),
            TacticAssetKind::MotionPath => TacticAssetSource::MotionPath(
                serde_json::from_slice(&self.canonical_bytes).map_err(serialization)?,
            ),
            TacticAssetKind::Roll => TacticAssetSource::Roll(
                serde_json::from_slice(&self.canonical_bytes).map_err(serialization)?,
            ),
            TacticAssetKind::ReactiveController => TacticAssetSource::ReactiveController(
                ControllerProgram::decode(&self.canonical_bytes)
                    .map_err(|error| invalid(error.to_string()))?,
            ),
            TacticAssetKind::RecordedTape => TacticAssetSource::RecordedTape(
                InputTape::decode(&self.canonical_bytes)
                    .map_err(|error| invalid(error.to_string()))?
                    .tape,
            ),
        };
        if source.kind() != self.kind || source.canonical_bytes()? != self.canonical_bytes {
            return Err(invalid(
                "encoded tactic source differs from its canonical content",
            ));
        }
        Ok(source)
    }

    pub fn content_sha256(&self) -> Result<Digest, TacticAssetError> {
        let source = self.decode()?;
        let mut hasher = Sha256::new();
        hasher.update(ENCODED_TACTIC_ASSET_SOURCE_SCHEMA_V1.as_bytes());
        hasher.update([source.kind().identity_tag()]);
        hasher.update((self.canonical_bytes.len() as u64).to_le_bytes());
        hasher.update(&self.canonical_bytes);
        Ok(Digest(hasher.finalize().into()))
    }
}

impl TacticAssetKind {
    fn identity_tag(self) -> u8 {
        match self {
            Self::GameTactic => 0,
            Self::NativeGenericTactic => 1,
            Self::MotionPath => 2,
            Self::Roll => 3,
            Self::ReactiveController => 4,
            Self::RecordedTape => 5,
        }
    }
}

impl TacticAssetSource {
    pub fn kind(&self) -> TacticAssetKind {
        match self {
            Self::GameTactic(_) => TacticAssetKind::GameTactic,
            Self::NativeGenericTactic(_) => TacticAssetKind::NativeGenericTactic,
            Self::MotionPath(_) => TacticAssetKind::MotionPath,
            Self::Roll(_) => TacticAssetKind::Roll,
            Self::ReactiveController(_) => TacticAssetKind::ReactiveController,
            Self::RecordedTape(_) => TacticAssetKind::RecordedTape,
        }
    }
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
    insert_recorded_controller_summary(&mut parameters, tape);
    let execution = OptionExecution::capture(
        option_id.into(),
        recorded_tape_option_type(tape),
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

fn recorded_tape_option_type(tape: &InputTape) -> OptionType {
    let movement = tape
        .frames
        .iter()
        .any(|frame| frame.pads[0].stick_x != 0 || frame.pads[0].stick_y != 0);
    let button_mask = tape
        .frames
        .iter()
        .fold(0_u16, |mask, frame| mask | frame.pads[0].buttons);
    if button_mask & 0x0100 != 0 {
        OptionType::Roll
    } else if movement {
        OptionType::Move
    } else if button_mask == 0 {
        OptionType::Neutral
    } else {
        OptionType::Custom("recorded_tape".into())
    }
}

/// Describe raw controller behavior without exposing the recording identity to
/// the learner. Recorded chunks are optional experience, not policy actions;
/// these factors let their authenticated returns train the same action
/// dimensions used by generated movement and prompted-button tactics.
fn insert_recorded_controller_summary(
    parameters: &mut BTreeMap<String, OptionParameter>,
    tape: &InputTape,
) {
    let duration = tape.frames.len().max(1);
    let pads = tape.frames.iter().map(|frame| frame.pads[0]);

    let mut magnitude_total = 0.0_f32;
    let mut heading_vector = [0.0_f32; 2];
    let mut previous_heading = None::<f32>;
    let mut initial_heading = None::<f32>;
    let mut internal_turn_radians = 0.0_f32;
    let mut button_mask = 0_u16;
    let mut button_active_ticks = 0_usize;
    let mut previous_buttons = 0_u16;
    let mut pulse_ticks = Vec::new();

    for (tick, pad) in pads.enumerate() {
        let x = f32::from(pad.stick_x);
        let y = f32::from(pad.stick_y);
        let magnitude = x.hypot(y).min(127.0);
        magnitude_total += magnitude;
        if magnitude > f32::EPSILON {
            // Native camera-relative movement uses x=-sin(heading),
            // y=cos(heading), matching GenericTactic::MaintainRelativeHeading.
            let heading = (-x).atan2(y);
            initial_heading.get_or_insert(heading);
            heading_vector[0] += heading.sin() * magnitude;
            heading_vector[1] += heading.cos() * magnitude;
            if let Some(previous) = previous_heading {
                internal_turn_radians += ((heading - previous + std::f32::consts::PI)
                    .rem_euclid(std::f32::consts::TAU)
                    - std::f32::consts::PI)
                    .abs();
            }
            previous_heading = Some(heading);
        }

        button_mask |= pad.buttons;
        button_active_ticks += usize::from(pad.buttons != 0);
        if pad.buttons & !previous_buttons != 0 {
            pulse_ticks.push(tick);
        }
        previous_buttons = pad.buttons;
    }

    let mean_magnitude = (magnitude_total / duration as f32)
        .round()
        .clamp(0.0, 127.0) as u64;
    parameters.insert(
        "command_stick_magnitude".into(),
        OptionParameter::Unsigned(mean_magnitude),
    );
    parameters.insert(
        "command_has_movement".into(),
        OptionParameter::Bool(previous_heading.is_some()),
    );
    parameters.insert(
        "command_internal_turn_radians".into(),
        OptionParameter::F32Bits(internal_turn_radians.to_bits()),
    );
    if heading_vector != [0.0, 0.0] {
        parameters.insert(
            "movement_heading".into(),
            OptionParameter::F32Bits(heading_vector[0].atan2(heading_vector[1]).to_bits()),
        );
    }
    if let Some(initial_heading) = initial_heading {
        parameters.insert(
            "command_initial_heading".into(),
            OptionParameter::F32Bits(initial_heading.to_bits()),
        );
    }

    parameters.insert(
        "command_button_mask".into(),
        OptionParameter::Unsigned(u64::from(button_mask)),
    );
    parameters.insert(
        "command_button_active_fraction".into(),
        OptionParameter::F32Bits(
            (button_active_ticks as f32 / duration as f32)
                .clamp(0.0, 1.0)
                .to_bits(),
        ),
    );
    parameters.insert(
        "command_button_pulse_count".into(),
        OptionParameter::Unsigned(pulse_ticks.len() as u64),
    );
    if let Some(first_tick) = pulse_ticks.first() {
        parameters.insert(
            "button_pulse_phase_tick".into(),
            OptionParameter::Unsigned(*first_tick as u64),
        );
    }
    if pulse_ticks.len() >= 2 {
        let total_interval = pulse_ticks
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .sum::<usize>();
        parameters.insert(
            "command_button_mean_interval_ticks".into(),
            OptionParameter::F32Bits(
                (total_interval as f32 / (pulse_ticks.len() - 1) as f32).to_bits(),
            ),
        );
    }
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
        let option_type = if statically_realizable {
            static_prompted_controller_option_type(self)
                .unwrap_or_else(|| OptionType::Custom("reactive_controller".into()))
        } else {
            OptionType::Custom("reactive_controller".into())
        };
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
                option_type,
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

fn static_prompted_controller_option_type(program: &ControllerProgram) -> Option<OptionType> {
    let tape = compile_static_controller(program).ok()?;
    let button_mask = tape
        .frames
        .iter()
        .fold(0_u16, |mask, frame| mask | frame.pads[0].buttons);
    const BUTTON_L: u16 = 0x0040;
    const BUTTON_A: u16 = 0x0100;
    match button_mask & (BUTTON_L | BUTTON_A) {
        BUTTON_L => Some(OptionType::Target),
        value if value == BUTTON_L | BUTTON_A => Some(OptionType::Custom("target_roll".into())),
        _ => None,
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
    // Observation-free controller programs have an exact PAD realization.
    // Summarize that realization with the same temporal command factors used
    // for recorded demonstrations so setup headings and turns remain visible
    // to cross-action learning instead of collapsing into an opaque digest.
    if let Ok(tape) = compile_static_controller(program) {
        insert_recorded_controller_summary(parameters, &tape);
    }
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

#[cfg(test)]
mod tests;
