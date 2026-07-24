//! Generic stateful tactics over phase-correct native observations.
//!
//! Tactics have input authority only: they read an explicit observation row and
//! emit a canonical PAD frame.  The query log and emitted frames are retained so
//! a selected option can be audited and replayed without granting the tactic any
//! gameplay-state or terminal authority.

use crate::artifact::Digest;
use crate::option_execution::{
    OptionCondition, OptionEndReason, OptionExecution, OptionParameter, OptionType, TapeRange,
};
use crate::option_values::{OptionActionDescriptor, OptionValueModel, RankedOption};
use crate::tape::{InputFrame, InputTape, RawPadState};
use dusklight_automation_contracts::actor_identity::PlacedActorSelector;
use dusklight_evidence::native_episode_shard::{
    NativeActorObservation, NativeLearningObservation, NativeObservationPhase,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::f32::consts::{PI, TAU};
use std::fmt;

pub const NATIVE_GENERIC_TACTIC_SCHEMA_V1: &str = "dusklight-native-generic-tactic/v1";
pub const NATIVE_TACTIC_EXECUTION_SCHEMA_V1: &str = "dusklight-native-generic-tactic-execution/v1";
pub const MINED_TACTIC_CONDITIONS_SCHEMA_V1: &str = "dusklight-mined-tactic-conditions/v1";
pub const MAX_NATIVE_TACTIC_TICKS: u32 = 10_000;
pub const MAX_NATIVE_TACTIC_ACTORS: usize = 4_096;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GenericTactic {
    MaintainRelativeHeading {
        /// Desired world heading relative to the observed camera yaw.
        heading_radians_f32_bits: u32,
        magnitude: u8,
    },
    MaintainOffset {
        target: PlacedActorSelector,
        offset_f32_bits: [u32; 3],
        tolerance_f32_bits: u32,
        magnitude: u8,
    },
    SeekCoordinate {
        coordinate_f32_bits: [u32; 3],
        tolerance_f32_bits: u32,
        magnitude: u8,
    },
    SeekActor {
        target: PlacedActorSelector,
        tolerance_f32_bits: u32,
        magnitude: u8,
    },
    ShortCurve {
        /// One cubic Bezier curve in raw main-stick space.
        control: [[i8; 2]; 4],
    },
    CameraWhileMoving {
        movement_heading_radians_f32_bits: u32,
        movement_magnitude: u8,
        camera_heading_radians_f32_bits: u32,
        camera_magnitude: u8,
    },
    SynchronizeButtonEdge {
        button_mask: u16,
        procedure_id: u16,
        animation_resource_id: u16,
        phase_f32_bits: u32,
        movement_heading_radians_f32_bits: Option<u32>,
        movement_magnitude: u8,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeGenericTacticPlan {
    pub schema: String,
    pub tactic: GenericTactic,
    pub minimum_ticks: u32,
    pub maximum_ticks: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticActor {
    pub selector: PlacedActorSelector,
    pub runtime_generation: u64,
    pub current_room: i8,
    pub position_f32_bits: [u32; 3],
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticObservation {
    pub boundary_index: u64,
    pub simulation_tick: u64,
    pub tape_frame: u64,
    pub state_identity: [u8; 16],
    pub stage: String,
    pub room: i8,
    pub player_position_f32_bits: [u32; 3],
    pub player_yaw: i16,
    pub player_procedure: u16,
    pub player_mode_flags: u32,
    pub player_contacts: u8,
    pub camera_yaw_radians_f32_bits: Option<u32>,
    pub action_lanes: Vec<NativeTacticActionLane>,
    pub actor_set_complete: bool,
    pub actors: Vec<NativeTacticActor>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticActionLane {
    pub resource_id: u16,
    pub frame_f32_bits: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTacticQueryRecord {
    pub local_tick: u32,
    pub boundary_index: u64,
    pub simulation_tick: u64,
    pub tape_frame: u64,
    pub state_identity: [u8; 16],
    pub queried_fields: Vec<String>,
    pub player_position_f32_bits: Option<[u32; 3]>,
    pub player_yaw: Option<i16>,
    pub camera_yaw_radians_f32_bits: Option<u32>,
    pub player_procedure: Option<u16>,
    pub action_lane: Option<NativeTacticActionLane>,
    pub target_actor: Option<NativeTacticActor>,
    pub target_reached: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeGenericTacticExecution {
    pub schema: &'static str,
    pub selected: RankedOption,
    pub tape: InputTape,
    pub execution: OptionExecution,
    pub queries: Vec<NativeTacticQueryRecord>,
    pub observation_schema_sha256: Digest,
    pub every_pad_frame_recorded: bool,
    pub every_read_only_query_recorded: bool,
    pub gameplay_write_authority: bool,
    pub terminal_authority: bool,
    pub promotion_authority: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeGenericTacticCandidate {
    descriptor: OptionActionDescriptor,
    plan: NativeGenericTacticPlan,
}

impl NativeGenericTacticCandidate {
    pub fn new(
        option_id: String,
        plan: NativeGenericTacticPlan,
    ) -> Result<Self, NativeTacticError> {
        plan.validate()?;
        Ok(Self {
            descriptor: plan.descriptor(option_id)?,
            plan,
        })
    }

    pub fn descriptor(&self) -> &OptionActionDescriptor {
        &self.descriptor
    }

    pub fn plan(&self) -> &NativeGenericTacticPlan {
        &self.plan
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticExperience {
    pub successful: bool,
    pub start: NativeTacticObservation,
    pub end: NativeTacticObservation,
    pub end_reason: OptionEndReason,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "field", content = "value", rename_all = "snake_case")]
pub enum MinedObservationPredicate {
    Stage(String),
    Room(i8),
    PlayerProcedure(u16),
    PlayerModeFlags(u32),
    PlayerContacts(u8),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MinedTacticConditions {
    pub schema: String,
    pub experience_count: u32,
    pub successful_count: u32,
    pub initiation: Vec<MinedObservationPredicate>,
    pub termination: Vec<MinedObservationPredicate>,
    pub coordinate_literals_embedded: bool,
    pub published_procedures_embedded: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeTacticError {
    InvalidPlan(&'static str),
    InvalidObservation(&'static str),
    EmptyCatalog,
    CatalogMismatch,
    MissingObservations,
    TargetAbsent,
    TargetAmbiguous,
    TargetUnknown,
    Tape(String),
    Values(String),
    Serialization(String),
}

impl fmt::Display for NativeTacticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPlan(message) => write!(f, "invalid generic tactic plan: {message}"),
            Self::InvalidObservation(message) => {
                write!(f, "invalid native tactic observation: {message}")
            }
            Self::EmptyCatalog => f.write_str("generic tactic catalog is empty"),
            Self::CatalogMismatch => {
                f.write_str("policy and executable generic tactic catalogs differ")
            }
            Self::MissingObservations => {
                f.write_str("generic tactic lacks phase-correct native observations")
            }
            Self::TargetAbsent => f.write_str("portable actor target is absent"),
            Self::TargetAmbiguous => {
                f.write_str("portable actor target matches multiple live instances")
            }
            Self::TargetUnknown => f.write_str(
                "portable actor target absence is unknown because the actor set is truncated",
            ),
            Self::Tape(message) => write!(f, "generic tactic tape is invalid: {message}"),
            Self::Values(message) => write!(f, "generic tactic policy failed: {message}"),
            Self::Serialization(message) => write!(f, "generic tactic identity failed: {message}"),
        }
    }
}

impl Error for NativeTacticError {}

impl NativeTacticObservation {
    pub fn from_native(value: &NativeLearningObservation) -> Result<Self, NativeTacticError> {
        if value.phase != NativeObservationPhase::PreInput {
            return Err(NativeTacticError::InvalidObservation(
                "observation is not pre-input",
            ));
        }
        Self::from_native_boundary(
            value,
            value.boundary_index,
            value.simulation_tick,
            value.tape_frame,
        )
    }

    /// Projects a completed native simulation step onto the immediately
    /// following pre-input boundary. Native shards record the post-simulation
    /// state with the next boundary identity, but the tick and tape coordinates
    /// of the input that produced it.
    pub fn from_post_simulation_boundary(
        value: &NativeLearningObservation,
    ) -> Result<Self, NativeTacticError> {
        if value.phase != NativeObservationPhase::PostSimulation {
            return Err(NativeTacticError::InvalidObservation(
                "observation is not post-simulation",
            ));
        }
        Self::from_native_boundary(
            value,
            value.boundary_index,
            value
                .simulation_tick
                .checked_add(1)
                .ok_or(NativeTacticError::InvalidObservation(
                    "simulation tick overflows next boundary",
                ))?,
            value
                .tape_frame
                .checked_add(1)
                .ok_or(NativeTacticError::InvalidObservation(
                    "tape frame overflows next boundary",
                ))?,
        )
    }

    fn from_native_boundary(
        value: &NativeLearningObservation,
        boundary_index: u64,
        simulation_tick: u64,
        tape_frame: u64,
    ) -> Result<Self, NativeTacticError> {
        let finite3 = |values: [f32; 3]| {
            values
                .iter()
                .all(|value| value.is_finite())
                .then_some(values.map(f32::to_bits))
        };
        let player_position_f32_bits = finite3(value.player_position).ok_or(
            NativeTacticError::InvalidObservation("player position is non-finite"),
        )?;
        let camera_yaw_radians_f32_bits = value
            .camera_yaw_radians
            .map(|yaw| {
                yaw.is_finite().then_some(yaw.to_bits()).ok_or(
                    NativeTacticError::InvalidObservation("camera yaw is non-finite"),
                )
            })
            .transpose()?;
        let action_lanes = value
            .player_action
            .as_ref()
            .map(|action| {
                action
                    .under_animations
                    .iter()
                    .chain(&action.upper_animations)
                    .filter(|lane| lane.frame.is_finite())
                    .map(|lane| NativeTacticActionLane {
                        resource_id: lane.resource_id,
                        frame_f32_bits: lane.frame.to_bits(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        if value.actors.len() > MAX_NATIVE_TACTIC_ACTORS {
            return Err(NativeTacticError::InvalidObservation(
                "actor set exceeds tactic bound",
            ));
        }
        let actors = value
            .actors
            .iter()
            .filter(|actor| actor.set_id != u16::MAX)
            .map(|actor| actor_snapshot(&value.stage, actor, finite3))
            .collect::<Result<Vec<_>, _>>()?;
        let observation = Self {
            boundary_index,
            simulation_tick,
            tape_frame,
            state_identity: value.state_identity,
            stage: value.stage.clone(),
            room: value.room,
            player_position_f32_bits,
            player_yaw: value.player_current_angle[1],
            player_procedure: value.player_procedure,
            player_mode_flags: value.player_mode_flags,
            player_contacts: value.player_contacts,
            camera_yaw_radians_f32_bits,
            action_lanes,
            actor_set_complete: !value.actors_truncated,
            actors,
        };
        observation.validate()?;
        Ok(observation)
    }

    pub fn validate(&self) -> Result<(), NativeTacticError> {
        if self.stage.is_empty()
            || self.stage.len() > 8
            || !finite_bits3(self.player_position_f32_bits)
            || self
                .camera_yaw_radians_f32_bits
                .is_some_and(|bits| !f32::from_bits(bits).is_finite())
            || self.actors.len() > MAX_NATIVE_TACTIC_ACTORS
            || self
                .action_lanes
                .iter()
                .any(|lane| !f32::from_bits(lane.frame_f32_bits).is_finite())
        {
            return Err(NativeTacticError::InvalidObservation(
                "shape or finite-value contract failed",
            ));
        }
        for actor in &self.actors {
            actor
                .selector
                .validate()
                .map_err(NativeTacticError::InvalidObservation)?;
            if !finite_bits3(actor.position_f32_bits) {
                return Err(NativeTacticError::InvalidObservation(
                    "actor position is non-finite",
                ));
            }
        }
        Ok(())
    }
}

fn actor_snapshot<F>(
    stage: &str,
    actor: &NativeActorObservation,
    finite3: F,
) -> Result<NativeTacticActor, NativeTacticError>
where
    F: Fn([f32; 3]) -> Option<[u32; 3]>,
{
    let selector = PlacedActorSelector {
        stage: stage.into(),
        home_room: actor.home_room,
        set_id: actor.set_id,
        actor_name: actor.actor_name,
    };
    selector
        .validate()
        .map_err(NativeTacticError::InvalidObservation)?;
    Ok(NativeTacticActor {
        selector,
        runtime_generation: actor.runtime_generation,
        current_room: actor.current_room,
        position_f32_bits: finite3(actor.position).ok_or(NativeTacticError::InvalidObservation(
            "actor position is non-finite",
        ))?,
    })
}

impl NativeGenericTacticPlan {
    pub fn new(tactic: GenericTactic, maximum_ticks: u32) -> Self {
        Self {
            schema: NATIVE_GENERIC_TACTIC_SCHEMA_V1.into(),
            tactic,
            minimum_ticks: 1,
            maximum_ticks,
        }
    }

    pub fn validate(&self) -> Result<(), NativeTacticError> {
        if self.schema != NATIVE_GENERIC_TACTIC_SCHEMA_V1
            || self.minimum_ticks == 0
            || self.minimum_ticks > self.maximum_ticks
            || self.maximum_ticks > MAX_NATIVE_TACTIC_TICKS
        {
            return Err(NativeTacticError::InvalidPlan(
                "schema or duration is invalid",
            ));
        }
        let magnitude = |value: u8| (1..=127).contains(&value);
        let finite = |bits: u32| f32::from_bits(bits).is_finite();
        let positive = |bits: u32| {
            let value = f32::from_bits(bits);
            value.is_finite() && value >= 0.0
        };
        let valid = match &self.tactic {
            GenericTactic::MaintainRelativeHeading {
                heading_radians_f32_bits,
                magnitude: value,
            } => finite(*heading_radians_f32_bits) && magnitude(*value),
            GenericTactic::MaintainOffset {
                target,
                offset_f32_bits,
                tolerance_f32_bits,
                magnitude: value,
            } => {
                target.validate().is_ok()
                    && finite_bits3(*offset_f32_bits)
                    && positive(*tolerance_f32_bits)
                    && magnitude(*value)
            }
            GenericTactic::SeekCoordinate {
                coordinate_f32_bits,
                tolerance_f32_bits,
                magnitude: value,
            } => {
                finite_bits3(*coordinate_f32_bits)
                    && positive(*tolerance_f32_bits)
                    && magnitude(*value)
            }
            GenericTactic::SeekActor {
                target,
                tolerance_f32_bits,
                magnitude: value,
            } => target.validate().is_ok() && positive(*tolerance_f32_bits) && magnitude(*value),
            GenericTactic::ShortCurve { .. } => true,
            GenericTactic::CameraWhileMoving {
                movement_heading_radians_f32_bits,
                movement_magnitude,
                camera_heading_radians_f32_bits,
                camera_magnitude,
            } => {
                finite(*movement_heading_radians_f32_bits)
                    && magnitude(*movement_magnitude)
                    && finite(*camera_heading_radians_f32_bits)
                    && magnitude(*camera_magnitude)
            }
            GenericTactic::SynchronizeButtonEdge {
                button_mask,
                phase_f32_bits,
                movement_heading_radians_f32_bits,
                movement_magnitude,
                ..
            } => {
                *button_mask != 0
                    && finite(*phase_f32_bits)
                    && movement_heading_radians_f32_bits.is_none_or(|bits| finite(bits))
                    && (*movement_magnitude == 0 || magnitude(*movement_magnitude))
            }
        };
        if !valid {
            return Err(NativeTacticError::InvalidPlan(
                "tactic parameters are invalid",
            ));
        }
        Ok(())
    }

    pub fn descriptor(
        &self,
        option_id: String,
    ) -> Result<OptionActionDescriptor, NativeTacticError> {
        self.validate()?;
        let mut parameters = BTreeMap::new();
        parameters.insert(
            "minimum_ticks".into(),
            OptionParameter::Unsigned(u64::from(self.minimum_ticks)),
        );
        parameters.insert(
            "maximum_ticks".into(),
            OptionParameter::Unsigned(u64::from(self.maximum_ticks)),
        );
        let option_type = match &self.tactic {
            GenericTactic::MaintainRelativeHeading {
                heading_radians_f32_bits,
                magnitude,
            } => {
                parameters.insert(
                    "heading_radians".into(),
                    OptionParameter::F32Bits(*heading_radians_f32_bits),
                );
                parameters.insert(
                    "magnitude".into(),
                    OptionParameter::Unsigned(u64::from(*magnitude)),
                );
                OptionType::MaintainHeading
            }
            GenericTactic::MaintainOffset {
                target,
                offset_f32_bits,
                tolerance_f32_bits,
                magnitude,
            } => {
                insert_target(&mut parameters, target)?;
                parameters.insert(
                    "offset".into(),
                    OptionParameter::Vec3F32Bits(*offset_f32_bits),
                );
                parameters.insert(
                    "tolerance".into(),
                    OptionParameter::F32Bits(*tolerance_f32_bits),
                );
                parameters.insert(
                    "magnitude".into(),
                    OptionParameter::Unsigned(u64::from(*magnitude)),
                );
                OptionType::MaintainOffset
            }
            GenericTactic::SeekCoordinate {
                coordinate_f32_bits,
                tolerance_f32_bits,
                magnitude,
            } => {
                parameters.insert(
                    "coordinate".into(),
                    OptionParameter::Vec3F32Bits(*coordinate_f32_bits),
                );
                parameters.insert(
                    "tolerance".into(),
                    OptionParameter::F32Bits(*tolerance_f32_bits),
                );
                parameters.insert(
                    "magnitude".into(),
                    OptionParameter::Unsigned(u64::from(*magnitude)),
                );
                OptionType::Move
            }
            GenericTactic::SeekActor {
                target,
                tolerance_f32_bits,
                magnitude,
            } => {
                insert_target(&mut parameters, target)?;
                parameters.insert(
                    "tolerance".into(),
                    OptionParameter::F32Bits(*tolerance_f32_bits),
                );
                parameters.insert(
                    "magnitude".into(),
                    OptionParameter::Unsigned(u64::from(*magnitude)),
                );
                OptionType::SeekActor
            }
            GenericTactic::ShortCurve { control } => {
                parameters.insert(
                    "control".into(),
                    OptionParameter::Text(
                        serde_json::to_string(control)
                            .map_err(|error| NativeTacticError::Serialization(error.to_string()))?,
                    ),
                );
                OptionType::Bezier
            }
            GenericTactic::CameraWhileMoving {
                movement_heading_radians_f32_bits,
                movement_magnitude,
                camera_heading_radians_f32_bits,
                camera_magnitude,
            } => {
                parameters.insert(
                    "movement_heading".into(),
                    OptionParameter::F32Bits(*movement_heading_radians_f32_bits),
                );
                parameters.insert(
                    "movement_magnitude".into(),
                    OptionParameter::Unsigned(u64::from(*movement_magnitude)),
                );
                parameters.insert(
                    "camera_heading".into(),
                    OptionParameter::F32Bits(*camera_heading_radians_f32_bits),
                );
                parameters.insert(
                    "camera_magnitude".into(),
                    OptionParameter::Unsigned(u64::from(*camera_magnitude)),
                );
                OptionType::Custom("camera_while_moving".into())
            }
            GenericTactic::SynchronizeButtonEdge {
                button_mask,
                procedure_id,
                animation_resource_id,
                phase_f32_bits,
                movement_heading_radians_f32_bits,
                movement_magnitude,
            } => {
                parameters.insert(
                    "button_mask".into(),
                    OptionParameter::Unsigned(u64::from(*button_mask)),
                );
                parameters.insert(
                    "procedure_id".into(),
                    OptionParameter::Unsigned(u64::from(*procedure_id)),
                );
                parameters.insert(
                    "animation_resource_id".into(),
                    OptionParameter::Unsigned(u64::from(*animation_resource_id)),
                );
                parameters.insert("phase".into(), OptionParameter::F32Bits(*phase_f32_bits));
                if let Some(bits) = movement_heading_radians_f32_bits {
                    parameters.insert("movement_heading".into(), OptionParameter::F32Bits(*bits));
                }
                parameters.insert(
                    "movement_magnitude".into(),
                    OptionParameter::Unsigned(u64::from(*movement_magnitude)),
                );
                OptionType::Custom("synchronize_button_edge".into())
            }
        };
        Ok(OptionActionDescriptor {
            option_id,
            option_type,
            parameters,
        })
    }

    pub fn termination_condition(&self) -> Result<OptionCondition, NativeTacticError> {
        self.validate()?;
        if matches!(
            self.tactic,
            GenericTactic::MaintainRelativeHeading { .. }
                | GenericTactic::ShortCurve { .. }
                | GenericTactic::CameraWhileMoving { .. }
        ) {
            return Ok(OptionCondition::DurationElapsed);
        }
        Ok(OptionCondition::Observation {
            observation_schema_sha256: observation_schema_sha256(),
            expression_sha256: Digest(Sha256::digest(canonical(self)?).into()),
        })
    }
}

pub fn select_and_execute_generic(
    model: &OptionValueModel,
    state: &[f32],
    candidates: &[NativeGenericTacticCandidate],
    tape_prefix: &InputTape,
    observations: &[NativeTacticObservation],
) -> Result<NativeGenericTacticExecution, NativeTacticError> {
    if candidates.is_empty() {
        return Err(NativeTacticError::EmptyCatalog);
    }
    let mut model_catalog = model
        .actions()
        .iter()
        .map(canonical)
        .collect::<Result<Vec<_>, _>>()?;
    let mut executable_catalog = candidates
        .iter()
        .map(|candidate| canonical(&candidate.descriptor))
        .collect::<Result<Vec<_>, _>>()?;
    model_catalog.sort();
    executable_catalog.sort();
    executable_catalog.dedup();
    if model_catalog != executable_catalog {
        return Err(NativeTacticError::CatalogMismatch);
    }
    let selected = model
        .rank_options(state)
        .map_err(|error| NativeTacticError::Values(error.to_string()))?
        .into_iter()
        .next()
        .ok_or(NativeTacticError::EmptyCatalog)?;
    let candidate = candidates
        .iter()
        .find(|candidate| candidate.descriptor == selected.descriptor)
        .ok_or(NativeTacticError::CatalogMismatch)?;
    execute_selected(selected, candidate, tape_prefix, observations)
}

fn execute_selected(
    selected: RankedOption,
    candidate: &NativeGenericTacticCandidate,
    tape_prefix: &InputTape,
    observations: &[NativeTacticObservation],
) -> Result<NativeGenericTacticExecution, NativeTacticError> {
    tape_prefix
        .validate()
        .map_err(|error| NativeTacticError::Tape(error.to_string()))?;
    let (frames, queries, end_reason) = realize(&candidate.plan, observations)?;
    let start_frame = tape_prefix.frames.len() as u64;
    let mut tape = tape_prefix.clone();
    tape.frames.extend(frames);
    tape.validate()
        .map_err(|error| NativeTacticError::Tape(error.to_string()))?;
    let end_frame_exclusive = tape.frames.len() as u64;
    let termination_condition = candidate.plan.termination_condition()?;
    let execution = OptionExecution::capture(
        candidate.descriptor.option_id.clone(),
        candidate.descriptor.option_type.clone(),
        candidate.descriptor.parameters.clone(),
        candidate.plan.minimum_ticks,
        candidate.plan.maximum_ticks,
        termination_condition,
        Vec::new(),
        end_reason,
        &tape,
        TapeRange {
            start_frame,
            end_frame_exclusive,
        },
    )
    .map_err(|error| NativeTacticError::Tape(error.to_string()))?;
    Ok(NativeGenericTacticExecution {
        schema: NATIVE_TACTIC_EXECUTION_SCHEMA_V1,
        selected,
        tape,
        execution,
        queries,
        observation_schema_sha256: observation_schema_sha256(),
        every_pad_frame_recorded: true,
        every_read_only_query_recorded: true,
        gameplay_write_authority: false,
        terminal_authority: false,
        promotion_authority: false,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeGenericTacticStep {
    pub frame: InputFrame,
    pub query: NativeTacticQueryRecord,
    pub end_reason: Option<OptionEndReason>,
}

#[derive(Clone, Debug)]
pub struct NativeGenericTacticStepper {
    plan: NativeGenericTacticPlan,
    next_tick: u32,
    previous_observation: Option<NativeTacticObservation>,
    prior_lane_frame: Option<f32>,
    stopped: bool,
}

impl NativeGenericTacticStepper {
    pub fn new(plan: NativeGenericTacticPlan) -> Result<Self, NativeTacticError> {
        plan.validate()?;
        Ok(Self {
            plan,
            next_tick: 0,
            previous_observation: None,
            prior_lane_frame: None,
            stopped: false,
        })
    }

    pub fn plan(&self) -> &NativeGenericTacticPlan {
        &self.plan
    }

    pub fn step(
        &mut self,
        observation: NativeTacticObservation,
    ) -> Result<NativeGenericTacticStep, NativeTacticError> {
        if self.stopped || self.next_tick >= self.plan.maximum_ticks {
            return Err(NativeTacticError::InvalidObservation(
                "native tactic stepper already stopped",
            ));
        }
        observation.validate()?;
        if let Some(previous) = &self.previous_observation
            && (observation.simulation_tick != previous.simulation_tick + 1
                || observation.tape_frame != previous.tape_frame + 1
                || observation.boundary_index != previous.boundary_index + 1)
        {
            return Err(NativeTacticError::InvalidObservation(
                "native tactic observations are not one contiguous boundary",
            ));
        }
        let local_tick = self.next_tick;
        let mut query = query_base(local_tick, &observation);
        let (pad, reached) = pad_for(
            &self.plan,
            &observation,
            local_tick,
            &mut self.prior_lane_frame,
            &mut query,
        )?;
        query.target_reached = reached;
        self.next_tick += 1;
        self.previous_observation = Some(observation);
        let end_reason = if reached && self.next_tick >= self.plan.minimum_ticks {
            Some(OptionEndReason::Terminated)
        } else if self.next_tick == self.plan.maximum_ticks {
            Some(OptionEndReason::MaximumDuration)
        } else {
            None
        };
        self.stopped = end_reason.is_some();
        Ok(NativeGenericTacticStep {
            frame: owned_frame(pad),
            query,
            end_reason,
        })
    }
}

fn realize(
    plan: &NativeGenericTacticPlan,
    observations: &[NativeTacticObservation],
) -> Result<
    (
        Vec<InputFrame>,
        Vec<NativeTacticQueryRecord>,
        OptionEndReason,
    ),
    NativeTacticError,
> {
    if observations.is_empty() {
        return Err(NativeTacticError::MissingObservations);
    }
    let mut stepper = NativeGenericTacticStepper::new(plan.clone())?;
    let limit = observations.len().min(plan.maximum_ticks as usize);
    let mut frames = Vec::with_capacity(limit);
    let mut queries = Vec::with_capacity(limit);
    let mut end_reason = None;
    for observation in observations.iter().take(limit) {
        let step = stepper.step(observation.clone())?;
        frames.push(step.frame);
        queries.push(step.query);
        if step.end_reason.is_some() {
            end_reason = step.end_reason;
            break;
        }
    }
    if frames.len() < plan.minimum_ticks as usize {
        return Err(NativeTacticError::MissingObservations);
    }
    Ok((
        frames,
        queries,
        end_reason.ok_or(NativeTacticError::MissingObservations)?,
    ))
}

fn pad_for(
    plan: &NativeGenericTacticPlan,
    observation: &NativeTacticObservation,
    local_tick: u32,
    prior_lane_frame: &mut Option<f32>,
    query: &mut NativeTacticQueryRecord,
) -> Result<(RawPadState, bool), NativeTacticError> {
    let player = bits3(observation.player_position_f32_bits);
    match &plan.tactic {
        GenericTactic::MaintainRelativeHeading {
            heading_radians_f32_bits,
            magnitude,
        } => {
            query
                .queried_fields
                .extend(["player_yaw".into(), "camera_yaw".into()]);
            query.player_yaw = Some(observation.player_yaw);
            query.camera_yaw_radians_f32_bits = observation.camera_yaw_radians_f32_bits;
            let camera = required_camera(observation)?;
            let desired = camera + f32::from_bits(*heading_radians_f32_bits);
            Ok((
                main_heading(desired - yaw_radians(observation.player_yaw), *magnitude),
                false,
            ))
        }
        GenericTactic::MaintainOffset {
            target,
            offset_f32_bits,
            tolerance_f32_bits,
            magnitude,
        } => {
            let actor = target_actor(observation, target)?;
            query_actor_position(query, observation, actor);
            let actor_position = bits3(actor.position_f32_bits);
            let offset = bits3(*offset_f32_bits);
            let goal = [
                actor_position[0] + offset[0],
                actor_position[1] + offset[1],
                actor_position[2] + offset[2],
            ];
            seek_pad(
                player,
                goal,
                f32::from_bits(*tolerance_f32_bits),
                *magnitude,
                observation,
                query,
            )
        }
        GenericTactic::SeekCoordinate {
            coordinate_f32_bits,
            tolerance_f32_bits,
            magnitude,
        } => {
            query
                .queried_fields
                .extend(["player_position".into(), "camera_yaw".into()]);
            query.player_position_f32_bits = Some(observation.player_position_f32_bits);
            query.camera_yaw_radians_f32_bits = observation.camera_yaw_radians_f32_bits;
            seek_pad(
                player,
                bits3(*coordinate_f32_bits),
                f32::from_bits(*tolerance_f32_bits),
                *magnitude,
                observation,
                query,
            )
        }
        GenericTactic::SeekActor {
            target,
            tolerance_f32_bits,
            magnitude,
        } => {
            let actor = target_actor(observation, target)?;
            query_actor_position(query, observation, actor);
            seek_pad(
                player,
                bits3(actor.position_f32_bits),
                f32::from_bits(*tolerance_f32_bits),
                *magnitude,
                observation,
                query,
            )
        }
        GenericTactic::ShortCurve { control } => {
            query.queried_fields.push("simulation_tick".into());
            let denominator = plan.maximum_ticks.saturating_sub(1).max(1);
            let t = local_tick as f32 / denominator as f32;
            let sample = |axis: usize| {
                let one = 1.0 - t;
                (one.powi(3) * f32::from(control[0][axis])
                    + 3.0 * one.powi(2) * t * f32::from(control[1][axis])
                    + 3.0 * one * t.powi(2) * f32::from(control[2][axis])
                    + t.powi(3) * f32::from(control[3][axis]))
                .round()
                .clamp(-128.0, 127.0) as i8
            };
            Ok((
                RawPadState {
                    stick_x: sample(0),
                    stick_y: sample(1),
                    ..RawPadState::default()
                },
                false,
            ))
        }
        GenericTactic::CameraWhileMoving {
            movement_heading_radians_f32_bits,
            movement_magnitude,
            camera_heading_radians_f32_bits,
            camera_magnitude,
        } => {
            query.queried_fields.push("camera_yaw".into());
            query.camera_yaw_radians_f32_bits = observation.camera_yaw_radians_f32_bits;
            let camera = required_camera(observation)?;
            let mut pad = main_heading(
                f32::from_bits(*movement_heading_radians_f32_bits) - camera,
                *movement_magnitude,
            );
            let camera_pad = direction_pad(
                f32::from_bits(*camera_heading_radians_f32_bits),
                *camera_magnitude,
            );
            pad.substick_x = camera_pad.stick_x;
            pad.substick_y = camera_pad.stick_y;
            Ok((pad, false))
        }
        GenericTactic::SynchronizeButtonEdge {
            button_mask,
            procedure_id,
            animation_resource_id,
            phase_f32_bits,
            movement_heading_radians_f32_bits,
            movement_magnitude,
        } => {
            query
                .queried_fields
                .extend(["player_procedure".into(), "player_action_lane".into()]);
            query.player_procedure = Some(observation.player_procedure);
            let lane = observation
                .action_lanes
                .iter()
                .find(|lane| lane.resource_id == *animation_resource_id)
                .copied();
            query.action_lane = lane;
            let current = lane.map(|lane| f32::from_bits(lane.frame_f32_bits));
            let phase = f32::from_bits(*phase_f32_bits);
            let crossed = observation.player_procedure == *procedure_id
                && current.is_some_and(|current| {
                    prior_lane_frame.is_some_and(|prior| prior < phase && current >= phase)
                });
            *prior_lane_frame = current;
            let mut pad = movement_heading_radians_f32_bits
                .map(|bits| direction_pad(f32::from_bits(bits), *movement_magnitude))
                .unwrap_or_default();
            if crossed {
                pad.buttons = *button_mask;
            }
            Ok((pad, crossed))
        }
    }
}

fn seek_pad(
    player: [f32; 3],
    goal: [f32; 3],
    tolerance: f32,
    magnitude: u8,
    observation: &NativeTacticObservation,
    query: &mut NativeTacticQueryRecord,
) -> Result<(RawPadState, bool), NativeTacticError> {
    let dx = goal[0] - player[0];
    let dz = goal[2] - player[2];
    let reached = dx.hypot(dz) <= tolerance;
    if reached {
        return Ok((RawPadState::default(), true));
    }
    let camera = required_camera(observation)?;
    query.camera_yaw_radians_f32_bits = observation.camera_yaw_radians_f32_bits;
    Ok((direction_pad(dx.atan2(dz) - camera, magnitude), false))
}

fn target_actor<'a>(
    observation: &'a NativeTacticObservation,
    selector: &PlacedActorSelector,
) -> Result<&'a NativeTacticActor, NativeTacticError> {
    let mut matches = observation
        .actors
        .iter()
        .filter(|actor| actor.selector == *selector);
    let first = matches.next();
    if matches.next().is_some() {
        return Err(NativeTacticError::TargetAmbiguous);
    }
    match first {
        Some(actor) => Ok(actor),
        None if observation.actor_set_complete => Err(NativeTacticError::TargetAbsent),
        None => Err(NativeTacticError::TargetUnknown),
    }
}

fn query_actor_position(
    query: &mut NativeTacticQueryRecord,
    observation: &NativeTacticObservation,
    actor: &NativeTacticActor,
) {
    query.queried_fields.extend([
        "player_position".into(),
        "camera_yaw".into(),
        "portable_actor_identity".into(),
        "actor_position".into(),
    ]);
    query.player_position_f32_bits = Some(observation.player_position_f32_bits);
    query.camera_yaw_radians_f32_bits = observation.camera_yaw_radians_f32_bits;
    query.target_actor = Some(actor.clone());
}

fn query_base(local_tick: u32, observation: &NativeTacticObservation) -> NativeTacticQueryRecord {
    NativeTacticQueryRecord {
        local_tick,
        boundary_index: observation.boundary_index,
        simulation_tick: observation.simulation_tick,
        tape_frame: observation.tape_frame,
        state_identity: observation.state_identity,
        queried_fields: Vec::new(),
        player_position_f32_bits: None,
        player_yaw: None,
        camera_yaw_radians_f32_bits: None,
        player_procedure: None,
        action_lane: None,
        target_actor: None,
        target_reached: false,
    }
}

fn required_camera(observation: &NativeTacticObservation) -> Result<f32, NativeTacticError> {
    observation
        .camera_yaw_radians_f32_bits
        .map(f32::from_bits)
        .ok_or(NativeTacticError::InvalidObservation(
            "tactic requires a sampled camera yaw",
        ))
}

fn direction_pad(angle: f32, magnitude: u8) -> RawPadState {
    RawPadState {
        stick_x: (angle.sin() * f32::from(magnitude))
            .round()
            .clamp(-128.0, 127.0) as i8,
        stick_y: (angle.cos() * f32::from(magnitude))
            .round()
            .clamp(-128.0, 127.0) as i8,
        ..RawPadState::default()
    }
}

fn main_heading(relative_error: f32, magnitude: u8) -> RawPadState {
    direction_pad(wrap_angle(relative_error), magnitude)
}

fn yaw_radians(yaw: i16) -> f32 {
    f32::from(yaw) * PI / 32768.0
}
fn wrap_angle(value: f32) -> f32 {
    (value + PI).rem_euclid(TAU) - PI
}
fn bits3(bits: [u32; 3]) -> [f32; 3] {
    bits.map(f32::from_bits)
}
fn finite_bits3(bits: [u32; 3]) -> bool {
    bits.into_iter()
        .all(|bits| f32::from_bits(bits).is_finite())
}

fn owned_frame(pad: RawPadState) -> InputFrame {
    InputFrame {
        owned_ports: 1,
        pads: [
            pad,
            RawPadState::default(),
            RawPadState::default(),
            RawPadState::default(),
        ],
        ..InputFrame::default()
    }
}

fn insert_target(
    parameters: &mut BTreeMap<String, OptionParameter>,
    target: &PlacedActorSelector,
) -> Result<(), NativeTacticError> {
    parameters.insert(
        "target".into(),
        OptionParameter::Text(
            serde_json::to_string(target)
                .map_err(|error| NativeTacticError::Serialization(error.to_string()))?,
        ),
    );
    Ok(())
}

fn observation_schema_sha256() -> Digest {
    Digest(
        Sha256::digest(
            b"dusklight-native-learning-observation/pre-input/v27:generic-tactic-query/v1",
        )
        .into(),
    )
}

fn canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, NativeTacticError> {
    serde_json::to_vec(value).map_err(|error| NativeTacticError::Serialization(error.to_string()))
}

pub fn mine_tactic_conditions(
    experiences: &[TacticExperience],
) -> Result<MinedTacticConditions, NativeTacticError> {
    if experiences.len() < 2 || experiences.iter().all(|experience| !experience.successful) {
        return Err(NativeTacticError::InvalidPlan(
            "condition mining requires successful and comparative experience",
        ));
    }
    for experience in experiences {
        experience.start.validate()?;
        experience.end.validate()?;
    }
    let positives = experiences
        .iter()
        .filter(|experience| experience.successful)
        .collect::<Vec<_>>();
    let negatives = experiences
        .iter()
        .filter(|experience| !experience.successful)
        .collect::<Vec<_>>();
    let positive_starts = positives
        .iter()
        .map(|experience| predicates(&experience.start))
        .collect::<Vec<_>>();
    let negative_starts = negatives
        .iter()
        .map(|experience| predicates(&experience.start))
        .collect::<Vec<_>>();
    let positive_ends = positives
        .iter()
        .map(|experience| predicates(&experience.end))
        .collect::<Vec<_>>();
    let initiation = discriminating_intersection(&positive_starts, &negative_starts);
    let termination = discriminating_intersection(&positive_ends, &positive_starts);
    Ok(MinedTacticConditions {
        schema: MINED_TACTIC_CONDITIONS_SCHEMA_V1.into(),
        experience_count: experiences.len() as u32,
        successful_count: positives.len() as u32,
        initiation,
        termination,
        coordinate_literals_embedded: false,
        published_procedures_embedded: false,
    })
}

fn predicates(observation: &NativeTacticObservation) -> BTreeSet<MinedObservationPredicate> {
    BTreeSet::from([
        MinedObservationPredicate::Stage(observation.stage.clone()),
        MinedObservationPredicate::Room(observation.room),
        MinedObservationPredicate::PlayerProcedure(observation.player_procedure),
        MinedObservationPredicate::PlayerModeFlags(observation.player_mode_flags),
        MinedObservationPredicate::PlayerContacts(observation.player_contacts),
    ])
}

fn discriminating_intersection(
    positive: &[BTreeSet<MinedObservationPredicate>],
    comparison: &[BTreeSet<MinedObservationPredicate>],
) -> Vec<MinedObservationPredicate> {
    let Some(first) = positive.first() else {
        return Vec::new();
    };
    first
        .iter()
        .filter(|predicate| {
            positive.iter().all(|row| row.contains(*predicate))
                && comparison.iter().all(|row| !row.contains(*predicate))
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fqi::FqiConfig;
    use crate::option_values::{OptionValueConfig, OptionValueSample};
    use dusklight_evidence::native_episode_shard::NativeEpisodeShard;

    fn observation(tick: u64, position: [f32; 3]) -> NativeTacticObservation {
        NativeTacticObservation {
            boundary_index: 4 + tick,
            simulation_tick: 100 + tick,
            tape_frame: 20 + tick,
            state_identity: [tick as u8; 16],
            stage: "F_SP104".into(),
            room: 1,
            player_position_f32_bits: position.map(f32::to_bits),
            player_yaw: 0,
            player_procedure: 7,
            player_mode_flags: 3,
            player_contacts: 1,
            camera_yaw_radians_f32_bits: Some(0.0_f32.to_bits()),
            action_lanes: Vec::new(),
            actor_set_complete: true,
            actors: Vec::new(),
        }
    }

    fn model(candidate: &NativeGenericTacticCandidate) -> OptionValueModel {
        let samples = [0.0_f32, 1.0]
            .into_iter()
            .enumerate()
            .map(|(index, state)| OptionValueSample {
                action: candidate.descriptor.clone(),
                state: vec![state],
                duration_ticks: 3,
                reward: 1.0,
                next_state: vec![state + 1.0],
                terminal: true,
                before_state_sha256: Digest([index as u8 + 3; 32]),
                after_state_sha256: Digest([index as u8 + 4; 32]),
                source_checkpoint_sha256: Digest([index as u8 + 5; 32]),
                next_checkpoint_sha256: Digest([index as u8 + 6; 32]),
                realized_tape_range: TapeRange {
                    start_frame: 0,
                    end_frame_exclusive: 3,
                },
                realized_tape_sha256: Digest([index as u8 + 1; 32]),
            })
            .collect::<Vec<_>>();
        OptionValueModel::fit(
            1,
            &samples,
            &[1, 2],
            &OptionValueConfig {
                fitted_q: FqiConfig {
                    iterations: 2,
                    trees_per_action: 2,
                    bootstrap: false,
                    seed: 4,
                    ..FqiConfig::default()
                },
            },
        )
        .unwrap()
    }

    #[test]
    fn projects_a_post_simulation_row_onto_the_next_tactic_boundary() {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        let step = &shard.episodes[0].steps[0];
        let before = NativeTacticObservation::from_native(&step.pre_input).unwrap();
        let after =
            NativeTacticObservation::from_post_simulation_boundary(&step.post_simulation).unwrap();

        assert_eq!(after.boundary_index, before.boundary_index + 1);
        assert_eq!(after.simulation_tick, before.simulation_tick + 1);
        assert_eq!(after.tape_frame, before.tape_frame + 1);
        assert_eq!(after.state_identity, step.post_simulation.state_identity);
    }

    #[test]
    fn policy_selects_seek_and_records_every_native_query_and_pad() {
        let plan = NativeGenericTacticPlan::new(
            GenericTactic::SeekCoordinate {
                coordinate_f32_bits: [3.0_f32.to_bits(), 0.0_f32.to_bits(), 0.0_f32.to_bits()],
                tolerance_f32_bits: 0.25_f32.to_bits(),
                magnitude: 100,
            },
            4,
        );
        let candidate = NativeGenericTacticCandidate::new("seek".into(), plan).unwrap();
        let observations = [
            observation(0, [0.0, 0.0, 0.0]),
            observation(1, [1.5, 0.0, 0.0]),
            observation(2, [3.0, 0.0, 0.0]),
        ];
        let result = select_and_execute_generic(
            &model(&candidate),
            &[0.0],
            &[candidate],
            &InputTape::default(),
            &observations,
        )
        .unwrap();
        assert_eq!(result.execution.duration.realized_ticks, 3);
        assert_eq!(result.execution.end_reason, OptionEndReason::Terminated);
        assert_eq!(
            result.queries.len(),
            result.execution.emitted_raw_actions.len()
        );
        assert_eq!(result.tape.frames, result.execution.emitted_raw_actions);
        assert!(
            result
                .queries
                .iter()
                .all(|query| query.queried_fields.contains(&"player_position".into()))
        );
        assert!(!result.gameplay_write_authority);
        assert!(!result.terminal_authority);
        result
            .execution
            .validate_against_tape(&result.tape)
            .unwrap();
    }

    #[test]
    fn portable_actor_target_refuses_ambiguous_or_truncated_absence() {
        let selector = PlacedActorSelector {
            stage: "F_SP104".into(),
            home_room: 1,
            set_id: 2,
            actor_name: 3,
        };
        let plan = NativeGenericTacticPlan::new(
            GenericTactic::SeekActor {
                target: selector.clone(),
                tolerance_f32_bits: 1.0_f32.to_bits(),
                magnitude: 80,
            },
            1,
        );
        let mut observed = observation(0, [0.0; 3]);
        observed.actor_set_complete = false;
        assert_eq!(
            realize(&plan, &[observed.clone()]).unwrap_err(),
            NativeTacticError::TargetUnknown
        );
        let actor = NativeTacticActor {
            selector,
            runtime_generation: 10,
            current_room: 1,
            position_f32_bits: [1.0_f32.to_bits(), 0.0_f32.to_bits(), 0.0_f32.to_bits()],
        };
        observed.actor_set_complete = true;
        observed.actors = vec![actor.clone(), actor];
        assert_eq!(
            realize(&plan, &[observed]).unwrap_err(),
            NativeTacticError::TargetAmbiguous
        );
    }

    #[test]
    fn synchronizes_one_button_edge_to_observed_action_phase() {
        let plan = NativeGenericTacticPlan::new(
            GenericTactic::SynchronizeButtonEdge {
                button_mask: 0x0100,
                procedure_id: 7,
                animation_resource_id: 12,
                phase_f32_bits: 4.0_f32.to_bits(),
                movement_heading_radians_f32_bits: None,
                movement_magnitude: 0,
            },
            4,
        );
        let mut observations = [observation(0, [0.0; 3]), observation(1, [0.0; 3])];
        observations[0].action_lanes = vec![NativeTacticActionLane {
            resource_id: 12,
            frame_f32_bits: 3.5_f32.to_bits(),
        }];
        observations[1].action_lanes = vec![NativeTacticActionLane {
            resource_id: 12,
            frame_f32_bits: 4.25_f32.to_bits(),
        }];
        let (frames, queries, reason) = realize(&plan, &observations).unwrap();
        assert_eq!(reason, OptionEndReason::Terminated);
        assert_eq!(frames[0].pads[0].buttons, 0);
        assert_eq!(frames[1].pads[0].buttons, 0x0100);
        assert_eq!(queries[1].action_lane.unwrap().resource_id, 12);
    }

    #[test]
    fn mines_initiation_and_termination_without_route_coordinates() {
        let mut success_start = observation(0, [12.0, 0.0, -3.0]);
        success_start.player_procedure = 7;
        let mut success_end = observation(1, [99.0, 0.0, 42.0]);
        success_end.player_procedure = 9;
        let mut failure_start = observation(2, [-400.0, 0.0, 8.0]);
        failure_start.player_procedure = 5;
        let failure_end = failure_start.clone();
        let mined = mine_tactic_conditions(&[
            TacticExperience {
                successful: true,
                start: success_start,
                end: success_end,
                end_reason: OptionEndReason::Terminated,
            },
            TacticExperience {
                successful: false,
                start: failure_start,
                end: failure_end,
                end_reason: OptionEndReason::MaximumDuration,
            },
        ])
        .unwrap();
        assert!(
            mined
                .initiation
                .contains(&MinedObservationPredicate::PlayerProcedure(7))
        );
        assert!(
            mined
                .termination
                .contains(&MinedObservationPredicate::PlayerProcedure(9))
        );
        assert!(!mined.coordinate_literals_embedded);
        assert!(!mined.published_procedures_embedded);
    }
}
