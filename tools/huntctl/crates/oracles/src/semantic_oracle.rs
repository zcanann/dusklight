//! Reached/avoided semantic oracles over immutable gameplay observations.

use crate::trace::{
    DecodedTrace, TraceAnimationLane, TraceChannel, TraceChannelStatus, TraceRecord,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fmt;

mod evaluation;
use evaluation::{anomaly_tick, evaluate_one, vector_length};

pub const SEMANTIC_ORACLE_SCHEMA_V1: &str = "dusklight-semantic-oracles/v1";
pub const RUN_OUTCOME_SCHEMA_V1: &str = "dusklight-run-outcome/v1";
const MAX_ORACLES: usize = 128;
const MAX_RUN_ANOMALIES: usize = 4_096;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticOracleProgram {
    pub schema: String,
    pub oracles: Vec<SemanticOracle>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticOracle {
    pub name: String,
    pub polarity: OraclePolarity,
    pub target: OracleTarget,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OraclePolarity {
    Reached,
    Avoided,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OracleTarget {
    Stage {
        stage: String,
    },
    Room {
        stage: String,
        room: i8,
    },
    Region {
        #[serde(default)]
        stage: Option<String>,
        #[serde(default)]
        room: Option<i8>,
        min: [f32; 3],
        max: [f32; 3],
    },
    Action {
        procedure_id: u16,
        #[serde(default)]
        mode_all: u32,
        #[serde(default)]
        mode_none: u32,
    },
    Animation {
        bank: AnimationBank,
        #[serde(default)]
        lane: Option<u8>,
        resource_id: u16,
        #[serde(default)]
        frame_min: Option<f32>,
        #[serde(default)]
        frame_max: Option<f32>,
    },
    Flag {
        domain: FlagDomain,
        #[serde(default)]
        room: Option<i8>,
        index: u16,
        value: bool,
    },
    ActorState {
        stage: String,
        home_room: i8,
        set_id: u16,
        actor_name: i16,
        #[serde(default)]
        current_room: Option<i8>,
        #[serde(default)]
        health: Option<i32>,
        #[serde(default)]
        status_all: u32,
        #[serde(default)]
        status_none: u32,
    },
    Event {
        #[serde(default)]
        id: Option<i16>,
        #[serde(default)]
        name_hash: Option<u32>,
        #[serde(default)]
        mode: Option<u8>,
        #[serde(default)]
        status: Option<u8>,
    },
    CollisionCrossing {
        point: [f32; 3],
        normal: [f32; 3],
        #[serde(default)]
        tolerance: f32,
        #[serde(default)]
        contact_mask: u32,
    },
    OutOfBounds {
        allowed_min: [f32; 3],
        allowed_max: [f32; 3],
    },
    VoidSurvival {
        below_y: f32,
        minimum_ticks: u32,
    },
    UnexpectedLoad {
        allowed_destinations: Vec<LocationTarget>,
    },
    WrongWarp {
        expected: LocationTarget,
    },
    ExcessiveMotion {
        #[serde(default)]
        max_displacement: Option<f32>,
        #[serde(default)]
        max_speed: Option<f32>,
    },
    NonFiniteState,
    ImpossibleCoordinates {
        max_abs: f32,
    },
    ActorCorruption {
        #[serde(default)]
        actor_name: Option<i16>,
        #[serde(default)]
        field: Option<String>,
    },
    SlotExhaustion,
    WatchedFieldCorruption {
        #[serde(default)]
        field: Option<String>,
    },
    HeapFailure {
        #[serde(default)]
        heap: Option<String>,
    },
    Crash,
    Hang {
        minimum_stalled_millis: u64,
    },
    Softlock {
        minimum_ticks: u64,
    },
    ControlLoss {
        minimum_ticks: u64,
    },
    DuplicateItemReward {
        #[serde(default)]
        grant_kind: Option<GrantKind>,
        #[serde(default)]
        id: Option<u32>,
    },
    PreservedStorageState {
        #[serde(default)]
        field: Option<String>,
    },
    EventQueueing {
        #[serde(default)]
        event_id: Option<i16>,
        minimum_depth: u32,
    },
    SequenceBreak {
        #[serde(default)]
        sequence: Option<String>,
    },
    SaveStateAnomaly {
        #[serde(default)]
        slot: Option<u8>,
        #[serde(default)]
        field: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantKind {
    Item,
    Reward,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocationTarget {
    pub stage: String,
    pub room: i8,
    #[serde(default)]
    pub layer: Option<i8>,
    #[serde(default)]
    pub point: Option<i16>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnimationBank {
    Under,
    Upper,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FlagDomain {
    Event,
    Temporary,
    Dungeon,
    Switch,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SupplementalObservations {
    pub snapshots: Vec<SupplementalSnapshot>,
    /// True only when every trace tick has every requested indexed flag.
    pub flags_complete: bool,
    /// True only when every trace tick has a complete actor population.
    pub actors_complete: bool,
    /// Process- and monitor-level evidence that cannot be represented by a
    /// successfully decoded gameplay trace.
    #[serde(default)]
    pub run_outcome: Option<RunOutcomeEvidence>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunOutcomeEvidence {
    pub schema: String,
    /// Each listed domain was monitored continuously for the represented run.
    pub monitored: Vec<RunEvidenceKind>,
    #[serde(default)]
    pub termination: Option<RunTermination>,
    #[serde(default)]
    pub anomalies: Vec<RunAnomalyObservation>,
}

impl RunOutcomeEvidence {
    pub fn validate(&self) -> Result<(), OracleError> {
        validate_run_outcome(self)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunEvidenceKind {
    ActorIntegrity,
    ActorSlots,
    WatchedFields,
    Heap,
    Progress,
    Control,
    InventoryRewards,
    Storage,
    EventQueue,
    Sequence,
    SaveState,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunTermination {
    Completed {
        exit_code: i32,
    },
    Crashed {
        #[serde(default)]
        exit_code: Option<i32>,
        #[serde(default)]
        signal: Option<i32>,
        reason: String,
    },
    TimedOut {
        wall_time_millis: u64,
        stalled_millis: u64,
        last_simulation_tick: u64,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunActorIdentity {
    #[serde(default)]
    pub process_id: Option<u32>,
    pub actor_name: i16,
    #[serde(default)]
    pub stage: Option<String>,
    #[serde(default)]
    pub home_room: Option<i8>,
    #[serde(default)]
    pub set_id: Option<u16>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunAnomalyObservation {
    ActorCorruption {
        simulation_tick: u64,
        #[serde(default)]
        tape_frame: Option<u64>,
        actor: RunActorIdentity,
        field: String,
        expected: String,
        actual: String,
    },
    SlotExhaustion {
        simulation_tick: u64,
        #[serde(default)]
        tape_frame: Option<u64>,
        active_slots: u32,
        capacity: u32,
        #[serde(default)]
        requested_actor_name: Option<i16>,
    },
    WatchedFieldCorruption {
        simulation_tick: u64,
        #[serde(default)]
        tape_frame: Option<u64>,
        field: String,
        expected: String,
        actual: String,
    },
    HeapFailure {
        #[serde(default)]
        simulation_tick: Option<u64>,
        #[serde(default)]
        tape_frame: Option<u64>,
        heap: String,
        operation: String,
        requested_bytes: u64,
        free_bytes: u64,
    },
    Softlock {
        start_tick: u64,
        end_tick: u64,
        #[serde(default)]
        tape_frame: Option<u64>,
        last_progress: String,
        reason: String,
    },
    ControlLoss {
        start_tick: u64,
        end_tick: u64,
        #[serde(default)]
        tape_frame: Option<u64>,
        #[serde(default)]
        procedure_id: Option<u16>,
        reason: String,
    },
    DuplicateItemReward {
        simulation_tick: u64,
        #[serde(default)]
        tape_frame: Option<u64>,
        grant_kind: GrantKind,
        id: u32,
        first_source: String,
        duplicate_source: String,
        total_grants: u32,
    },
    PreservedStorageState {
        simulation_tick: u64,
        #[serde(default)]
        tape_frame: Option<u64>,
        field: String,
        expected_reset: String,
        actual: String,
    },
    EventQueueing {
        simulation_tick: u64,
        #[serde(default)]
        tape_frame: Option<u64>,
        #[serde(default)]
        running_event_id: Option<i16>,
        queued_event_ids: Vec<i16>,
    },
    SequenceBreak {
        simulation_tick: u64,
        #[serde(default)]
        tape_frame: Option<u64>,
        sequence: String,
        expected_step: String,
        actual_step: String,
    },
    SaveStateAnomaly {
        #[serde(default)]
        simulation_tick: Option<u64>,
        #[serde(default)]
        tape_frame: Option<u64>,
        slot: u8,
        field: String,
        expected: String,
        actual: String,
    },
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SupplementalSnapshot {
    pub simulation_tick: u64,
    pub flags: Vec<FlagObservation>,
    pub actors: Vec<ActorObservation>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FlagObservation {
    pub domain: FlagDomain,
    pub room: Option<i8>,
    pub index: u16,
    pub value: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActorObservation {
    pub stage: String,
    pub home_room: i8,
    pub set_id: u16,
    pub actor_name: i16,
    pub current_room: i8,
    pub health: i32,
    pub status: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OracleDisposition {
    Satisfied,
    Violated,
    Indeterminate,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SemanticOracleReport {
    pub schema: &'static str,
    pub trace_complete: bool,
    pub results: Vec<SemanticOracleResult>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SemanticOracleResult {
    pub name: String,
    pub polarity: OraclePolarity,
    pub disposition: OracleDisposition,
    pub inspected_observations: usize,
    pub first_match: Option<OracleMatch>,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OracleMatch {
    pub simulation_tick: u64,
    pub tape_frame: Option<u64>,
    pub facts: OracleFacts,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OracleFacts {
    Stage {
        stage: String,
    },
    Room {
        stage: String,
        room: i8,
    },
    Region {
        stage: String,
        room: i8,
        position: [f32; 3],
    },
    Action {
        procedure_id: u16,
        mode_flags: u32,
    },
    Animation {
        bank: AnimationBank,
        lane: u8,
        resource_id: u16,
        frame: f32,
        rate: f32,
    },
    Flag {
        domain: FlagDomain,
        room: Option<i8>,
        index: u16,
        value: bool,
    },
    ActorState {
        stage: String,
        home_room: i8,
        set_id: u16,
        actor_name: i16,
        current_room: i8,
        health: i32,
        status: u32,
    },
    Event {
        id: i16,
        name_hash: Option<u32>,
        mode: u8,
        status: u8,
    },
    CollisionCrossing {
        previous_position: [f32; 3],
        position: [f32; 3],
        previous_signed_distance: f32,
        signed_distance: f32,
        collision_flags: u32,
    },
    OutOfBounds {
        position: [f32; 3],
    },
    VoidSurvival {
        position: [f32; 3],
        ticks_without_ground: u32,
    },
    UnexpectedLoad {
        destination: LocationTarget,
    },
    WrongWarp {
        destination: LocationTarget,
        expected: LocationTarget,
    },
    ExcessiveMotion {
        previous_position: [f32; 3],
        position: [f32; 3],
        displacement: f32,
        speed: f32,
    },
    NonFiniteState {
        field: String,
    },
    ImpossibleCoordinates {
        position: [f32; 3],
        max_abs: f32,
    },
    ActorCorruption {
        actor: RunActorIdentity,
        field: String,
        expected: String,
        actual: String,
    },
    SlotExhaustion {
        active_slots: u32,
        capacity: u32,
        requested_actor_name: Option<i16>,
    },
    WatchedFieldCorruption {
        field: String,
        expected: String,
        actual: String,
    },
    HeapFailure {
        heap: String,
        operation: String,
        requested_bytes: u64,
        free_bytes: u64,
    },
    Crash {
        exit_code: Option<i32>,
        signal: Option<i32>,
        reason: String,
    },
    Hang {
        wall_time_millis: u64,
        stalled_millis: u64,
        last_simulation_tick: u64,
    },
    Softlock {
        start_tick: u64,
        end_tick: u64,
        ticks_without_progress: u64,
        last_progress: String,
        reason: String,
    },
    ControlLoss {
        start_tick: u64,
        end_tick: u64,
        ticks_without_control: u64,
        procedure_id: Option<u16>,
        reason: String,
    },
    DuplicateItemReward {
        grant_kind: GrantKind,
        id: u32,
        first_source: String,
        duplicate_source: String,
        total_grants: u32,
    },
    PreservedStorageState {
        field: String,
        expected_reset: String,
        actual: String,
    },
    EventQueueing {
        running_event_id: Option<i16>,
        queued_event_ids: Vec<i16>,
    },
    SequenceBreak {
        sequence: String,
        expected_step: String,
        actual_step: String,
    },
    SaveStateAnomaly {
        slot: u8,
        field: String,
        expected: String,
        actual: String,
    },
}

impl SemanticOracleProgram {
    pub fn validate(&self) -> Result<(), OracleError> {
        if self.schema != SEMANTIC_ORACLE_SCHEMA_V1 {
            return Err(OracleError::new("unsupported semantic-oracle schema"));
        }
        if self.oracles.is_empty() || self.oracles.len() > MAX_ORACLES {
            return Err(OracleError::new("semantic-oracle count is outside 1..=128"));
        }
        let mut names = HashSet::new();
        for oracle in &self.oracles {
            if oracle.name.is_empty() || oracle.name.len() > 96 || !names.insert(&oracle.name) {
                return Err(OracleError::new("oracle names must be unique and bounded"));
            }
            validate_target(&oracle.target)?;
        }
        Ok(())
    }

    pub fn evaluate(
        &self,
        trace: &DecodedTrace,
        supplemental: &SupplementalObservations,
    ) -> Result<SemanticOracleReport, OracleError> {
        self.validate()?;
        validate_supplemental(trace, supplemental)?;
        if let Some(outcome) = &supplemental.run_outcome {
            validate_run_outcome(outcome)?;
        }
        let snapshots = supplemental
            .snapshots
            .iter()
            .map(|snapshot| (snapshot.simulation_tick, snapshot))
            .collect::<BTreeMap<_, _>>();
        if snapshots.len() != supplemental.snapshots.len() {
            return Err(OracleError::new("duplicate supplemental simulation tick"));
        }
        let trace_complete = !trace.capacity_exhausted && !trace.records.is_empty();
        let results = self
            .oracles
            .iter()
            .map(|oracle| evaluate_one(oracle, trace, supplemental, &snapshots, trace_complete))
            .collect();
        Ok(SemanticOracleReport {
            schema: "dusklight-semantic-oracle-results/v1",
            trace_complete,
            results,
        })
    }
}

fn validate_supplemental(
    trace: &DecodedTrace,
    supplemental: &SupplementalObservations,
) -> Result<(), OracleError> {
    let trace_ticks = trace
        .records
        .iter()
        .map(|record| record.simulation_tick)
        .collect::<HashSet<_>>();
    for snapshot in &supplemental.snapshots {
        if !trace_ticks.contains(&snapshot.simulation_tick) {
            return Err(OracleError::new(
                "supplemental observation does not align to a trace tick",
            ));
        }
        let mut flags = HashSet::new();
        for flag in &snapshot.flags {
            let max = flag_index_max(flag.domain);
            if flag.index > max
                || (flag.domain == FlagDomain::Switch) != flag.room.is_some()
                || !flags.insert((flag.domain, flag.room, flag.index))
            {
                return Err(OracleError::new("invalid or duplicate supplemental flag"));
            }
        }
        let mut actors = HashSet::new();
        for actor in &snapshot.actors {
            if !stage_is_valid(&actor.stage)
                || actor.set_id == u16::MAX
                || !actors.insert((
                    actor.stage.as_str(),
                    actor.home_room,
                    actor.set_id,
                    actor.actor_name,
                ))
            {
                return Err(OracleError::new(
                    "invalid or duplicate supplemental actor identity",
                ));
            }
        }
    }
    Ok(())
}

fn validate_run_outcome(outcome: &RunOutcomeEvidence) -> Result<(), OracleError> {
    if outcome.schema != RUN_OUTCOME_SCHEMA_V1 {
        return Err(OracleError::new("unsupported run-outcome schema"));
    }
    let monitored = outcome.monitored.iter().copied().collect::<HashSet<_>>();
    if monitored.len() != outcome.monitored.len() {
        return Err(OracleError::new("duplicate monitored run-evidence domain"));
    }
    if outcome.anomalies.len() > MAX_RUN_ANOMALIES {
        return Err(OracleError::new("too many run anomaly observations"));
    }
    if let Some(termination) = &outcome.termination {
        match termination {
            RunTermination::Completed { .. } => {}
            RunTermination::Crashed { reason, .. } => {
                validate_evidence_text(reason, "crash reason")?
            }
            RunTermination::TimedOut {
                wall_time_millis,
                stalled_millis,
                ..
            } if *wall_time_millis == 0 || stalled_millis > wall_time_millis => {
                return Err(OracleError::new("invalid timeout duration"));
            }
            RunTermination::TimedOut { .. } => {}
        }
    }
    let mut previous_tick = None;
    for anomaly in &outcome.anomalies {
        let tick = anomaly_tick(anomaly);
        if previous_tick.is_some_and(|previous| tick < previous) {
            return Err(OracleError::new(
                "run anomaly observations are not chronological",
            ));
        }
        previous_tick = Some(tick);
        match anomaly {
            RunAnomalyObservation::ActorCorruption {
                actor,
                field,
                expected,
                actual,
                ..
            } => {
                validate_run_actor(actor)?;
                validate_evidence_text(field, "actor field")?;
                validate_evidence_text(expected, "expected actor value")?;
                validate_evidence_text(actual, "actual actor value")?;
            }
            RunAnomalyObservation::SlotExhaustion {
                active_slots,
                capacity,
                ..
            } if *capacity == 0 || active_slots < capacity => {
                return Err(OracleError::new("invalid actor slot exhaustion"));
            }
            RunAnomalyObservation::SlotExhaustion { .. } => {}
            RunAnomalyObservation::WatchedFieldCorruption {
                field,
                expected,
                actual,
                ..
            } => {
                validate_evidence_text(field, "watched field")?;
                validate_evidence_text(expected, "expected watched value")?;
                validate_evidence_text(actual, "actual watched value")?;
            }
            RunAnomalyObservation::HeapFailure {
                heap,
                operation,
                requested_bytes,
                ..
            } => {
                validate_evidence_text(heap, "heap name")?;
                validate_evidence_text(operation, "heap operation")?;
                if *requested_bytes == 0 {
                    return Err(OracleError::new("heap failure requested zero bytes"));
                }
            }
            RunAnomalyObservation::Softlock {
                start_tick,
                end_tick,
                last_progress,
                reason,
                ..
            } => {
                validate_tick_range(*start_tick, *end_tick, "softlock")?;
                validate_evidence_text(last_progress, "last semantic progress")?;
                validate_evidence_text(reason, "softlock reason")?;
            }
            RunAnomalyObservation::ControlLoss {
                start_tick,
                end_tick,
                reason,
                ..
            } => {
                validate_tick_range(*start_tick, *end_tick, "control loss")?;
                validate_evidence_text(reason, "control-loss reason")?;
            }
            RunAnomalyObservation::DuplicateItemReward {
                first_source,
                duplicate_source,
                total_grants,
                ..
            } => {
                validate_evidence_text(first_source, "first grant source")?;
                validate_evidence_text(duplicate_source, "duplicate grant source")?;
                if *total_grants < 2 {
                    return Err(OracleError::new("duplicate grant count is below two"));
                }
            }
            RunAnomalyObservation::PreservedStorageState {
                field,
                expected_reset,
                actual,
                ..
            } => {
                validate_evidence_text(field, "preserved storage field")?;
                validate_evidence_text(expected_reset, "expected reset value")?;
                validate_evidence_text(actual, "preserved storage value")?;
            }
            RunAnomalyObservation::EventQueueing {
                queued_event_ids, ..
            } if queued_event_ids.is_empty() || queued_event_ids.len() > 256 => {
                return Err(OracleError::new("invalid queued event population"));
            }
            RunAnomalyObservation::EventQueueing { .. } => {}
            RunAnomalyObservation::SequenceBreak {
                sequence,
                expected_step,
                actual_step,
                ..
            } => {
                validate_evidence_text(sequence, "sequence name")?;
                validate_evidence_text(expected_step, "expected sequence step")?;
                validate_evidence_text(actual_step, "actual sequence step")?;
                if expected_step == actual_step {
                    return Err(OracleError::new("sequence break has identical steps"));
                }
            }
            RunAnomalyObservation::SaveStateAnomaly {
                slot,
                field,
                expected,
                actual,
                ..
            } => {
                if *slot > 2 {
                    return Err(OracleError::new("invalid save slot"));
                }
                validate_evidence_text(field, "save-state field")?;
                validate_evidence_text(expected, "expected save-state value")?;
                validate_evidence_text(actual, "actual save-state value")?;
            }
        }
    }
    Ok(())
}

fn validate_run_actor(actor: &RunActorIdentity) -> Result<(), OracleError> {
    if actor.process_id.is_some_and(|process_id| process_id == 0) || actor.set_id == Some(u16::MAX)
    {
        return Err(OracleError::new("invalid run actor identity"));
    }
    let placed_fields = [
        actor.stage.is_some(),
        actor.home_room.is_some(),
        actor.set_id.is_some(),
    ];
    if placed_fields.iter().any(|present| *present)
        && (!placed_fields.iter().all(|present| *present)
            || !actor.stage.as_deref().is_some_and(stage_is_valid))
    {
        return Err(OracleError::new("incomplete placed run actor identity"));
    }
    Ok(())
}

fn validate_tick_range(start: u64, end: u64, label: &str) -> Result<(), OracleError> {
    if end < start {
        Err(OracleError::new(format!("invalid {label} tick range")))
    } else {
        Ok(())
    }
}

fn validate_evidence_text(value: &str, label: &str) -> Result<(), OracleError> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        Err(OracleError::new(format!("invalid {label}")))
    } else {
        Ok(())
    }
}

fn validate_target(target: &OracleTarget) -> Result<(), OracleError> {
    let validate_stage = |stage: &str| {
        stage_is_valid(stage)
            .then_some(())
            .ok_or_else(|| OracleError::new("invalid oracle stage"))
    };
    match target {
        OracleTarget::Stage { stage } | OracleTarget::Room { stage, .. } => validate_stage(stage)?,
        OracleTarget::Region {
            stage, min, max, ..
        } => {
            if let Some(stage) = stage {
                validate_stage(stage)?;
            }
            if !(0..3).all(|axis| {
                min[axis].is_finite() && max[axis].is_finite() && min[axis] <= max[axis]
            }) {
                return Err(OracleError::new("invalid oracle region"));
            }
        }
        OracleTarget::Action {
            mode_all,
            mode_none,
            ..
        } if mode_all & mode_none != 0 => {
            return Err(OracleError::new("action mode masks overlap"));
        }
        OracleTarget::Animation {
            lane,
            frame_min,
            frame_max,
            ..
        } => {
            if lane.is_some_and(|lane| lane > 2)
                || frame_min.is_some_and(|v| !v.is_finite())
                || frame_max.is_some_and(|v| !v.is_finite())
                || matches!((frame_min, frame_max), (Some(min), Some(max)) if min > max)
            {
                return Err(OracleError::new("invalid animation lane or frame range"));
            }
        }
        OracleTarget::Flag {
            domain,
            room,
            index,
            ..
        } => {
            let max = flag_index_max(*domain);
            if *index > max || (*domain == FlagDomain::Switch) != room.is_some() {
                return Err(OracleError::new("invalid indexed flag selector"));
            }
        }
        OracleTarget::ActorState {
            stage,
            set_id,
            status_all,
            status_none,
            ..
        } => {
            validate_stage(stage)?;
            if *set_id == u16::MAX || status_all & status_none != 0 {
                return Err(OracleError::new("invalid actor-state selector"));
            }
        }
        OracleTarget::Event {
            id,
            name_hash,
            mode,
            status,
        } if id.is_none() && name_hash.is_none() && mode.is_none() && status.is_none() => {
            return Err(OracleError::new("event oracle has no selector"));
        }
        OracleTarget::CollisionCrossing {
            point,
            normal,
            tolerance,
            ..
        } => {
            if point.iter().chain(normal).any(|value| !value.is_finite())
                || vector_length(*normal) <= f32::EPSILON
                || !tolerance.is_finite()
                || *tolerance < 0.0
            {
                return Err(OracleError::new("invalid collision-crossing plane"));
            }
        }
        OracleTarget::OutOfBounds {
            allowed_min,
            allowed_max,
        } => validate_bounds(*allowed_min, *allowed_max)?,
        OracleTarget::VoidSurvival {
            below_y,
            minimum_ticks,
        } if !below_y.is_finite() || *minimum_ticks == 0 || *minimum_ticks > 100_000 => {
            return Err(OracleError::new("invalid void-survival bounds"));
        }
        OracleTarget::UnexpectedLoad {
            allowed_destinations,
        } => {
            if allowed_destinations.len() > 32 {
                return Err(OracleError::new("too many allowed load destinations"));
            }
            for destination in allowed_destinations {
                validate_location(destination)?;
            }
        }
        OracleTarget::WrongWarp { expected } => validate_location(expected)?,
        OracleTarget::ExcessiveMotion {
            max_displacement,
            max_speed,
        } => {
            if max_displacement.is_none() && max_speed.is_none()
                || [*max_displacement, *max_speed]
                    .into_iter()
                    .flatten()
                    .any(|value| !value.is_finite() || value <= 0.0)
            {
                return Err(OracleError::new("invalid excessive-motion threshold"));
            }
        }
        OracleTarget::ImpossibleCoordinates { max_abs }
            if !max_abs.is_finite() || *max_abs <= 0.0 =>
        {
            return Err(OracleError::new("invalid impossible-coordinate bound"));
        }
        OracleTarget::ActorCorruption { field, .. }
        | OracleTarget::WatchedFieldCorruption { field } => {
            if let Some(field) = field {
                validate_evidence_text(field, "oracle field selector")?;
            }
        }
        OracleTarget::HeapFailure { heap: Some(heap) } => {
            validate_evidence_text(heap, "oracle heap selector")?;
        }
        OracleTarget::Hang {
            minimum_stalled_millis,
        } if *minimum_stalled_millis == 0 || *minimum_stalled_millis > 86_400_000 => {
            return Err(OracleError::new("invalid hang threshold"));
        }
        OracleTarget::Softlock { minimum_ticks } | OracleTarget::ControlLoss { minimum_ticks }
            if *minimum_ticks == 0 || *minimum_ticks > 10_000_000 =>
        {
            return Err(OracleError::new("invalid run anomaly tick threshold"));
        }
        OracleTarget::PreservedStorageState { field: Some(field) } => {
            validate_evidence_text(field, "oracle state-field selector")?;
        }
        OracleTarget::SaveStateAnomaly { slot, field } => {
            if slot.is_some_and(|slot| slot > 2) {
                return Err(OracleError::new("invalid oracle save slot"));
            }
            if let Some(field) = field {
                validate_evidence_text(field, "oracle state-field selector")?;
            }
        }
        OracleTarget::EventQueueing { minimum_depth, .. }
            if *minimum_depth == 0 || *minimum_depth > 256 =>
        {
            return Err(OracleError::new("invalid event-queue depth"));
        }
        OracleTarget::SequenceBreak {
            sequence: Some(sequence),
        } => {
            validate_evidence_text(sequence, "oracle sequence selector")?;
        }
        _ => {}
    }
    Ok(())
}

fn validate_bounds(min: [f32; 3], max: [f32; 3]) -> Result<(), OracleError> {
    if (0..3).all(|axis| min[axis].is_finite() && max[axis].is_finite() && min[axis] <= max[axis]) {
        Ok(())
    } else {
        Err(OracleError::new("invalid oracle coordinate bounds"))
    }
}

fn validate_location(location: &LocationTarget) -> Result<(), OracleError> {
    if stage_is_valid(&location.stage) {
        Ok(())
    } else {
        Err(OracleError::new("invalid oracle location"))
    }
}

fn flag_index_max(domain: FlagDomain) -> u16 {
    match domain {
        FlagDomain::Event => 821,
        FlagDomain::Temporary => 184,
        FlagDomain::Dungeon => 63,
        FlagDomain::Switch => 239,
    }
}

fn stage_is_valid(stage: &str) -> bool {
    !stage.is_empty()
        && stage.len() <= 16
        && stage
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && byte != b',')
}

#[derive(Debug)]
pub struct OracleError(String);
impl OracleError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}
impl fmt::Display for OracleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl Error for OracleError {}

#[cfg(test)]
#[path = "semantic_oracle/tests.rs"]
mod tests;
