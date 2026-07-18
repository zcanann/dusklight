//! Reached/avoided semantic oracles over immutable gameplay observations.

use crate::trace::{
    DecodedTrace, TraceAnimationLane, TraceChannel, TraceChannelStatus, TraceRecord,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fmt;

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

fn evaluate_one(
    oracle: &SemanticOracle,
    trace: &DecodedTrace,
    supplemental: &SupplementalObservations,
    snapshots: &BTreeMap<u64, &SupplementalSnapshot>,
    trace_complete: bool,
) -> SemanticOracleResult {
    let first_match = first_target_match(
        &oracle.target,
        trace,
        snapshots,
        supplemental.run_outcome.as_ref(),
    );
    let coverage = target_coverage(
        &oracle.target,
        trace,
        supplemental,
        snapshots,
        trace_complete,
    );
    let (disposition, reason) = match (oracle.polarity, first_match.is_some(), coverage) {
        (OraclePolarity::Reached, true, _) => (OracleDisposition::Satisfied, "target was observed"),
        (OraclePolarity::Reached, false, true) => (
            OracleDisposition::Violated,
            "complete evidence never matched the target",
        ),
        (OraclePolarity::Reached, false, false) => (
            OracleDisposition::Indeterminate,
            "evidence is incomplete or unavailable",
        ),
        (OraclePolarity::Avoided, true, _) => {
            (OracleDisposition::Violated, "forbidden target was observed")
        }
        (OraclePolarity::Avoided, false, true) => (
            OracleDisposition::Satisfied,
            "complete evidence proves the target was avoided",
        ),
        (OraclePolarity::Avoided, false, false) => (
            OracleDisposition::Indeterminate,
            "avoidance requires complete evidence",
        ),
    };
    SemanticOracleResult {
        name: oracle.name.clone(),
        polarity: oracle.polarity,
        disposition,
        inspected_observations: if is_run_outcome_target(&oracle.target) {
            supplemental.run_outcome.as_ref().map_or(0, |outcome| {
                outcome.anomalies.len() + usize::from(outcome.termination.is_some())
            })
        } else {
            trace.records.len()
        },
        first_match,
        reason: reason.into(),
    }
}

fn first_target_match(
    target: &OracleTarget,
    trace: &DecodedTrace,
    snapshots: &BTreeMap<u64, &SupplementalSnapshot>,
    run_outcome: Option<&RunOutcomeEvidence>,
) -> Option<OracleMatch> {
    if is_run_outcome_target(target) {
        return run_outcome.and_then(|outcome| match_run_outcome(target, outcome));
    }
    match target {
        OracleTarget::CollisionCrossing { .. }
        | OracleTarget::WrongWarp { .. }
        | OracleTarget::ExcessiveMotion { .. } => trace
            .records
            .windows(2)
            .find_map(|pair| match_record_pair(target, &pair[0], &pair[1])),
        OracleTarget::VoidSurvival {
            below_y,
            minimum_ticks,
        } => match_void_survival(trace, *below_y, *minimum_ticks),
        _ => trace.records.iter().find_map(|record| {
            match_target(
                target,
                record,
                snapshots.get(&record.simulation_tick).copied(),
            )
        }),
    }
}

fn target_coverage(
    target: &OracleTarget,
    trace: &DecodedTrace,
    supplemental: &SupplementalObservations,
    snapshots: &BTreeMap<u64, &SupplementalSnapshot>,
    trace_complete: bool,
) -> bool {
    if !trace_complete && !is_run_outcome_target(target) {
        return false;
    }
    match target {
        OracleTarget::Stage { .. } | OracleTarget::Room { .. } => {
            channel_known(trace, TraceChannel::Stage)
        }
        OracleTarget::Region { .. } => {
            channel_known(trace, TraceChannel::Stage)
                && channel_known(trace, TraceChannel::PlayerMotion)
        }
        OracleTarget::Action { .. } | OracleTarget::Animation { .. } => {
            channel_known(trace, TraceChannel::PlayerAction)
        }
        OracleTarget::Event { name_hash, .. } => {
            channel_known(trace, TraceChannel::Event)
                && (name_hash.is_none()
                    || trace
                        .records
                        .iter()
                        .all(|record| !record.event_running() || record.event_name_hash_present))
        }
        OracleTarget::Flag {
            domain,
            room,
            index,
            ..
        } => {
            supplemental.flags_complete
                && supplemental_ticks_complete(trace, snapshots)
                && snapshots.values().all(|snapshot| {
                    snapshot.flags.iter().any(|flag| {
                        flag.domain == *domain && flag.room == *room && flag.index == *index
                    })
                })
        }
        OracleTarget::ActorState { .. } => {
            supplemental.actors_complete && supplemental_ticks_complete(trace, snapshots)
        }
        OracleTarget::CollisionCrossing { .. } | OracleTarget::VoidSurvival { .. } => {
            channel_known(trace, TraceChannel::PlayerMotion)
                && channel_known(trace, TraceChannel::PlayerBackgroundCollision)
        }
        OracleTarget::OutOfBounds { .. }
        | OracleTarget::ExcessiveMotion { .. }
        | OracleTarget::NonFiniteState
        | OracleTarget::ImpossibleCoordinates { .. } => {
            channel_known(trace, TraceChannel::PlayerMotion)
        }
        OracleTarget::UnexpectedLoad { .. } | OracleTarget::WrongWarp { .. } => {
            channel_known(trace, TraceChannel::Stage)
        }
        OracleTarget::ActorCorruption { .. } => run_domain_covered(
            supplemental.run_outcome.as_ref(),
            RunEvidenceKind::ActorIntegrity,
        ),
        OracleTarget::SlotExhaustion => run_domain_covered(
            supplemental.run_outcome.as_ref(),
            RunEvidenceKind::ActorSlots,
        ),
        OracleTarget::WatchedFieldCorruption { .. } => run_domain_covered(
            supplemental.run_outcome.as_ref(),
            RunEvidenceKind::WatchedFields,
        ),
        OracleTarget::HeapFailure { .. } => {
            run_domain_covered(supplemental.run_outcome.as_ref(), RunEvidenceKind::Heap)
        }
        OracleTarget::Crash => supplemental
            .run_outcome
            .as_ref()
            .is_some_and(|outcome| outcome.termination.is_some()),
        OracleTarget::Hang { .. } => {
            supplemental
                .run_outcome
                .as_ref()
                .is_some_and(|outcome| match outcome.termination {
                    Some(RunTermination::Completed { .. } | RunTermination::Crashed { .. }) => true,
                    Some(RunTermination::TimedOut { .. }) => {
                        outcome.monitored.contains(&RunEvidenceKind::Progress)
                    }
                    None => false,
                })
        }
        OracleTarget::Softlock { .. } => {
            run_domain_covered(supplemental.run_outcome.as_ref(), RunEvidenceKind::Progress)
        }
        OracleTarget::ControlLoss { .. } => {
            run_domain_covered(supplemental.run_outcome.as_ref(), RunEvidenceKind::Control)
        }
        OracleTarget::DuplicateItemReward { .. } => run_domain_covered(
            supplemental.run_outcome.as_ref(),
            RunEvidenceKind::InventoryRewards,
        ),
        OracleTarget::PreservedStorageState { .. } => {
            run_domain_covered(supplemental.run_outcome.as_ref(), RunEvidenceKind::Storage)
        }
        OracleTarget::EventQueueing { .. } => run_domain_covered(
            supplemental.run_outcome.as_ref(),
            RunEvidenceKind::EventQueue,
        ),
        OracleTarget::SequenceBreak { .. } => {
            run_domain_covered(supplemental.run_outcome.as_ref(), RunEvidenceKind::Sequence)
        }
        OracleTarget::SaveStateAnomaly { .. } => run_domain_covered(
            supplemental.run_outcome.as_ref(),
            RunEvidenceKind::SaveState,
        ),
    }
}

fn is_run_outcome_target(target: &OracleTarget) -> bool {
    matches!(
        target,
        OracleTarget::ActorCorruption { .. }
            | OracleTarget::SlotExhaustion
            | OracleTarget::WatchedFieldCorruption { .. }
            | OracleTarget::HeapFailure { .. }
            | OracleTarget::Crash
            | OracleTarget::Hang { .. }
            | OracleTarget::Softlock { .. }
            | OracleTarget::ControlLoss { .. }
            | OracleTarget::DuplicateItemReward { .. }
            | OracleTarget::PreservedStorageState { .. }
            | OracleTarget::EventQueueing { .. }
            | OracleTarget::SequenceBreak { .. }
            | OracleTarget::SaveStateAnomaly { .. }
    )
}

fn run_domain_covered(outcome: Option<&RunOutcomeEvidence>, kind: RunEvidenceKind) -> bool {
    outcome.is_some_and(|outcome| outcome.monitored.contains(&kind))
}

fn match_run_outcome(target: &OracleTarget, outcome: &RunOutcomeEvidence) -> Option<OracleMatch> {
    match (target, &outcome.termination) {
        (
            OracleTarget::Crash,
            Some(RunTermination::Crashed {
                exit_code,
                signal,
                reason,
            }),
        ) => {
            return Some(OracleMatch {
                simulation_tick: outcome_last_tick(outcome),
                tape_frame: None,
                facts: OracleFacts::Crash {
                    exit_code: *exit_code,
                    signal: *signal,
                    reason: reason.clone(),
                },
            });
        }
        (
            OracleTarget::Hang {
                minimum_stalled_millis,
            },
            Some(RunTermination::TimedOut {
                wall_time_millis,
                stalled_millis,
                last_simulation_tick,
            }),
        ) if stalled_millis >= minimum_stalled_millis => {
            return Some(OracleMatch {
                simulation_tick: *last_simulation_tick,
                tape_frame: None,
                facts: OracleFacts::Hang {
                    wall_time_millis: *wall_time_millis,
                    stalled_millis: *stalled_millis,
                    last_simulation_tick: *last_simulation_tick,
                },
            });
        }
        _ => {}
    }

    outcome
        .anomalies
        .iter()
        .find_map(|observation| match_run_anomaly(target, observation))
}

fn match_run_anomaly(
    target: &OracleTarget,
    observation: &RunAnomalyObservation,
) -> Option<OracleMatch> {
    let (simulation_tick, tape_frame, facts) = match (target, observation) {
        (
            OracleTarget::ActorCorruption { actor_name, field },
            RunAnomalyObservation::ActorCorruption {
                simulation_tick,
                tape_frame,
                actor,
                field: observed_field,
                expected,
                actual,
            },
        ) if actor_name.is_none_or(|name| name == actor.actor_name)
            && field.as_ref().is_none_or(|field| field == observed_field) =>
        {
            (
                *simulation_tick,
                *tape_frame,
                OracleFacts::ActorCorruption {
                    actor: actor.clone(),
                    field: observed_field.clone(),
                    expected: expected.clone(),
                    actual: actual.clone(),
                },
            )
        }
        (
            OracleTarget::SlotExhaustion,
            RunAnomalyObservation::SlotExhaustion {
                simulation_tick,
                tape_frame,
                active_slots,
                capacity,
                requested_actor_name,
            },
        ) => (
            *simulation_tick,
            *tape_frame,
            OracleFacts::SlotExhaustion {
                active_slots: *active_slots,
                capacity: *capacity,
                requested_actor_name: *requested_actor_name,
            },
        ),
        (
            OracleTarget::WatchedFieldCorruption { field },
            RunAnomalyObservation::WatchedFieldCorruption {
                simulation_tick,
                tape_frame,
                field: observed_field,
                expected,
                actual,
            },
        ) if field.as_ref().is_none_or(|field| field == observed_field) => (
            *simulation_tick,
            *tape_frame,
            OracleFacts::WatchedFieldCorruption {
                field: observed_field.clone(),
                expected: expected.clone(),
                actual: actual.clone(),
            },
        ),
        (
            OracleTarget::HeapFailure { heap },
            RunAnomalyObservation::HeapFailure {
                simulation_tick,
                tape_frame,
                heap: observed_heap,
                operation,
                requested_bytes,
                free_bytes,
            },
        ) if heap.as_ref().is_none_or(|heap| heap == observed_heap) => (
            simulation_tick.unwrap_or(0),
            *tape_frame,
            OracleFacts::HeapFailure {
                heap: observed_heap.clone(),
                operation: operation.clone(),
                requested_bytes: *requested_bytes,
                free_bytes: *free_bytes,
            },
        ),
        (
            OracleTarget::Softlock { minimum_ticks },
            RunAnomalyObservation::Softlock {
                start_tick,
                end_tick,
                tape_frame,
                last_progress,
                reason,
            },
        ) if tick_span(*start_tick, *end_tick) >= *minimum_ticks => (
            *end_tick,
            *tape_frame,
            OracleFacts::Softlock {
                start_tick: *start_tick,
                end_tick: *end_tick,
                ticks_without_progress: tick_span(*start_tick, *end_tick),
                last_progress: last_progress.clone(),
                reason: reason.clone(),
            },
        ),
        (
            OracleTarget::ControlLoss { minimum_ticks },
            RunAnomalyObservation::ControlLoss {
                start_tick,
                end_tick,
                tape_frame,
                procedure_id,
                reason,
            },
        ) if tick_span(*start_tick, *end_tick) >= *minimum_ticks => (
            *end_tick,
            *tape_frame,
            OracleFacts::ControlLoss {
                start_tick: *start_tick,
                end_tick: *end_tick,
                ticks_without_control: tick_span(*start_tick, *end_tick),
                procedure_id: *procedure_id,
                reason: reason.clone(),
            },
        ),
        (
            OracleTarget::DuplicateItemReward { grant_kind, id },
            RunAnomalyObservation::DuplicateItemReward {
                simulation_tick,
                tape_frame,
                grant_kind: observed_kind,
                id: observed_id,
                first_source,
                duplicate_source,
                total_grants,
            },
        ) if grant_kind.is_none_or(|kind| kind == *observed_kind)
            && id.is_none_or(|id| id == *observed_id) =>
        {
            (
                *simulation_tick,
                *tape_frame,
                OracleFacts::DuplicateItemReward {
                    grant_kind: *observed_kind,
                    id: *observed_id,
                    first_source: first_source.clone(),
                    duplicate_source: duplicate_source.clone(),
                    total_grants: *total_grants,
                },
            )
        }
        (
            OracleTarget::PreservedStorageState { field },
            RunAnomalyObservation::PreservedStorageState {
                simulation_tick,
                tape_frame,
                field: observed_field,
                expected_reset,
                actual,
            },
        ) if field.as_ref().is_none_or(|field| field == observed_field) => (
            *simulation_tick,
            *tape_frame,
            OracleFacts::PreservedStorageState {
                field: observed_field.clone(),
                expected_reset: expected_reset.clone(),
                actual: actual.clone(),
            },
        ),
        (
            OracleTarget::EventQueueing {
                event_id,
                minimum_depth,
            },
            RunAnomalyObservation::EventQueueing {
                simulation_tick,
                tape_frame,
                running_event_id,
                queued_event_ids,
            },
        ) if queued_event_ids.len() >= *minimum_depth as usize
            && event_id.is_none_or(|id| {
                *running_event_id == Some(id) || queued_event_ids.contains(&id)
            }) =>
        {
            (
                *simulation_tick,
                *tape_frame,
                OracleFacts::EventQueueing {
                    running_event_id: *running_event_id,
                    queued_event_ids: queued_event_ids.clone(),
                },
            )
        }
        (
            OracleTarget::SequenceBreak { sequence },
            RunAnomalyObservation::SequenceBreak {
                simulation_tick,
                tape_frame,
                sequence: observed_sequence,
                expected_step,
                actual_step,
            },
        ) if sequence
            .as_ref()
            .is_none_or(|sequence| sequence == observed_sequence) =>
        {
            (
                *simulation_tick,
                *tape_frame,
                OracleFacts::SequenceBreak {
                    sequence: observed_sequence.clone(),
                    expected_step: expected_step.clone(),
                    actual_step: actual_step.clone(),
                },
            )
        }
        (
            OracleTarget::SaveStateAnomaly { slot, field },
            RunAnomalyObservation::SaveStateAnomaly {
                simulation_tick,
                tape_frame,
                slot: observed_slot,
                field: observed_field,
                expected,
                actual,
            },
        ) if slot.is_none_or(|slot| slot == *observed_slot)
            && field.as_ref().is_none_or(|field| field == observed_field) =>
        {
            (
                simulation_tick.unwrap_or(0),
                *tape_frame,
                OracleFacts::SaveStateAnomaly {
                    slot: *observed_slot,
                    field: observed_field.clone(),
                    expected: expected.clone(),
                    actual: actual.clone(),
                },
            )
        }
        _ => return None,
    };
    Some(OracleMatch {
        simulation_tick,
        tape_frame,
        facts,
    })
}

fn outcome_last_tick(outcome: &RunOutcomeEvidence) -> u64 {
    match outcome.termination {
        Some(RunTermination::TimedOut {
            last_simulation_tick,
            ..
        }) => last_simulation_tick,
        _ => outcome
            .anomalies
            .iter()
            .map(anomaly_tick)
            .max()
            .unwrap_or(0),
    }
}

fn anomaly_tick(observation: &RunAnomalyObservation) -> u64 {
    match observation {
        RunAnomalyObservation::ActorCorruption {
            simulation_tick, ..
        }
        | RunAnomalyObservation::SlotExhaustion {
            simulation_tick, ..
        }
        | RunAnomalyObservation::WatchedFieldCorruption {
            simulation_tick, ..
        }
        | RunAnomalyObservation::DuplicateItemReward {
            simulation_tick, ..
        }
        | RunAnomalyObservation::PreservedStorageState {
            simulation_tick, ..
        }
        | RunAnomalyObservation::EventQueueing {
            simulation_tick, ..
        }
        | RunAnomalyObservation::SequenceBreak {
            simulation_tick, ..
        } => *simulation_tick,
        RunAnomalyObservation::HeapFailure {
            simulation_tick, ..
        }
        | RunAnomalyObservation::SaveStateAnomaly {
            simulation_tick, ..
        } => simulation_tick.unwrap_or(0),
        RunAnomalyObservation::Softlock { end_tick, .. }
        | RunAnomalyObservation::ControlLoss { end_tick, .. } => *end_tick,
    }
}

fn tick_span(start_tick: u64, end_tick: u64) -> u64 {
    end_tick.saturating_sub(start_tick).saturating_add(1)
}

fn channel_known(trace: &DecodedTrace, channel: TraceChannel) -> bool {
    trace.records.iter().all(|record| {
        matches!(
            record.channel_status.get(&channel),
            Some(TraceChannelStatus::Present | TraceChannelStatus::Absent)
        )
    })
}

fn supplemental_ticks_complete(
    trace: &DecodedTrace,
    snapshots: &BTreeMap<u64, &SupplementalSnapshot>,
) -> bool {
    trace
        .records
        .iter()
        .all(|record| snapshots.contains_key(&record.simulation_tick))
}

fn match_target(
    target: &OracleTarget,
    record: &TraceRecord,
    supplemental: Option<&SupplementalSnapshot>,
) -> Option<OracleMatch> {
    let facts = match target {
        OracleTarget::Stage { stage }
            if channel_present(record, TraceChannel::Stage) && &record.stage_name == stage =>
        {
            OracleFacts::Stage {
                stage: record.stage_name.clone(),
            }
        }
        OracleTarget::Room { stage, room }
            if channel_present(record, TraceChannel::Stage)
                && &record.stage_name == stage
                && &record.room == room =>
        {
            OracleFacts::Room {
                stage: record.stage_name.clone(),
                room: record.room,
            }
        }
        OracleTarget::Region {
            stage,
            room,
            min,
            max,
        } if stage
            .as_ref()
            .is_none_or(|stage| stage == &record.stage_name)
            && room.is_none_or(|room| room == record.room)
            && channel_present(record, TraceChannel::Stage)
            && channel_present(record, TraceChannel::PlayerMotion)
            && (0..3).all(|axis| {
                record.position[axis] >= min[axis] && record.position[axis] <= max[axis]
            }) =>
        {
            OracleFacts::Region {
                stage: record.stage_name.clone(),
                room: record.room,
                position: record.position,
            }
        }
        OracleTarget::Action {
            procedure_id,
            mode_all,
            mode_none,
        } => {
            if !channel_present(record, TraceChannel::PlayerAction) {
                return None;
            }
            let action = record.player_action.as_ref()?;
            (action.procedure_id == *procedure_id
                && action.mode_flags & mode_all == *mode_all
                && action.mode_flags & mode_none == 0)
                .then(|| OracleFacts::Action {
                    procedure_id: action.procedure_id,
                    mode_flags: action.mode_flags,
                })?
        }
        OracleTarget::Animation {
            bank,
            lane,
            resource_id,
            frame_min,
            frame_max,
        } => {
            if !channel_present(record, TraceChannel::PlayerAction) {
                return None;
            }
            let action = record.player_action.as_ref()?;
            let lanes = match bank {
                AnimationBank::Under => &action.under_animations,
                AnimationBank::Upper => &action.upper_animations,
            };
            let (index, animation) = lanes.iter().enumerate().find(|(index, animation)| {
                lane.is_none_or(|lane| usize::from(lane) == *index)
                    && animation.resource_id == *resource_id
                    && frame_min.is_none_or(|min| animation.frame >= min)
                    && frame_max.is_none_or(|max| animation.frame <= max)
            })?;
            animation_facts(*bank, index, animation)
        }
        OracleTarget::Event {
            id,
            name_hash,
            mode,
            status,
        } if id.is_none_or(|id| id == record.event_id)
            && channel_present(record, TraceChannel::Event)
            && name_hash.is_none_or(|hash| {
                record.event_name_hash_present && hash == record.event_name_hash
            })
            && mode.is_none_or(|mode| mode == record.event_mode)
            && status.is_none_or(|status| status == record.event_status) =>
        {
            OracleFacts::Event {
                id: record.event_id,
                name_hash: record
                    .event_name_hash_present
                    .then_some(record.event_name_hash),
                mode: record.event_mode,
                status: record.event_status,
            }
        }
        OracleTarget::Flag {
            domain,
            room,
            index,
            value,
        } => {
            let flag = supplemental?.flags.iter().find(|flag| {
                flag.domain == *domain
                    && flag.room == *room
                    && flag.index == *index
                    && flag.value == *value
            })?;
            OracleFacts::Flag {
                domain: flag.domain,
                room: flag.room,
                index: flag.index,
                value: flag.value,
            }
        }
        OracleTarget::ActorState {
            stage,
            home_room,
            set_id,
            actor_name,
            current_room,
            health,
            status_all,
            status_none,
        } => {
            let actor = supplemental?.actors.iter().find(|actor| {
                &actor.stage == stage
                    && actor.home_room == *home_room
                    && actor.set_id == *set_id
                    && actor.actor_name == *actor_name
                    && current_room.is_none_or(|room| room == actor.current_room)
                    && health.is_none_or(|health| health == actor.health)
                    && actor.status & status_all == *status_all
                    && actor.status & status_none == 0
            })?;
            OracleFacts::ActorState {
                stage: actor.stage.clone(),
                home_room: actor.home_room,
                set_id: actor.set_id,
                actor_name: actor.actor_name,
                current_room: actor.current_room,
                health: actor.health,
                status: actor.status,
            }
        }
        OracleTarget::OutOfBounds {
            allowed_min,
            allowed_max,
        } if channel_present(record, TraceChannel::PlayerMotion)
            && (0..3).any(|axis| {
                record.position[axis] < allowed_min[axis]
                    || record.position[axis] > allowed_max[axis]
            }) =>
        {
            OracleFacts::OutOfBounds {
                position: record.position,
            }
        }
        OracleTarget::UnexpectedLoad {
            allowed_destinations,
        } if channel_present(record, TraceChannel::Stage) && record.next_stage_enabled => {
            let destination = pending_location(record);
            if allowed_destinations
                .iter()
                .any(|allowed| location_matches(allowed, &destination))
            {
                return None;
            }
            OracleFacts::UnexpectedLoad { destination }
        }
        OracleTarget::NonFiniteState
            if channel_present(record, TraceChannel::PlayerMotion)
                && player_nonfinite_field(record).is_some() =>
        {
            OracleFacts::NonFiniteState {
                field: player_nonfinite_field(record).expect("guard checked field"),
            }
        }
        OracleTarget::ImpossibleCoordinates { max_abs }
            if channel_present(record, TraceChannel::PlayerMotion)
                && record
                    .position
                    .iter()
                    .any(|coordinate| coordinate.abs() > *max_abs) =>
        {
            OracleFacts::ImpossibleCoordinates {
                position: record.position,
                max_abs: *max_abs,
            }
        }
        _ => return None,
    };
    Some(OracleMatch {
        simulation_tick: record.simulation_tick,
        tape_frame: record.tape_frame,
        facts,
    })
}

fn pending_location(record: &TraceRecord) -> LocationTarget {
    LocationTarget {
        stage: record.next_stage_name.clone(),
        room: record.next_room,
        layer: Some(record.next_layer),
        point: Some(record.next_point),
    }
}

fn current_location(record: &TraceRecord) -> LocationTarget {
    LocationTarget {
        stage: record.stage_name.clone(),
        room: record.room,
        layer: Some(record.layer),
        point: Some(record.point),
    }
}

fn location_matches(expected: &LocationTarget, actual: &LocationTarget) -> bool {
    expected.stage == actual.stage
        && expected.room == actual.room
        && expected
            .layer
            .is_none_or(|layer| Some(layer) == actual.layer)
        && expected
            .point
            .is_none_or(|point| Some(point) == actual.point)
}

fn player_nonfinite_field(record: &TraceRecord) -> Option<String> {
    for (name, values) in [
        ("player.position", record.position.as_slice()),
        ("player.velocity", record.velocity.as_slice()),
        (
            "player.forward_speed",
            std::slice::from_ref(&record.forward_speed),
        ),
    ] {
        if values.iter().any(|value| !value.is_finite()) {
            return Some(name.into());
        }
    }
    None
}

fn match_record_pair(
    target: &OracleTarget,
    previous: &TraceRecord,
    record: &TraceRecord,
) -> Option<OracleMatch> {
    let facts = match target {
        OracleTarget::CollisionCrossing {
            point,
            normal,
            tolerance,
            contact_mask,
        } if channel_present(previous, TraceChannel::PlayerMotion)
            && channel_present(record, TraceChannel::PlayerMotion)
            && channel_present(record, TraceChannel::PlayerBackgroundCollision) =>
        {
            let length = vector_length(*normal);
            let signed = |position: [f32; 3]| {
                ((position[0] - point[0]) * normal[0]
                    + (position[1] - point[1]) * normal[1]
                    + (position[2] - point[2]) * normal[2])
                    / length
            };
            let before = signed(previous.position);
            let after = signed(record.position);
            let crossed = (before < -*tolerance && after > *tolerance)
                || (before > *tolerance && after < -*tolerance);
            let collision_flags = record.player_background_collision.as_ref()?.flags;
            if !crossed || collision_flags & contact_mask != 0 {
                return None;
            }
            OracleFacts::CollisionCrossing {
                previous_position: previous.position,
                position: record.position,
                previous_signed_distance: before,
                signed_distance: after,
                collision_flags,
            }
        }
        OracleTarget::WrongWarp { expected }
            if channel_present(previous, TraceChannel::Stage)
                && channel_present(record, TraceChannel::Stage) =>
        {
            let before = current_location(previous);
            let destination = current_location(record);
            if before == destination || location_matches(expected, &destination) {
                return None;
            }
            OracleFacts::WrongWarp {
                destination,
                expected: expected.clone(),
            }
        }
        OracleTarget::ExcessiveMotion {
            max_displacement,
            max_speed,
        } if channel_present(previous, TraceChannel::PlayerMotion)
            && channel_present(record, TraceChannel::PlayerMotion) =>
        {
            let displacement = vector_length([
                record.position[0] - previous.position[0],
                record.position[1] - previous.position[1],
                record.position[2] - previous.position[2],
            ]);
            let speed = vector_length(record.velocity).max(record.forward_speed.abs());
            if !max_displacement.is_some_and(|limit| displacement > limit)
                && !max_speed.is_some_and(|limit| speed > limit)
            {
                return None;
            }
            OracleFacts::ExcessiveMotion {
                previous_position: previous.position,
                position: record.position,
                displacement,
                speed,
            }
        }
        _ => return None,
    };
    Some(OracleMatch {
        simulation_tick: record.simulation_tick,
        tape_frame: record.tape_frame,
        facts,
    })
}

fn match_void_survival(
    trace: &DecodedTrace,
    below_y: f32,
    minimum_ticks: u32,
) -> Option<OracleMatch> {
    const GROUND_CONTACT: u32 = 1 << 1;
    let mut consecutive = 0_u32;
    let mut previous_tick = None;
    for record in &trace.records {
        let collision = record.player_background_collision.as_ref();
        let eligible = channel_present(record, TraceChannel::PlayerMotion)
            && channel_present(record, TraceChannel::PlayerBackgroundCollision)
            && record.position[1] < below_y
            && collision.is_some_and(|collision| collision.flags & GROUND_CONTACT == 0)
            && previous_tick.is_none_or(|tick| record.simulation_tick == tick + 1);
        consecutive = if eligible { consecutive + 1 } else { 0 };
        previous_tick = Some(record.simulation_tick);
        if consecutive >= minimum_ticks {
            return Some(OracleMatch {
                simulation_tick: record.simulation_tick,
                tape_frame: record.tape_frame,
                facts: OracleFacts::VoidSurvival {
                    position: record.position,
                    ticks_without_ground: consecutive,
                },
            });
        }
    }
    None
}

fn vector_length(vector: [f32; 3]) -> f32 {
    (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt()
}

fn channel_present(record: &TraceRecord, channel: TraceChannel) -> bool {
    record.channel_status.get(&channel) == Some(&TraceChannelStatus::Present)
}

fn animation_facts(
    bank: AnimationBank,
    lane: usize,
    animation: &TraceAnimationLane,
) -> OracleFacts {
    OracleFacts::Animation {
        bank,
        lane: lane as u8,
        resource_id: animation.resource_id,
        frame: animation.frame,
        rate: animation.rate,
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
        OracleTarget::HeapFailure { heap } => {
            if let Some(heap) = heap {
                validate_evidence_text(heap, "oracle heap selector")?;
            }
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
        OracleTarget::PreservedStorageState { field } => {
            if let Some(field) = field {
                validate_evidence_text(field, "oracle state-field selector")?;
            }
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
        OracleTarget::SequenceBreak { sequence } => {
            if let Some(sequence) = sequence {
                validate_evidence_text(sequence, "oracle sequence selector")?;
            }
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
mod tests {
    use super::*;
    use crate::tape::TapeBoot;
    use crate::trace::{
        TraceAnimationLane, TraceCollisionWall, TracePhase, TracePlayerAction,
        TracePlayerBackgroundCollision,
    };

    fn collision(flags: u32) -> TracePlayerBackgroundCollision {
        TracePlayerBackgroundCollision {
            flags,
            ground_height: 0.0,
            roof_height: 0.0,
            water_height: 0.0,
            ground_bg_index: None,
            ground_poly_index: None,
            ground_owner_session_process_id: None,
            ground_plane: [0.0; 4],
            ground_identity_present: false,
            roof_bg_index: None,
            roof_poly_index: None,
            roof_owner_session_process_id: None,
            roof_identity_present: false,
            water_bg_index: None,
            water_poly_index: None,
            water_owner_session_process_id: None,
            water_identity_present: false,
            walls: std::array::from_fn(|_| TraceCollisionWall {
                identity_present: false,
                bg_index: None,
                poly_index: None,
                owner_session_process_id: None,
                angle_y: 0,
                flags: 0,
            }),
            old_position: [0.0; 3],
            resolved_frame_displacement: [0.0; 3],
            final_position: [0.0; 3],
        }
    }

    fn trace(exhausted: bool) -> DecodedTrace {
        let mut records = Vec::new();
        for tick in 1..=2 {
            let mut record = TraceRecord {
                simulation_tick: tick,
                boundary_index: tick + 1,
                tape_frame: Some(tick - 1),
                observation_phase: TracePhase::PostSimulation,
                stage_name: "F_SP103".into(),
                room: 1,
                position: [tick as f32, 0.0, 0.0],
                event_id: 9,
                ..TraceRecord::default()
            };
            for channel in [
                TraceChannel::Stage,
                TraceChannel::PlayerMotion,
                TraceChannel::PlayerAction,
                TraceChannel::Event,
            ] {
                record
                    .channel_status
                    .insert(channel, TraceChannelStatus::Present);
            }
            record.player_action = Some(TracePlayerAction {
                procedure_id: 7,
                mode_flags: 4,
                procedure_context_raw: [0; 6],
                damage_wait_timer: 0,
                sword_at_up_time: 0,
                ice_damage_wait_timer: 0,
                sword_change_wait_timer: 0,
                under_animations: std::array::from_fn(|_| TraceAnimationLane {
                    resource_id: if tick == 2 { 42 } else { 1 },
                    frame: 3.0,
                    rate: 1.0,
                }),
                upper_animations: std::array::from_fn(|_| TraceAnimationLane {
                    resource_id: 2,
                    frame: 0.0,
                    rate: 1.0,
                }),
                do_status: 0,
                talk_partner: None,
                grabbed_actor: None,
            });
            records.push(record);
        }
        DecodedTrace {
            version: 2,
            boot: TapeBoot::Process,
            tick_rate_numerator: 30,
            tick_rate_denominator: 1,
            requested_channels: 0,
            capacity_exhausted: exhausted,
            retention: None,
            channel_formats: BTreeMap::new(),
            records,
        }
    }

    #[test]
    fn reached_and_avoided_cover_trace_and_supplemental_domains() {
        let targets = vec![
            (
                "stage",
                OraclePolarity::Reached,
                OracleTarget::Stage {
                    stage: "F_SP103".into(),
                },
            ),
            (
                "region",
                OraclePolarity::Reached,
                OracleTarget::Region {
                    stage: Some("F_SP103".into()),
                    room: Some(1),
                    min: [2.0, 0.0, 0.0],
                    max: [2.0, 0.0, 0.0],
                },
            ),
            (
                "action",
                OraclePolarity::Reached,
                OracleTarget::Action {
                    procedure_id: 7,
                    mode_all: 4,
                    mode_none: 2,
                },
            ),
            (
                "animation",
                OraclePolarity::Reached,
                OracleTarget::Animation {
                    bank: AnimationBank::Under,
                    lane: None,
                    resource_id: 42,
                    frame_min: Some(3.0),
                    frame_max: Some(3.0),
                },
            ),
            (
                "event",
                OraclePolarity::Reached,
                OracleTarget::Event {
                    id: Some(9),
                    name_hash: None,
                    mode: None,
                    status: None,
                },
            ),
            (
                "flag",
                OraclePolarity::Reached,
                OracleTarget::Flag {
                    domain: FlagDomain::Event,
                    room: None,
                    index: 5,
                    value: true,
                },
            ),
            (
                "actor",
                OraclePolarity::Reached,
                OracleTarget::ActorState {
                    stage: "F_SP103".into(),
                    home_room: 1,
                    set_id: 2,
                    actor_name: 3,
                    current_room: Some(1),
                    health: Some(4),
                    status_all: 1,
                    status_none: 2,
                },
            ),
            (
                "avoided",
                OraclePolarity::Avoided,
                OracleTarget::Room {
                    stage: "F_SP103".into(),
                    room: 9,
                },
            ),
        ];
        let program = SemanticOracleProgram {
            schema: SEMANTIC_ORACLE_SCHEMA_V1.into(),
            oracles: targets
                .into_iter()
                .map(|(name, polarity, target)| SemanticOracle {
                    name: name.into(),
                    polarity,
                    target,
                })
                .collect(),
        };
        let snapshots = (1..=2)
            .map(|tick| SupplementalSnapshot {
                simulation_tick: tick,
                flags: vec![FlagObservation {
                    domain: FlagDomain::Event,
                    room: None,
                    index: 5,
                    value: true,
                }],
                actors: vec![ActorObservation {
                    stage: "F_SP103".into(),
                    home_room: 1,
                    set_id: 2,
                    actor_name: 3,
                    current_room: 1,
                    health: 4,
                    status: 1,
                }],
            })
            .collect();
        let report = program
            .evaluate(
                &trace(false),
                &SupplementalObservations {
                    snapshots,
                    flags_complete: true,
                    actors_complete: true,
                    run_outcome: None,
                },
            )
            .unwrap();
        assert!(
            report
                .results
                .iter()
                .all(|result| result.disposition == OracleDisposition::Satisfied)
        );
        assert_eq!(
            report.results[3]
                .first_match
                .as_ref()
                .unwrap()
                .simulation_tick,
            2
        );
    }

    #[test]
    fn avoidance_is_indeterminate_when_trace_or_supplemental_coverage_is_incomplete() {
        let program = SemanticOracleProgram {
            schema: SEMANTIC_ORACLE_SCHEMA_V1.into(),
            oracles: vec![
                SemanticOracle {
                    name: "avoid-room".into(),
                    polarity: OraclePolarity::Avoided,
                    target: OracleTarget::Room {
                        stage: "F_SP103".into(),
                        room: 9,
                    },
                },
                SemanticOracle {
                    name: "avoid-flag".into(),
                    polarity: OraclePolarity::Avoided,
                    target: OracleTarget::Flag {
                        domain: FlagDomain::Event,
                        room: None,
                        index: 5,
                        value: true,
                    },
                },
            ],
        };
        let report = program
            .evaluate(&trace(true), &SupplementalObservations::default())
            .unwrap();
        assert!(
            report
                .results
                .iter()
                .all(|result| result.disposition == OracleDisposition::Indeterminate)
        );
    }

    #[test]
    fn checked_in_semantic_oracle_catalog_is_valid() {
        let program: SemanticOracleProgram = serde_json::from_str(include_str!(
            "../../../tests/fixtures/automation/semantic_oracles.json"
        ))
        .unwrap();
        program.validate().unwrap();
    }

    #[test]
    fn checked_in_run_outcome_fixture_is_valid() {
        let outcome: RunOutcomeEvidence = serde_json::from_str(include_str!(
            "../../../tests/fixtures/automation/run_outcome.json"
        ))
        .unwrap();
        validate_run_outcome(&outcome).unwrap();
    }

    #[test]
    fn collision_load_motion_and_invalid_state_oracles_retain_exact_evidence() {
        let mut source = trace(false);
        source.records[0].position = [-2.0, 0.0, 0.0];
        source.records[1].position = [2.0, -20.0, 0.0];
        source.records[1].velocity = [100.0, 0.0, 0.0];
        source.records[1].next_stage_enabled = true;
        source.records[1].next_stage_name = "F_WRONG".into();
        source.records[1].next_room = 3;
        source.records.push(source.records[1].clone());
        source.records[2].simulation_tick = 3;
        source.records[2].boundary_index = 4;
        source.records[2].tape_frame = Some(2);
        source.records[2].position = [5000.0, -20.0, 0.0];
        source.records.push(source.records[2].clone());
        source.records[3].simulation_tick = 4;
        source.records[3].boundary_index = 5;
        source.records[3].tape_frame = Some(3);
        source.records[3].stage_name = "F_WRONG".into();
        source.records[3].room = 3;
        for record in &mut source.records {
            record.channel_status.insert(
                TraceChannel::PlayerBackgroundCollision,
                TraceChannelStatus::Present,
            );
            record.player_background_collision = Some(collision(0));
        }
        let location = |stage: &str, room| LocationTarget {
            stage: stage.into(),
            room,
            layer: None,
            point: None,
        };
        let targets = vec![
            OracleTarget::CollisionCrossing {
                point: [0.0; 3],
                normal: [1.0, 0.0, 0.0],
                tolerance: 0.1,
                contact_mask: 2,
            },
            OracleTarget::OutOfBounds {
                allowed_min: [-1000.0; 3],
                allowed_max: [1000.0; 3],
            },
            OracleTarget::VoidSurvival {
                below_y: -10.0,
                minimum_ticks: 2,
            },
            OracleTarget::UnexpectedLoad {
                allowed_destinations: vec![location("F_EXPECT", 1)],
            },
            OracleTarget::WrongWarp {
                expected: location("F_EXPECT", 1),
            },
            OracleTarget::ExcessiveMotion {
                max_displacement: Some(100.0),
                max_speed: Some(50.0),
            },
            OracleTarget::ImpossibleCoordinates { max_abs: 4096.0 },
        ];
        let program = SemanticOracleProgram {
            schema: SEMANTIC_ORACLE_SCHEMA_V1.into(),
            oracles: targets
                .into_iter()
                .enumerate()
                .map(|(index, target)| SemanticOracle {
                    name: format!("safety-{index}"),
                    polarity: OraclePolarity::Reached,
                    target,
                })
                .collect(),
        };
        let report = program
            .evaluate(&source, &SupplementalObservations::default())
            .unwrap();
        assert!(
            report
                .results
                .iter()
                .all(|result| result.disposition == OracleDisposition::Satisfied)
        );
        assert_eq!(
            report.results[0]
                .first_match
                .as_ref()
                .unwrap()
                .simulation_tick,
            2
        );
        assert_eq!(
            report.results[2]
                .first_match
                .as_ref()
                .unwrap()
                .simulation_tick,
            3
        );

        source.records[0].position[0] = f32::NAN;
        let nonfinite = SemanticOracleProgram {
            schema: SEMANTIC_ORACLE_SCHEMA_V1.into(),
            oracles: vec![SemanticOracle {
                name: "nan".into(),
                polarity: OraclePolarity::Reached,
                target: OracleTarget::NonFiniteState,
            }],
        }
        .evaluate(&source, &SupplementalObservations::default())
        .unwrap();
        assert_eq!(
            nonfinite.results[0].disposition,
            OracleDisposition::Satisfied
        );
    }

    #[test]
    fn run_outcome_oracles_retain_typed_failure_evidence() {
        let targets = vec![
            OracleTarget::ActorCorruption {
                actor_name: Some(77),
                field: Some("health".into()),
            },
            OracleTarget::SlotExhaustion,
            OracleTarget::WatchedFieldCorruption {
                field: Some("player.inventory.wallet".into()),
            },
            OracleTarget::HeapFailure {
                heap: Some("game".into()),
            },
            OracleTarget::Crash,
            OracleTarget::Softlock { minimum_ticks: 10 },
            OracleTarget::ControlLoss { minimum_ticks: 5 },
        ];
        let program = SemanticOracleProgram {
            schema: SEMANTIC_ORACLE_SCHEMA_V1.into(),
            oracles: targets
                .into_iter()
                .enumerate()
                .map(|(index, target)| SemanticOracle {
                    name: format!("run-failure-{index}"),
                    polarity: OraclePolarity::Reached,
                    target,
                })
                .collect(),
        };
        let outcome = RunOutcomeEvidence {
            schema: RUN_OUTCOME_SCHEMA_V1.into(),
            monitored: vec![
                RunEvidenceKind::ActorIntegrity,
                RunEvidenceKind::ActorSlots,
                RunEvidenceKind::WatchedFields,
                RunEvidenceKind::Heap,
                RunEvidenceKind::Progress,
                RunEvidenceKind::Control,
            ],
            termination: Some(RunTermination::Crashed {
                exit_code: None,
                signal: Some(11),
                reason: "segmentation fault".into(),
            }),
            anomalies: vec![
                RunAnomalyObservation::ActorCorruption {
                    simulation_tick: 10,
                    tape_frame: Some(9),
                    actor: RunActorIdentity {
                        process_id: Some(19),
                        actor_name: 77,
                        stage: Some("F_SP103".into()),
                        home_room: Some(1),
                        set_id: Some(4),
                    },
                    field: "health".into(),
                    expected: "4".into(),
                    actual: "-2147483648".into(),
                },
                RunAnomalyObservation::SlotExhaustion {
                    simulation_tick: 11,
                    tape_frame: Some(10),
                    active_slots: 256,
                    capacity: 256,
                    requested_actor_name: Some(80),
                },
                RunAnomalyObservation::WatchedFieldCorruption {
                    simulation_tick: 12,
                    tape_frame: Some(11),
                    field: "player.inventory.wallet".into(),
                    expected: "0..=999".into(),
                    actual: "65535".into(),
                },
                RunAnomalyObservation::HeapFailure {
                    simulation_tick: Some(13),
                    tape_frame: Some(12),
                    heap: "game".into(),
                    operation: "alloc".into(),
                    requested_bytes: 4096,
                    free_bytes: 1024,
                },
                RunAnomalyObservation::Softlock {
                    start_tick: 20,
                    end_tick: 29,
                    tape_frame: Some(28),
                    last_progress: "event 4 phase 2".into(),
                    reason: "simulation advanced without semantic progress".into(),
                },
                RunAnomalyObservation::ControlLoss {
                    start_tick: 30,
                    end_tick: 34,
                    tape_frame: Some(33),
                    procedure_id: Some(7),
                    reason: "input ownership stayed disabled".into(),
                },
            ],
        };
        let report = program
            .evaluate(
                &trace(true),
                &SupplementalObservations {
                    run_outcome: Some(outcome),
                    ..SupplementalObservations::default()
                },
            )
            .unwrap();
        assert!(
            report
                .results
                .iter()
                .all(|result| result.disposition == OracleDisposition::Satisfied)
        );
        assert!(matches!(
            report.results[0].first_match.as_ref().unwrap().facts,
            OracleFacts::ActorCorruption { .. }
        ));
        assert!(matches!(
            report.results[4].first_match.as_ref().unwrap().facts,
            OracleFacts::Crash {
                signal: Some(11),
                ..
            }
        ));
    }

    #[test]
    fn hang_and_avoided_run_failures_require_declared_coverage() {
        let reached_hang = SemanticOracleProgram {
            schema: SEMANTIC_ORACLE_SCHEMA_V1.into(),
            oracles: vec![SemanticOracle {
                name: "hung".into(),
                polarity: OraclePolarity::Reached,
                target: OracleTarget::Hang {
                    minimum_stalled_millis: 2_000,
                },
            }],
        };
        let timeout = RunOutcomeEvidence {
            schema: RUN_OUTCOME_SCHEMA_V1.into(),
            monitored: vec![RunEvidenceKind::Progress],
            termination: Some(RunTermination::TimedOut {
                wall_time_millis: 10_000,
                stalled_millis: 3_000,
                last_simulation_tick: 99,
            }),
            anomalies: vec![],
        };
        let report = reached_hang
            .evaluate(
                &trace(true),
                &SupplementalObservations {
                    run_outcome: Some(timeout),
                    ..SupplementalObservations::default()
                },
            )
            .unwrap();
        assert_eq!(report.results[0].disposition, OracleDisposition::Satisfied);
        assert_eq!(
            report.results[0]
                .first_match
                .as_ref()
                .unwrap()
                .simulation_tick,
            99
        );

        let avoided = SemanticOracleProgram {
            schema: SEMANTIC_ORACLE_SCHEMA_V1.into(),
            oracles: vec![
                SemanticOracle {
                    name: "no-crash".into(),
                    polarity: OraclePolarity::Avoided,
                    target: OracleTarget::Crash,
                },
                SemanticOracle {
                    name: "no-control-loss".into(),
                    polarity: OraclePolarity::Avoided,
                    target: OracleTarget::ControlLoss { minimum_ticks: 5 },
                },
            ],
        };
        let missing = avoided
            .evaluate(&trace(false), &SupplementalObservations::default())
            .unwrap();
        assert!(
            missing
                .results
                .iter()
                .all(|result| result.disposition == OracleDisposition::Indeterminate)
        );
        let clean = RunOutcomeEvidence {
            schema: RUN_OUTCOME_SCHEMA_V1.into(),
            monitored: vec![RunEvidenceKind::Control],
            termination: Some(RunTermination::Completed { exit_code: 0 }),
            anomalies: vec![],
        };
        let report = avoided
            .evaluate(
                &trace(true),
                &SupplementalObservations {
                    run_outcome: Some(clean),
                    ..SupplementalObservations::default()
                },
            )
            .unwrap();
        assert!(
            report
                .results
                .iter()
                .all(|result| result.disposition == OracleDisposition::Satisfied)
        );
    }

    #[test]
    fn progression_and_save_oracles_retain_semantic_source_facts() {
        let targets = vec![
            OracleTarget::DuplicateItemReward {
                grant_kind: Some(GrantKind::Reward),
                id: Some(42),
            },
            OracleTarget::PreservedStorageState {
                field: Some("carry.actor".into()),
            },
            OracleTarget::EventQueueing {
                event_id: Some(5),
                minimum_depth: 2,
            },
            OracleTarget::SequenceBreak {
                sequence: Some("forest-entry".into()),
            },
            OracleTarget::SaveStateAnomaly {
                slot: Some(1),
                field: Some("event_flags.42".into()),
            },
        ];
        let program = SemanticOracleProgram {
            schema: SEMANTIC_ORACLE_SCHEMA_V1.into(),
            oracles: targets
                .into_iter()
                .enumerate()
                .map(|(index, target)| SemanticOracle {
                    name: format!("progression-{index}"),
                    polarity: OraclePolarity::Reached,
                    target,
                })
                .collect(),
        };
        let outcome = RunOutcomeEvidence {
            schema: RUN_OUTCOME_SCHEMA_V1.into(),
            monitored: vec![
                RunEvidenceKind::InventoryRewards,
                RunEvidenceKind::Storage,
                RunEvidenceKind::EventQueue,
                RunEvidenceKind::Sequence,
                RunEvidenceKind::SaveState,
            ],
            termination: Some(RunTermination::Completed { exit_code: 0 }),
            anomalies: vec![
                RunAnomalyObservation::DuplicateItemReward {
                    simulation_tick: 50,
                    tape_frame: Some(49),
                    grant_kind: GrantKind::Reward,
                    id: 42,
                    first_source: "chest actor 100".into(),
                    duplicate_source: "event reward 9".into(),
                    total_grants: 2,
                },
                RunAnomalyObservation::PreservedStorageState {
                    simulation_tick: 51,
                    tape_frame: Some(50),
                    field: "carry.actor".into(),
                    expected_reset: "none".into(),
                    actual: "placed:F_SP103:1:17".into(),
                },
                RunAnomalyObservation::EventQueueing {
                    simulation_tick: 52,
                    tape_frame: Some(51),
                    running_event_id: Some(4),
                    queued_event_ids: vec![5, 9],
                },
                RunAnomalyObservation::SequenceBreak {
                    simulation_tick: 53,
                    tape_frame: Some(52),
                    sequence: "forest-entry".into(),
                    expected_step: "talk-to-ordona".into(),
                    actual_step: "enter-faron".into(),
                },
                RunAnomalyObservation::SaveStateAnomaly {
                    simulation_tick: Some(54),
                    tape_frame: Some(53),
                    slot: 1,
                    field: "event_flags.42".into(),
                    expected: "false".into(),
                    actual: "true".into(),
                },
            ],
        };
        let report = program
            .evaluate(
                &trace(true),
                &SupplementalObservations {
                    run_outcome: Some(outcome),
                    ..SupplementalObservations::default()
                },
            )
            .unwrap();
        assert!(
            report
                .results
                .iter()
                .all(|result| result.disposition == OracleDisposition::Satisfied)
        );
        assert!(matches!(
            report.results[4].first_match.as_ref().unwrap().facts,
            OracleFacts::SaveStateAnomaly {
                slot: 1,
                ref field,
                ..
            } if field == "event_flags.42"
        ));
    }
}
